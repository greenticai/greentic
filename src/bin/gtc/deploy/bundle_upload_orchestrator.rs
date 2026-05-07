// greentic/src/bin/gtc/deploy/bundle_upload_orchestrator.rs
//! Spawns `greentic-start warmup`, `greentic-setup bundle build`, and
//! `greentic-deployer bundle-upload upload` to bridge a local bundle directory
//! to a remote URL + digest pair consumable by the existing deploy flow.

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

/// Locations where an answers file might live, relative to the bundle directory.
/// Order = priority. First match wins.
const ANSWERS_FILE_CANDIDATES: &[&str] = &["answers.json", "setup-answers.json"];

/// Return the first candidate answers file that exists relative to `bundle_dir`,
/// or sibling to `bundle_dir` (e.g. `<bundle-dir-name>-answer.json` in the parent).
pub fn detect_answers_file(bundle_dir: &Path) -> Option<PathBuf> {
    for candidate in ANSWERS_FILE_CANDIDATES {
        let path = bundle_dir.join(candidate);
        if path.exists() {
            return Some(path);
        }
    }
    // Sibling convention: <bundle-dir>/../<basename>-answer.json
    if let (Some(parent), Some(name)) = (
        bundle_dir.parent(),
        bundle_dir.file_name().and_then(|n| n.to_str()),
    ) {
        let sibling = parent.join(format!("{name}-answer.json"));
        if sibling.exists() {
            return Some(sibling);
        }
    }
    None
}

/// Run `greentic-start warmup --bundle <bundle_dir>` to populate the cache.
/// Idempotent — re-running with already-warmed cache is fast (~seconds).
fn run_warmup(bundle_dir: &Path) -> GtcResult<()> {
    let status = ProcessCommand::new("greentic-start")
        .arg("warmup")
        .arg("--bundle")
        .arg(bundle_dir)
        .status()
        .map_err(|e| {
            GtcError::message(format!(
                "failed to spawn greentic-start warmup: {e}. Install greentic-start (>= 0.5.18) and ensure it is on PATH."
            ))
        })?;
    if !status.success() {
        return Err(GtcError::message(format!(
            "greentic-start warmup exited with status {:?}",
            status.code()
        )));
    }
    Ok(())
}

/// Run `greentic-setup bundle build --bundle <bundle_dir> --out <out_dir> --non-interactive --no-ui [--answers <file>]`
/// to rebuild the `.gtbundle`, embedding the warmed cache. Returns the path to the produced `.gtbundle`.
fn run_bundle_build(
    bundle_dir: &Path,
    out_dir: &Path,
    answers: Option<&Path>,
) -> GtcResult<PathBuf> {
    let mut cmd = ProcessCommand::new("greentic-setup");
    cmd.arg("bundle")
        .arg("build")
        .arg("--bundle")
        .arg(bundle_dir)
        .arg("--out")
        .arg(out_dir)
        .arg("--non-interactive")
        .arg("--no-ui");
    if let Some(answers_path) = answers {
        cmd.arg("--answers").arg(answers_path);
    }
    let status = cmd.status().map_err(|e| {
        GtcError::message(format!(
            "failed to spawn greentic-setup bundle build: {e}. Install greentic-setup and ensure it is on PATH."
        ))
    })?;
    if !status.success() {
        return Err(GtcError::message(format!(
            "greentic-setup bundle build exited with status {:?}. \
             If the bundle uses baked secrets, ensure an answers file exists at {}/answers.json or pass --upload-bundle-answers (TODO).",
            status.code(),
            bundle_dir.display(),
        )));
    }

    // Find the produced .gtbundle. greentic-setup bundle build writes a portable
    // bundle directory to <out>; the packaged .gtbundle lands at <out>/dist/*.gtbundle.
    // Fall back to <out>/*.gtbundle for forward-compat with formats that flatten output.
    let candidates = [out_dir.join("dist"), out_dir.to_path_buf()];
    let mut found = None;
    for dir in &candidates {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("gtbundle") {
                found = Some(path);
                break;
            }
        }
        if found.is_some() {
            break;
        }
    }
    found.ok_or_else(|| {
        GtcError::message(format!(
            "greentic-setup bundle build produced no .gtbundle in {}",
            out_dir.display()
        ))
    })
}

/// Run the full warmup → rebuild chain.
/// `bundle_dir` is a bundle directory (containing `bundle.yaml`).
/// Returns the path to a freshly built warmed `.gtbundle` in a temp directory.
pub fn prepare_warmed_bundle(bundle_dir: &Path) -> GtcResult<PathBuf> {
    if !bundle_dir.is_dir() {
        return Err(GtcError::message(format!(
            "--upload-bundle requires a bundle DIRECTORY (containing bundle.yaml); got: {}",
            bundle_dir.display()
        )));
    }
    eprintln!(
        "Warming bundle cache: greentic-start warmup --bundle {}",
        bundle_dir.display()
    );
    run_warmup(bundle_dir)?;

    let unique = format!(
        "gtc-bundle-build-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    );
    let out_dir = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&out_dir).map_err(|e| {
        GtcError::message(format!("create build out dir {}: {e}", out_dir.display()))
    })?;

    let answers = detect_answers_file(bundle_dir);
    if let Some(a) = answers.as_ref() {
        eprintln!("Using answers file: {}", a.display());
    }
    eprintln!(
        "Rebuilding bundle with cache embedded: greentic-setup bundle build --bundle {} --out {}",
        bundle_dir.display(),
        out_dir.display()
    );
    run_bundle_build(bundle_dir, &out_dir, answers.as_deref())
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
    fn detect_answers_finds_answers_json_in_bundle_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let bundle_dir = tmp.path().join("my-bundle");
        std::fs::create_dir_all(&bundle_dir).unwrap();
        let answers = bundle_dir.join("answers.json");
        std::fs::write(&answers, b"{}").unwrap();
        assert_eq!(detect_answers_file(&bundle_dir), Some(answers));
    }

    #[test]
    fn detect_answers_finds_setup_answers_json() {
        let tmp = tempfile::TempDir::new().unwrap();
        let bundle_dir = tmp.path().join("my-bundle");
        std::fs::create_dir_all(&bundle_dir).unwrap();
        let answers = bundle_dir.join("setup-answers.json");
        std::fs::write(&answers, b"{}").unwrap();
        assert_eq!(detect_answers_file(&bundle_dir), Some(answers));
    }

    #[test]
    fn detect_answers_finds_sibling_named_answer_json() {
        let tmp = tempfile::TempDir::new().unwrap();
        let bundle_dir = tmp.path().join("my-bundle");
        std::fs::create_dir_all(&bundle_dir).unwrap();
        let sibling = tmp.path().join("my-bundle-answer.json");
        std::fs::write(&sibling, b"{}").unwrap();
        assert_eq!(detect_answers_file(&bundle_dir), Some(sibling));
    }

    #[test]
    fn detect_answers_returns_none_when_missing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let bundle_dir = tmp.path().join("my-bundle");
        std::fs::create_dir_all(&bundle_dir).unwrap();
        assert!(detect_answers_file(&bundle_dir).is_none());
    }

    #[test]
    fn detect_answers_prefers_answers_json_over_setup_answers() {
        let tmp = tempfile::TempDir::new().unwrap();
        let bundle_dir = tmp.path().join("my-bundle");
        std::fs::create_dir_all(&bundle_dir).unwrap();
        std::fs::write(bundle_dir.join("answers.json"), b"{}").unwrap();
        std::fs::write(bundle_dir.join("setup-answers.json"), b"{}").unwrap();
        let detected = detect_answers_file(&bundle_dir).unwrap();
        assert!(detected.ends_with("answers.json"));
        assert!(!detected.to_string_lossy().contains("setup-answers"));
    }
}
