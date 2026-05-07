// greentic/src/bin/gtc/deploy/bundle_upload_orchestrator.rs
//! Spawns `greentic-bundle build --warmup` and `greentic-deployer bundle-upload upload`
//! to bridge a local bundle directory to a remote URL + digest pair consumable by
//! the existing deploy flow.

use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use serde::Deserialize;

use gtc::error::{GtcError, GtcResult};

use crate::DEPLOYER_BIN;
use crate::process::resolve_companion_command;

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
    let deployer_bin = resolve_companion_command(DEPLOYER_BIN);
    let output = ProcessCommand::new(&deployer_bin)
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
    let deployer_bin = resolve_companion_command(DEPLOYER_BIN);
    let output = ProcessCommand::new(&deployer_bin)
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
    use crate::tests::env_test_lock;
    use std::env;
    use std::fs;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

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

    #[cfg(unix)]
    fn write_fake_bundle_upload_script(script: &Path, log_path: &Path) {
        let body = format!(
            r#"#!/bin/sh
printf '%s\n' "$*" >> "{log}"
if [ "$1" = "bundle-upload" ] && [ "$2" = "upload" ]; then
  printf '%s\n' '{{"url":"https://example.test/uploaded.gtbundle","digest":"sha256:abc","expires_at":"2026-05-14T00:00:00Z","object_ref":"s3://bucket/key"}}'
  exit 0
fi
if [ "$1" = "bundle-upload" ] && [ "$2" = "refresh-url" ]; then
  printf '%s\n' '{{"url":"https://example.test/refreshed.gtbundle","digest":"sha256:def","expires_at":"2026-05-15T00:00:00Z","object_ref":"s3://bucket/key"}}'
  exit 0
fi
echo "unexpected args: $*" >&2
exit 1
"#,
            log = log_path.display()
        );
        fs::write(script, body).expect("write fake bundle upload script");
        let mut perms = fs::metadata(script).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(script, perms).expect("chmod");
    }

    #[cfg(unix)]
    #[test]
    fn upload_bundle_respects_greentic_deployer_bin_override() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let script = dir.path().join("greentic-deployer");
        let log_path = dir.path().join("upload.log");
        let bundle = dir.path().join("demo.gtbundle");
        write_fake_bundle_upload_script(&script, &log_path);
        fs::write(&bundle, b"bundle").expect("bundle file");

        let original = env::var_os("GREENTIC_DEPLOYER_BIN");
        unsafe {
            env::set_var("GREENTIC_DEPLOYER_BIN", &script);
        }

        let result = upload_bundle("s3://bucket/prefix", &bundle, 123).expect("upload");

        unsafe {
            match original {
                Some(value) => env::set_var("GREENTIC_DEPLOYER_BIN", value),
                None => env::remove_var("GREENTIC_DEPLOYER_BIN"),
            }
        }

        assert_eq!(result.digest, "sha256:abc");
        let logged = fs::read_to_string(&log_path).expect("read log");
        assert!(logged.contains("bundle-upload upload"));
        assert!(logged.contains("--target s3://bucket/prefix"));
        assert!(logged.contains("--bundle"));
        assert!(logged.contains("--presign-expires 123"));
    }

    #[cfg(unix)]
    #[test]
    fn refresh_bundle_url_respects_greentic_deployer_bin_override() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let script = dir.path().join("greentic-deployer");
        let log_path = dir.path().join("refresh.log");
        write_fake_bundle_upload_script(&script, &log_path);

        let original = env::var_os("GREENTIC_DEPLOYER_BIN");
        unsafe {
            env::set_var("GREENTIC_DEPLOYER_BIN", &script);
        }

        let result = refresh_bundle_url("s3://bucket/key", 456).expect("refresh");

        unsafe {
            match original {
                Some(value) => env::set_var("GREENTIC_DEPLOYER_BIN", value),
                None => env::remove_var("GREENTIC_DEPLOYER_BIN"),
            }
        }

        assert_eq!(result.digest, "sha256:def");
        let logged = fs::read_to_string(&log_path).expect("read log");
        assert!(logged.contains("bundle-upload refresh-url"));
        assert!(logged.contains("--object-ref s3://bucket/key"));
        assert!(logged.contains("--presign-expires 456"));
    }
}
