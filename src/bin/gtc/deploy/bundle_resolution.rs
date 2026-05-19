use std::fs;
use std::io::{BufReader, Read, Write};
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
    let extracted = temp.path().join("bundle");
    fs::create_dir_all(&extracted)
        .map_err(|e| GtcError::io(format!("failed to create {}", extracted.display()), e))?;
    expand_bundle_archive(&archive_path, &extracted)?;
    let bundle_dir = detect_bundle_root(&extracted);
    Ok(StartBundleResolution {
        bundle_dir,
        deployment_key,
        deploy_artifact: Some(archive_path),
        _hold: Some(temp),
    })
}

pub(super) fn expand_bundle_archive(archive_path: &Path, extracted: &Path) -> GtcResult<()> {
    let data = fs::read(archive_path)
        .map_err(|e| GtcError::io(format!("failed to read {}", archive_path.display()), e))?;
    if looks_like_squashfs(&data) {
        return extract_squashfs_archive(archive_path, extracted);
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
    fs::copy(archive_path, &staged_path).map_err(|e| {
        GtcError::io(
            format!(
                "failed to stage bundle artifact {} -> {}",
                archive_path.display(),
                staged_path.display()
            ),
            e,
        )
    })?;

    expand_into_target(&staging, extracted).map_err(|e| GtcError::message(e.to_string()))
}

fn looks_like_squashfs(data: &[u8]) -> bool {
    data.starts_with(b"hsqs") || data.starts_with(b"sqsh")
}

fn extract_squashfs_archive(archive_path: &Path, extracted: &Path) -> GtcResult<()> {
    let file = fs::File::open(archive_path)
        .map_err(|e| GtcError::io(format!("failed to open {}", archive_path.display()), e))?;
    let reader = backhand::FilesystemReader::from_reader(BufReader::new(file)).map_err(|e| {
        GtcError::message(format!(
            "failed to read SquashFS bundle {}: {e}",
            archive_path.display()
        ))
    })?;

    fs::create_dir_all(extracted)
        .map_err(|e| GtcError::io(format!("failed to create {}", extracted.display()), e))?;

    for node in reader.files() {
        let path_str = node.fullpath.to_string_lossy();
        if path_str == "/" || path_str.is_empty() {
            continue;
        }
        if path_str.contains("..") {
            return Err(GtcError::message(format!(
                "invalid path in SquashFS bundle: {path_str}"
            )));
        }

        let relative_path = path_str.trim_start_matches('/');
        let out_path = extracted.join(relative_path);
        match &node.inner {
            backhand::InnerNode::Dir(_) => fs::create_dir_all(&out_path)
                .map_err(|e| GtcError::io(format!("failed to create {}", out_path.display()), e))?,
            backhand::InnerNode::File(file_reader) => {
                if let Some(parent) = out_path.parent() {
                    fs::create_dir_all(parent).map_err(|e| {
                        GtcError::io(format!("failed to create {}", parent.display()), e)
                    })?;
                }
                let mut out_file = fs::File::create(&out_path).map_err(|e| {
                    GtcError::io(format!("failed to create {}", out_path.display()), e)
                })?;
                let content = reader.file(file_reader);
                let mut decompressed = Vec::new();
                content
                    .reader()
                    .read_to_end(&mut decompressed)
                    .map_err(|e| {
                        GtcError::io(
                            format!("failed to decompress {}", node.fullpath.display()),
                            e,
                        )
                    })?;
                out_file.write_all(&decompressed).map_err(|e| {
                    GtcError::io(format!("failed to write {}", out_path.display()), e)
                })?;
                set_permissions_from_squashfs_header(&out_path, node.header.permissions)?;
            }
            backhand::InnerNode::Symlink(link) => {
                #[cfg(not(unix))]
                let _ = link;
                #[cfg(unix)]
                {
                    if let Some(parent) = out_path.parent() {
                        fs::create_dir_all(parent).map_err(|e| {
                            GtcError::io(format!("failed to create {}", parent.display()), e)
                        })?;
                    }
                    std::os::unix::fs::symlink(&link.link, &out_path).map_err(|e| {
                        GtcError::io(
                            format!("failed to create symlink {}", out_path.display()),
                            e,
                        )
                    })?;
                }
            }
            _ => {}
        }
    }

    Ok(())
}

#[cfg(unix)]
fn set_permissions_from_squashfs_header(path: &Path, permissions: u16) -> GtcResult<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(u32::from(permissions))).map_err(|e| {
        GtcError::io(
            format!("failed to set permissions on {}", path.display()),
            e,
        )
    })
}

#[cfg(not(unix))]
fn set_permissions_from_squashfs_header(_path: &Path, _permissions: u16) -> GtcResult<()> {
    Ok(())
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
        deployment_key_for_path, detect_bundle_root, download_https_bundle_to_tempfile,
        expand_bundle_archive, fingerprint_bundle_dir, looks_like_squashfs, map_registry_target,
        map_remote_bundle_ref, normalize_bundle_fingerprint, parse_local_bundle_ref,
        resolve_archive_bundle_path, resolve_bundle_reference, resolve_local_mutable_bundle_dir,
        sanitize_identifier, should_ignore_fingerprint_path,
    };
    use crate::tests::env_test_lock;
    use std::env;
    use std::fs;
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use std::path::{Path, PathBuf};
    use std::thread;

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
        assert!(should_ignore_fingerprint_path(Path::new(
            "state/pids/app.pid"
        )));
        assert!(should_ignore_fingerprint_path(Path::new(
            "state/runtime/cache"
        )));
        assert!(should_ignore_fingerprint_path(Path::new("state/runs/last")));
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
        assert_eq!(parse_local_bundle_ref("file://   "), None);
        assert_eq!(
            parse_local_bundle_ref("https://example.test/bundle.gtbundle"),
            None
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
    fn looks_like_squashfs_detects_squashfs_magic() {
        assert!(looks_like_squashfs(b"hsqs\x01\x00"));
        assert!(looks_like_squashfs(b"sqsh\x01\x00"));
        assert!(!looks_like_squashfs(b"PK\x03\x04"));
        assert!(!looks_like_squashfs(b""));
    }

    #[test]
    fn deployment_key_for_path_uses_path_when_canonicalization_fails() {
        let key = deployment_key_for_path(Path::new("./does/not/exist"));
        assert!(!key.is_empty());
        assert!(key.contains("does"));
    }

    #[test]
    fn normalize_bundle_fingerprint_keeps_paths_and_drops_noise() {
        let raw = [
            "",
            "dir:packs",
            "dir:.greentic/dev",
            "file:packs/demo.gtpack:42:abcdef",
            "file:logs/runtime.log:99:ignored",
            "file:state/runtime/cache.bin:1:ignored",
            "invalid:entry",
        ]
        .join("\n");

        assert_eq!(
            normalize_bundle_fingerprint(&raw),
            "dir:packs\nfile:packs/demo.gtpack:42"
        );
    }

    #[test]
    fn fingerprint_bundle_dir_includes_config_changes_and_ignores_runtime_noise() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = dir
            .path()
            .join("assets")
            .join("example-pack")
            .join("config");
        fs::create_dir_all(&config).expect("mkdir");
        fs::write(config.join("runtime.json"), "{}\n").expect("write");
        fs::create_dir_all(dir.path().join("logs")).expect("logs");
        fs::write(dir.path().join("logs").join("runtime.log"), "ignored\n").expect("log");

        let first = fingerprint_bundle_dir(dir.path()).expect("fingerprint");
        fs::write(config.join("runtime.json"), "{\"enabled\":true}\n").expect("rewrite");
        let second = fingerprint_bundle_dir(dir.path()).expect("fingerprint");

        assert_ne!(first, second);
        assert!(second.contains("assets/example-pack/config/runtime.json"));
        assert!(!second.contains("runtime.log"));
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
            map_registry_target("demo", Some("ghcr.io/base/".to_string())),
            Some("ghcr.io/base/demo".to_string())
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
        let mapped = map_remote_bundle_ref("repo://providers/demo").unwrap();
        unsafe {
            env::remove_var("GREENTIC_REPO_REGISTRY_BASE");
        }
        assert_eq!(mapped, "ghcr.io/greentic/repo/providers/demo");
    }

    #[test]
    fn map_remote_bundle_ref_requires_registry_base_for_unqualified_refs() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            env::remove_var("GREENTIC_REPO_REGISTRY_BASE");
            env::remove_var("GREENTIC_STORE_REGISTRY_BASE");
        }
        let err = map_remote_bundle_ref("repo://demo").unwrap_err();
        assert!(err.contains("GREENTIC_REPO_REGISTRY_BASE"));

        let err = map_remote_bundle_ref("store://demo").unwrap_err();
        assert!(err.contains("GREENTIC_STORE_REGISTRY_BASE"));
    }

    #[test]
    fn map_remote_bundle_ref_rejects_unknown_schemes() {
        let err = map_remote_bundle_ref("ftp://example.test/bundle").unwrap_err();
        assert!(err.contains("unsupported bundle scheme"));
    }

    #[test]
    fn detect_bundle_root_falls_back_to_extracted_root_when_unrecognized() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("nested");
        fs::create_dir_all(&nested).expect("mkdir");
        assert_eq!(detect_bundle_root(dir.path()), dir.path());
    }

    #[test]
    fn detect_bundle_root_accepts_direct_runtime_markers() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(
            dir.path().join("greentic.operator.yaml"),
            "operator: true\n",
        )
        .expect("write");
        assert_eq!(detect_bundle_root(dir.path()), dir.path());
    }

    #[test]
    fn detect_bundle_root_uses_single_nested_runtime_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("bundle");
        fs::create_dir_all(&nested).expect("mkdir");
        fs::write(nested.join("bundle.yaml"), "name: demo\n").expect("bundle");
        fs::write(nested.join("bundle-manifest.json"), "{}\n").expect("manifest");

        assert_eq!(detect_bundle_root(dir.path()), nested);
    }

    #[test]
    fn detect_bundle_root_falls_back_when_nested_root_is_ambiguous() {
        let dir = tempfile::tempdir().expect("tempdir");
        for name in ["one", "two"] {
            let nested = dir.path().join(name);
            fs::create_dir_all(&nested).expect("mkdir");
            fs::write(nested.join("greentic.demo.yaml"), "demo: true\n").expect("write");
        }

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
    fn resolve_local_mutable_bundle_dir_accepts_directory_file_url() {
        let dir = tempfile::tempdir().expect("tempdir");
        let reference = format!("file://{}", dir.path().display());
        assert_eq!(
            resolve_local_mutable_bundle_dir(&reference).expect("dir"),
            dir.path()
        );
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

    #[test]
    fn resolve_bundle_reference_downloads_http_gtbundle_archives() {
        let bytes = zipped_runtime_bundle_bytes();
        let (url, handle) = serve_http_once("bundle.gtbundle", bytes);

        let resolved = resolve_bundle_reference(&url, "en").expect("resolved");
        handle.join().expect("server thread");

        assert!(resolved.bundle_dir.join("greentic.demo.yaml").exists());
        assert!(resolved.deploy_artifact.as_deref().is_some_and(|path| {
            path.file_name().and_then(|name| name.to_str()) == Some("bundle.gtbundle")
        }));
    }

    #[test]
    fn download_https_bundle_to_tempfile_rejects_non_gtbundle_urls() {
        let (url, handle) = serve_http_once("bundle.txt", b"not a bundle".to_vec());

        let err = download_https_bundle_to_tempfile(&url, "en").unwrap_err();
        handle.join().expect("server thread");

        assert!(err.contains("must point to a .gtbundle archive"));
    }

    #[test]
    fn resolve_bundle_reference_rejects_empty_and_unknown_scheme_refs() {
        let err = resolve_bundle_reference("   ", "en").unwrap_err();
        assert!(err.contains("empty"));

        let err = resolve_bundle_reference("ftp://example.test/bundle.gtbundle", "en").unwrap_err();
        assert!(err.contains("unsupported bundle scheme"));
    }

    #[test]
    fn resolve_archive_bundle_path_rejects_non_file_artifacts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = resolve_archive_bundle_path(dir.path().to_path_buf(), "demo".to_string())
            .expect_err("directory is not an archive file");
        assert!(err.contains("bundle artifact is not a file"));
    }

    #[test]
    fn expand_bundle_archive_stages_non_squashfs_artifacts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let archive = dir.path().join("bundle.gtbundle");
        fs::write(&archive, b"bundle bytes").expect("write");
        let extracted = dir.path().join("extracted");
        fs::create_dir_all(&extracted).expect("mkdir");

        expand_bundle_archive(&archive, &extracted).expect("expanded");
        assert!(extracted.join("bundle.gtbundle").exists());
    }

    fn zipped_runtime_bundle_bytes() -> Vec<u8> {
        let cursor = std::io::Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("bundle/greentic.demo.yaml", options)
            .expect("start");
        zip.write_all(b"demo: true\n").expect("write");
        zip.finish().expect("finish").into_inner()
    }

    fn serve_http_once(file_name: &str, body: Vec<u8>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).expect("read request");
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(headers.as_bytes()).expect("headers");
            stream.write_all(&body).expect("body");
        });
        (format!("http://{addr}/{file_name}"), handle)
    }
}
