// greentic/src/bin/gtc/deploy/bundle_upload_orchestrator.rs
//! Spawns `greentic-start warmup` and `greentic-deployer bundle-upload upload`
//! to bridge a local `.gtbundle` to a remote URL + digest pair consumable by
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

/// Detect whether a bundle file is already warmed by checking the filename
/// pattern `bundle-warmed-*.gtbundle`. Conservative: any other name triggers
/// a fresh warmup pass.
pub fn is_warmed(bundle_path: &Path) -> bool {
    bundle_path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|name| name.starts_with("bundle-warmed-"))
        .unwrap_or(false)
}

/// Spawn `greentic-start warmup --bundle <input> --output <out>` and return the warmed path.
pub fn warmup_bundle(input: &Path, out_dir: &Path) -> GtcResult<PathBuf> {
    let warmed = out_dir.join("bundle-warmed.gtbundle");
    let status = ProcessCommand::new("greentic-start")
        .arg("warmup")
        .arg("--bundle")
        .arg(input)
        .arg("--output")
        .arg(&warmed)
        .status()
        .map_err(|e| {
            GtcError::message(format!(
                "failed to spawn greentic-start warmup: {e}. Install greentic-start and ensure it is on PATH."
            ))
        })?;
    if !status.success() {
        return Err(GtcError::message(format!(
            "greentic-start warmup exited with status {:?}",
            status.code()
        )));
    }
    Ok(warmed)
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
    fn detects_warmed_bundle_filename() {
        assert!(is_warmed(Path::new(
            "bundle-warmed-0.5.18-keyed-1113.gtbundle"
        )));
        assert!(is_warmed(Path::new(
            "/some/path/bundle-warmed-foo.gtbundle"
        )));
    }

    #[test]
    fn detects_unwarmed_bundle_filename() {
        assert!(!is_warmed(Path::new("deep-research-demo-bundle.gtbundle")));
        assert!(!is_warmed(Path::new("bundle.gtbundle")));
    }
}
