use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command as ProcessCommand;

use gtc::error::{GtcError, GtcResult};
use gtc::perf_targets;
use tempfile::TempDir;

use super::bundle_resolution::{detect_bundle_root, expand_bundle_archive};
use super::{StartBundleResolution, StartTarget};
use crate::BUNDLE_BIN;
use crate::process::resolve_companion_command;

#[derive(Debug)]
pub(crate) struct PreparedBundle {
    pub(crate) input_ref: String,
    pub(crate) prepared_root: PathBuf,
    pub(crate) artifact_path: PathBuf,
    pub(crate) digest: String,
    pub(crate) source_kind: PreparedBundleSourceKind,
    pub(crate) was_rebuilt: bool,
    pub(crate) included_asset_config_count: usize,
    pub(crate) _hold: TempDir,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreparedBundleSourceKind {
    LocalDirectory,
    ResolvedArchive,
}

impl PreparedBundleSourceKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            PreparedBundleSourceKind::LocalDirectory => "local-directory",
            PreparedBundleSourceKind::ResolvedArchive => "resolved-archive",
        }
    }
}

pub(crate) fn prepare_bundle_for_start(
    input_ref: &str,
    resolved: &StartBundleResolution,
    debug: bool,
    locale: &str,
) -> GtcResult<PreparedBundle> {
    if !resolved.bundle_dir.is_dir() {
        return Err(GtcError::message(format!(
            "prepared bundle requires a resolved bundle directory, got {}",
            resolved.bundle_dir.display()
        )));
    }

    let hold = tempfile::tempdir()
        .map_err(|err| GtcError::io("failed to create prepared bundle tempdir", err))?;
    let artifact_path = hold
        .path()
        .join(format!("{}.gtbundle", resolved.deployment_key));
    run_warmed_bundle_build(&resolved.bundle_dir, &artifact_path, debug, locale)?;

    let extracted = hold.path().join("prepared-root");
    fs::create_dir_all(&extracted).map_err(|err| {
        GtcError::io(
            format!("failed to create prepared root {}", extracted.display()),
            err,
        )
    })?;
    expand_bundle_archive(&artifact_path, &extracted)?;
    let prepared_root = detect_bundle_root(&extracted);
    overlay_workspace_files(&resolved.bundle_dir, &prepared_root)?;
    scrub_prepared_artifact_root(&prepared_root)?;
    write_zip_bundle_artifact(&prepared_root, &artifact_path)?;
    let digest = perf_targets::sha256_file(&artifact_path).map_err(GtcError::message)?;
    let included_asset_config_count = count_included_asset_config_files(&prepared_root)?;

    Ok(PreparedBundle {
        input_ref: input_ref.to_string(),
        prepared_root,
        artifact_path,
        digest,
        source_kind: if resolved.deploy_artifact.is_some() {
            PreparedBundleSourceKind::ResolvedArchive
        } else {
            PreparedBundleSourceKind::LocalDirectory
        },
        was_rebuilt: true,
        included_asset_config_count,
        _hold: hold,
    })
}

pub(crate) fn print_prepared_bundle_debug(
    prepared: &PreparedBundle,
    target: StartTarget,
    deployer_bundle_source: Option<&str>,
    deployer_bundle_digest: Option<&str>,
) {
    println!("Prepared bundle:");
    println!("  input ref: {}", prepared.input_ref);
    println!("  root: {}", prepared.prepared_root.display());
    println!("  artifact: {}", prepared.artifact_path.display());
    println!("  digest: {}", prepared.digest);
    println!("  source kind: {}", prepared.source_kind.as_str());
    println!("  rebuilt: {}", prepared.was_rebuilt);
    println!("  target: {}", target.as_str());
    println!(
        "  included asset/config files: {}",
        prepared.included_asset_config_count
    );
    if let Some(source) = deployer_bundle_source {
        println!("  deployer bundle source: {source}");
    }
    if let Some(digest) = deployer_bundle_digest {
        println!("  deployer bundle digest: {digest}");
    }
}

fn run_warmed_bundle_build(
    bundle_dir: &Path,
    output_file: &Path,
    debug: bool,
    _locale: &str,
) -> GtcResult<()> {
    let bundle_bin = resolve_companion_command(BUNDLE_BIN);
    let args = [
        "build".to_string(),
        "--root".to_string(),
        bundle_dir.display().to_string(),
        "--output".to_string(),
        output_file.display().to_string(),
        "--warmup".to_string(),
    ];
    if debug {
        eprintln!("exec {} {:?}", bundle_bin.display(), args);
    }
    let status = ProcessCommand::new(&bundle_bin)
        .args(&args)
        .status()
        .map_err(|err| {
            GtcError::io(
                "failed to execute greentic-bundle for prepared bundle warmup",
                err,
            )
        })?;
    if !status.success() {
        return Err(GtcError::message(format!(
            "greentic-bundle build --warmup exited with status {}",
            status.code().unwrap_or(1)
        )));
    }
    if !output_file.is_file() {
        return Err(GtcError::message(format!(
            "greentic-bundle build --warmup reported success but produced no artifact at {}",
            output_file.display()
        )));
    }
    Ok(())
}

fn count_included_asset_config_files(root: &Path) -> GtcResult<usize> {
    let mut count = 0usize;
    count_asset_config_files(root, root, &mut count)?;
    Ok(count)
}

fn overlay_workspace_files(source_root: &Path, prepared_root: &Path) -> GtcResult<()> {
    overlay_workspace_dir(source_root, source_root, prepared_root)
}

fn overlay_workspace_dir(source_root: &Path, dir: &Path, prepared_root: &Path) -> GtcResult<()> {
    for entry in fs::read_dir(dir)
        .map_err(|err| GtcError::io(format!("failed to read {}", dir.display()), err))?
    {
        let entry = entry.map_err(|err| GtcError::message(err.to_string()))?;
        let path = entry.path();
        let relative = path
            .strip_prefix(source_root)
            .map_err(|err| GtcError::message(err.to_string()))?;
        if should_skip_source_overlay_path(relative) {
            continue;
        }
        let file_type = entry
            .file_type()
            .map_err(|err| GtcError::io(format!("failed to stat {}", path.display()), err))?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            overlay_workspace_dir(source_root, &path, prepared_root)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let target = prepared_root.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                GtcError::io(format!("failed to create {}", parent.display()), err)
            })?;
        }
        fs::copy(&path, &target).map_err(|err| {
            GtcError::io(
                format!(
                    "failed to overlay bundle workspace file {} -> {}",
                    path.display(),
                    target.display()
                ),
                err,
            )
        })?;
    }
    Ok(())
}

fn should_skip_source_overlay_path(path: &Path) -> bool {
    let mut components = path.components();
    let first = components.next();
    let second = components.next();

    if matches!(first, Some(Component::Normal(value)) if value == ".git"
        || value == ".cache"
        || value == "logs"
        || value == "state"
        || value == "target")
    {
        return true;
    }
    if matches!(
        (first, second),
        (
            Some(Component::Normal(first)),
            Some(Component::Normal(second))
        ) if first == ".greentic"
            && (second == "cache" || second == "dev" || second == "local")
    ) {
        return true;
    }
    path.components().any(|component| {
        matches!(component, Component::Normal(value) if value == ".dev.secrets.env"
            || value == ".env"
            || value == "tmp"
            || value == "temp")
    })
}

fn should_skip_prepared_artifact_path(path: &Path) -> bool {
    let mut components = path.components();
    let first = components.next();
    let second = components.next();

    if matches!(first, Some(Component::Normal(value)) if value == ".git"
        || value == "logs"
        || value == "state"
        || value == "target")
    {
        return true;
    }
    if matches!(
        (first, second),
        (
            Some(Component::Normal(first)),
            Some(Component::Normal(second))
        ) if first == ".greentic"
            && (second == "cache" || second == "dev" || second == "local")
    ) {
        return true;
    }
    path.components().any(|component| {
        matches!(component, Component::Normal(value) if value == ".dev.secrets.env"
            || value == ".env"
            || value == "tmp"
            || value == "temp")
    })
}

fn scrub_prepared_artifact_root(root: &Path) -> GtcResult<()> {
    scrub_prepared_artifact_dir(root, root)
}

fn scrub_prepared_artifact_dir(root: &Path, dir: &Path) -> GtcResult<()> {
    for entry in fs::read_dir(dir)
        .map_err(|err| GtcError::io(format!("failed to read {}", dir.display()), err))?
    {
        let entry = entry.map_err(|err| GtcError::message(err.to_string()))?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|err| GtcError::message(err.to_string()))?;
        let file_type = entry
            .file_type()
            .map_err(|err| GtcError::io(format!("failed to stat {}", path.display()), err))?;
        if should_skip_prepared_artifact_path(relative) {
            if file_type.is_dir() {
                fs::remove_dir_all(&path).map_err(|err| {
                    GtcError::io(format!("failed to remove {}", path.display()), err)
                })?;
            } else {
                fs::remove_file(&path).map_err(|err| {
                    GtcError::io(format!("failed to remove {}", path.display()), err)
                })?;
            }
            continue;
        }
        if file_type.is_dir() {
            scrub_prepared_artifact_dir(root, &path)?;
        }
    }
    Ok(())
}

fn write_zip_bundle_artifact(root: &Path, artifact_path: &Path) -> GtcResult<()> {
    let mut files = Vec::new();
    collect_artifact_files(root, root, &mut files)?;
    files.sort();

    let file = fs::File::create(artifact_path).map_err(|err| {
        GtcError::io(format!("failed to create {}", artifact_path.display()), err)
    })?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    for path in files {
        let relative = path
            .strip_prefix(root)
            .map_err(|err| GtcError::message(err.to_string()))?;
        let name = relative.to_string_lossy().replace('\\', "/");
        zip.start_file(&name, options)
            .map_err(|err| GtcError::message(format!("failed to write bundle zip entry: {err}")))?;
        let mut input = fs::File::open(&path)
            .map_err(|err| GtcError::io(format!("failed to open {}", path.display()), err))?;
        let mut bytes = Vec::new();
        input
            .read_to_end(&mut bytes)
            .map_err(|err| GtcError::io(format!("failed to read {}", path.display()), err))?;
        zip.write_all(&bytes)
            .map_err(|err| GtcError::io(format!("failed to write bundle zip entry {name}"), err))?;
    }

    zip.finish()
        .map_err(|err| GtcError::message(format!("failed to finish prepared bundle zip: {err}")))?;
    Ok(())
}

fn collect_artifact_files(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> GtcResult<()> {
    for entry in fs::read_dir(dir)
        .map_err(|err| GtcError::io(format!("failed to read {}", dir.display()), err))?
    {
        let entry = entry.map_err(|err| GtcError::message(err.to_string()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|err| GtcError::io(format!("failed to stat {}", path.display()), err))?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            collect_artifact_files(root, &path, out)?;
        } else if file_type.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|err| GtcError::message(err.to_string()))?;
            if !should_skip_prepared_artifact_path(relative) {
                out.push(path);
            }
        }
    }
    Ok(())
}

fn count_asset_config_files(root: &Path, dir: &Path, count: &mut usize) -> GtcResult<()> {
    for entry in fs::read_dir(dir)
        .map_err(|err| GtcError::io(format!("failed to read {}", dir.display()), err))?
    {
        let entry = entry.map_err(|err| GtcError::message(err.to_string()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|err| GtcError::io(format!("failed to stat {}", path.display()), err))?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            count_asset_config_files(root, &path, count)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|err| GtcError::message(err.to_string()))?;
        if is_asset_or_config_path(relative) {
            *count += 1;
        }
    }
    Ok(())
}

fn is_asset_or_config_path(path: &Path) -> bool {
    let mut components = path.components();
    let first = components.next();
    if matches!(first, Some(Component::Normal(value)) if value == "assets") {
        return true;
    }
    path.components()
        .any(|component| matches!(component, Component::Normal(value) if value == "config"))
}

#[cfg(test)]
mod tests {
    use super::{is_asset_or_config_path, prepare_bundle_for_start};
    use crate::deploy::StartBundleResolution;
    use crate::deploy::bundle_resolution::expand_bundle_archive;
    use crate::tests::env_test_lock;
    use std::env;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    #[test]
    fn asset_config_counter_treats_paths_as_opaque_files() {
        assert!(is_asset_or_config_path(Path::new(
            "assets/webchat-gui/config/tenants/demo.json"
        )));
        assert!(is_asset_or_config_path(Path::new(
            "assets/example-pack/public/runtime.json"
        )));
        assert!(is_asset_or_config_path(Path::new(
            "packs/example/config/runtime.json"
        )));
        assert!(!is_asset_or_config_path(Path::new(
            "state/logs/runtime.log"
        )));
    }

    #[cfg(unix)]
    #[test]
    fn prepare_bundle_for_start_builds_warmed_artifact_and_extracted_root() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let script = dir.path().join("greentic-bundle");
        write_fake_bundle_builder(&script);

        let original = env::var_os("GREENTIC_BUNDLE_BIN");
        unsafe {
            env::set_var("GREENTIC_BUNDLE_BIN", &script);
        }

        let bundle_dir = dir.path().join("bundle");
        fs::create_dir_all(bundle_dir.join("assets/example-pack/config")).expect("mkdir");
        fs::write(bundle_dir.join("bundle.yaml"), "bundle_id: demo\n").expect("bundle");
        fs::write(
            bundle_dir.join("assets/example-pack/config/runtime.json"),
            "{}\n",
        )
        .expect("config");

        let resolved = StartBundleResolution {
            bundle_dir,
            deployment_key: "demo".to_string(),
            deploy_artifact: None,
            _hold: None,
        };
        let prepared =
            prepare_bundle_for_start("fixture", &resolved, false, "en").expect("prepared");

        unsafe {
            match original {
                Some(value) => env::set_var("GREENTIC_BUNDLE_BIN", value),
                None => env::remove_var("GREENTIC_BUNDLE_BIN"),
            }
        }

        assert!(prepared.artifact_path.is_file());
        assert!(prepared.digest.starts_with("sha256:"));
        assert!(prepared.prepared_root.join("bundle.yaml").is_file());
        assert!(
            prepared
                .prepared_root
                .join("assets/example-pack/config/runtime.json")
                .is_file()
        );
        assert_eq!(prepared.included_asset_config_count, 1);
    }

    #[cfg(unix)]
    #[test]
    fn prepare_bundle_for_start_digest_changes_for_same_size_config_edit() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let script = dir.path().join("greentic-bundle");
        write_fake_bundle_builder(&script);

        let original = env::var_os("GREENTIC_BUNDLE_BIN");
        unsafe {
            env::set_var("GREENTIC_BUNDLE_BIN", &script);
        }

        let bundle_dir = dir.path().join("bundle");
        let config_path = bundle_dir.join("assets/example-pack/config/runtime.json");
        fs::create_dir_all(config_path.parent().expect("parent")).expect("mkdir");
        fs::write(bundle_dir.join("bundle.yaml"), "bundle_id: demo\n").expect("bundle");
        fs::write(&config_path, r#"{"mode":"aaaa"}"#).expect("config");

        let resolved = StartBundleResolution {
            bundle_dir: bundle_dir.clone(),
            deployment_key: "demo".to_string(),
            deploy_artifact: None,
            _hold: None,
        };
        let first =
            prepare_bundle_for_start("fixture", &resolved, false, "en").expect("first prepared");
        fs::write(&config_path, r#"{"mode":"bbbb"}"#).expect("config edit");
        let second =
            prepare_bundle_for_start("fixture", &resolved, false, "en").expect("second prepared");

        unsafe {
            match original {
                Some(value) => env::set_var("GREENTIC_BUNDLE_BIN", value),
                None => env::remove_var("GREENTIC_BUNDLE_BIN"),
            }
        }

        assert_eq!(r#"{"mode":"aaaa"}"#.len(), r#"{"mode":"bbbb"}"#.len());
        assert_ne!(first.digest, second.digest);
    }

    #[cfg(unix)]
    #[test]
    fn prepare_bundle_for_start_overlays_source_assets_into_normalized_artifact() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let script = dir.path().join("greentic-bundle");
        write_fake_normalizing_bundle_builder(&script);

        let original = env::var_os("GREENTIC_BUNDLE_BIN");
        unsafe {
            env::set_var("GREENTIC_BUNDLE_BIN", &script);
        }

        let bundle_dir = dir.path().join("bundle");
        let tenant_config = bundle_dir
            .join("assets/webchat-gui/config/tenants")
            .join("demo.json");
        fs::create_dir_all(tenant_config.parent().expect("parent")).expect("mkdir");
        fs::write(bundle_dir.join("bundle.yaml"), "bundle_id: demo\n").expect("bundle");
        fs::write(&tenant_config, "{\"nav_links\":[]}\n").expect("config");
        fs::create_dir_all(bundle_dir.join(".greentic/dev")).expect("dev");
        fs::write(
            bundle_dir.join(".greentic/dev/.dev.secrets.env"),
            "SECRET=value\n",
        )
        .expect("secret");

        let resolved = StartBundleResolution {
            bundle_dir,
            deployment_key: "demo".to_string(),
            deploy_artifact: None,
            _hold: None,
        };
        let prepared =
            prepare_bundle_for_start("fixture", &resolved, false, "en").expect("prepared");

        unsafe {
            match original {
                Some(value) => env::set_var("GREENTIC_BUNDLE_BIN", value),
                None => env::remove_var("GREENTIC_BUNDLE_BIN"),
            }
        }

        assert!(prepared.prepared_root.join("bundle.yaml").is_file());
        assert!(
            prepared
                .prepared_root
                .join("assets/webchat-gui/config/tenants/demo.json")
                .is_file()
        );
        assert!(
            prepared
                .prepared_root
                .join(".cache/v1/profile/warmed")
                .is_file()
        );
        assert!(
            !prepared
                .prepared_root
                .join(".greentic/dev/.dev.secrets.env")
                .exists()
        );
        assert_eq!(prepared.included_asset_config_count, 1);

        let extracted = dir.path().join("artifact-check");
        fs::create_dir_all(&extracted).expect("mkdir");
        expand_bundle_archive(&prepared.artifact_path, &extracted).expect("extract artifact");
        assert!(
            extracted
                .join("assets/webchat-gui/config/tenants/demo.json")
                .is_file()
        );
        assert!(extracted.join(".cache/v1/profile/warmed").is_file());
        assert!(!extracted.join(".greentic/dev/.dev.secrets.env").exists());
    }

    #[cfg(unix)]
    fn write_fake_bundle_builder(script: &Path) {
        let body = r#"#!/bin/sh
root=""
output=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --root) root="$2"; shift 2 ;;
    --output) output="$2"; shift 2 ;;
    *) shift ;;
  esac
done
cd "$root" || exit 1
python3 - "$output" <<'PY'
import os
import sys
import zipfile

output = sys.argv[1]
with zipfile.ZipFile(output, "w", zipfile.ZIP_DEFLATED) as zf:
    for base, dirs, files in os.walk("."):
        dirs[:] = [d for d in dirs if d not in {".git"}]
        for name in files:
            path = os.path.join(base, name)
            rel = path[2:] if path.startswith("./") else path
            zf.write(path, rel)
PY
"#;
        fs::write(script, body).expect("script");
        let mut perms = fs::metadata(script).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(script, perms).expect("chmod");
    }

    #[cfg(unix)]
    fn write_fake_normalizing_bundle_builder(script: &Path) {
        let body = r#"#!/bin/sh
output=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --output) output="$2"; shift 2 ;;
    *) shift ;;
  esac
done
python3 - "$output" <<'PY'
import sys
import zipfile

output = sys.argv[1]
with zipfile.ZipFile(output, "w", zipfile.ZIP_DEFLATED) as zf:
    zf.writestr("bundle.yaml", "bundle_id: normalized\n")
    zf.writestr("bundle-manifest.json", "{}\n")
    zf.writestr(".cache/v1/profile/warmed", "cache\n")
    zf.writestr(".greentic/dev/.dev.secrets.env", "SECRET=value\n")
PY
"#;
        fs::write(script, body).expect("script");
        let mut perms = fs::metadata(script).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(script, perms).expect("chmod");
    }
}
