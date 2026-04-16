use super::{
    AdminRegistryDocument, DEV_BIN, StartTarget, admin_registry_path, build_cli, build_wizard_args,
    collect_tail, detect_bundle_root, detect_locale, ensure_admin_certs_ready, extract_tar_archive,
    fingerprint_bundle_dir, locale_from_args, normalize_bundle_fingerprint,
    normalize_expected_sha256, normalize_install_arch, parse_prompt_choice,
    parse_start_cli_options, parse_start_request, parse_stop_cli_options, parse_stop_request,
    remove_admin_registry_entry, resolve_admin_cert_dir,
    resolve_canonical_target_provider_pack_from, resolve_companion_binary_from,
    resolve_deploy_app_pack_path, resolve_local_mutable_bundle_dir, resolve_target_provider_pack,
    resolve_tenant_key, rewrite_store_tenant_placeholder, route_passthrough_subcommand,
    run_admin_access, run_admin_health, run_admin_token, run_admin_tunnel, save_admin_registry,
    select_start_target, should_send_auth_header, tenant_env_var_name, upsert_admin_registry_entry,
    verify_sha256_digest,
};
#[cfg(unix)]
use super::{
    StartBundleResolution, apply_default_deploy_env_for_target, extract_zip_bytes,
    validate_cloud_deploy_inputs, write_single_vm_spec,
};
use clap::{Arg, ArgMatches, Command};
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{Cursor, Write};
use std::net::{TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::thread;
#[cfg(unix)]
use tempfile::TempDir;
use tempfile::tempdir;

#[test]
fn locale_arg_is_detected_from_equals_flag() {
    let args = vec!["gtc".to_string(), "--locale=nl-NL".to_string()];
    assert_eq!(locale_from_args(&args), Some("nl-NL".to_string()));
}

#[test]
fn locale_arg_is_detected_from_split_flag() {
    let args = vec![
        "gtc".to_string(),
        "--locale".to_string(),
        "en-US".to_string(),
    ];
    assert_eq!(locale_from_args(&args), Some("en-US".to_string()));
}

#[test]
fn unsupported_locale_falls_back_to_en() {
    let args = vec!["gtc".to_string(), "--locale".to_string(), "xx".to_string()];
    assert_eq!(detect_locale(&args, "en"), "en");
}

#[test]
fn raw_passthrough_global_flags_match_cli_globals() {
    let cli = build_cli("en");
    let declared_globals: BTreeMap<String, bool> = cli
        .get_arguments()
        .filter(|arg| arg.is_global_set())
        .filter_map(|arg| Some((arg.get_long()?.to_string(), arg.get_action().takes_values())))
        .collect();

    let raw_router_globals: BTreeMap<String, bool> =
        crate::router::raw_passthrough_global_flag_specs()
            .iter()
            .map(|(name, takes_value)| ((*name).to_string(), *takes_value))
            .collect();

    assert_eq!(
        raw_router_globals, declared_globals,
        "raw passthrough parser must track every global CLI flag and whether it consumes a value"
    );
}

#[test]
fn collect_tail_reads_passthrough_args() {
    let matches: ArgMatches = Command::new("test")
        .arg(
            Arg::new("args")
                .num_args(0..)
                .trailing_var_arg(true)
                .allow_hyphen_values(true),
        )
        .try_get_matches_from(["test", "--a", "b"])
        .expect("matches");

    assert_eq!(
        collect_tail(&matches),
        vec!["--a".to_string(), "b".to_string()]
    );
}

#[test]
fn tenant_env_var_name_normalization_matches_contract() {
    assert_eq!(tenant_env_var_name("acme"), "GREENTIC_ACME_KEY");
    assert_eq!(tenant_env_var_name("acme-dev"), "GREENTIC_ACME_DEV_KEY");
    assert_eq!(
        tenant_env_var_name("Acme.Dev-01"),
        "GREENTIC_ACME_DEV_01_KEY"
    );
}

#[test]
fn key_resolution_prefers_cli_then_env() {
    let tenant = "acme";
    let locale = "en";
    let env_name = tenant_env_var_name(tenant);

    unsafe {
        std::env::set_var(&env_name, "env-token");
    }

    let from_cli = resolve_tenant_key(Some("cli-token".to_string()), tenant, locale).unwrap();
    assert_eq!(from_cli, "cli-token");

    let from_env = resolve_tenant_key(None, tenant, locale).unwrap();
    assert_eq!(from_env, "env-token");

    unsafe {
        std::env::remove_var(&env_name);
    }
}

#[test]
fn build_wizard_args_prepends_wizard_and_locale() {
    let args = build_wizard_args(&["--answers".to_string(), "a.json".to_string()], "en");
    assert_eq!(
        args,
        vec![
            "wizard".to_string(),
            "--locale".to_string(),
            "en".to_string(),
            "--answers".to_string(),
            "a.json".to_string()
        ]
    );
}

#[test]
fn build_wizard_args_preserves_explicit_locale() {
    let args = build_wizard_args(
        &[
            "--locale".to_string(),
            "fr".to_string(),
            "--answers".to_string(),
            "a.json".to_string(),
        ],
        "en",
    );
    assert_eq!(
        args,
        vec![
            "wizard".to_string(),
            "--locale".to_string(),
            "fr".to_string(),
            "--answers".to_string(),
            "a.json".to_string()
        ]
    );
}

#[test]
fn build_wizard_args_includes_schema_passthrough() {
    let args = build_wizard_args(&["--schema".to_string()], "en");
    assert_eq!(
        args,
        vec![
            "wizard".to_string(),
            "--locale".to_string(),
            "en".to_string(),
            "--schema".to_string()
        ]
    );
}

#[test]
fn route_passthrough_subcommand_routes_wizard_to_greentic_dev() {
    let tail = vec!["--help".to_string()];
    let (binary, args) = route_passthrough_subcommand("wizard", &tail, "en").expect("wizard route");

    assert_eq!(binary, DEV_BIN);
    assert_eq!(
        args,
        vec![
            "wizard".to_string(),
            "--locale".to_string(),
            "en".to_string(),
            "--help".to_string()
        ]
    );
}

#[test]
fn parse_start_request_maps_common_flags() {
    let request = parse_start_request(
        &[
            "--tenant".to_string(),
            "demo".to_string(),
            "--team=default".to_string(),
            "--nats".to_string(),
            "off".to_string(),
            "--cloudflared=off".to_string(),
            "--ngrok".to_string(),
            "off".to_string(),
            "--runner-binary".to_string(),
            "/tmp/runner".to_string(),
        ],
        PathBuf::from("/tmp/bundle"),
    )
    .expect("request");

    assert_eq!(request.bundle.as_deref(), Some("/tmp/bundle"));
    assert_eq!(request.tenant.as_deref(), Some("demo"));
    assert_eq!(request.team.as_deref(), Some("default"));
    assert_eq!(
        request.runner_binary.as_deref(),
        Some(Path::new("/tmp/runner"))
    );
}

#[test]
fn parse_start_request_maps_admin_flags() {
    let request = parse_start_request(
        &[
            "--admin".to_string(),
            "--admin-port".to_string(),
            "9443".to_string(),
            "--admin-certs-dir=/tmp/admin-certs".to_string(),
            "--admin-allowed-clients".to_string(),
            "CN=alice,CN=bob".to_string(),
        ],
        PathBuf::from("/tmp/bundle"),
    )
    .expect("request");

    assert!(request.admin);
    assert_eq!(request.admin_port, 9443);
    assert_eq!(
        request.admin_certs_dir.as_deref(),
        Some(Path::new("/tmp/admin-certs"))
    );
    assert_eq!(
        request.admin_allowed_clients,
        vec!["CN=alice".to_string(), "CN=bob".to_string()]
    );
}

#[test]
fn resolve_companion_binary_uses_env_override() {
    let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::set_var("GREENTIC_DEV_BIN", "/tmp/custom-dev");
    }
    let resolved =
        resolve_companion_binary_from(Some(Path::new("/tmp/gtc")), DEV_BIN).expect("path");
    assert_eq!(resolved, PathBuf::from("/tmp/custom-dev"));
    unsafe {
        std::env::remove_var("GREENTIC_DEV_BIN");
    }
}

#[test]
fn resolve_companion_binary_falls_back_to_cargo_home_bin() {
    let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
    let temp = tempfile::tempdir().expect("tempdir");
    let cargo_home = temp.path().join("cargo-home");
    let cargo_bin = cargo_home.join("bin");
    let cargo_binary = cargo_bin.join(DEV_BIN);
    std::fs::create_dir_all(&cargo_bin).expect("mkdir cargo bin");
    std::fs::write(&cargo_binary, b"").expect("write cargo binary");

    unsafe {
        std::env::remove_var("GREENTIC_DEV_BIN");
        std::env::set_var("CARGO_HOME", &cargo_home);
    }

    let resolved = resolve_companion_binary_from(None, DEV_BIN).expect("path");
    assert_eq!(resolved, cargo_binary);

    unsafe {
        std::env::remove_var("CARGO_HOME");
    }
}

#[test]
fn resolve_companion_binary_falls_back_to_sibling_binary() {
    let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
    let temp = tempfile::tempdir().expect("tempdir");
    let exe_dir = temp.path().join("bin");
    std::fs::create_dir_all(&exe_dir).expect("mkdir");
    let current_exe = exe_dir.join("gtc");
    let sibling = exe_dir.join(DEV_BIN);
    std::fs::write(&current_exe, b"").expect("write gtc");
    std::fs::write(&sibling, b"").expect("write companion");

    unsafe {
        std::env::remove_var("GREENTIC_DEV_BIN");
    }
    let resolved =
        resolve_companion_binary_from(Some(current_exe.as_path()), DEV_BIN).expect("path");
    assert_eq!(resolved, sibling);
}

#[test]
fn resolve_companion_binary_falls_back_to_workspace_local_binary() {
    let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path();
    let current_exe = workspace
        .join("greentic")
        .join("target")
        .join("debug")
        .join("gtc");
    let companion = workspace
        .join("greentic-dev")
        .join("target")
        .join("debug")
        .join(DEV_BIN);
    std::fs::create_dir_all(current_exe.parent().expect("gtc dir")).expect("mkdir gtc");
    std::fs::create_dir_all(companion.parent().expect("dev dir")).expect("mkdir dev");
    std::fs::write(&current_exe, b"").expect("write gtc");
    std::fs::write(&companion, b"").expect("write companion");

    unsafe {
        std::env::remove_var("GREENTIC_DEV_BIN");
    }
    let resolved =
        resolve_companion_binary_from(Some(current_exe.as_path()), DEV_BIN).expect("path");
    assert_eq!(resolved, companion);
}

#[test]
fn parse_start_request_rejects_bundle_override() {
    let err = parse_start_request(
        &["--bundle".to_string(), "/tmp/other".to_string()],
        PathBuf::from("/tmp/bundle"),
    )
    .unwrap_err();
    assert!(err.contains("--bundle is managed by gtc start"));
}

#[test]
fn parse_start_cli_options_strips_deploy_flags() {
    let options = parse_start_cli_options(&[
        "--target".to_string(),
        "aws".to_string(),
        "--deploy-bundle-source".to_string(),
        "https://example.com/demo.gtbundle".to_string(),
        "--environment=prod".to_string(),
        "--provider-pack".to_string(),
        "/tmp/provider.gtpack".to_string(),
        "--app-pack=/tmp/app.gtpack".to_string(),
        "--tenant".to_string(),
        "demo".to_string(),
    ])
    .expect("options");

    assert_eq!(
        options.explicit_target.map(|value| value.as_str()),
        Some("aws")
    );
    assert_eq!(
        options.deploy_bundle_source.as_deref(),
        Some("https://example.com/demo.gtbundle")
    );
    assert_eq!(options.environment.as_deref(), Some("prod"));
    assert_eq!(
        options.provider_pack.as_deref(),
        Some(Path::new("/tmp/provider.gtpack"))
    );
    assert_eq!(
        options.app_pack.as_deref(),
        Some(Path::new("/tmp/app.gtpack"))
    );
    assert_eq!(
        options.start_args,
        vec!["--tenant".to_string(), "demo".to_string()]
    );
}

#[test]
#[cfg(unix)]
fn validate_cloud_deploy_inputs_accepts_aws_remote_bundle_when_required_envs_present() {
    let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
    let _path_guard = temp_path_with_binary("terraform");
    let _deployer = fake_deployer_contract(None);
    let bundle_dir = TempDir::new().expect("tempdir");
    clear_aws_credential_env();
    unsafe {
        env::set_var(
            "GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE_DIGEST",
            "sha256:test",
        );
        env::set_var(
            "GREENTIC_DEPLOY_TERRAFORM_VAR_REMOTE_STATE_BACKEND",
            "s3://bucket/state",
        );
        env::set_var("AWS_PROFILE", "demo");
    }

    let result = validate_cloud_deploy_inputs(
        StartTarget::Aws,
        Some("https://example.com/demo.gtbundle"),
        bundle_dir.path(),
        "en",
    );

    unsafe {
        env::remove_var("GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE_DIGEST");
        env::remove_var("GREENTIC_DEPLOY_TERRAFORM_VAR_REMOTE_STATE_BACKEND");
    }
    clear_aws_credential_env();

    assert!(result.is_ok());
}

#[test]
#[cfg(unix)]
fn validate_cloud_deploy_inputs_rejects_local_bundle_for_aws() {
    let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
    let _path_guard = temp_path_with_binary("terraform");
    let _deployer = fake_deployer_contract(None);
    let bundle_dir = TempDir::new().expect("tempdir");
    clear_aws_credential_env();
    unsafe {
        env::set_var(
            "GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE_DIGEST",
            "sha256:test",
        );
        env::set_var(
            "GREENTIC_DEPLOY_TERRAFORM_VAR_REMOTE_STATE_BACKEND",
            "s3://bucket/state",
        );
        env::set_var("AWS_PROFILE", "demo");
    }

    let err = validate_cloud_deploy_inputs(
        StartTarget::Aws,
        Some("./demo.gtbundle"),
        bundle_dir.path(),
        "en",
    )
    .err()
    .expect("expected validation failure");

    unsafe {
        env::remove_var("GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE_DIGEST");
        env::remove_var("GREENTIC_DEPLOY_TERRAFORM_VAR_REMOTE_STATE_BACKEND");
    }
    clear_aws_credential_env();

    assert!(err.contains("aws deploy requires a remote bundle source"));
}

#[test]
#[cfg(unix)]
fn validate_cloud_deploy_inputs_does_not_accept_partial_aws_access_key_env() {
    let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
    let _path_guard = temp_path_with_binary("terraform");
    let _deployer = fake_deployer_contract(None);
    let bundle_dir = TempDir::new().expect("tempdir");
    clear_aws_credential_env();
    unsafe {
        env::set_var(
            "GREENTIC_DEPLOY_TERRAFORM_VAR_REMOTE_STATE_BACKEND",
            "s3://bucket/state",
        );
        env::set_var("AWS_ACCESS_KEY_ID", "demo");
        env::remove_var("AWS_SECRET_ACCESS_KEY");
        env::remove_var("AWS_PROFILE");
        env::remove_var("AWS_DEFAULT_PROFILE");
        env::remove_var("AWS_WEB_IDENTITY_TOKEN_FILE");
    }

    let err = validate_cloud_deploy_inputs(
        StartTarget::Aws,
        Some("https://example.com/demo.gtbundle"),
        bundle_dir.path(),
        "en",
    )
    .err()
    .expect("expected validation failure");

    unsafe {
        env::remove_var("GREENTIC_DEPLOY_TERRAFORM_VAR_REMOTE_STATE_BACKEND");
    }
    clear_aws_credential_env();

    assert!(err.contains("missing cloud credentials"));
}

#[cfg(unix)]
fn clear_aws_credential_env() {
    unsafe {
        env::remove_var("AWS_ACCESS_KEY_ID");
        env::remove_var("AWS_SECRET_ACCESS_KEY");
        env::remove_var("AWS_SESSION_TOKEN");
        env::remove_var("AWS_PROFILE");
        env::remove_var("AWS_DEFAULT_PROFILE");
        env::remove_var("AWS_WEB_IDENTITY_TOKEN_FILE");
        env::remove_var("AWS_ROLE_ARN");
    }
}

pub(crate) fn env_test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[cfg(unix)]
struct PathGuard {
    _temp_dir: tempfile::TempDir,
    original: Option<std::ffi::OsString>,
}

#[cfg(unix)]
impl Drop for PathGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.original {
                Some(value) => env::set_var("PATH", value),
                None => env::remove_var("PATH"),
            }
        }
    }
}

#[cfg(unix)]
fn temp_path_with_binary(binary: &str) -> PathGuard {
    let dir = tempdir().expect("tempdir");
    let script = dir.path().join(binary);
    fs::write(&script, "#!/bin/sh\nexit 0\n").expect("write shim");
    let mut perms = fs::metadata(&script).expect("metadata").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&script, perms).expect("chmod");
    let original = env::var_os("PATH");
    let mut merged = dir.path().display().to_string();
    if let Some(existing) = &original {
        merged.push(':');
        merged.push_str(&existing.to_string_lossy());
    }
    unsafe {
        env::set_var("PATH", merged);
    }
    PathGuard {
        _temp_dir: dir,
        original,
    }
}

#[cfg(unix)]
pub(crate) struct EnvVarGuard {
    name: &'static str,
    original: Option<std::ffi::OsString>,
}

#[cfg(unix)]
pub(crate) struct FakeDeployerGuard {
    _temp_dir: tempfile::TempDir,
    _env_guard: EnvVarGuard,
}

#[cfg(unix)]
impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.original {
                Some(value) => env::set_var(self.name, value),
                None => env::remove_var(self.name),
            }
        }
    }
}

#[cfg(unix)]
pub(crate) fn write_fake_deployer_contract_script(script: &Path, log_path: Option<&Path>) {
    let log_snippet = log_path.map_or_else(String::new, |path| {
        format!("printf '%s\\n' \"$*\" >> '{}'\n", path.display())
    });
    let body_template = r#"#!/bin/sh
__LOG_SNIPPET__if [ "$1" = "target-requirements" ] && [ "$2" = "--provider" ]; then
  provider="$3"
  case "$provider" in
    aws)
      label="AWS"
      help="Pass --deploy-bundle-source https://.../bundle.gtbundle or set GREENTIC_DEPLOY_BUNDLE_SOURCE"
      creds='[{"label":"AWS credentials","env_vars":["AWS_ACCESS_KEY_ID","AWS_SECRET_ACCESS_KEY","AWS_PROFILE","AWS_DEFAULT_PROFILE","AWS_WEB_IDENTITY_TOKEN_FILE","AWS_ROLE_ARN"],"satisfaction_env_groups":[["AWS_PROFILE"],["AWS_DEFAULT_PROFILE"],["AWS_ACCESS_KEY_ID","AWS_SECRET_ACCESS_KEY"],["AWS_WEB_IDENTITY_TOKEN_FILE","AWS_ROLE_ARN"]],"prompt_fields":[],"help":"AWS credentials"}]'
      ;;
    gcp)
      label="GCP"
      help="Pass --deploy-bundle-source https://.../bundle.gtbundle or set GREENTIC_DEPLOY_BUNDLE_SOURCE"
      creds='[{"label":"GCP credentials","env_vars":["GOOGLE_APPLICATION_CREDENTIALS","GOOGLE_OAUTH_ACCESS_TOKEN","CLOUDSDK_AUTH_ACCESS_TOKEN"],"satisfaction_env_groups":[["GOOGLE_APPLICATION_CREDENTIALS"],["GOOGLE_OAUTH_ACCESS_TOKEN"],["CLOUDSDK_AUTH_ACCESS_TOKEN"]],"prompt_fields":[],"help":"GCP credentials"}]'
      ;;
    azure)
      label="Azure"
      help="Pass --deploy-bundle-source https://.../bundle.gtbundle or set GREENTIC_DEPLOY_BUNDLE_SOURCE"
      creds='[{"label":"Azure credentials","env_vars":["ARM_CLIENT_ID","ARM_TENANT_ID","ARM_SUBSCRIPTION_ID","ARM_USE_OIDC","AZURE_CLIENT_ID","AZURE_TENANT_ID","AZURE_SUBSCRIPTION_ID"],"satisfaction_env_groups":[["ARM_CLIENT_ID","ARM_TENANT_ID","ARM_SUBSCRIPTION_ID","ARM_USE_OIDC"],["AZURE_CLIENT_ID","AZURE_TENANT_ID","AZURE_SUBSCRIPTION_ID"]],"prompt_fields":[],"help":"Azure credentials"}]'
      ;;
    *)
      echo "unknown provider: $provider" >&2
      exit 1
      ;;
  esac

  case "$provider" in
    aws)
      vars="[{\"name\":\"GREENTIC_DEPLOY_TERRAFORM_VAR_REMOTE_STATE_BACKEND\",\"required\":true,\"prompt\":null,\"default_value\":null}]"
      ;;
    gcp)
      vars="[{\"name\":\"GREENTIC_DEPLOY_TERRAFORM_VAR_REMOTE_STATE_BACKEND\",\"required\":true,\"prompt\":null,\"default_value\":null},{\"name\":\"GREENTIC_DEPLOY_TERRAFORM_VAR_GCP_PROJECT_ID\",\"required\":true,\"prompt\":null,\"default_value\":null},{\"name\":\"GREENTIC_DEPLOY_TERRAFORM_VAR_GCP_REGION\",\"required\":true,\"prompt\":null,\"default_value\":\"us-central1\"}]"
      ;;
    azure)
      vars="[{\"name\":\"GREENTIC_DEPLOY_TERRAFORM_VAR_REMOTE_STATE_BACKEND\",\"required\":true,\"prompt\":null,\"default_value\":null}]"
      ;;
  esac
  printf '%s\n' "{\"target\":\"$provider\",\"target_label\":\"$label\",\"provider_pack_filename\":\"terraform.gtpack\",\"remote_bundle_source_required\":true,\"remote_bundle_source_help\":\"$help\",\"informational_notes\":[],\"credential_requirements\":$creds,\"variable_requirements\":$vars}"
  exit 0
fi
if [ "$1" = "single-vm" ] && [ "$2" = "render-spec" ]; then
  shift 2
  while [ $# -gt 0 ]; do
    case "$1" in
      --out) out="$2"; shift 2 ;;
      --name) name="$2"; shift 2 ;;
      --bundle-source) bundle_source="$2"; shift 2 ;;
      --state-dir) state_dir="$2"; shift 2 ;;
      --cache-dir) cache_dir="$2"; shift 2 ;;
      --log-dir) log_dir="$2"; shift 2 ;;
      --temp-dir) temp_dir="$2"; shift 2 ;;
      --admin-bind) admin_bind="$2"; shift 2 ;;
      --admin-ca-file) admin_ca_file="$2"; shift 2 ;;
      --admin-cert-file) admin_cert_file="$2"; shift 2 ;;
      --admin-key-file) admin_key_file="$2"; shift 2 ;;
      --image) image="$2"; shift 2 ;;
      *) shift ;;
    esac
  done
  : "${admin_bind:=127.0.0.1:8433}"
  : "${image:=ghcr.io/greentic-ai/operator-distroless:0.1.0-distroless}"
  {
    printf '%s\n' "apiVersion: greentic.ai/v1alpha1"
    printf '%s\n' "kind: Deployment"
    printf '%s\n' "metadata:"
    printf '%s\n' "  name: $name"
    printf '%s\n' "spec:"
    printf '%s\n' "  target: single-vm"
    printf '%s\n' "  bundle:"
    printf '%s\n' "    source: '$bundle_source'"
    printf '%s\n' "    format: squashfs"
    printf '%s\n' "  runtime:"
    printf '%s\n' "    image: '$image'"
    printf '%s\n' "    arch: x86_64"
    printf '%s\n' "    admin:"
    printf '%s\n' "      bind: $admin_bind"
    printf '%s\n' "      mtls:"
    printf '%s\n' "        caFile: '$admin_ca_file'"
    printf '%s\n' "        certFile: '$admin_cert_file'"
    printf '%s\n' "        keyFile: '$admin_key_file'"
    printf '%s\n' "  storage:"
    printf '%s\n' "    stateDir: '$state_dir'"
    printf '%s\n' "    cacheDir: '$cache_dir'"
    printf '%s\n' "    logDir: '$log_dir'"
    printf '%s\n' "    tempDir: '$temp_dir'"
    printf '%s\n' "  service:"
    printf '%s\n' "    manager: systemd"
    printf '%s\n' "    user: greentic"
    printf '%s\n' "    group: greentic"
    printf '%s\n' "  health:"
    printf '%s\n' "    readinessPath: /ready"
    printf '%s\n' "    livenessPath: /health"
    printf '%s\n' "    startupTimeoutSeconds: 120"
    printf '%s\n' "  rollout:"
    printf '%s\n' "    strategy: recreate"
  } > "$out"
  exit 0
fi
exit 0
"#;
    let body = body_template.replace("__LOG_SNIPPET__", &log_snippet);
    fs::write(script, body).expect("write fake deployer");
    let mut perms = fs::metadata(script).expect("metadata").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(script, perms).expect("chmod");
}

#[cfg(unix)]
pub(crate) fn fake_deployer_contract(log_path: Option<&Path>) -> FakeDeployerGuard {
    let dir = tempdir().expect("tempdir");
    let script = dir.path().join("greentic-deployer");
    write_fake_deployer_contract_script(&script, log_path);

    let original = env::var_os("GREENTIC_DEPLOYER_BIN");
    unsafe {
        env::set_var("GREENTIC_DEPLOYER_BIN", &script);
    }

    FakeDeployerGuard {
        _temp_dir: dir,
        _env_guard: EnvVarGuard {
            name: "GREENTIC_DEPLOYER_BIN",
            original,
        },
    }
}

#[test]
fn parse_stop_request_maps_common_flags() {
    let request = parse_stop_request(
        &[
            "--tenant".to_string(),
            "demo".to_string(),
            "--team=default".to_string(),
            "--state-dir".to_string(),
            "/tmp/state".to_string(),
        ],
        PathBuf::from("/tmp/bundle"),
    )
    .expect("request");

    assert_eq!(request.bundle.as_deref(), Some("/tmp/bundle"));
    assert_eq!(request.tenant, "demo");
    assert_eq!(request.team, "default");
    assert_eq!(request.state_dir.as_deref(), Some(Path::new("/tmp/state")));
}

#[test]
fn parse_stop_cli_options_strips_destroy_flags() {
    let options = parse_stop_cli_options(&[
        "--destroy".to_string(),
        "--target".to_string(),
        "aws".to_string(),
        "--environment=prod".to_string(),
        "--provider-pack".to_string(),
        "/tmp/provider.gtpack".to_string(),
        "--app-pack=/tmp/app.gtpack".to_string(),
        "--tenant".to_string(),
        "demo".to_string(),
    ])
    .expect("options");

    assert!(options.destroy);
    assert_eq!(
        options.explicit_target.map(|value| value.as_str()),
        Some("aws")
    );
    assert_eq!(options.environment.as_deref(), Some("prod"));
    assert_eq!(
        options.provider_pack.as_deref(),
        Some(Path::new("/tmp/provider.gtpack"))
    );
    assert_eq!(
        options.app_pack.as_deref(),
        Some(Path::new("/tmp/app.gtpack"))
    );
    assert_eq!(
        options.stop_args,
        vec!["--tenant".to_string(), "demo".to_string()]
    );
}

#[test]
fn select_start_target_defaults_to_runtime_without_deployer_targets() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = select_start_target(dir.path(), None, "en").expect("target");
    assert_eq!(target.as_str(), "runtime");
}

#[test]
fn select_start_target_prefers_single_explicit_deployer_target() {
    let dir = tempfile::tempdir().expect("tempdir");
    let greentic_dir = dir.path().join(".greentic");
    std::fs::create_dir_all(&greentic_dir).expect("create .greentic");
    std::fs::write(
        greentic_dir.join("deployment-targets.json"),
        r#"{"targets":[{"target":"aws","provider_pack":"packs/aws.gtpack","default":true}]}"#,
    )
    .expect("write targets");

    let target = select_start_target(dir.path(), None, "en").expect("target");
    assert_eq!(target.as_str(), "aws");
}

#[test]
fn select_start_target_uses_default_from_metadata() {
    let dir = tempfile::tempdir().expect("tempdir");
    let greentic_dir = dir.path().join(".greentic");
    std::fs::create_dir_all(&greentic_dir).expect("create .greentic");
    std::fs::write(
        greentic_dir.join("deployment-targets.json"),
        r#"{"targets":[
                {"target":"aws","provider_pack":"packs/aws.gtpack"},
                {"target":"gcp","provider_pack":"packs/gcp.gtpack","default":true}
            ]}"#,
    )
    .expect("write targets");

    let target = select_start_target(dir.path(), None, "en").expect("target");
    assert_eq!(target.as_str(), "gcp");
}

#[test]
fn resolve_target_provider_pack_reads_metadata() {
    let dir = tempfile::tempdir().expect("tempdir");
    let greentic_dir = dir.path().join(".greentic");
    std::fs::create_dir_all(&greentic_dir).expect("create .greentic");
    let expected = dir.path().join("packs").join("gcp.gtpack");
    std::fs::create_dir_all(expected.parent().expect("parent")).expect("mkdir");
    std::fs::write(&expected, b"fixture").expect("write provider");
    std::fs::write(
        greentic_dir.join("deployment-targets.json"),
        r#"{"targets":[{"target":"gcp","provider_pack":"packs/gcp.gtpack","default":true}]}"#,
    )
    .expect("write targets");

    let resolved =
        resolve_target_provider_pack(dir.path(), StartTarget::Gcp, None).expect("provider");
    assert_eq!(resolved, expected);
}

#[test]
fn resolve_canonical_target_provider_pack_from_workspace_deployer_dist() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let repo_dir = workspace.path().join("greentic-deployer");
    let exe_dir = repo_dir.join("target").join("debug");
    let deployer_bin = exe_dir.join("greentic-deployer");
    let dist_pack = repo_dir.join("dist").join("terraform.gtpack");
    std::fs::create_dir_all(&exe_dir).expect("mkdir exe dir");
    std::fs::create_dir_all(dist_pack.parent().expect("dist parent")).expect("mkdir dist");
    std::fs::write(&deployer_bin, b"").expect("write deployer");
    std::fs::write(&dist_pack, b"").expect("write pack");

    let resolved = resolve_canonical_target_provider_pack_from(
        Some(deployer_bin.as_path()),
        "terraform.gtpack",
    )
    .expect("canonical pack");
    assert_eq!(resolved, dist_pack);
}

#[test]
fn resolve_admin_cert_dir_prefers_bundle_local_admin_certs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let certs = dir.path().join(".greentic").join("admin").join("certs");
    std::fs::create_dir_all(&certs).expect("mkdir");
    std::fs::write(certs.join("ca.crt"), b"ca").expect("ca");
    std::fs::write(certs.join("server.crt"), b"cert").expect("cert");
    std::fs::write(certs.join("server.key"), b"key").expect("key");

    let resolved = resolve_admin_cert_dir(dir.path());
    assert_eq!(resolved, certs);
}

#[test]
fn resolve_admin_cert_dir_falls_back_to_bundle_certs_dir() {
    let dir = tempfile::tempdir().expect("tempdir");
    let certs = dir.path().join("certs");
    std::fs::create_dir_all(&certs).expect("mkdir");
    std::fs::write(certs.join("ca.crt"), b"ca").expect("ca");
    std::fs::write(certs.join("server.crt"), b"cert").expect("cert");
    std::fs::write(certs.join("server.key"), b"key").expect("key");

    let resolved = resolve_admin_cert_dir(dir.path());
    assert_eq!(resolved, certs);
}

#[test]
fn ensure_admin_certs_ready_generates_bundle_local_certs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let resolved = ensure_admin_certs_ready(dir.path(), None).expect("certs");
    assert_eq!(
        resolved,
        dir.path().join(".greentic").join("admin").join("certs")
    );
    assert!(resolved.join("ca.crt").exists());
    assert!(resolved.join("server.crt").exists());
    assert!(resolved.join("server.key").exists());
    assert!(resolved.join("client.crt").exists());
    assert!(resolved.join("client.key").exists());
}

#[test]
fn ensure_admin_certs_ready_preserves_explicit_dir() {
    let dir = tempfile::tempdir().expect("tempdir");
    let certs = dir.path().join("custom-certs");
    std::fs::create_dir_all(&certs).expect("mkdir");
    std::fs::write(certs.join("ca.crt"), b"ca").expect("ca");
    std::fs::write(certs.join("server.crt"), b"cert").expect("cert");
    std::fs::write(certs.join("server.key"), b"key").expect("key");

    let resolved = ensure_admin_certs_ready(dir.path(), Some(&certs)).expect("certs");
    assert_eq!(resolved, certs);
}

#[cfg(unix)]
#[test]
fn write_single_vm_spec_uses_bundle_local_server_certs() {
    let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
    let _deployer_guard = fake_deployer_contract(None);
    let dir = tempfile::tempdir().expect("tempdir");
    let state_home = dir.path().join("xdg-state");
    std::fs::create_dir_all(&state_home).expect("mkdir state home");
    unsafe {
        env::set_var("XDG_STATE_HOME", &state_home);
    }
    let bundle_dir = dir.path().join("bundle");
    let certs = bundle_dir.join(".greentic").join("admin").join("certs");
    std::fs::create_dir_all(&certs).expect("mkdir certs");
    std::fs::write(certs.join("ca.crt"), b"ca").expect("ca");
    std::fs::write(certs.join("server.crt"), b"server cert").expect("server cert");
    std::fs::write(certs.join("server.key"), b"server key").expect("server key");

    let artifact_path = dir.path().join("bundle.gtbundle");
    std::fs::write(&artifact_path, b"fixture").expect("artifact");

    let resolved = StartBundleResolution {
        bundle_dir: bundle_dir.clone(),
        deployment_key: "demo-deploy".to_string(),
        deploy_artifact: Some(artifact_path.clone()),
        _hold: None,
    };
    let request = parse_start_request(
        &[
            "--tenant".to_string(),
            "demo".to_string(),
            "--team".to_string(),
            "default".to_string(),
        ],
        bundle_dir.clone(),
    )
    .expect("request");

    let spec_path = write_single_vm_spec(
        "demo-bundle",
        &resolved,
        &request,
        &artifact_path,
        false,
        "en",
    )
    .expect("spec");
    let spec = std::fs::read_to_string(&spec_path).expect("read spec");

    assert!(spec.contains("source: 'file://"));
    assert!(spec.contains(&artifact_path.display().to_string()));
    assert!(spec.contains("certFile: '"));
    assert!(spec.contains(".greentic/admin/certs/server.crt"));
    assert!(spec.contains("keyFile: '"));
    assert!(spec.contains(".greentic/admin/certs/server.key"));
    assert!(!spec.contains("client.crt"));
    assert!(!spec.contains("client.key"));

    unsafe {
        env::remove_var("XDG_STATE_HOME");
    }
}

#[cfg(unix)]
#[test]
fn resolve_deploy_app_pack_path_prefers_bundle_metadata_app_pack() {
    let dir = tempfile::tempdir().expect("tempdir");
    let packs_dir = dir.path().join("packs");
    std::fs::create_dir_all(&packs_dir).expect("create packs dir");
    let app_pack = packs_dir.join("cards-demo.gtpack");
    let other_pack = packs_dir.join("default.gtpack");
    std::fs::write(&app_pack, b"app").expect("write app pack");
    std::fs::write(&other_pack, b"other").expect("write other pack");
    std::fs::write(
        dir.path().join("bundle.yaml"),
        "bundle_id: demo\napp_packs:\n  - /tmp/build/cards-demo.gtpack\n",
    )
    .expect("write bundle");

    let resolved = resolve_deploy_app_pack_path(dir.path(), None).expect("app pack");
    assert_eq!(resolved, app_pack);
}

#[test]
fn resolve_deploy_app_pack_path_prefers_bundle_metadata_remote_app_pack() {
    let dir = tempfile::tempdir().expect("tempdir");
    let packs_dir = dir.path().join("packs");
    std::fs::create_dir_all(&packs_dir).expect("create packs dir");
    let app_pack = packs_dir.join("cards-demo.gtpack");
    let other_pack = packs_dir.join("default.gtpack");
    std::fs::write(&app_pack, b"app").expect("write app pack");
    std::fs::write(&other_pack, b"other").expect("write other pack");
    std::fs::write(
            dir.path().join("bundle.yaml"),
            "bundle_id: demo\napp_packs:\n  - https://example.com/releases/latest/download/cards-demo.gtpack\n",
        )
        .expect("write bundle");

    let resolved = resolve_deploy_app_pack_path(dir.path(), None).expect("app pack");
    assert_eq!(resolved, app_pack);
}

#[test]
fn resolve_deploy_app_pack_path_rejects_multiple_candidates_without_canonical_metadata() {
    let dir = tempfile::tempdir().expect("tempdir");
    let packs_dir = dir.path().join("packs");
    std::fs::create_dir_all(&packs_dir).expect("create packs dir");
    std::fs::write(packs_dir.join("cards-demo.gtpack"), b"app").expect("write app pack");
    std::fs::write(packs_dir.join("other.gtpack"), b"other").expect("write other pack");

    let err = resolve_deploy_app_pack_path(dir.path(), None).unwrap_err();
    assert!(err.contains("cloud deployment requires a canonical app pack"));
}

#[test]
fn detect_bundle_root_accepts_normalized_bundle_at_root() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("bundle.yaml"), "bundle_id: demo\n").expect("bundle");
    std::fs::create_dir_all(dir.path().join("resolved")).expect("resolved");

    assert_eq!(detect_bundle_root(dir.path()), dir.path());
}

#[test]
fn detect_bundle_root_accepts_single_nested_normalized_bundle() {
    let dir = tempfile::tempdir().expect("tempdir");
    let bundle_root = dir.path().join("squashfs-root");
    std::fs::create_dir_all(bundle_root.join("resolved")).expect("resolved");
    std::fs::write(bundle_root.join("bundle.yaml"), "bundle_id: demo\n").expect("bundle");

    assert_eq!(detect_bundle_root(dir.path()), bundle_root);
}

#[test]
fn normalize_bundle_fingerprint_ignores_runtime_noise() {
    let normalized = normalize_bundle_fingerprint(
        "dir:.greentic\n\
             dir:.greentic/dev\n\
             file:.greentic/dev/.dev.secrets.env:1009:1773411510\n\
             dir:logs\n\
             file:logs/operator.log:0:1773411782\n\
             dir:state\n\
             dir:state/logs\n\
             file:state/logs/runtime.log:123:1773411782\n\
             dir:state/pids\n\
             file:state/pids/operator.pid:5:1773411782\n\
             dir:packs\n\
             file:packs/cards-demo.gtpack:5267324:1773411414\n\
             file:state/config/platform/static-routes.json:206:1773411510",
    );

    assert_eq!(
        normalized,
        "dir:.greentic\n\
dir:state\n\
dir:packs\n\
file:packs/cards-demo.gtpack:5267324\n\
file:state/config/platform/static-routes.json:206"
    );
}

#[test]
fn fingerprint_bundle_dir_ignores_runtime_noise_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".greentic/dev")).expect("create dev dir");
    std::fs::create_dir_all(dir.path().join("logs")).expect("create logs dir");
    std::fs::create_dir_all(dir.path().join("state/logs")).expect("create state logs dir");
    std::fs::create_dir_all(dir.path().join("packs")).expect("create packs dir");
    std::fs::write(
        dir.path().join(".greentic/dev/.dev.secrets.env"),
        "SECRET=demo\n",
    )
    .expect("write dev secrets");
    std::fs::write(dir.path().join("logs/operator.log"), "").expect("write operator log");
    std::fs::write(dir.path().join("state/logs/runtime.log"), "runtime\n")
        .expect("write runtime log");
    std::fs::write(dir.path().join("packs/cards-demo.gtpack"), "fixture").expect("write app pack");

    let fingerprint = fingerprint_bundle_dir(dir.path()).expect("fingerprint");

    assert!(fingerprint.contains("file:packs/cards-demo.gtpack"));
    assert!(!fingerprint.contains(".greentic/dev/.dev.secrets.env"));
    assert!(!fingerprint.contains("logs/operator.log"));
    assert!(!fingerprint.contains("state/logs/runtime.log"));
}

#[test]
fn normalize_bundle_fingerprint_ignores_file_mtime() {
    let before = "file:packs/cards-demo.gtpack:5267324:1773411414";
    let after = "file:packs/cards-demo.gtpack:5267324:1773516076";

    assert_eq!(
        normalize_bundle_fingerprint(before),
        normalize_bundle_fingerprint(after)
    );
}

#[test]
fn admin_registry_roundtrip_and_remove_by_cn() {
    let dir = tempdir().expect("tempdir");
    let mut registry = AdminRegistryDocument { admins: Vec::new() };

    upsert_admin_registry_entry(
        &mut registry,
        Some("alice".to_string()),
        "CN=alice".to_string(),
        "ssh-ed25519 AAAA alice".to_string(),
    );
    assert_eq!(registry.admins.len(), 1);
    assert_eq!(registry.admins[0].name.as_deref(), Some("alice"));

    save_admin_registry(dir.path(), &registry).expect("save registry");
    let raw = std::fs::read_to_string(admin_registry_path(dir.path())).expect("read registry");
    assert!(raw.contains("CN=alice"));

    let removed = remove_admin_registry_entry(&mut registry, Some("CN=alice"), None);
    assert!(removed);
    assert!(registry.admins.is_empty());
}

#[test]
fn resolve_local_mutable_bundle_dir_rejects_archive_paths() {
    let dir = tempdir().expect("tempdir");
    let archive = dir.path().join("bundle.gtbundle");
    std::fs::write(&archive, b"fixture").expect("archive");

    let err = resolve_local_mutable_bundle_dir(archive.to_str().expect("utf8")).unwrap_err();
    assert!(err.contains("local bundle directory"));
}

#[test]
fn normalize_install_arch_maps_common_aliases() {
    assert_eq!(normalize_install_arch("arm64"), Some("aarch64"));
    assert_eq!(normalize_install_arch("aarch64"), Some("aarch64"));
    assert_eq!(normalize_install_arch("amd64"), Some("x86_64"));
    assert_eq!(normalize_install_arch("x86_64"), Some("x86_64"));
}

#[test]
fn normalize_install_arch_rejects_unknown_values() {
    assert_eq!(normalize_install_arch("armv7"), None);
}

#[test]
fn rewrite_store_tenant_placeholder_substitutes_template_segment() {
    let input = "store://greentic-biz/{tenant}/providers/routing-hook/fast2flow.gtpack:latest";
    let output = rewrite_store_tenant_placeholder(input, "3point");
    assert_eq!(
        output,
        "store://greentic-biz/3point/providers/routing-hook/fast2flow.gtpack:latest"
    );
}

#[test]
#[cfg(unix)]
fn apply_default_deploy_env_for_target_does_not_inject_deployer_defaults() {
    let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
    let _deployer = fake_deployer_contract(None);
    unsafe {
        env::set_var(
            "GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE",
            "custom/image@sha256:deadbeef",
        );
        env::remove_var("GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE_DIGEST");
    }
    let mut process = ProcessCommand::new("env");

    apply_default_deploy_env_for_target(&mut process, Some(StartTarget::Gcp), "en")
        .expect("default deploy env");

    let envs: HashMap<_, _> = process
        .get_envs()
        .filter_map(|(key, value)| {
            Some((
                key.to_string_lossy().to_string(),
                value?.to_string_lossy().to_string(),
            ))
        })
        .collect();
    assert!(
        !envs.contains_key("GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE"),
        "gtc should not inject deployer-owned image defaults"
    );
    assert_eq!(
        envs.get("GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE_DIGEST")
            .map(String::as_str),
        None
    );

    unsafe {
        env::remove_var("GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE");
        env::remove_var("GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE_DIGEST");
    }
}

#[test]
fn build_cli_can_be_constructed_multiple_times_with_localized_strings() {
    let first = super::build_cli("en");
    let second = super::build_cli("en");

    assert_eq!(first.get_name(), "gtc");
    assert_eq!(second.get_name(), "gtc");
}

#[test]
fn admin_tunnel_rejects_non_aws_target() {
    let matches = Command::new("test")
        .arg(Arg::new("bundle-ref").required(true))
        .arg(Arg::new("target").long("target").num_args(1))
        .arg(
            Arg::new("local-port")
                .long("local-port")
                .default_value("8443"),
        )
        .arg(
            Arg::new("container")
                .long("container")
                .default_value("greentic-admin"),
        )
        .try_get_matches_from(["test", "./bundle", "--target", "gcp"])
        .expect("matches");

    assert_eq!(run_admin_tunnel(&matches, "en"), 2);
}

#[cfg(unix)]
#[test]
fn admin_access_runs_deployer_for_gcp_bundle() {
    let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
    let _path_guard = temp_path_with_binary("greentic-deployer");
    let bundle = tempdir().expect("tempdir");

    let cli = build_cli("en");
    let matches = cli
        .try_get_matches_from([
            "gtc",
            "admin",
            "access",
            bundle.path().to_str().expect("bundle path"),
            "--target",
            "gcp",
            "--output",
            "json",
        ])
        .expect("matches");
    let (_, admin_matches) = matches.subcommand().expect("admin");
    let ("access", access_matches) = admin_matches.subcommand().expect("access") else {
        panic!("expected access subcommand");
    };

    assert_eq!(run_admin_access(access_matches, "en"), 0);
}

#[cfg(unix)]
#[test]
fn admin_token_runs_deployer_for_azure_bundle() {
    let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
    let _path_guard = temp_path_with_binary("greentic-deployer");
    let bundle = tempdir().expect("tempdir");

    let cli = build_cli("en");
    let matches = cli
        .try_get_matches_from([
            "gtc",
            "admin",
            "token",
            bundle.path().to_str().expect("bundle path"),
            "--target",
            "azure",
        ])
        .expect("matches");
    let (_, admin_matches) = matches.subcommand().expect("admin");
    let ("token", token_matches) = admin_matches.subcommand().expect("token") else {
        panic!("expected token subcommand");
    };

    assert_eq!(run_admin_token(token_matches, "en"), 0);
}

#[cfg(unix)]
#[test]
fn admin_health_runs_deployer_for_gcp_bundle() {
    let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
    let _path_guard = temp_path_with_binary("greentic-deployer");
    let bundle = tempdir().expect("tempdir");

    let cli = build_cli("en");
    let matches = cli
        .try_get_matches_from([
            "gtc",
            "admin",
            "health",
            bundle.path().to_str().expect("bundle path"),
            "--target",
            "gcp",
        ])
        .expect("matches");
    let (_, admin_matches) = matches.subcommand().expect("admin");
    let ("health", health_matches) = admin_matches.subcommand().expect("health") else {
        panic!("expected health subcommand");
    };

    assert_eq!(run_admin_health(health_matches, "en"), 0);
}

#[test]
fn admin_tunnel_errors_when_bundle_dir_is_missing() {
    let cli = build_cli("en");
    let matches = cli
        .try_get_matches_from(["gtc", "admin", "tunnel", "/definitely/missing/bundle"])
        .expect("matches");
    let (_, admin_matches) = matches.subcommand().expect("admin");
    let ("tunnel", tunnel_matches) = admin_matches.subcommand().expect("tunnel") else {
        panic!("expected tunnel subcommand");
    };

    assert_eq!(run_admin_tunnel(tunnel_matches, "en"), 1);
}

#[cfg(unix)]
#[test]
fn admin_tunnel_runs_deployer_for_local_bundle() {
    let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
    let _path_guard = temp_path_with_binary("greentic-deployer");
    let bundle = tempdir().expect("tempdir");

    let cli = build_cli("en");
    let matches = cli
        .try_get_matches_from([
            "gtc",
            "admin",
            "tunnel",
            bundle.path().to_str().expect("bundle path"),
            "--local-port",
            "9443",
            "--container",
            "ops",
        ])
        .expect("matches");
    let (_, admin_matches) = matches.subcommand().expect("admin");
    let ("tunnel", tunnel_matches) = admin_matches.subcommand().expect("tunnel") else {
        panic!("expected tunnel subcommand");
    };

    assert_eq!(run_admin_tunnel(tunnel_matches, "en"), 0);
}

#[test]
fn production_source_does_not_mutate_process_env_globally() {
    let source = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/bin/gtc.rs"));
    let production = source
        .split("#[cfg(test)]")
        .next()
        .expect("production source");

    assert!(
        !production.contains("set_var("),
        "production code should not mutate process env"
    );
    assert!(
        !production.contains("remove_var("),
        "production code should not mutate process env"
    );
}

#[test]
fn parse_prompt_choice_rejects_zero() {
    assert_eq!(
        parse_prompt_choice(0, 3).unwrap_err().to_string(),
        "invalid selection"
    );
}

#[test]
fn parse_prompt_choice_accepts_first_option() {
    assert_eq!(parse_prompt_choice(1, 3).expect("choice"), 0);
}

#[test]
fn child_process_env_applies_without_mutating_parent_env() {
    let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        env::remove_var("TEST_CHILD_ENV");
    }

    let mut child_env = super::ChildProcessEnv::new();
    child_env.set("TEST_CHILD_ENV", "demo");
    let mut process = ProcessCommand::new("env");
    child_env.apply(&mut process);

    let envs: HashMap<_, _> = process
        .get_envs()
        .filter_map(|(key, value)| {
            Some((
                key.to_string_lossy().to_string(),
                value?.to_string_lossy().to_string(),
            ))
        })
        .collect();

    assert_eq!(envs.get("TEST_CHILD_ENV").map(String::as_str), Some("demo"));
    assert!(env::var_os("TEST_CHILD_ENV").is_none());
}

#[test]
fn secret_prompt_apis_use_zeroizing_strings() {
    let _prompt_secret: fn(&str) -> gtc::error::GtcResult<zeroize::Zeroizing<String>> =
        super::prompt_secret;
    let _prompt_optional_secret: fn(
        &str,
    )
        -> gtc::error::GtcResult<Option<zeroize::Zeroizing<String>>> =
        super::prompt_optional_secret;
}

#[test]
fn should_send_auth_header_only_for_same_authority() {
    let original = reqwest::Url::parse("https://api.example.com/releases/latest").unwrap();
    let same_host = reqwest::Url::parse("https://api.example.com/releases/next").unwrap();
    let other_host = reqwest::Url::parse("https://cdn.example.com/download").unwrap();
    let other_scheme = reqwest::Url::parse("http://api.example.com/releases/next").unwrap();

    assert!(should_send_auth_header(&original, &same_host));
    assert!(!should_send_auth_header(&original, &other_host));
    assert!(!should_send_auth_header(&original, &other_scheme));
}

#[test]
#[ignore = "requires local socket binding"]
fn fetch_https_bytes_keeps_auth_on_same_host_redirect() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let (tx, rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().expect("accept");
            let request = read_http_request(&mut stream);
            tx.send(request.auth.clone()).expect("send auth");
            if request.path == "/start" {
                write_http_response(
                    &mut stream,
                    "HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\n\r\n",
                );
            } else {
                write_http_response(
                    &mut stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok",
                );
            }
        }
    });

    let url = format!("http://{addr}/start");
    let bytes = super::fetch_https_bytes(&url, "secret-token", "en", "application/octet-stream")
        .expect("fetch");
    assert_eq!(bytes, b"ok");
    assert_eq!(
        rx.recv().expect("first auth"),
        Some("Bearer secret-token".to_string())
    );
    assert_eq!(
        rx.recv().expect("second auth"),
        Some("Bearer secret-token".to_string())
    );
    handle.join().expect("join");
}

#[test]
#[ignore = "requires local socket binding"]
fn fetch_https_bytes_drops_auth_on_cross_host_redirect() {
    let first_listener = TcpListener::bind("127.0.0.1:0").expect("bind first");
    let second_listener = TcpListener::bind("127.0.0.1:0").expect("bind second");
    let first_addr = first_listener.local_addr().expect("first addr");
    let second_addr = second_listener.local_addr().expect("second addr");
    let (tx, rx) = mpsc::channel();

    let first = thread::spawn(move || {
        let (mut stream, _) = first_listener.accept().expect("accept first");
        let request = read_http_request(&mut stream);
        tx.send(("first".to_string(), request.auth.clone()))
            .expect("send first auth");
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: http://{second_addr}/final\r\nContent-Length: 0\r\n\r\n"
        );
        write_http_response(&mut stream, &response);
    });

    let (tx2, rx2) = mpsc::channel();
    let second = thread::spawn(move || {
        let (mut stream, _) = second_listener.accept().expect("accept second");
        let request = read_http_request(&mut stream);
        tx2.send(request.auth.clone()).expect("send second auth");
        write_http_response(
            &mut stream,
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok",
        );
    });

    let url = format!("http://{first_addr}/start");
    let bytes = super::fetch_https_bytes(&url, "secret-token", "en", "application/octet-stream")
        .expect("fetch");
    assert_eq!(bytes, b"ok");
    assert_eq!(
        rx.recv().expect("first auth"),
        ("first".to_string(), Some("Bearer secret-token".to_string()))
    );
    assert_eq!(rx2.recv().expect("second auth"), None);
    first.join().expect("join first");
    second.join().expect("join second");
}

#[test]
#[cfg(unix)]
fn generated_admin_private_keys_are_owner_only() {
    let dir = tempfile::tempdir().expect("tempdir");
    let resolved = ensure_admin_certs_ready(dir.path(), None).expect("certs");

    for key_name in ["ca.key", "server.key", "client.key"] {
        let mode = fs::metadata(resolved.join(key_name))
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "{key_name} should be owner-only");
    }
}

#[test]
fn verify_sha256_digest_rejects_mismatched_artifact() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("artifact.bin");
    fs::write(&path, b"fixture").expect("write artifact");

    let err = verify_sha256_digest(&path, "deadbeef").unwrap_err();
    assert!(err.to_string().contains("integrity check"));
}

#[test]
fn verify_sha256_digest_accepts_matching_artifact() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("artifact.bin");
    fs::write(&path, b"fixture").expect("write artifact");
    let actual = super::sha256_file(&path).expect("sha256");

    verify_sha256_digest(&path, &actual).expect("digest should match");
}

#[test]
fn normalize_expected_sha256_accepts_prefixed_and_unprefixed_values() {
    assert_eq!(
        normalize_expected_sha256("abc123"),
        "sha256:abc123".to_string()
    );
    assert_eq!(
        normalize_expected_sha256("sha256:def456"),
        "sha256:def456".to_string()
    );
}

#[test]
fn rewrite_store_tenant_placeholder_replaces_multiple_positions() {
    let input = "store://{tenant}/providers/{tenant}";
    let output = rewrite_store_tenant_placeholder(input, "acme");
    assert_eq!(output, "store://acme/providers/acme");
}

#[test]
fn extract_tar_archive_rejects_symlink_entries() {
    let dir = tempdir().expect("tempdir");
    let outside = tempdir().expect("outside");
    let mut bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut bytes);
        let mut link = tar::Header::new_gnu();
        link.set_entry_type(tar::EntryType::Symlink);
        link.set_path("escape").expect("set path");
        link.set_link_name(outside.path()).expect("set link");
        link.set_size(0);
        link.set_mode(0o777);
        link.set_cksum();
        builder
            .append(&link, Cursor::new(Vec::<u8>::new()))
            .expect("append link");
        builder.finish().expect("finish tar");
    }

    let mut archive = tar::Archive::new(Cursor::new(bytes));
    let err = extract_tar_archive(&mut archive, dir.path()).unwrap_err();
    assert!(err.to_string().contains("unsupported link type"));
}

#[test]
#[cfg(unix)]
fn extract_tar_archive_does_not_write_through_symlink_parent() {
    let out_dir = tempdir().expect("out_dir");
    let outside = tempdir().expect("outside");
    symlink(outside.path(), out_dir.path().join("escape")).expect("symlink");

    let mut bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut bytes);
        let payload = b"pwned";
        let mut header = tar::Header::new_gnu();
        header.set_path("escape/pwned.txt").expect("path");
        header.set_size(payload.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append(&header, Cursor::new(payload.as_slice()))
            .expect("append");
        builder.finish().expect("finish tar");
    }

    let mut archive = tar::Archive::new(Cursor::new(bytes));
    let err = extract_tar_archive(&mut archive, out_dir.path()).unwrap_err();
    assert!(err.to_string().contains("symlinked path"));
    assert!(!outside.path().join("pwned.txt").exists());
}

#[test]
#[cfg(unix)]
fn extract_tar_archive_does_not_mark_text_files_executable() {
    let out_dir = tempdir().expect("tempdir");
    let mut bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut bytes);
        let payload = b"{\"ok\":true}";
        let mut header = tar::Header::new_gnu();
        header.set_path("config.json").expect("path");
        header.set_size(payload.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append(&header, Cursor::new(payload.as_slice()))
            .expect("append");
        builder.finish().expect("finish tar");
    }

    let mut archive = tar::Archive::new(Cursor::new(bytes));
    extract_tar_archive(&mut archive, out_dir.path()).expect("extract tar");
    let mode = fs::metadata(out_dir.path().join("config.json"))
        .expect("metadata")
        .permissions()
        .mode()
        & 0o111;
    assert_eq!(mode, 0, "text files should not be executable");
}

#[test]
#[cfg(unix)]
fn extract_zip_bytes_does_not_mark_text_files_executable() {
    let out_dir = tempdir().expect("tempdir");
    let mut bytes = Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut bytes);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("config.json", options).expect("start file");
        zip.write_all(br#"{"ok":true}"#).expect("write file");
        zip.finish().expect("finish zip");
    }

    extract_zip_bytes(bytes.get_ref(), out_dir.path()).expect("extract zip");
    let mode = fs::metadata(out_dir.path().join("config.json"))
        .expect("metadata")
        .permissions()
        .mode()
        & 0o111;
    assert_eq!(mode, 0, "text files should not be executable");
}

#[test]
#[cfg(unix)]
fn recurse_files_skips_symlinked_directories() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root");
    let nested = root.join("nested");
    fs::create_dir_all(&nested).expect("mkdir");
    fs::write(nested.join("file.txt"), "fixture").expect("write");
    symlink(&root, nested.join("loop")).expect("symlink");

    let files = super::list_files_recursive(&root).expect("files");
    assert_eq!(files.len(), 1);
    assert_eq!(
        files[0].file_name().and_then(|v| v.to_str()),
        Some("file.txt")
    );
}

#[test]
#[cfg(unix)]
fn fingerprint_bundle_dir_skips_symlink_cycles() {
    let dir = tempfile::tempdir().expect("tempdir");
    let nested = dir.path().join("packs");
    fs::create_dir_all(&nested).expect("mkdir");
    fs::write(nested.join("cards-demo.gtpack"), "fixture").expect("write pack");
    symlink(dir.path(), nested.join("loop")).expect("symlink");

    let fingerprint = fingerprint_bundle_dir(dir.path()).expect("fingerprint");

    assert!(fingerprint.contains("file:packs/cards-demo.gtpack"));
    assert!(!fingerprint.contains("loop"));
}

struct HttpRequest {
    path: String,
    auth: Option<String>,
}

fn read_http_request(stream: &mut TcpStream) -> HttpRequest {
    let mut buf = [0u8; 4096];
    let mut request = Vec::new();
    loop {
        let read = std::io::Read::read(stream, &mut buf).expect("read request");
        request.extend_from_slice(&buf[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let request = String::from_utf8(request).expect("utf8 request");
    let mut lines = request.split("\r\n");
    let request_line = lines.next().expect("request line");
    let path = request_line
        .split_whitespace()
        .nth(1)
        .expect("request path")
        .to_string();
    let auth = lines.find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.eq_ignore_ascii_case("authorization") {
            Some(value.trim().to_string())
        } else {
            None
        }
    });
    HttpRequest { path, auth }
}

fn write_http_response(stream: &mut TcpStream, response: &str) {
    stream
        .write_all(response.as_bytes())
        .expect("write response");
    stream.flush().expect("flush response");
}
