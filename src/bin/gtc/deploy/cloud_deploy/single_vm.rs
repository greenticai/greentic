use std::fs;
use std::path::{Path, PathBuf};

use directories::BaseDirs;
use gtc::error::{GtcError, GtcResult};
use gtc::start_stop_parsing::{
    CloudflaredModeArg, NatsModeArg, NgrokModeArg, StartRequest, StopRequest,
};
use serde_json::Value;

use super::super::{StartBundleResolution, StartTarget};
use super::deployment_state::{
    deployment_state_path, load_deployment_state, prepare_deployable_bundle_artifact,
};
use crate::DEPLOYER_BIN;
use crate::admin::resolve_admin_cert_dir;
use crate::process::{run_binary_capture, run_binary_checked};

pub(crate) fn write_single_vm_spec(
    bundle_ref: &str,
    resolved: &StartBundleResolution,
    request: &StartRequest,
    artifact_path: &Path,
) -> GtcResult<PathBuf> {
    let cert_dir = resolve_admin_cert_dir(&resolved.bundle_dir);
    let deployment_name = deployment_name(bundle_ref, request);
    let state_root = deployment_runtime_root(&resolved.deployment_key)?;
    let spec_dir = state_root.join("spec");
    fs::create_dir_all(&spec_dir).map_err(|err| {
        GtcError::io(
            format!("failed to create spec directory {}", spec_dir.display()),
            err,
        )
    })?;
    let spec_path = spec_dir.join("single-vm.deployment.yaml");
    let spec = format!(
        "apiVersion: greentic.ai/v1alpha1\nkind: Deployment\nmetadata:\n  name: {name}\nspec:\n  target: single-vm\n  bundle:\n    source: {bundle}\n    format: squashfs\n  runtime:\n    image: {image}\n    arch: x86_64\n    admin:\n      bind: 127.0.0.1:8433\n      mtls:\n        caFile: {ca}\n        certFile: {cert}\n        keyFile: {key}\n  storage:\n    stateDir: {state_dir}\n    cacheDir: {cache_dir}\n    logDir: {log_dir}\n    tempDir: {temp_dir}\n  service:\n    manager: systemd\n    user: greentic\n    group: greentic\n  health:\n    readinessPath: /ready\n    livenessPath: /health\n    startupTimeoutSeconds: 120\n  rollout:\n    strategy: recreate\n",
        name = deployment_name,
        bundle = yaml_string(&format!("file://{}", artifact_path.display())),
        image = yaml_string("ghcr.io/greentic-ai/operator-distroless:0.1.0-distroless"),
        ca = yaml_string(&cert_dir.join("ca.crt").display().to_string()),
        cert = yaml_string(&cert_dir.join("server.crt").display().to_string()),
        key = yaml_string(&cert_dir.join("server.key").display().to_string()),
        state_dir = yaml_string(&state_root.join("state").display().to_string()),
        cache_dir = yaml_string(&state_root.join("cache").display().to_string()),
        log_dir = yaml_string(&state_root.join("log").display().to_string()),
        temp_dir = yaml_string(&state_root.join("tmp").display().to_string()),
    );
    fs::write(&spec_path, spec).map_err(|err| {
        GtcError::io(
            format!("failed to write deployment spec {}", spec_path.display()),
            err,
        )
    })?;
    Ok(spec_path)
}

pub(super) fn stop_request_to_start_request(
    request: &StopRequest,
    resolved: &StartBundleResolution,
    artifact_path: &Path,
) -> StartRequest {
    StartRequest {
        bundle: Some(resolved.bundle_dir.display().to_string()),
        tenant: Some(request.tenant.clone()),
        team: Some(request.team.clone()),
        no_nats: false,
        nats: NatsModeArg::Off,
        nats_url: None,
        config: None,
        cloudflared: CloudflaredModeArg::Off,
        cloudflared_binary: None,
        ngrok: NgrokModeArg::Off,
        ngrok_binary: None,
        runner_binary: Some(artifact_path.to_path_buf()),
        restart: Vec::new(),
        log_dir: None,
        verbose: false,
        quiet: false,
        admin: false,
        admin_port: 8443,
        admin_certs_dir: None,
        admin_allowed_clients: Vec::new(),
    }
}

pub(super) fn load_or_prepare_single_vm_artifact(
    resolved: &StartBundleResolution,
    request: &StopRequest,
    debug: bool,
    locale: &str,
) -> GtcResult<PathBuf> {
    let state_path = deployment_state_path(&resolved.deployment_key, StartTarget::SingleVm)?;
    if let Some(state) = load_deployment_state(&state_path)?
        && let Some(path) = state.artifact_path
    {
        let artifact = PathBuf::from(path);
        if artifact.exists() {
            return Ok(artifact);
        }
    }
    let _ = request;
    prepare_deployable_bundle_artifact(resolved, debug, locale)
}

pub(super) fn read_single_vm_status(
    spec_path: &Path,
    debug: bool,
    locale: &str,
) -> GtcResult<Option<Value>> {
    let args = vec![
        "single-vm".to_string(),
        "status".to_string(),
        "--spec".to_string(),
        spec_path.display().to_string(),
        "--output".to_string(),
        "json".to_string(),
    ];
    let output = run_binary_capture(DEPLOYER_BIN, &args, debug, locale)?;
    if output.trim().is_empty() {
        return Ok(None);
    }
    let parsed = serde_json::from_str(&output)
        .map_err(|err| GtcError::json("failed to parse deployer status output as JSON", err))?;
    Ok(Some(parsed))
}

pub(super) fn run_single_vm_apply(spec_path: &Path, debug: bool, locale: &str) -> GtcResult<()> {
    let args = vec![
        "single-vm".to_string(),
        "apply".to_string(),
        "--spec".to_string(),
        spec_path.display().to_string(),
        "--execute".to_string(),
        "--output".to_string(),
        "json".to_string(),
    ];
    run_binary_checked(DEPLOYER_BIN, &args, debug, locale, "single-vm apply")
}

pub(super) fn run_single_vm_destroy(spec_path: &Path, debug: bool, locale: &str) -> GtcResult<()> {
    let args = vec![
        "single-vm".to_string(),
        "destroy".to_string(),
        "--spec".to_string(),
        spec_path.display().to_string(),
        "--execute".to_string(),
        "--output".to_string(),
        "json".to_string(),
    ];
    run_binary_checked(DEPLOYER_BIN, &args, debug, locale, "single-vm destroy")
}

fn deployment_runtime_root(deployment_key: &str) -> GtcResult<PathBuf> {
    let base = BaseDirs::new().ok_or_else(|| {
        GtcError::message("failed to resolve base directories for deployment runtime")
    })?;
    Ok(base
        .state_dir()
        .unwrap_or_else(|| base.data_local_dir())
        .join("greentic")
        .join("gtc")
        .join("single-vm")
        .join(deployment_key))
}

fn yaml_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn deployment_name(bundle_ref: &str, request: &StartRequest) -> String {
    let mut parts = Vec::new();
    if let Some(tenant) = request.tenant.as_deref() {
        parts.push(tenant.to_string());
    }
    if let Some(team) = request.team.as_deref() {
        parts.push(team.to_string());
    }
    parts.push(sanitize_identifier(bundle_ref));
    let joined = parts.join("-");
    truncate_identifier(&joined, 63)
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

fn truncate_identifier(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_string();
    }
    value.chars().take(limit).collect()
}

#[cfg(test)]
mod tests {
    use super::{
        deployment_name, sanitize_identifier, stop_request_to_start_request, truncate_identifier,
        yaml_string,
    };
    #[cfg(unix)]
    use super::{load_or_prepare_single_vm_artifact, write_single_vm_spec};
    use crate::deploy::StartBundleResolution;
    #[cfg(unix)]
    use crate::deploy::StartTarget;
    #[cfg(unix)]
    use crate::deploy::cloud_deploy::deployment_state::deployment_state_path;
    #[cfg(unix)]
    use crate::tests::env_test_lock;
    use gtc::start_stop_parsing::{
        CloudflaredModeArg, NatsModeArg, NgrokModeArg, StartRequest, StopRequest,
    };
    #[cfg(unix)]
    use insta::assert_snapshot;
    use proptest::prelude::*;
    #[cfg(unix)]
    use std::env;
    #[cfg(unix)]
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn yaml_string_escapes_single_quotes() {
        assert_eq!(yaml_string("demo's bundle"), "'demo''s bundle'");
    }

    #[test]
    fn sanitize_identifier_and_truncate_identifier_normalize_strings() {
        assert_eq!(sanitize_identifier("Acme/Demo Bundle"), "acme-demo-bundle");
        assert_eq!(truncate_identifier("abcdef", 4), "abcd");
    }

    #[test]
    fn deployment_name_combines_tenant_team_and_bundle() {
        let request = StartRequest {
            bundle: None,
            tenant: Some("tenant".to_string()),
            team: Some("team".to_string()),
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
        };

        let name = deployment_name("Demo Bundle", &request);
        assert_eq!(name, "tenant-team-demo-bundle");
    }

    #[test]
    fn stop_request_to_start_request_preserves_identity_and_artifact() {
        let request = StopRequest {
            bundle: Some("/tmp/bundle".to_string()),
            tenant: "demo".to_string(),
            team: "ops".to_string(),
            state_dir: None,
        };
        let artifact = PathBuf::from("/tmp/bundle.gtbundle");
        let resolved = StartBundleResolution {
            bundle_dir: PathBuf::from("/tmp/bundle"),
            deployment_key: "demo".to_string(),
            deploy_artifact: Some(artifact.clone()),
            _hold: None,
        };

        let start = stop_request_to_start_request(&request, &resolved, &artifact);
        assert_eq!(start.tenant.as_deref(), Some("demo"));
        assert_eq!(start.team.as_deref(), Some("ops"));
        assert_eq!(start.runner_binary.as_deref(), Some(artifact.as_path()));
    }

    #[cfg(unix)]
    #[test]
    fn load_or_prepare_single_vm_artifact_reuses_saved_artifact() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let state_home = dir.path().join("state-home");
        fs::create_dir_all(&state_home).expect("mkdir");
        unsafe {
            env::set_var("XDG_STATE_HOME", &state_home);
        }
        let artifact = dir.path().join("bundle.gtbundle");
        fs::write(&artifact, b"fixture").expect("write");

        let state_path = deployment_state_path("demo", StartTarget::SingleVm).expect("state path");
        fs::create_dir_all(state_path.parent().expect("parent")).expect("mkdir");
        fs::write(
            &state_path,
            format!(
                "{{\"target\":\"single-vm\",\"bundle_fingerprint\":\"fp\",\"bundle_ref\":\"demo\",\"deployed_at_epoch_s\":1,\"artifact_path\":\"{}\"}}",
                artifact.display()
            ),
        )
        .expect("write state");

        let resolved = StartBundleResolution {
            bundle_dir: dir.path().join("bundle"),
            deployment_key: "demo".to_string(),
            deploy_artifact: None,
            _hold: None,
        };
        let request = StopRequest {
            bundle: None,
            tenant: "demo".to_string(),
            team: "ops".to_string(),
            state_dir: None,
        };

        let reused =
            load_or_prepare_single_vm_artifact(&resolved, &request, false, "en").expect("artifact");
        unsafe {
            env::remove_var("XDG_STATE_HOME");
        }
        assert_eq!(reused, artifact);
    }

    #[cfg(unix)]
    #[cfg_attr(target_os = "macos", ignore)]
    #[test]
    fn write_single_vm_spec_matches_snapshot() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let state_home = dir.path().join("state-home");
        let data_home = dir.path().join("data-home");
        let bundle_dir = dir.path().join("bundle");
        let cert_dir = bundle_dir.join(".greentic").join("admin").join("certs");
        fs::create_dir_all(&cert_dir).expect("cert dir");
        fs::write(cert_dir.join("ca.crt"), "ca").expect("write ca");
        fs::write(cert_dir.join("server.crt"), "server").expect("write server crt");
        fs::write(cert_dir.join("server.key"), "server-key").expect("write server key");
        fs::create_dir_all(&state_home).expect("state home");
        fs::create_dir_all(&data_home).expect("data home");
        unsafe {
            env::set_var("XDG_STATE_HOME", &state_home);
            env::set_var("XDG_DATA_HOME", &data_home);
        }

        let resolved = StartBundleResolution {
            bundle_dir: bundle_dir.clone(),
            deployment_key: "demo-key".to_string(),
            deploy_artifact: None,
            _hold: None,
        };
        let request = StartRequest {
            bundle: Some(bundle_dir.display().to_string()),
            tenant: Some("tenant".to_string()),
            team: Some("team".to_string()),
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
        };
        let artifact = dir.path().join("bundle.gtbundle");
        fs::write(&artifact, "artifact").expect("write artifact");

        let spec_path = write_single_vm_spec("Demo Bundle", &resolved, &request, &artifact)
            .expect("write spec");
        let snapshot = fs::read_to_string(&spec_path)
            .expect("read spec")
            .replace(&dir.path().display().to_string(), "<TMP>");

        unsafe {
            env::remove_var("XDG_STATE_HOME");
            env::remove_var("XDG_DATA_HOME");
        }

        assert_snapshot!(snapshot, @r###"
        apiVersion: greentic.ai/v1alpha1
        kind: Deployment
        metadata:
          name: tenant-team-demo-bundle
        spec:
          target: single-vm
          bundle:
            source: 'file://<TMP>/bundle.gtbundle'
            format: squashfs
          runtime:
            image: 'ghcr.io/greentic-ai/operator-distroless:0.1.0-distroless'
            arch: x86_64
            admin:
              bind: 127.0.0.1:8433
              mtls:
                caFile: '<TMP>/bundle/.greentic/admin/certs/ca.crt'
                certFile: '<TMP>/bundle/.greentic/admin/certs/server.crt'
                keyFile: '<TMP>/bundle/.greentic/admin/certs/server.key'
          storage:
            stateDir: '<TMP>/state-home/greentic/gtc/single-vm/demo-key/state'
            cacheDir: '<TMP>/state-home/greentic/gtc/single-vm/demo-key/cache'
            logDir: '<TMP>/state-home/greentic/gtc/single-vm/demo-key/log'
            tempDir: '<TMP>/state-home/greentic/gtc/single-vm/demo-key/tmp'
          service:
            manager: systemd
            user: greentic
            group: greentic
          health:
            readinessPath: /ready
            livenessPath: /health
            startupTimeoutSeconds: 120
          rollout:
            strategy: recreate
        "###);
    }

    proptest! {
        #[test]
        fn sanitize_identifier_only_emits_lowercase_alnum_and_hyphen(
            input in ".*"
        ) {
            let normalized = sanitize_identifier(&input);
            prop_assert!(normalized
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-'));
            prop_assert!(!normalized.starts_with('-'));
            prop_assert!(!normalized.ends_with('-'));
            prop_assert!(!normalized.contains("--"));
        }

        #[test]
        fn truncate_identifier_never_exceeds_limit(
            input in ".*",
            limit in 0usize..64
        ) {
            let truncated = truncate_identifier(&input, limit);
            prop_assert!(truncated.chars().count() <= limit);
        }
    }
}
