use std::fs;
use std::path::{Path, PathBuf};

use gtc::error::{GtcError, GtcResult};
use serde::Deserialize;
use serde_yaml_bw::Value as YamlValue;

use super::super::StartTarget;
use super::canonical_provider_pack_filename_for_gtc;
use crate::DEPLOYER_BIN;
use crate::process::resolve_companion_binary;

#[derive(Debug, Deserialize)]
struct DeploymentTargetsDocument {
    targets: Vec<DeploymentTargetRecord>,
}

#[derive(Debug, Deserialize)]
struct DeploymentTargetRecord {
    target: String,
    provider_pack: Option<String>,
}

pub(crate) fn resolve_target_provider_pack(
    bundle_dir: &Path,
    target: StartTarget,
    override_path: Option<&PathBuf>,
) -> GtcResult<PathBuf> {
    if let Some(path) = override_path {
        return Ok(path.clone());
    }
    if let Some(path) = resolve_target_provider_pack_from_metadata(bundle_dir, target)? {
        return Ok(path);
    }
    if let Some(path) = resolve_canonical_target_provider_pack(target) {
        return Ok(path);
    }
    Err(GtcError::message(format!(
        "no deployer provider pack found for target {}; define deployment_targets metadata or install greentic-deployer with dist packs",
        target.as_str(),
    )))
}

fn resolve_canonical_target_provider_pack(target: StartTarget) -> Option<PathBuf> {
    let filename = canonical_target_provider_pack_filename(target)
        .ok()
        .flatten()?;
    let deployer_bin = resolve_companion_binary(DEPLOYER_BIN)?;
    resolve_canonical_target_provider_pack_from(Some(deployer_bin.as_path()), &filename)
}

pub(crate) fn resolve_canonical_target_provider_pack_from(
    deployer_bin: Option<&Path>,
    filename: &str,
) -> Option<PathBuf> {
    let deployer_bin = deployer_bin?;
    let exe_dir = deployer_bin.parent()?;
    let mut candidates = Vec::new();
    candidates.push(exe_dir.join("dist").join(filename));
    if let Some(repo_dir) = exe_dir.parent().and_then(Path::parent) {
        candidates.push(repo_dir.join("dist").join(filename));
    }
    candidates.into_iter().find(|candidate| candidate.is_file())
}

fn canonical_target_provider_pack_filename(target: StartTarget) -> GtcResult<Option<String>> {
    canonical_provider_pack_filename_for_gtc(target, "en")
}

fn resolve_target_provider_pack_from_metadata(
    bundle_dir: &Path,
    target: StartTarget,
) -> GtcResult<Option<PathBuf>> {
    let Some(doc) = load_deployment_targets_document(bundle_dir)? else {
        return Ok(None);
    };
    for record in doc.targets {
        let parsed_target = match record.target.trim() {
            "runtime" | "local" => StartTarget::Runtime,
            "single-vm" | "single_vm" => StartTarget::SingleVm,
            "aws" => StartTarget::Aws,
            "gcp" => StartTarget::Gcp,
            "azure" => StartTarget::Azure,
            other => {
                return Err(GtcError::message(format!(
                    "unsupported --target value {other}; expected runtime, single-vm, aws, gcp, or azure"
                )));
            }
        };
        if parsed_target != target {
            continue;
        }
        let Some(provider_pack) = record.provider_pack else {
            return Ok(None);
        };
        let candidate = bundle_dir.join(provider_pack);
        if candidate.exists() {
            return Ok(Some(candidate));
        }
        return Ok(None);
    }
    Ok(None)
}

fn load_deployment_targets_document(
    bundle_dir: &Path,
) -> GtcResult<Option<DeploymentTargetsDocument>> {
    let path = bundle_dir.join(".greentic").join("deployment-targets.json");
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path)
        .map_err(|err| GtcError::io(format!("failed to read {}", path.display()), err))?;
    let doc: DeploymentTargetsDocument = serde_json::from_str(&raw)
        .map_err(|err| GtcError::json(format!("failed to parse {}", path.display()), err))?;
    Ok(Some(doc))
}

pub(crate) fn resolve_deploy_app_pack_path(
    bundle_dir: &Path,
    override_path: Option<&PathBuf>,
) -> GtcResult<PathBuf> {
    if let Some(path) = override_path {
        return Ok(path.clone());
    }
    if let Some(path) = resolve_app_pack_path_from_bundle_metadata(bundle_dir)? {
        return Ok(path);
    }
    let default_pack_ref = bundle_dir.join("default.gtpack");
    if default_pack_ref.exists() {
        let raw = fs::read_to_string(&default_pack_ref).map_err(|err| {
            GtcError::io(
                format!(
                    "failed to read default pack reference {}",
                    default_pack_ref.display()
                ),
                err,
            )
        })?;
        let pack_ref = raw.trim();
        if !pack_ref.is_empty() {
            let candidate = bundle_dir.join(pack_ref);
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }
    let packs_dir = bundle_dir.join("packs");
    if !packs_dir.exists() {
        return Err(GtcError::message(format!(
            "bundle has no packs directory: {}",
            packs_dir.display()
        )));
    }
    let mut candidates = Vec::new();
    for entry in fs::read_dir(&packs_dir)
        .map_err(|err| GtcError::io(format!("failed to read {}", packs_dir.display()), err))?
    {
        let entry = entry.map_err(|err| GtcError::message(err.to_string()))?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) == Some("gtpack") || path.is_dir() {
            candidates.push(path);
        }
    }
    candidates.sort();
    match candidates.len() {
        0 => Err(GtcError::message(format!(
            "no app pack found under {}",
            packs_dir.display()
        ))),
        1 => Ok(candidates.remove(0)),
        _ => Err(GtcError::message(
            "cloud deployment requires a canonical app pack; set bundle.yaml app_packs, add default.gtpack, or pass --app-pack explicitly",
        )),
    }
}

fn resolve_app_pack_path_from_bundle_metadata(bundle_dir: &Path) -> GtcResult<Option<PathBuf>> {
    let bundle_path = bundle_dir.join("bundle.yaml");
    if !bundle_path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&bundle_path)
        .map_err(|err| GtcError::io(format!("failed to read {}", bundle_path.display()), err))?;
    let doc: YamlValue = serde_yaml_bw::from_str(&raw).map_err(|err| {
        GtcError::message(format!("failed to parse {}: {err}", bundle_path.display()))
    })?;
    let Some(app_packs) = doc.get("app_packs").and_then(YamlValue::as_sequence) else {
        return Ok(None);
    };
    let Some(reference) = app_packs.first().and_then(YamlValue::as_str) else {
        return Ok(None);
    };
    let candidate = if reference.contains("://") {
        if let Some(file_name) = reference
            .split('?')
            .next()
            .and_then(|value| value.rsplit('/').next())
            .filter(|value| !value.is_empty())
        {
            let bundled = bundle_dir.join("packs").join(file_name);
            if bundled.exists() {
                bundled
            } else {
                PathBuf::from(reference)
            }
        } else {
            PathBuf::from(reference)
        }
    } else if Path::new(reference).is_absolute() {
        if let Some(file_name) = Path::new(reference).file_name() {
            let bundled = bundle_dir.join("packs").join(file_name);
            if bundled.exists() {
                bundled
            } else {
                PathBuf::from(reference)
            }
        } else {
            PathBuf::from(reference)
        }
    } else {
        bundle_dir.join(reference)
    };
    if candidate.exists() {
        return Ok(Some(candidate));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::{
        canonical_target_provider_pack_filename, load_deployment_targets_document,
        resolve_app_pack_path_from_bundle_metadata, resolve_canonical_target_provider_pack_from,
        resolve_target_provider_pack_from_metadata,
    };
    use crate::deploy::StartTarget;
    use crate::tests::fake_deployer_contract;
    use std::fs;
    use std::path::Path;

    #[test]
    fn canonical_target_provider_pack_filename_matches_cloud_targets() {
        let (_deployer_dir, _deployer_guard) = fake_deployer_contract(None);
        assert_eq!(
            canonical_target_provider_pack_filename(StartTarget::Aws).expect("aws filename"),
            Some("terraform.gtpack".to_string())
        );
        assert_eq!(
            canonical_target_provider_pack_filename(StartTarget::Runtime)
                .expect("runtime filename"),
            None
        );
    }

    #[test]
    fn load_deployment_targets_document_errors_on_invalid_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let greentic = dir.path().join(".greentic");
        fs::create_dir_all(&greentic).expect("mkdir");
        fs::write(greentic.join("deployment-targets.json"), "{not json").expect("write");

        let err = load_deployment_targets_document(dir.path()).unwrap_err();
        assert!(err.contains("failed to parse"));
    }

    #[test]
    fn resolve_target_provider_pack_from_metadata_errors_on_unknown_target() {
        let dir = tempfile::tempdir().expect("tempdir");
        let greentic = dir.path().join(".greentic");
        fs::create_dir_all(&greentic).expect("mkdir");
        fs::write(
            greentic.join("deployment-targets.json"),
            r#"{"targets":[{"target":"weird","provider_pack":"packs/demo.gtpack"}]}"#,
        )
        .expect("write");

        let err =
            resolve_target_provider_pack_from_metadata(dir.path(), StartTarget::Aws).unwrap_err();
        assert!(err.contains("unsupported --target value"));
    }

    #[test]
    fn resolve_target_provider_pack_from_metadata_returns_none_when_file_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let greentic = dir.path().join(".greentic");
        fs::create_dir_all(&greentic).expect("mkdir");
        fs::write(
            greentic.join("deployment-targets.json"),
            r#"{"targets":[{"target":"aws","provider_pack":"packs/missing.gtpack"}]}"#,
        )
        .expect("write");

        let resolved =
            resolve_target_provider_pack_from_metadata(dir.path(), StartTarget::Aws).unwrap();
        assert_eq!(resolved, None);
    }

    #[test]
    fn resolve_app_pack_path_from_bundle_metadata_supports_relative_reference() {
        let dir = tempfile::tempdir().expect("tempdir");
        let packs = dir.path().join("packs");
        fs::create_dir_all(&packs).expect("mkdir");
        let pack = packs.join("demo.gtpack");
        fs::write(&pack, b"fixture").expect("write");
        fs::write(
            dir.path().join("bundle.yaml"),
            "app_packs:\n  - packs/demo.gtpack\n",
        )
        .expect("write");

        let resolved = resolve_app_pack_path_from_bundle_metadata(dir.path()).unwrap();
        assert_eq!(resolved.as_deref(), Some(Path::new(&pack)));
    }

    #[test]
    fn resolve_app_pack_path_from_bundle_metadata_ignores_unbundled_remote_reference() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(
            dir.path().join("bundle.yaml"),
            "app_packs:\n  - https://example.com/releases/demo.gtpack\n",
        )
        .expect("write");

        let resolved = resolve_app_pack_path_from_bundle_metadata(dir.path()).unwrap();
        assert_eq!(resolved, None);
    }

    #[test]
    fn resolve_canonical_target_provider_pack_from_checks_adjacent_dist_dirs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let exe = dir
            .path()
            .join("repo")
            .join("target")
            .join("debug")
            .join("greentic-deployer");
        fs::create_dir_all(exe.parent().expect("parent")).expect("mkdir");
        fs::write(&exe, b"bin").expect("write");

        let dist = exe.parent().expect("parent").join("dist");
        fs::create_dir_all(&dist).expect("mkdir");
        let pack = dist.join("terraform.gtpack");
        fs::write(&pack, b"fixture").expect("write pack");

        let resolved = resolve_canonical_target_provider_pack_from(Some(&exe), "terraform.gtpack");
        assert_eq!(resolved.as_deref(), Some(pack.as_path()));
    }
}
