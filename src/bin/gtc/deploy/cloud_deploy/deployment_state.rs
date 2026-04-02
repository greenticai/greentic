use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use directories::BaseDirs;
use gtc::config::GtcConfig;
use gtc::error::{GtcError, GtcResult};
use gtc::perf_targets;
use gtc::start_stop_parsing::{StartRequest, StopRequest};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::super::bundle_resolution::{fingerprint_bundle_dir, normalize_bundle_fingerprint};
use super::super::{StartBundleResolution, StartCliOptions, StartTarget, StopCliOptions};
use super::provider_packs::{resolve_deploy_app_pack_path, resolve_target_provider_pack};
use super::single_vm::{
    load_or_prepare_single_vm_artifact, read_single_vm_status, run_single_vm_apply,
    run_single_vm_destroy, stop_request_to_start_request, write_single_vm_spec,
};
use super::{
    append_bundle_registry_args, default_operator_image_for_target,
    describe_cloud_target_requirements_for_gtc, validate_cloud_deploy_inputs,
};
use crate::process::{
    run_binary_checked, run_binary_checked_with_target, run_binary_checked_with_target_and_env,
};
use crate::{DEPLOYER_BIN, SETUP_BIN};

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct StartDeploymentState {
    target: String,
    bundle_fingerprint: String,
    bundle_ref: String,
    deployed_at_epoch_s: u64,
    pub(super) artifact_path: Option<String>,
}

pub(crate) fn ensure_started_or_deployed(
    bundle_ref: &str,
    resolved: &StartBundleResolution,
    request: &StartRequest,
    cli_options: &StartCliOptions,
    target: StartTarget,
    debug: bool,
    locale: &str,
) -> GtcResult<()> {
    ensure_bundle_deployed(
        bundle_ref,
        resolved,
        request,
        cli_options,
        target,
        debug,
        locale,
    )
}

pub(crate) fn destroy_deployment(
    bundle_ref: &str,
    resolved: &StartBundleResolution,
    request: &StopRequest,
    cli_options: &StopCliOptions,
    target: StartTarget,
    debug: bool,
    locale: &str,
) -> GtcResult<()> {
    match target {
        StartTarget::SingleVm => {
            let artifact_path =
                load_or_prepare_single_vm_artifact(resolved, request, debug, locale)?;
            let start_request = stop_request_to_start_request(request, resolved, &artifact_path);
            let spec_path =
                write_single_vm_spec(bundle_ref, resolved, &start_request, &artifact_path)?;
            run_single_vm_destroy(&spec_path, debug, locale)?;
            remove_deployment_state_file(&resolved.deployment_key, target)?;
            Ok(())
        }
        StartTarget::Aws | StartTarget::Gcp | StartTarget::Azure => {
            run_multi_target_deployer_destroy(
                resolved,
                request,
                cli_options,
                target,
                debug,
                locale,
            )?;
            remove_deployment_state_file(&resolved.deployment_key, target)?;
            Ok(())
        }
        StartTarget::Runtime => Err(GtcError::message(
            "runtime target cannot be destroyed via deployer",
        )),
    }
}

fn ensure_bundle_deployed(
    bundle_ref: &str,
    resolved: &StartBundleResolution,
    request: &StartRequest,
    cli_options: &StartCliOptions,
    target: StartTarget,
    debug: bool,
    locale: &str,
) -> GtcResult<()> {
    let fingerprint = fingerprint_bundle_dir(&resolved.bundle_dir)?;
    let state_path = deployment_state_path(&resolved.deployment_key, target)?;
    let previous_state = load_deployment_state(&state_path)?;
    let deploy_needed = previous_state
        .as_ref()
        .map(|state| normalize_bundle_fingerprint(&state.bundle_fingerprint) != fingerprint)
        .unwrap_or(true);
    match target {
        StartTarget::SingleVm => {
            println!("Preparing deployable artifact for target: single-vm");
            let artifact_path = prepare_deployable_bundle_artifact(resolved, debug, locale)?;
            println!("Deployable artifact: {}", artifact_path.display());
            let spec_path = write_single_vm_spec(bundle_ref, resolved, request, &artifact_path)?;
            println!("Single-vm deployment spec: {}", spec_path.display());
            let current_status = read_single_vm_status(&spec_path, debug, locale)?;
            let status_applied = current_status
                .as_ref()
                .and_then(|value| value.get("status"))
                .and_then(Value::as_str)
                .map(|value| value == "applied")
                .unwrap_or(false);
            if status_applied && !deploy_needed {
                println!("single-vm deployment already up-to-date");
                return Ok(());
            }
            println!("Applying single-vm deployment...");
            run_single_vm_apply(&spec_path, debug, locale)?;
            let state = StartDeploymentState {
                target: target.as_str().to_string(),
                bundle_fingerprint: fingerprint,
                bundle_ref: bundle_ref.to_string(),
                deployed_at_epoch_s: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|err| GtcError::message(err.to_string()))?
                    .as_secs(),
                artifact_path: Some(artifact_path.display().to_string()),
            };
            save_deployment_state(&state_path, &state)?;
            Ok(())
        }
        StartTarget::Aws | StartTarget::Gcp | StartTarget::Azure => {
            println!("Applying cloud deployment target: {}", target.as_str());
            run_multi_target_deployer_apply(
                bundle_ref,
                resolved,
                request,
                cli_options,
                target,
                debug,
                locale,
            )?;
            let state = StartDeploymentState {
                target: target.as_str().to_string(),
                bundle_fingerprint: fingerprint,
                bundle_ref: bundle_ref.to_string(),
                deployed_at_epoch_s: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|err| GtcError::message(err.to_string()))?
                    .as_secs(),
                artifact_path: None,
            };
            save_deployment_state(&state_path, &state)?;
            Ok(())
        }
        StartTarget::Runtime => Ok(()),
    }
}

pub(super) fn prepare_deployable_bundle_artifact(
    resolved: &StartBundleResolution,
    debug: bool,
    locale: &str,
) -> GtcResult<PathBuf> {
    if let Some(path) = resolved.deploy_artifact.as_ref() {
        return Ok(path.clone());
    }

    let artifact_root = deployment_artifacts_root()?;
    fs::create_dir_all(&artifact_root).map_err(|err| {
        GtcError::io(
            format!(
                "failed to create deployment artifact root {}",
                artifact_root.display()
            ),
            err,
        )
    })?;
    let out_path = artifact_root.join(format!("{}.gtbundle", resolved.deployment_key));
    let args = vec![
        "bundle".to_string(),
        "build".to_string(),
        "--bundle".to_string(),
        resolved.bundle_dir.display().to_string(),
        "--out".to_string(),
        out_path.display().to_string(),
    ];
    run_binary_checked(SETUP_BIN, &args, debug, locale, "bundle build")?;
    Ok(out_path)
}

fn resolve_remote_deploy_bundle_source_override() -> Option<String> {
    GtcConfig::from_env().deploy_bundle_source_override()
}

fn deployment_artifacts_root() -> GtcResult<PathBuf> {
    let base = BaseDirs::new().ok_or_else(|| {
        GtcError::message("failed to resolve base directories for deployment artifacts")
    })?;
    Ok(base
        .data_local_dir()
        .join("greentic")
        .join("gtc")
        .join("bundles"))
}

pub(super) fn deployment_state_path(
    deployment_key: &str,
    target: StartTarget,
) -> GtcResult<PathBuf> {
    let base = BaseDirs::new().ok_or_else(|| {
        GtcError::message("failed to resolve base directories for deployment state")
    })?;
    Ok(base
        .state_dir()
        .unwrap_or_else(|| base.data_local_dir())
        .join("greentic")
        .join("gtc")
        .join("deployments")
        .join(format!("{deployment_key}-{}.json", target.as_str())))
}

pub(super) fn load_deployment_state(path: &Path) -> GtcResult<Option<StartDeploymentState>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path).map_err(|err| {
        GtcError::io(
            format!("failed to read deployment state {}", path.display()),
            err,
        )
    })?;
    let state = serde_json::from_str(&raw).map_err(|err| {
        GtcError::json(
            format!("failed to parse deployment state {}", path.display()),
            err,
        )
    })?;
    Ok(Some(state))
}

fn save_deployment_state(path: &Path, state: &StartDeploymentState) -> GtcResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            GtcError::io(
                format!(
                    "failed to create deployment state directory {}",
                    parent.display()
                ),
                err,
            )
        })?;
    }
    let raw = serde_json::to_vec_pretty(state)
        .map_err(|err| GtcError::message(format!("failed to serialize deployment state: {err}")))?;
    fs::write(path, raw).map_err(|err| {
        GtcError::io(
            format!("failed to write deployment state {}", path.display()),
            err,
        )
    })
}

fn run_multi_target_deployer_apply(
    _bundle_ref: &str,
    resolved: &StartBundleResolution,
    request: &StartRequest,
    cli_options: &StartCliOptions,
    target: StartTarget,
    debug: bool,
    locale: &str,
) -> GtcResult<()> {
    let bundle_artifact = prepare_deployable_bundle_artifact(resolved, debug, locale)?;
    let bundle_digest = perf_targets::sha256_file(&bundle_artifact).map_err(GtcError::message)?;
    let remote_override = cli_options
        .deploy_bundle_source
        .clone()
        .or_else(resolve_remote_deploy_bundle_source_override);
    let child_env = validate_cloud_deploy_inputs(
        target,
        remote_override.as_deref(),
        &resolved.bundle_dir,
        locale,
    )?;
    let deploy_bundle_source = remote_override
        .clone()
        .unwrap_or_else(|| bundle_artifact.display().to_string());
    let app_pack =
        resolve_deploy_app_pack_path(&resolved.bundle_dir, cli_options.app_pack.as_ref())?;
    let provider_pack = resolve_target_provider_pack(
        &resolved.bundle_dir,
        target,
        cli_options.provider_pack.as_ref(),
    )?;
    let tenant = request.tenant.clone().unwrap_or_else(|| "demo".to_string());
    let target_name = target.as_str().to_string();
    println!("Deployment artifact: {}", bundle_artifact.display());
    println!("Deployment bundle source: {deploy_bundle_source}");
    println!("Deployment bundle digest: {bundle_digest}");
    if remote_override.is_none() {
        println!(
            "Note: no GREENTIC_DEPLOY_BUNDLE_SOURCE override set; cloud deploy will use the local artifact path above."
        );
    }
    print_cloud_deploy_contract_hint(target, locale)?;
    let mut args = vec![
        target_name,
        "apply".to_string(),
        "--tenant".to_string(),
        tenant,
        "--bundle-pack".to_string(),
        app_pack.display().to_string(),
        "--provider-pack".to_string(),
        provider_pack.display().to_string(),
        "--bundle-source".to_string(),
        deploy_bundle_source.clone(),
        "--bundle-digest".to_string(),
        bundle_digest,
        "--execute".to_string(),
        "--output".to_string(),
        "json".to_string(),
    ];
    append_bundle_registry_args(&mut args, &deploy_bundle_source)?;
    if let Some(environment) = cli_options.environment.as_deref() {
        args.push("--environment".to_string());
        args.push(environment.to_string());
    }
    run_binary_checked_with_target_and_env(
        DEPLOYER_BIN,
        &args,
        debug,
        locale,
        "multi-target deploy apply",
        Some(target),
        Some(&child_env),
    )
    .map_err(|e| GtcError::message(e.to_string()))
}

fn print_cloud_deploy_contract_hint(target: StartTarget, locale: &str) -> GtcResult<()> {
    let requirements = describe_cloud_target_requirements_for_gtc(target, locale)?;
    println!("Cloud deploy contract:");
    if requirements.remote_bundle_source_required {
        println!("  required remote bundle source:");
        println!(
            "    {}",
            requirements
                .remote_bundle_source_help
                .as_deref()
                .unwrap_or("--deploy-bundle-source https://.../bundle.gtbundle")
        );
    }
    let required_vars: Vec<_> = requirements
        .variable_requirements
        .iter()
        .filter(|requirement| requirement.required)
        .collect();
    if !required_vars.is_empty() {
        println!("  required external Terraform vars:");
        for requirement in required_vars {
            println!("    {}", requirement.name);
        }
    }
    let optional_vars: Vec<_> = requirements
        .variable_requirements
        .iter()
        .filter(|requirement| !requirement.required)
        .collect();
    if !optional_vars.is_empty() {
        println!("  optional Terraform vars:");
        for requirement in optional_vars {
            println!("    {}", requirement.name);
            if requirement.name == "GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE" {
                println!(
                    "      default: {}",
                    default_operator_image_for_target(target).unwrap_or_default()
                );
                continue;
            }
            if requirement.name == "GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE_DIGEST" {
                if let Some(default_value) = requirement.default_value.as_deref() {
                    println!("      fallback default: {default_value}");
                }
                continue;
            }
            if let Some(default_value) = requirement.default_value.as_deref() {
                println!("      default: {default_value}");
            }
            if requirement.name == "GREENTIC_DEPLOY_TERRAFORM_VAR_DNS_NAME" {
                println!("      personalized mode only");
            }
        }
    }
    if !requirements.informational_notes.is_empty() {
        println!("  deployer-managed notes:");
        for note in &requirements.informational_notes {
            println!("    {note}");
        }
    }
    Ok(())
}

fn run_multi_target_deployer_destroy(
    resolved: &StartBundleResolution,
    request: &StopRequest,
    cli_options: &StopCliOptions,
    target: StartTarget,
    debug: bool,
    locale: &str,
) -> GtcResult<()> {
    let app_pack =
        resolve_deploy_app_pack_path(&resolved.bundle_dir, cli_options.app_pack.as_ref())?;
    let provider_pack = resolve_target_provider_pack(
        &resolved.bundle_dir,
        target,
        cli_options.provider_pack.as_ref(),
    )?;
    let mut args = vec![
        target.as_str().to_string(),
        "destroy".to_string(),
        "--tenant".to_string(),
        request.tenant.clone(),
        "--bundle-pack".to_string(),
        app_pack.display().to_string(),
        "--provider-pack".to_string(),
        provider_pack.display().to_string(),
        "--execute".to_string(),
        "--output".to_string(),
        "json".to_string(),
    ];
    if let Some(environment) = cli_options.environment.as_deref() {
        args.push("--environment".to_string());
        args.push(environment.to_string());
    }
    run_binary_checked_with_target(
        DEPLOYER_BIN,
        &args,
        debug,
        locale,
        "multi-target deploy destroy",
        Some(target),
    )
    .map_err(|e| GtcError::message(e.to_string()))
}

fn remove_deployment_state_file(deployment_key: &str, target: StartTarget) -> GtcResult<()> {
    let path = deployment_state_path(deployment_key, target)?;
    if !path.exists() {
        return Ok(());
    }
    fs::remove_file(&path).map_err(|err| {
        GtcError::io(
            format!("failed to remove deployment state {}", path.display()),
            err,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{
        StartDeploymentState, deployment_artifacts_root, deployment_state_path,
        load_deployment_state, prepare_deployable_bundle_artifact, remove_deployment_state_file,
        resolve_remote_deploy_bundle_source_override, save_deployment_state,
    };
    #[cfg(unix)]
    use super::{run_multi_target_deployer_apply, run_multi_target_deployer_destroy};
    use crate::deploy::{StartBundleResolution, StartTarget};
    #[cfg(unix)]
    use crate::deploy::{StartCliOptions, StopCliOptions};
    use crate::tests::{env_test_lock, fake_deployer_contract};
    #[cfg(unix)]
    use gtc::start_stop_parsing::{
        CloudflaredModeArg, NatsModeArg, NgrokModeArg, StartRequest, StopRequest,
    };
    use std::env;
    use std::fs;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn resolve_remote_deploy_bundle_source_override_trims_blank_values() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            env::set_var(
                "GREENTIC_DEPLOY_BUNDLE_SOURCE",
                "  https://example.com/demo  ",
            );
        }
        let value = resolve_remote_deploy_bundle_source_override();
        unsafe {
            env::remove_var("GREENTIC_DEPLOY_BUNDLE_SOURCE");
        }
        assert_eq!(value.as_deref(), Some("https://example.com/demo"));
    }

    #[cfg(unix)]
    #[cfg_attr(target_os = "macos", ignore)]
    #[test]
    fn deployment_roots_follow_xdg_state_and_data_home() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let state_home = dir.path().join("state");
        let data_home = dir.path().join("data");
        fs::create_dir_all(&state_home).expect("mkdir");
        fs::create_dir_all(&data_home).expect("mkdir");
        unsafe {
            env::set_var("XDG_STATE_HOME", &state_home);
            env::set_var("XDG_DATA_HOME", &data_home);
        }

        let state_path = deployment_state_path("demo", StartTarget::Aws).expect("state path");
        let artifact_root = deployment_artifacts_root().expect("artifact root");

        unsafe {
            env::remove_var("XDG_STATE_HOME");
            env::remove_var("XDG_DATA_HOME");
        }

        assert!(state_path.starts_with(&state_home));
        assert!(artifact_root.starts_with(&data_home));
    }

    #[test]
    fn save_and_load_deployment_state_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let state = StartDeploymentState {
            target: "aws".to_string(),
            bundle_fingerprint: "fp".to_string(),
            bundle_ref: "demo".to_string(),
            deployed_at_epoch_s: 1,
            artifact_path: Some("/tmp/demo.gtbundle".to_string()),
        };

        save_deployment_state(&path, &state).expect("save");
        let loaded = load_deployment_state(&path).expect("load").expect("state");

        assert_eq!(loaded.target, "aws");
        assert_eq!(loaded.bundle_fingerprint, "fp");
        assert_eq!(loaded.artifact_path.as_deref(), Some("/tmp/demo.gtbundle"));
    }

    #[test]
    fn load_deployment_state_returns_none_for_missing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("missing.json");
        assert!(load_deployment_state(&path).expect("load").is_none());
    }

    #[test]
    fn prepare_deployable_bundle_artifact_reuses_existing_artifact() {
        let dir = tempfile::tempdir().expect("tempdir");
        let artifact = dir.path().join("bundle.gtbundle");
        fs::write(&artifact, b"fixture").expect("write");
        let resolved = StartBundleResolution {
            bundle_dir: dir.path().join("bundle"),
            deployment_key: "demo".to_string(),
            deploy_artifact: Some(artifact.clone()),
            _hold: None,
        };

        let prepared =
            prepare_deployable_bundle_artifact(&resolved, false, "en").expect("artifact");
        assert_eq!(prepared, artifact);
    }

    #[test]
    fn remove_deployment_state_file_removes_existing_state() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let state_home = dir.path().join("state");
        fs::create_dir_all(&state_home).expect("mkdir");
        unsafe {
            env::set_var("XDG_STATE_HOME", &state_home);
        }
        let path = deployment_state_path("demo", StartTarget::Gcp).expect("state path");
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(&path, "{}").expect("write");

        remove_deployment_state_file("demo", StartTarget::Gcp).expect("remove");
        unsafe {
            env::remove_var("XDG_STATE_HOME");
        }
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn multi_target_deployer_apply_uses_resolved_packs_and_remote_source() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let bundle_dir = dir.path().join("bundle");
        fs::create_dir_all(&bundle_dir).expect("mkdir");
        let app_pack = dir.path().join("app.gtpack");
        let provider_pack = dir.path().join("terraform.gtpack");
        fs::write(&app_pack, b"app").expect("write");
        fs::write(&provider_pack, b"provider").expect("write");

        let artifact = dir.path().join("bundle.gtbundle");
        fs::write(&artifact, b"bundle").expect("write");

        let terraform_dir = dir.path().join("terraform-bin");
        fs::create_dir_all(&terraform_dir).expect("mkdir");
        let terraform = terraform_dir.join("terraform");
        fs::write(&terraform, "#!/bin/sh\nexit 0\n").expect("write");
        fs::set_permissions(&terraform, fs::Permissions::from_mode(0o755)).expect("chmod");

        let log = dir.path().join("deployer.log");
        let (_deployer_dir, _deployer_guard) = fake_deployer_contract(Some(&log));

        let request = StartRequest {
            bundle: Some(bundle_dir.display().to_string()),
            tenant: Some("demo".to_string()),
            team: Some("ops".to_string()),
            no_nats: false,
            nats: NatsModeArg::Off,
            nats_url: None,
            config: None,
            cloudflared: CloudflaredModeArg::Off,
            cloudflared_binary: None,
            ngrok: NgrokModeArg::Off,
            ngrok_binary: None,
            runner_binary: None,
            restart: Vec::new(),
            log_dir: None,
            verbose: false,
            quiet: false,
            admin: false,
            admin_port: 8443,
            admin_certs_dir: None,
            admin_allowed_clients: Vec::new(),
            tunnel_explicit: true,
        };
        let cli_options = StartCliOptions {
            start_args: Vec::new(),
            explicit_target: Some(StartTarget::Gcp),
            environment: Some("prod".to_string()),
            provider_pack: Some(provider_pack),
            app_pack: Some(app_pack),
            deploy_bundle_source: Some("https://example.com/demo.gtbundle".to_string()),
        };
        let resolved = StartBundleResolution {
            bundle_dir: bundle_dir.clone(),
            deployment_key: "demo".to_string(),
            deploy_artifact: Some(artifact),
            _hold: None,
        };

        let original_path = env::var_os("PATH");
        unsafe {
            env::set_var("PATH", &terraform_dir);
            env::set_var("CLOUDSDK_AUTH_ACCESS_TOKEN", "token");
            env::set_var("GREENTIC_DEPLOY_TERRAFORM_VAR_REMOTE_STATE_BACKEND", "gcs");
            env::set_var("GREENTIC_DEPLOY_TERRAFORM_VAR_GCP_PROJECT_ID", "project");
            env::set_var("GREENTIC_DEPLOY_TERRAFORM_VAR_GCP_REGION", "europe-west1");
        }

        run_multi_target_deployer_apply(
            "bundle-ref",
            &resolved,
            &request,
            &cli_options,
            StartTarget::Gcp,
            false,
            "en",
        )
        .expect("apply");

        unsafe {
            match original_path {
                Some(path) => env::set_var("PATH", path),
                None => env::remove_var("PATH"),
            }
            env::remove_var("CLOUDSDK_AUTH_ACCESS_TOKEN");
            env::remove_var("GREENTIC_DEPLOY_TERRAFORM_VAR_REMOTE_STATE_BACKEND");
            env::remove_var("GREENTIC_DEPLOY_TERRAFORM_VAR_GCP_PROJECT_ID");
            env::remove_var("GREENTIC_DEPLOY_TERRAFORM_VAR_GCP_REGION");
        }

        let logged = fs::read_to_string(log).expect("read");
        assert!(logged.contains("gcp apply"));
        assert!(logged.contains("--bundle-source https://example.com/demo.gtbundle"));
        assert!(logged.contains("--environment prod"));
    }

    #[cfg(unix)]
    #[test]
    fn multi_target_deployer_destroy_uses_resolved_packs() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let bundle_dir = dir.path().join("bundle");
        fs::create_dir_all(&bundle_dir).expect("mkdir");
        let app_pack = dir.path().join("app.gtpack");
        let provider_pack = dir.path().join("terraform.gtpack");
        fs::write(&app_pack, b"app").expect("write");
        fs::write(&provider_pack, b"provider").expect("write");

        let log = dir.path().join("deployer.log");
        let (_deployer_dir, _deployer_guard) = fake_deployer_contract(Some(&log));
        let original_operator_image = env::var_os("GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE");
        let original_operator_digest =
            env::var_os("GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE_DIGEST");
        unsafe {
            env::set_var(
                "GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE",
                "ghcr.io/greenticai/greentic-start-distroless@sha256:a7f4741a1206900b73a77c5e40860c2695206274374546dd3bb9cab8e752f79b",
            );
            env::set_var(
                "GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE_DIGEST",
                "sha256:a7f4741a1206900b73a77c5e40860c2695206274374546dd3bb9cab8e752f79b",
            );
        }

        let request = StopRequest {
            bundle: Some(bundle_dir.display().to_string()),
            state_dir: None,
            tenant: "demo".to_string(),
            team: "ops".to_string(),
        };
        let cli_options = StopCliOptions {
            stop_args: Vec::new(),
            explicit_target: Some(StartTarget::Aws),
            environment: Some("prod".to_string()),
            provider_pack: Some(provider_pack),
            app_pack: Some(app_pack),
            destroy: true,
        };
        let resolved = StartBundleResolution {
            bundle_dir: bundle_dir.clone(),
            deployment_key: "demo".to_string(),
            deploy_artifact: None,
            _hold: None,
        };

        run_multi_target_deployer_destroy(
            &resolved,
            &request,
            &cli_options,
            StartTarget::Aws,
            false,
            "en",
        )
        .expect("destroy");
        unsafe {
            match original_operator_image {
                Some(path) => env::set_var("GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE", path),
                None => env::remove_var("GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE"),
            }
            match original_operator_digest {
                Some(path) => {
                    env::set_var("GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE_DIGEST", path)
                }
                None => env::remove_var("GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE_DIGEST"),
            }
        }

        let logged = fs::read_to_string(log).expect("read");
        assert!(logged.contains("aws destroy"));
        assert!(logged.contains("--environment prod"));
    }
}
