// greentic/src/bin/gtc/deploy/bundle_upload_orchestrator.rs
//! Spawns `greentic-bundle build --warmup` and `greentic-deployer bundle-upload upload`
//! to bridge a local bundle directory to a remote URL + digest pair consumable by
//! the existing deploy flow.

use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use serde::Deserialize;

use gtc::error::{GtcError, GtcResult};

#[derive(Debug, Clone, Deserialize)]
pub struct UploadedBundle {
    pub url: String,
    pub digest: String,
    pub expires_at: Option<String>,
    pub object_ref: String,
}

/// Run `greentic-bundle build --root <bundle_dir> --output <file> --warmup`.
/// `--warmup` triggers an internal `greentic-start warmup` pass, embedding
/// `.cache/v1/<engine_profile_id>/...` into the produced .gtbundle so the
/// operator's `adopt_bundle_cache_dir` picks it up on cold start.
fn run_bundle_build(bundle_dir: &Path, output_file: &Path) -> GtcResult<()> {
    let status = ProcessCommand::new("greentic-bundle")
        .arg("build")
        .arg("--root")
        .arg(bundle_dir)
        .arg("--output")
        .arg(output_file)
        .arg("--warmup")
        .status()
        .map_err(|e| {
            GtcError::message(format!(
                "failed to spawn greentic-bundle build: {e}. Install greentic-bundle (>= 0.5.7) and greentic-start (>= 0.5.18) on PATH."
            ))
        })?;
    if !status.success() {
        return Err(GtcError::message(format!(
            "greentic-bundle build --warmup exited with status {:?}",
            status.code()
        )));
    }
    if !output_file.is_file() {
        return Err(GtcError::message(format!(
            "greentic-bundle build reported success but produced no .gtbundle at {}",
            output_file.display()
        )));
    }
    Ok(())
}

/// Build a warmed `.gtbundle` from the source bundle directory and return its path
/// in a temp location. `bundle_dir` must contain `bundle.yaml`.
pub fn prepare_warmed_bundle(bundle_dir: &Path) -> GtcResult<PathBuf> {
    if !bundle_dir.is_dir() {
        return Err(GtcError::message(format!(
            "--upload-bundle requires a bundle DIRECTORY (containing bundle.yaml); got: {}",
            bundle_dir.display()
        )));
    }

    let unique = format!(
        "gtc-bundle-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    );
    let out_dir = std::env::temp_dir().join(&unique);
    std::fs::create_dir_all(&out_dir).map_err(|e| {
        GtcError::message(format!("create build out dir {}: {e}", out_dir.display()))
    })?;

    let bundle_name = bundle_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("bundle");
    let output_file = out_dir.join(format!("{bundle_name}.gtbundle"));

    eprintln!(
        "Building warmed bundle: greentic-bundle build --root {} --output {} --warmup",
        bundle_dir.display(),
        output_file.display()
    );
    run_bundle_build(bundle_dir, &output_file)?;
    Ok(output_file)
}

/// Spawn `greentic-deployer bundle-upload upload --target <url> --bundle <path> --presign-expires <secs>`
/// and parse JSON stdout into `UploadedBundle`.
pub fn upload_bundle(
    target: &str,
    bundle: &Path,
    presign_expires: u64,
) -> GtcResult<UploadedBundle> {
    let output = ProcessCommand::new("greentic-deployer")
        .arg("bundle-upload")
        .arg("upload")
        .arg("--target")
        .arg(target)
        .arg("--bundle")
        .arg(bundle)
        .arg("--presign-expires")
        .arg(presign_expires.to_string())
        .output()
        .map_err(|e| {
            GtcError::message(format!(
                "failed to spawn greentic-deployer bundle-upload: {e}. Install with `cargo install greentic-deployer`."
            ))
        })?;
    if !output.status.success() {
        return Err(GtcError::message(format!(
            "greentic-deployer bundle-upload upload failed (exit {:?}): {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    serde_json::from_slice::<UploadedBundle>(&output.stdout).map_err(|e| {
        GtcError::message(format!(
            "invalid JSON from greentic-deployer bundle-upload: {e}"
        ))
    })
}

/// Spawn `greentic-deployer bundle-upload refresh-url --object-ref <ref> --presign-expires <secs>`
/// and parse JSON stdout into `UploadedBundle`.
pub fn refresh_bundle_url(object_ref: &str, presign_expires: u64) -> GtcResult<UploadedBundle> {
    let output = ProcessCommand::new("greentic-deployer")
        .arg("bundle-upload")
        .arg("refresh-url")
        .arg("--object-ref")
        .arg(object_ref)
        .arg("--presign-expires")
        .arg(presign_expires.to_string())
        .output()
        .map_err(|e| {
            GtcError::message(format!(
                "failed to spawn greentic-deployer bundle-upload refresh-url: {e}"
            ))
        })?;
    if !output.status.success() {
        return Err(GtcError::message(format!(
            "greentic-deployer bundle-upload refresh-url failed (exit {:?}): {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    serde_json::from_slice::<UploadedBundle>(&output.stdout)
        .map_err(|e| GtcError::message(format!("invalid JSON from refresh-url: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepare_warmed_bundle_rejects_non_directory() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let err = prepare_warmed_bundle(tmp.path()).unwrap_err();
        let msg = format!("{err:?}");
        assert!(
            msg.contains("requires a bundle DIRECTORY"),
            "expected directory error, got: {msg}"
        );
    }
}
