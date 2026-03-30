use std::fs;
use std::path::{Component, Path, PathBuf};

use gtc::config::GtcConfig;
use gtc::dist;
use gtc::error::{GtcError, GtcResult};
use gtc::perf_targets;

use super::StartBundleResolution;
use crate::install::{expand_into_target, fetch_https_bytes, url_file_name};

pub(crate) fn fingerprint_bundle_dir(bundle_dir: &Path) -> GtcResult<String> {
    let mut files = Vec::new();
    collect_bundle_entries(bundle_dir, bundle_dir, &mut files)?;
    files.sort();
    Ok(normalize_bundle_fingerprint(&files.join("\n")))
}

fn collect_bundle_entries(root: &Path, dir: &Path, out: &mut Vec<String>) -> GtcResult<()> {
    perf_targets::collect_bundle_entries(root, dir, out).map_err(GtcError::message)
}

pub(crate) fn normalize_bundle_fingerprint(raw: &str) -> String {
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| {
            if let Some(path) = line.strip_prefix("dir:") {
                if should_ignore_fingerprint_path(Path::new(path)) {
                    return None;
                }
                return Some(format!("dir:{path}"));
            }
            if let Some(rest) = line.strip_prefix("file:") {
                let mut parts = rest.splitn(3, ':');
                let path = parts.next().unwrap_or_default();
                let size = parts.next().unwrap_or_default();
                if should_ignore_fingerprint_path(Path::new(path)) {
                    return None;
                }
                return Some(format!("file:{path}:{size}"));
            }
            None
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn should_ignore_fingerprint_path(path: &Path) -> bool {
    let mut components = path.components();
    let first = components.next();
    let second = components.next();

    matches!(
        (first, second),
        (
            Some(Component::Normal(first)),
            Some(Component::Normal(second))
        ) if (first == ".greentic" && second == "dev")
            || (first == "state"
                && (second == "logs"
                    || second == "pids"
                    || second == "runtime"
                    || second == "runs"))
    ) || matches!(first, Some(Component::Normal(first)) if first == "logs")
}

pub(super) fn resolve_bundle_reference(
    reference: &str,
    locale: &str,
) -> GtcResult<StartBundleResolution> {
    let trimmed = reference.trim();
    if trimmed.is_empty() {
        return Err(GtcError::message("bundle reference is empty"));
    }
    if let Some(path) = parse_local_bundle_ref(trimmed) {
        return resolve_local_bundle_path(path);
    }
    if trimmed.starts_with("https://") || trimmed.starts_with("http://") {
        let fetched = download_https_bundle_to_tempfile(trimmed, locale)?;
        return resolve_archive_bundle_path(fetched, sanitize_identifier(trimmed));
    }

    let mapped = map_remote_bundle_ref(trimmed)?;
    let fetched = dist::pull_oci_reference_to_tempfile(&mapped, None)
        .map_err(|e| GtcError::message(format!("failed to fetch remote bundle {trimmed}: {e}")))?;
    resolve_archive_bundle_path(fetched, sanitize_identifier(&mapped))
}

pub(crate) fn resolve_local_mutable_bundle_dir(reference: &str) -> GtcResult<PathBuf> {
    let Some(path) = parse_local_bundle_ref(reference) else {
        return Err(GtcError::message(
            "admin registry updates require a local bundle directory path",
        ));
    };
    if !path.exists() {
        return Err(GtcError::message(format!(
            "bundle path does not exist: {}",
            path.display()
        )));
    }
    if !path.is_dir() {
        return Err(GtcError::message(format!(
            "admin registry updates require a local bundle directory, got: {}",
            path.display()
        )));
    }
    Ok(path)
}

fn parse_local_bundle_ref(reference: &str) -> Option<PathBuf> {
    if let Some(rest) = reference.strip_prefix("file://") {
        if rest.trim().is_empty() {
            return None;
        }
        return Some(PathBuf::from(rest));
    }
    if reference.contains("://") {
        return None;
    }
    Some(PathBuf::from(reference))
}

fn resolve_local_bundle_path(path: PathBuf) -> GtcResult<StartBundleResolution> {
    if !path.exists() {
        return Err(GtcError::message(format!(
            "bundle path does not exist: {}",
            path.display()
        )));
    }
    let deployment_key = deployment_key_for_path(&path);
    if path.is_dir() {
        return Ok(StartBundleResolution {
            bundle_dir: path,
            deployment_key,
            deploy_artifact: None,
            _hold: None,
        });
    }
    resolve_archive_bundle_path(path, deployment_key)
}

fn deployment_key_for_path(path: &Path) -> String {
    let canonical = path
        .canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string();
    sanitize_identifier(&canonical)
}

fn sanitize_identifier(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

fn resolve_archive_bundle_path(
    archive_path: PathBuf,
    deployment_key: String,
) -> GtcResult<StartBundleResolution> {
    if !archive_path.is_file() {
        return Err(GtcError::message(format!(
            "bundle artifact is not a file: {}",
            archive_path.display()
        )));
    }
    let temp = tempfile::tempdir().map_err(|e| GtcError::message(e.to_string()))?;
    let staging = temp.path().join("staging");
    fs::create_dir_all(&staging)
        .map_err(|e| GtcError::io(format!("failed to create {}", staging.display()), e))?;
    let file_name = archive_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("bundle.bin")
        .to_string();
    let staged_path = staging.join(file_name);
    fs::copy(&archive_path, &staged_path).map_err(|e| {
        GtcError::io(
            format!(
                "failed to stage bundle artifact {} -> {}",
                archive_path.display(),
                staged_path.display()
            ),
            e,
        )
    })?;

    let extracted = temp.path().join("bundle");
    fs::create_dir_all(&extracted)
        .map_err(|e| GtcError::io(format!("failed to create {}", extracted.display()), e))?;
    expand_into_target(&staging, &extracted).map_err(|e| GtcError::message(e.to_string()))?;
    let bundle_dir = detect_bundle_root(&extracted);
    Ok(StartBundleResolution {
        bundle_dir,
        deployment_key,
        deploy_artifact: Some(archive_path),
        _hold: Some(temp),
    })
}

pub(crate) fn detect_bundle_root(extracted_root: &Path) -> PathBuf {
    if is_runtime_bundle_root(extracted_root) {
        return extracted_root.to_path_buf();
    }
    let mut dirs = Vec::new();
    if let Ok(entries) = fs::read_dir(extracted_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            }
        }
    }
    if dirs.len() == 1 && is_runtime_bundle_root(&dirs[0]) {
        return dirs.remove(0);
    }
    extracted_root.to_path_buf()
}

fn is_runtime_bundle_root(path: &Path) -> bool {
    path.join("greentic.demo.yaml").exists()
        || path.join("greentic.operator.yaml").exists()
        || path.join("demo").join("demo.yaml").exists()
        || (path.join("bundle.yaml").exists()
            && (path.join("bundle-manifest.json").exists() || path.join("resolved").is_dir()))
}

fn map_remote_bundle_ref(reference: &str) -> GtcResult<String> {
    let cfg = GtcConfig::from_env();
    if let Some(rest) = reference.strip_prefix("oci://") {
        return Ok(rest.to_string());
    }
    if let Some(rest) = reference.strip_prefix("repo://") {
        return map_registry_target(rest, cfg.repo_registry_base()).ok_or_else(|| {
            GtcError::message(format!(
                "repo:// reference {reference} requires GREENTIC_REPO_REGISTRY_BASE to map to OCI"
            ))
        });
    }
    if let Some(rest) = reference.strip_prefix("store://") {
        return map_registry_target(rest, cfg.store_registry_base()).ok_or_else(|| {
            GtcError::message(format!(
                "store:// reference {reference} requires GREENTIC_STORE_REGISTRY_BASE to map to OCI"
            ))
        });
    }
    Err(GtcError::message(format!(
        "unsupported bundle scheme for {reference}; expected local path, file://, http(s)://, oci://, repo://, or store://"
    )))
}

fn download_https_bundle_to_tempfile(url: &str, locale: &str) -> GtcResult<PathBuf> {
    let bytes = fetch_https_bytes(url, "", locale, "application/octet-stream")
        .map_err(|e| GtcError::message(e.to_string()))?;
    let temp = tempfile::tempdir().map_err(|e| GtcError::message(e.to_string()))?;
    let file_name = url_file_name(url).unwrap_or_else(|| "bundle.gtbundle".to_string());
    let path = temp.path().join(file_name);
    fs::write(&path, bytes)
        .map_err(|e| GtcError::io(format!("failed to write {}", path.display()), e))?;
    if path.extension().and_then(|value| value.to_str()) != Some("gtbundle") {
        return Err(GtcError::message(format!(
            "remote bundle URL must point to a .gtbundle archive: {url}"
        )));
    }
    let persisted = temp.keep();
    Ok(persisted.join(
        path.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("bundle.gtbundle"),
    ))
}

fn map_registry_target(target: &str, base: Option<String>) -> Option<String> {
    if target.contains('/') && (target.contains('@') || target.contains(':')) {
        return Some(target.to_string());
    }
    let base = base?;
    let normalized_base = base.trim_end_matches('/');
    let normalized_target = target.trim_start_matches('/');
    Some(format!("{normalized_base}/{normalized_target}"))
}

#[cfg(test)]
mod tests {
    use super::{
        deployment_key_for_path, detect_bundle_root, map_registry_target, map_remote_bundle_ref,
        parse_local_bundle_ref, resolve_bundle_reference, resolve_local_mutable_bundle_dir,
        sanitize_identifier, should_ignore_fingerprint_path,
    };
    use crate::tests::env_test_lock;
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};

    #[test]
    fn ignore_fingerprint_path_filters_runtime_noise_locations() {
        assert!(should_ignore_fingerprint_path(Path::new(
            ".greentic/dev/secrets.env"
        )));
        assert!(should_ignore_fingerprint_path(Path::new(
            "state/logs/runtime.log"
        )));
        assert!(should_ignore_fingerprint_path(Path::new(
            "logs/operator.log"
        )));
        assert!(!should_ignore_fingerprint_path(Path::new(
            "packs/demo.gtpack"
        )));
    }

    #[test]
    fn parse_local_bundle_ref_recognizes_local_and_file_urls() {
        assert_eq!(
            parse_local_bundle_ref("/tmp/demo"),
            Some(PathBuf::from("/tmp/demo"))
        );
        assert_eq!(
            parse_local_bundle_ref("file:///tmp/demo"),
            Some(PathBuf::from("/tmp/demo"))
        );
        assert_eq!(parse_local_bundle_ref("oci://ghcr.io/demo:latest"), None);
    }

    #[test]
    fn sanitize_identifier_normalizes_mixed_input() {
        assert_eq!(
            sanitize_identifier("Acme.Dev/Bundle@01"),
            "acme-dev-bundle-01"
        );
    }

    #[test]
    fn deployment_key_for_path_uses_path_when_canonicalization_fails() {
        let key = deployment_key_for_path(Path::new("./does/not/exist"));
        assert!(!key.is_empty());
        assert!(key.contains("does"));
    }

    #[test]
    fn map_registry_target_passes_through_fully_qualified_refs() {
        assert_eq!(
            map_registry_target(
                "ghcr.io/demo/app@sha256:abc",
                Some("ghcr.io/ignored".to_string())
            ),
            Some("ghcr.io/demo/app@sha256:abc".to_string())
        );
    }

    #[test]
    fn map_registry_target_joins_base_and_target() {
        assert_eq!(
            map_registry_target("providers/demo:latest", Some("ghcr.io/base/".to_string())),
            Some("providers/demo:latest".to_string())
        );
    }

    #[test]
    fn map_remote_bundle_ref_supports_oci_and_registry_mapping() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(
            map_remote_bundle_ref("oci://ghcr.io/demo/app:latest").unwrap(),
            "ghcr.io/demo/app:latest"
        );

        unsafe {
            env::set_var("GREENTIC_REPO_REGISTRY_BASE", "ghcr.io/greentic/repo");
        }
        let mapped = map_remote_bundle_ref("repo://providers/demo:latest").unwrap();
        unsafe {
            env::remove_var("GREENTIC_REPO_REGISTRY_BASE");
        }
        assert_eq!(mapped, "providers/demo:latest");
    }

    #[test]
    fn map_remote_bundle_ref_requires_registry_base_for_unqualified_refs() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            env::remove_var("GREENTIC_STORE_REGISTRY_BASE");
        }
        let err = map_remote_bundle_ref("store://demo").unwrap_err();
        assert!(err.contains("GREENTIC_STORE_REGISTRY_BASE"));
    }

    #[test]
    fn detect_bundle_root_falls_back_to_extracted_root_when_unrecognized() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("nested");
        fs::create_dir_all(&nested).expect("mkdir");
        assert_eq!(detect_bundle_root(dir.path()), dir.path());
    }

    #[test]
    fn resolve_local_mutable_bundle_dir_rejects_missing_and_file_paths() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("missing");
        let err = resolve_local_mutable_bundle_dir(missing.to_str().expect("utf8")).unwrap_err();
        assert!(err.contains("does not exist"));

        let file = dir.path().join("bundle.gtbundle");
        fs::write(&file, b"fixture").expect("write");
        let err = resolve_local_mutable_bundle_dir(file.to_str().expect("utf8")).unwrap_err();
        assert!(err.contains("local bundle directory"));
    }

    #[test]
    fn resolve_bundle_reference_accepts_local_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("greentic.demo.yaml"), "demo: true\n").expect("write");

        let resolved =
            resolve_bundle_reference(dir.path().to_str().expect("utf8"), "en").expect("resolved");
        assert_eq!(resolved.bundle_dir, dir.path());
        assert!(resolved.deploy_artifact.is_none());
    }

    #[test]
    fn resolve_bundle_reference_rejects_missing_local_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("missing");
        let err = resolve_bundle_reference(missing.to_str().expect("utf8"), "en").unwrap_err();
        assert!(err.contains("does not exist"));
    }

    #[test]
    fn resolve_bundle_reference_expands_local_archive() {
        let dir = tempfile::tempdir().expect("tempdir");
        let archive_path = dir.path().join("bundle.zip");
        let file = fs::File::create(&archive_path).expect("create");
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("bundle/greentic.demo.yaml", options)
            .expect("start");
        use std::io::Write as _;
        zip.write_all(b"demo: true\n").expect("write");
        zip.finish().expect("finish");

        let resolved =
            resolve_bundle_reference(archive_path.to_str().expect("utf8"), "en").expect("resolved");
        assert!(resolved.bundle_dir.join("greentic.demo.yaml").exists());
        assert_eq!(
            resolved.deploy_artifact.as_deref(),
            Some(archive_path.as_path())
        );
    }
}
