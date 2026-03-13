#[path = "../dist.rs"]
mod dist;

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Arg, ArgAction, ArgMatches, Command};
use directories::BaseDirs;
use greentic_distributor_client::{
    OciPackFetcher, PackFetchOptions, oci_packs::DefaultRegistryClient,
};
use greentic_i18n::{normalize_locale, select_locale_with_sources};
use greentic_start::{
    CloudflaredModeArg, NatsModeArg, NgrokModeArg, RestartTarget, StartRequest, run_start_request,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tempfile::TempDir;

use crate::dist::build_adapter;

const DEV_BIN: &str = "greentic-dev";

const OP_BIN: &str = "greentic-operator";
const BUNDLE_BIN: &str = "greentic-bundle";
const PACK_BIN: &str = "greentic-pack";
const DEPLOYER_BIN: &str = "greentic-deployer";
const SETUP_BIN: &str = "greentic-setup";

const LOCALES_JSON: &str = include_str!("../../assets/i18n/locales.json");
include!(concat!(env!("OUT_DIR"), "/embedded_i18n.rs"));

fn main() {
    let raw_args: Vec<String> = env::args().collect();
    let exit_code = run(raw_args);
    std::process::exit(exit_code);
}

fn run(raw_args: Vec<String>) -> i32 {
    let i18n = i18n();
    let locale = detect_locale(&raw_args, i18n.default_locale());

    let cli = build_cli(&locale);
    let matches = match cli.try_get_matches_from(raw_args) {
        Ok(matches) => matches,
        Err(err) => {
            let _ = err.print();
            return err.exit_code();
        }
    };

    let debug = matches.get_flag("debug-router");

    match matches.subcommand() {
        Some(("version", _)) => {
            println!("gtc {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Some(("doctor", _)) => run_doctor(&locale),
        Some(("install", sub_matches)) => run_install(sub_matches, debug, &locale),
        Some(("start", sub_matches)) => run_start(sub_matches, debug, &locale),
        Some(("dev", sub_matches)) => {
            let tail = collect_tail(sub_matches);
            passthrough(DEV_BIN, &tail, debug, &locale)
        }
        Some(("op", sub_matches)) => {
            let tail = collect_tail(sub_matches);
            let rewritten = rewrite_legacy_op_args(&tail);
            passthrough(OP_BIN, &rewritten, debug, &locale)
        }
        Some(("wizard", sub_matches)) => {
            let tail = collect_tail(sub_matches);
            if tail.is_empty() {
                return run_wizard_menu(debug, &locale);
            }
            let forwarded = build_operator_wizard_args(&tail, &locale);
            passthrough(OP_BIN, &forwarded, debug, &locale)
        }
        Some(("setup", sub_matches)) => {
            let tail = collect_tail(sub_matches);
            passthrough(SETUP_BIN, &tail, debug, &locale)
        }
        _ => 2,
    }
}

fn build_cli(locale: &str) -> Command {
    let cmd_args = passthrough_args();

    Command::new(leak_str(t(locale, "gtc.app.name").into_owned()))
        .version(env!("CARGO_PKG_VERSION"))
        .about(t(locale, "gtc.app.about").into_owned())
        .arg(
            Arg::new("locale")
                .long("locale")
                .value_name("BCP47")
                .num_args(1)
                .global(true)
                .help(t(locale, "gtc.arg.locale.help").into_owned()),
        )
        .arg(
            Arg::new("debug-router")
                .long("debug-router")
                .action(ArgAction::SetTrue)
                .global(true)
                .help(t(locale, "gtc.arg.debug_router.help").into_owned()),
        )
        .subcommand(Command::new("version").about(t(locale, "gtc.cmd.version.about").into_owned()))
        .subcommand(Command::new("doctor").about(t(locale, "gtc.cmd.doctor.about").into_owned()))
        .subcommand(
            Command::new("install")
                .about(t(locale, "gtc.cmd.install.about").into_owned())
                .arg(
                    Arg::new("tenant")
                        .long("tenant")
                        .value_name("TENANT")
                        .num_args(1)
                        .help(t(locale, "gtc.arg.tenant.help").into_owned()),
                )
                .arg(
                    Arg::new("key")
                        .long("key")
                        .value_name("KEY")
                        .num_args(1)
                        .help(t(locale, "gtc.arg.key.help").into_owned()),
                ),
        )
        .subcommand(
            Command::new("start")
                .about(t_or(
                    locale,
                    "gtc.cmd.start.about",
                    "Start a bundle from local or remote reference.",
                ))
                .arg(
                    Arg::new("bundle-ref")
                        .value_name("BUNDLE_REF")
                        .required(true)
                        .help(t_or(
                            locale,
                            "gtc.arg.bundle_ref.help",
                            "Bundle path/ref: local path, file://, oci://, repo://, store://",
                        )),
                )
                .arg(cmd_args.clone()),
        )
        .subcommand(
            Command::new("dev")
                .about(t(locale, "gtc.cmd.dev.about").into_owned())
                .arg(cmd_args.clone()),
        )
        .subcommand(
            Command::new("op")
                .about(t(locale, "gtc.cmd.op.about").into_owned())
                .arg(cmd_args.clone()),
        )
        .subcommand(
            Command::new("wizard")
                .about(t(locale, "gtc.cmd.wizard.about").into_owned())
                .arg(cmd_args.clone()),
        )
        .subcommand(
            Command::new("setup")
                .about(t(locale, "gtc.cmd.setup.about").into_owned())
                .arg(cmd_args),
        )
}

fn passthrough_args() -> Arg {
    Arg::new("args")
        .num_args(0..)
        .trailing_var_arg(true)
        .allow_hyphen_values(true)
}

fn collect_tail(matches: &ArgMatches) -> Vec<String> {
    matches
        .get_many::<String>("args")
        .map(|vals| vals.cloned().collect())
        .unwrap_or_default()
}

fn rewrite_legacy_op_args(args: &[String]) -> Vec<String> {
    let Some(first) = args.first() else {
        return args.to_vec();
    };

    match first.as_str() {
        "setup" => {
            let mut out = vec!["demo".to_string(), "setup".to_string()];
            out.extend_from_slice(&args[1..]);
            ensure_flag_value(&mut out, "tenant", "default");
            ensure_flag_value(&mut out, "team", "default");
            out
        }
        "start" => {
            let mut out = vec!["demo".to_string(), "start".to_string()];
            out.extend_from_slice(&args[1..]);
            ensure_flag_value(&mut out, "tenant", "default");
            ensure_flag_value(&mut out, "team", "default");
            ensure_flag_value(&mut out, "cloudflared", "off");
            out
        }
        _ => args.to_vec(),
    }
}

fn build_operator_wizard_args(args: &[String], locale: &str) -> Vec<String> {
    let mut forwarded = vec!["wizard".to_string()];
    if !has_flag(args, "locale") {
        ensure_flag_value(&mut forwarded, "locale", locale);
    }
    forwarded.extend_from_slice(args);
    forwarded
}

fn ensure_flag_value(args: &mut Vec<String>, flag: &str, value: &str) {
    if has_flag(args, flag) {
        return;
    }
    args.push(format!("--{flag}"));
    if !value.is_empty() {
        args.push(value.to_string());
    }
}

fn has_flag(args: &[String], flag: &str) -> bool {
    let long = format!("--{flag}");
    let with_eq = format!("{long}=");
    args.iter()
        .any(|arg| arg == &long || arg.starts_with(&with_eq))
}

fn detect_locale(raw_args: &[String], default_locale: &str) -> String {
    let cli_locale = locale_from_args(raw_args);
    let env_locale = env::var("GTC_LOCALE").ok();

    let selected = select_locale_with_sources(
        cli_locale.as_deref(),
        Some(default_locale),
        env_locale.as_deref(),
        None,
    );

    i18n().normalize_or_default(&selected)
}

fn locale_from_args(raw_args: &[String]) -> Option<String> {
    let mut iter = raw_args.iter().skip(1);
    while let Some(arg) = iter.next() {
        if arg == "--locale" {
            return iter.next().cloned();
        }
        if let Some(value) = arg.strip_prefix("--locale=") {
            return Some(value.to_string());
        }
    }
    None
}

struct StartBundleResolution {
    bundle_dir: PathBuf,
    deployment_key: String,
    deploy_artifact: Option<PathBuf>,
    _hold: Option<TempDir>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartTarget {
    Runtime,
    SingleVm,
    Aws,
}

impl StartTarget {
    fn as_str(self) -> &'static str {
        match self {
            StartTarget::Runtime => "runtime",
            StartTarget::SingleVm => "single-vm",
            StartTarget::Aws => "aws",
        }
    }
}

#[derive(Debug)]
struct StartCliOptions {
    start_args: Vec<String>,
    explicit_target: Option<StartTarget>,
    environment: Option<String>,
    provider_pack: Option<PathBuf>,
    app_pack: Option<PathBuf>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StartDeploymentState {
    target: String,
    bundle_fingerprint: String,
    bundle_ref: String,
    deployed_at_epoch_s: u64,
    artifact_path: Option<String>,
}

fn run_start(sub_matches: &ArgMatches, debug: bool, locale: &str) -> i32 {
    let Some(bundle_ref) = sub_matches.get_one::<String>("bundle-ref") else {
        eprintln!(
            "{}",
            t_or(
                locale,
                "gtc.start.err.bundle_required",
                "bundle ref is required"
            )
        );
        return 2;
    };
    let tail = collect_tail(sub_matches);
    let cli_options = match parse_start_cli_options(&tail) {
        Ok(value) => value,
        Err(err) => {
            eprintln!(
                "{}: {err}",
                t_or(
                    locale,
                    "gtc.start.err.invalid_args",
                    "invalid start arguments"
                )
            );
            return 2;
        }
    };
    let resolved = match resolve_bundle_reference(bundle_ref) {
        Ok(value) => value,
        Err(err) => {
            eprintln!(
                "{}: {err}",
                t_or(
                    locale,
                    "gtc.start.err.resolve_failed",
                    "failed to resolve bundle"
                )
            );
            return 1;
        }
    };
    let request = match parse_start_request(&cli_options.start_args, resolved.bundle_dir.clone()) {
        Ok(value) => value,
        Err(err) => {
            eprintln!(
                "{}: {err}",
                t_or(
                    locale,
                    "gtc.start.err.invalid_args",
                    "invalid start arguments"
                )
            );
            return 2;
        }
    };
    let target =
        match select_start_target(&resolved.bundle_dir, cli_options.explicit_target, locale) {
            Ok(value) => value,
            Err(err) => {
                eprintln!(
                    "{}: {err}",
                    t_or(
                        locale,
                        "gtc.start.err.target_select_failed",
                        "failed to choose deployment target"
                    )
                );
                return 2;
            }
        };
    if target != StartTarget::Runtime {
        let deploy_result = ensure_bundle_deployed(
            bundle_ref,
            &resolved,
            &request,
            &cli_options,
            target,
            debug,
            locale,
        );
        match deploy_result {
            Ok(()) => return 0,
            Err(err) => {
                eprintln!(
                    "{}: {err}",
                    t_or(
                        locale,
                        "gtc.start.err.deploy_failed",
                        "failed to deploy bundle before start"
                    )
                );
                return 1;
            }
        }
    }
    if debug {
        eprintln!(
            "{} gtc-start-lib bundle={:?} tenant={:?} team={:?}",
            t(locale, "gtc.debug.exec"),
            request.bundle,
            request.tenant,
            request.team
        );
    }
    match run_start_request(request) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!(
                "{}: {err}",
                t_or(locale, "gtc.start.err.run_failed", "failed to start bundle")
            );
            1
        }
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
) -> Result<(), String> {
    let fingerprint = fingerprint_bundle_dir(&resolved.bundle_dir)?;
    let state_path = deployment_state_path(&resolved.deployment_key, target)?;
    let previous_state = load_deployment_state(&state_path)?;
    let deploy_needed = previous_state
        .as_ref()
        .map(|state| state.bundle_fingerprint != fingerprint)
        .unwrap_or(true);
    match target {
        StartTarget::SingleVm => {
            let artifact_path = prepare_deployable_bundle_artifact(resolved, debug, locale)?;
            let spec_path = write_single_vm_spec(bundle_ref, resolved, request, &artifact_path)?;
            let current_status = read_single_vm_status(&spec_path, debug, locale)?;
            let status_applied = current_status
                .as_ref()
                .and_then(|value| value.get("status"))
                .and_then(Value::as_str)
                .map(|value| value == "applied")
                .unwrap_or(false);
            if status_applied && !deploy_needed {
                return Ok(());
            }
            run_single_vm_apply(&spec_path, debug, locale)?;
            let state = StartDeploymentState {
                target: target.as_str().to_string(),
                bundle_fingerprint: fingerprint,
                bundle_ref: bundle_ref.to_string(),
                deployed_at_epoch_s: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|err| err.to_string())?
                    .as_secs(),
                artifact_path: Some(artifact_path.display().to_string()),
            };
            save_deployment_state(&state_path, &state)?;
            Ok(())
        }
        StartTarget::Aws => {
            if !deploy_needed && previous_state.is_some() {
                return Ok(());
            }
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
                    .map_err(|err| err.to_string())?
                    .as_secs(),
                artifact_path: None,
            };
            save_deployment_state(&state_path, &state)?;
            Ok(())
        }
        StartTarget::Runtime => Ok(()),
    }
}

fn prepare_deployable_bundle_artifact(
    resolved: &StartBundleResolution,
    debug: bool,
    locale: &str,
) -> Result<PathBuf, String> {
    if let Some(path) = resolved.deploy_artifact.as_ref() {
        return Ok(path.clone());
    }

    let artifact_root = deployment_artifacts_root()?;
    fs::create_dir_all(&artifact_root).map_err(|err| {
        format!(
            "failed to create deployment artifact root {}: {err}",
            artifact_root.display()
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

fn deployment_artifacts_root() -> Result<PathBuf, String> {
    let base = BaseDirs::new()
        .ok_or_else(|| "failed to resolve base directories for deployment artifacts".to_string())?;
    Ok(base
        .data_local_dir()
        .join("greentic")
        .join("gtc")
        .join("bundles"))
}

fn deployment_state_path(deployment_key: &str, target: StartTarget) -> Result<PathBuf, String> {
    let base = BaseDirs::new()
        .ok_or_else(|| "failed to resolve base directories for deployment state".to_string())?;
    Ok(base
        .state_dir()
        .unwrap_or_else(|| base.data_local_dir())
        .join("greentic")
        .join("gtc")
        .join("deployments")
        .join(format!("{deployment_key}-{}.json", target.as_str())))
}

fn load_deployment_state(path: &Path) -> Result<Option<StartDeploymentState>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path)
        .map_err(|err| format!("failed to read deployment state {}: {err}", path.display()))?;
    let state = serde_json::from_str(&raw)
        .map_err(|err| format!("failed to parse deployment state {}: {err}", path.display()))?;
    Ok(Some(state))
}

fn save_deployment_state(path: &Path, state: &StartDeploymentState) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            format!(
                "failed to create deployment state directory {}: {err}",
                parent.display()
            )
        })?;
    }
    let raw = serde_json::to_vec_pretty(state)
        .map_err(|err| format!("failed to serialize deployment state: {err}"))?;
    fs::write(path, raw)
        .map_err(|err| format!("failed to write deployment state {}: {err}", path.display()))
}

fn write_single_vm_spec(
    bundle_ref: &str,
    resolved: &StartBundleResolution,
    request: &StartRequest,
    artifact_path: &Path,
) -> Result<PathBuf, String> {
    let cert_dir = resolve_admin_cert_dir(&resolved.bundle_dir);
    let deployment_name = deployment_name(bundle_ref, request);
    let state_root = deployment_runtime_root(&resolved.deployment_key)?;
    let spec_dir = state_root.join("spec");
    fs::create_dir_all(&spec_dir).map_err(|err| {
        format!(
            "failed to create spec directory {}: {err}",
            spec_dir.display()
        )
    })?;
    let spec_path = spec_dir.join("single-vm.deployment.yaml");
    let spec = format!(
        "apiVersion: greentic.ai/v1alpha1\nkind: Deployment\nmetadata:\n  name: {name}\nspec:\n  target: single-vm\n  bundle:\n    source: {bundle}\n    format: squashfs\n  runtime:\n    image: {image}\n    arch: x86_64\n    admin:\n      bind: 127.0.0.1:8433\n      mtls:\n        caFile: {ca}\n        certFile: {cert}\n        keyFile: {key}\n  storage:\n    stateDir: {state_dir}\n    cacheDir: {cache_dir}\n    logDir: {log_dir}\n    tempDir: {temp_dir}\n  service:\n    manager: systemd\n    user: greentic\n    group: greentic\n  health:\n    readinessPath: /ready\n    livenessPath: /health\n    startupTimeoutSeconds: 120\n  rollout:\n    strategy: recreate\n",
        name = deployment_name,
        bundle = yaml_string(&format!("file://{}", artifact_path.display())),
        image = yaml_string("ghcr.io/greenticai/greentic-runtime:0.1.0"),
        ca = yaml_string(&cert_dir.join("ca.crt").display().to_string()),
        cert = yaml_string(&cert_dir.join("client.crt").display().to_string()),
        key = yaml_string(&cert_dir.join("client.key").display().to_string()),
        state_dir = yaml_string(&state_root.join("state").display().to_string()),
        cache_dir = yaml_string(&state_root.join("cache").display().to_string()),
        log_dir = yaml_string(&state_root.join("log").display().to_string()),
        temp_dir = yaml_string(&state_root.join("tmp").display().to_string()),
    );
    fs::write(&spec_path, spec).map_err(|err| {
        format!(
            "failed to write deployment spec {}: {err}",
            spec_path.display()
        )
    })?;
    Ok(spec_path)
}

fn resolve_admin_cert_dir(bundle_dir: &Path) -> PathBuf {
    let bundle_certs = bundle_dir.join("certs");
    if bundle_certs.join("ca.crt").exists()
        && bundle_certs.join("client.crt").exists()
        && bundle_certs.join("client.key").exists()
    {
        return bundle_certs;
    }
    PathBuf::from("/etc/greentic/admin")
}

fn run_multi_target_deployer_apply(
    _bundle_ref: &str,
    resolved: &StartBundleResolution,
    request: &StartRequest,
    cli_options: &StartCliOptions,
    target: StartTarget,
    debug: bool,
    locale: &str,
) -> Result<(), String> {
    let app_pack = resolve_app_pack_path(&resolved.bundle_dir, cli_options.app_pack.as_ref())?;
    let provider_pack = resolve_target_provider_pack(
        &resolved.bundle_dir,
        target,
        cli_options.provider_pack.as_ref(),
    )?;
    let tenant = request.tenant.clone().unwrap_or_else(|| "demo".to_string());
    let target_name = target.as_str().to_string();
    let mut args = vec![
        target_name,
        "apply".to_string(),
        "--tenant".to_string(),
        tenant,
        "--pack".to_string(),
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
    run_binary_checked(
        DEPLOYER_BIN,
        &args,
        debug,
        locale,
        "multi-target deploy apply",
    )
}

fn deployment_runtime_root(deployment_key: &str) -> Result<PathBuf, String> {
    let base = BaseDirs::new()
        .ok_or_else(|| "failed to resolve base directories for deployment runtime".to_string())?;
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

fn read_single_vm_status(
    spec_path: &Path,
    debug: bool,
    locale: &str,
) -> Result<Option<Value>, String> {
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
        .map_err(|err| format!("failed to parse deployer status output as JSON: {err}"))?;
    Ok(Some(parsed))
}

fn run_single_vm_apply(spec_path: &Path, debug: bool, locale: &str) -> Result<(), String> {
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

fn run_binary_checked(
    binary: &str,
    args: &[String],
    debug: bool,
    locale: &str,
    operation: &str,
) -> Result<(), String> {
    let status = run_binary_status(binary, args, debug, locale)?;
    if status.success() {
        return Ok(());
    }
    Err(format!(
        "{operation} failed via {binary} with status {}",
        status.code().unwrap_or(1)
    ))
}

fn run_binary_capture(
    binary: &str,
    args: &[String],
    debug: bool,
    locale: &str,
) -> Result<String, String> {
    if debug {
        eprintln!("{} {} {:?}", t(locale, "gtc.debug.exec"), binary, args);
    }
    let output = ProcessCommand::new(binary)
        .args(args)
        .env("GREENTIC_LOCALE", locale)
        .output()
        .map_err(|err| format!("failed to execute {binary}: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            return Err(format!(
                "{binary} exited with status {}",
                output.status.code().unwrap_or(1)
            ));
        }
        return Err(stderr);
    }
    String::from_utf8(output.stdout).map_err(|err| format!("invalid UTF-8 from {binary}: {err}"))
}

fn run_binary_status(
    binary: &str,
    args: &[String],
    debug: bool,
    locale: &str,
) -> Result<std::process::ExitStatus, String> {
    if debug {
        eprintln!("{} {} {:?}", t(locale, "gtc.debug.exec"), binary, args);
    }
    ProcessCommand::new(binary)
        .args(args)
        .env("GREENTIC_LOCALE", locale)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|err| format!("failed to execute {binary}: {err}"))
}

fn fingerprint_bundle_dir(bundle_dir: &Path) -> Result<String, String> {
    let mut files = Vec::new();
    collect_bundle_entries(bundle_dir, bundle_dir, &mut files)?;
    files.sort();
    Ok(files.join("\n"))
}

fn collect_bundle_entries(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    for entry in fs::read_dir(dir)
        .map_err(|err| format!("failed to read bundle directory {}: {err}", dir.display()))?
    {
        let entry = entry.map_err(|err| err.to_string())?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|err| err.to_string())?
            .display()
            .to_string();
        if path.is_dir() {
            out.push(format!("dir:{relative}"));
            collect_bundle_entries(root, &path, out)?;
            continue;
        }
        let metadata = entry.metadata().map_err(|err| err.to_string())?;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
            .map(|value| value.as_secs())
            .unwrap_or(0);
        out.push(format!("file:{relative}:{}:{modified}", metadata.len()));
    }
    Ok(())
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

fn parse_start_request(tail: &[String], bundle_dir: PathBuf) -> Result<StartRequest, String> {
    let mut request = StartRequest {
        bundle: Some(bundle_dir.display().to_string()),
        tenant: None,
        team: None,
        no_nats: false,
        nats: NatsModeArg::Off,
        nats_url: None,
        config: None,
        cloudflared: CloudflaredModeArg::On,
        cloudflared_binary: None,
        ngrok: NgrokModeArg::Off,
        ngrok_binary: None,
        runner_binary: None,
        restart: Vec::new(),
        log_dir: None,
        verbose: false,
        quiet: false,
    };

    let mut idx = 0usize;
    while idx < tail.len() {
        let arg = &tail[idx];
        match arg.as_str() {
            "--tenant" => {
                idx += 1;
                request.tenant = Some(required_value(tail, idx, "--tenant")?);
            }
            "--team" => {
                idx += 1;
                request.team = Some(required_value(tail, idx, "--team")?);
            }
            "--no-nats" => request.no_nats = true,
            "--nats" => {
                idx += 1;
                request.nats = parse_nats_mode(&required_value(tail, idx, "--nats")?)?;
            }
            "--nats-url" => {
                idx += 1;
                request.nats_url = Some(required_value(tail, idx, "--nats-url")?);
            }
            "--config" => {
                idx += 1;
                request.config = Some(PathBuf::from(required_value(tail, idx, "--config")?));
            }
            "--cloudflared" => {
                idx += 1;
                request.cloudflared =
                    parse_cloudflared_mode(&required_value(tail, idx, "--cloudflared")?)?;
            }
            "--cloudflared-binary" => {
                idx += 1;
                request.cloudflared_binary = Some(PathBuf::from(required_value(
                    tail,
                    idx,
                    "--cloudflared-binary",
                )?));
            }
            "--ngrok" => {
                idx += 1;
                request.ngrok = parse_ngrok_mode(&required_value(tail, idx, "--ngrok")?)?;
            }
            "--ngrok-binary" => {
                idx += 1;
                request.ngrok_binary =
                    Some(PathBuf::from(required_value(tail, idx, "--ngrok-binary")?));
            }
            "--runner-binary" => {
                idx += 1;
                request.runner_binary =
                    Some(PathBuf::from(required_value(tail, idx, "--runner-binary")?));
            }
            "--restart" => {
                idx += 1;
                let value = required_value(tail, idx, "--restart")?;
                for part in value.split(',').filter(|part| !part.is_empty()) {
                    request.restart.push(parse_restart_target(part)?);
                }
            }
            "--log-dir" => {
                idx += 1;
                request.log_dir = Some(PathBuf::from(required_value(tail, idx, "--log-dir")?));
            }
            "--verbose" => request.verbose = true,
            "--quiet" => request.quiet = true,
            "--bundle" => {
                return Err(
                    "--bundle is managed by gtc start; pass the bundle ref as the main argument"
                        .to_string(),
                );
            }
            other => {
                if let Some(value) = other.strip_prefix("--tenant=") {
                    request.tenant = Some(value.to_string());
                } else if let Some(value) = other.strip_prefix("--team=") {
                    request.team = Some(value.to_string());
                } else if let Some(value) = other.strip_prefix("--nats=") {
                    request.nats = parse_nats_mode(value)?;
                } else if let Some(value) = other.strip_prefix("--nats-url=") {
                    request.nats_url = Some(value.to_string());
                } else if let Some(value) = other.strip_prefix("--config=") {
                    request.config = Some(PathBuf::from(value));
                } else if let Some(value) = other.strip_prefix("--cloudflared=") {
                    request.cloudflared = parse_cloudflared_mode(value)?;
                } else if let Some(value) = other.strip_prefix("--cloudflared-binary=") {
                    request.cloudflared_binary = Some(PathBuf::from(value));
                } else if let Some(value) = other.strip_prefix("--ngrok=") {
                    request.ngrok = parse_ngrok_mode(value)?;
                } else if let Some(value) = other.strip_prefix("--ngrok-binary=") {
                    request.ngrok_binary = Some(PathBuf::from(value));
                } else if let Some(value) = other.strip_prefix("--runner-binary=") {
                    request.runner_binary = Some(PathBuf::from(value));
                } else if let Some(value) = other.strip_prefix("--restart=") {
                    for part in value.split(',').filter(|part| !part.is_empty()) {
                        request.restart.push(parse_restart_target(part)?);
                    }
                } else if let Some(value) = other.strip_prefix("--log-dir=") {
                    request.log_dir = Some(PathBuf::from(value));
                } else if other.starts_with("--bundle=") {
                    return Err(
                        "--bundle is managed by gtc start; pass the bundle ref as the main argument"
                            .to_string(),
                    );
                } else {
                    return Err(format!("unsupported start argument: {other}"));
                }
            }
        }
        idx += 1;
    }

    Ok(request)
}

fn parse_start_cli_options(tail: &[String]) -> Result<StartCliOptions, String> {
    let mut start_args = Vec::new();
    let mut explicit_target = None;
    let mut environment = None;
    let mut provider_pack = None;
    let mut app_pack = None;
    let mut idx = 0usize;
    while idx < tail.len() {
        let arg = &tail[idx];
        match arg.as_str() {
            "--target" => {
                idx += 1;
                explicit_target =
                    Some(parse_start_target(&required_value(tail, idx, "--target")?)?);
            }
            "--environment" => {
                idx += 1;
                environment = Some(required_value(tail, idx, "--environment")?);
            }
            "--provider-pack" => {
                idx += 1;
                provider_pack = Some(PathBuf::from(required_value(tail, idx, "--provider-pack")?));
            }
            "--app-pack" => {
                idx += 1;
                app_pack = Some(PathBuf::from(required_value(tail, idx, "--app-pack")?));
            }
            _ => {
                if let Some(value) = arg.strip_prefix("--target=") {
                    explicit_target = Some(parse_start_target(value)?);
                } else if let Some(value) = arg.strip_prefix("--environment=") {
                    environment = Some(value.to_string());
                } else if let Some(value) = arg.strip_prefix("--provider-pack=") {
                    provider_pack = Some(PathBuf::from(value));
                } else if let Some(value) = arg.strip_prefix("--app-pack=") {
                    app_pack = Some(PathBuf::from(value));
                } else {
                    start_args.push(arg.clone());
                }
            }
        }
        idx += 1;
    }
    Ok(StartCliOptions {
        start_args,
        explicit_target,
        environment,
        provider_pack,
        app_pack,
    })
}

fn parse_start_target(value: &str) -> Result<StartTarget, String> {
    match value.trim() {
        "runtime" | "local" => Ok(StartTarget::Runtime),
        "single-vm" | "single_vm" => Ok(StartTarget::SingleVm),
        "aws" => Ok(StartTarget::Aws),
        other => Err(format!(
            "unsupported --target value {other}; expected runtime, single-vm, or aws"
        )),
    }
}

fn select_start_target(
    bundle_dir: &Path,
    explicit_target: Option<StartTarget>,
    locale: &str,
) -> Result<StartTarget, String> {
    if let Some(target) = explicit_target {
        return Ok(target);
    }
    let mut targets = vec![StartTarget::Runtime];
    targets.extend(detect_bundle_deployment_targets(bundle_dir)?);
    targets.sort_by_key(|value| match value {
        StartTarget::Runtime => 0,
        StartTarget::SingleVm => 1,
        StartTarget::Aws => 2,
    });
    targets.dedup();
    if targets.len() == 1 {
        return Ok(targets[0]);
    }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(format!(
            "multiple start targets are available ({}); rerun with --target",
            targets
                .iter()
                .map(|value| value.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    prompt_start_target(&targets, locale)
}

fn detect_bundle_deployment_targets(bundle_dir: &Path) -> Result<Vec<StartTarget>, String> {
    let mut targets = Vec::new();
    let deployer_dir = bundle_dir.join("providers").join("deployer");
    if !deployer_dir.exists() {
        return Ok(targets);
    }
    targets.push(StartTarget::SingleVm);
    if resolve_target_provider_pack(bundle_dir, StartTarget::Aws, None).is_ok() {
        targets.push(StartTarget::Aws);
    }
    Ok(targets)
}

fn prompt_start_target(targets: &[StartTarget], locale: &str) -> Result<StartTarget, String> {
    println!(
        "{}",
        t_or(
            locale,
            "gtc.start.prompt.target",
            "Select start/deployment target:"
        )
    );
    for (idx, target) in targets.iter().enumerate() {
        println!("{} ) {}", idx + 1, target.as_str());
    }
    print!("> ");
    io::stdout().flush().map_err(|err| err.to_string())?;
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(|err| err.to_string())?;
    let choice = input
        .trim()
        .parse::<usize>()
        .map_err(|_| "invalid target selection".to_string())?;
    targets
        .get(choice.saturating_sub(1))
        .copied()
        .ok_or_else(|| "invalid target selection".to_string())
}

fn resolve_target_provider_pack(
    bundle_dir: &Path,
    target: StartTarget,
    override_path: Option<&PathBuf>,
) -> Result<PathBuf, String> {
    if let Some(path) = override_path {
        return Ok(path.clone());
    }
    let deployer_dir = bundle_dir.join("providers").join("deployer");
    if !deployer_dir.exists() {
        return Err(format!(
            "bundle has no deployer providers directory: {}",
            deployer_dir.display()
        ));
    }
    let needle = match target {
        StartTarget::Aws => "aws",
        StartTarget::SingleVm | StartTarget::Runtime => {
            return Err(format!(
                "target {} does not use provider pack discovery",
                target.as_str()
            ));
        }
    };
    let mut candidates = Vec::new();
    for entry in fs::read_dir(&deployer_dir)
        .map_err(|err| format!("failed to read {}: {err}", deployer_dir.display()))?
    {
        let entry = entry.map_err(|err| err.to_string())?;
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if name.contains(needle) {
            candidates.push(path);
        }
    }
    candidates.sort();
    candidates.into_iter().next().ok_or_else(|| {
        format!(
            "no deployer provider pack found for target {}",
            target.as_str()
        )
    })
}

fn resolve_app_pack_path(
    bundle_dir: &Path,
    override_path: Option<&PathBuf>,
) -> Result<PathBuf, String> {
    if let Some(path) = override_path {
        return Ok(path.clone());
    }
    let packs_dir = bundle_dir.join("packs");
    if !packs_dir.exists() {
        return Err(format!(
            "bundle has no packs directory: {}",
            packs_dir.display()
        ));
    }
    let mut candidates = Vec::new();
    for entry in fs::read_dir(&packs_dir)
        .map_err(|err| format!("failed to read {}: {err}", packs_dir.display()))?
    {
        let entry = entry.map_err(|err| err.to_string())?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) == Some("gtpack") || path.is_dir() {
            candidates.push(path);
        }
    }
    candidates.sort();
    match candidates.len() {
        0 => Err(format!("no app pack found under {}", packs_dir.display())),
        1 => Ok(candidates.remove(0)),
        _ => Err(format!(
            "multiple app packs found under {}; rerun with --app-pack",
            packs_dir.display()
        )),
    }
}

fn required_value(args: &[String], idx: usize, flag: &str) -> Result<String, String> {
    args.get(idx)
        .cloned()
        .ok_or_else(|| format!("missing value for {flag}"))
}

fn parse_nats_mode(value: &str) -> Result<NatsModeArg, String> {
    match value {
        "off" => Ok(NatsModeArg::Off),
        "on" => Ok(NatsModeArg::On),
        "external" => Ok(NatsModeArg::External),
        other => Err(format!("unsupported --nats value: {other}")),
    }
}

fn parse_cloudflared_mode(value: &str) -> Result<CloudflaredModeArg, String> {
    match value {
        "on" => Ok(CloudflaredModeArg::On),
        "off" => Ok(CloudflaredModeArg::Off),
        other => Err(format!("unsupported --cloudflared value: {other}")),
    }
}

fn parse_ngrok_mode(value: &str) -> Result<NgrokModeArg, String> {
    match value {
        "on" => Ok(NgrokModeArg::On),
        "off" => Ok(NgrokModeArg::Off),
        other => Err(format!("unsupported --ngrok value: {other}")),
    }
}

fn parse_restart_target(value: &str) -> Result<RestartTarget, String> {
    match value {
        "all" => Ok(RestartTarget::All),
        "cloudflared" => Ok(RestartTarget::Cloudflared),
        "ngrok" => Ok(RestartTarget::Ngrok),
        "nats" => Ok(RestartTarget::Nats),
        "gateway" => Ok(RestartTarget::Gateway),
        "egress" => Ok(RestartTarget::Egress),
        "subscriptions" => Ok(RestartTarget::Subscriptions),
        other => Err(format!("unsupported --restart target: {other}")),
    }
}

fn resolve_bundle_reference(reference: &str) -> Result<StartBundleResolution, String> {
    let trimmed = reference.trim();
    if trimmed.is_empty() {
        return Err("bundle reference is empty".to_string());
    }
    if let Some(path) = parse_local_bundle_ref(trimmed) {
        return resolve_local_bundle_path(path);
    }

    let mapped = map_remote_bundle_ref(trimmed)?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("failed to build tokio runtime: {e}"))?;
    let fetcher: OciPackFetcher<DefaultRegistryClient> = OciPackFetcher::new(PackFetchOptions {
        allow_tags: true,
        offline: false,
        ..PackFetchOptions::default()
    });
    let fetched = rt
        .block_on(fetcher.fetch_pack_to_cache(&mapped))
        .map_err(|e| format!("failed to fetch remote bundle {trimmed}: {e}"))?;
    resolve_archive_bundle_path(fetched.path, sanitize_identifier(&mapped))
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

fn resolve_local_bundle_path(path: PathBuf) -> Result<StartBundleResolution, String> {
    if !path.exists() {
        return Err(format!("bundle path does not exist: {}", path.display()));
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

fn resolve_archive_bundle_path(
    archive_path: PathBuf,
    deployment_key: String,
) -> Result<StartBundleResolution, String> {
    if !archive_path.is_file() {
        return Err(format!(
            "bundle artifact is not a file: {}",
            archive_path.display()
        ));
    }
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let staging = temp.path().join("staging");
    fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let file_name = archive_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("bundle.bin")
        .to_string();
    fs::copy(&archive_path, staging.join(file_name)).map_err(|e| e.to_string())?;

    let extracted = temp.path().join("bundle");
    fs::create_dir_all(&extracted).map_err(|e| e.to_string())?;
    expand_into_target(&staging, &extracted)?;
    let bundle_dir = detect_bundle_root(&extracted);
    Ok(StartBundleResolution {
        bundle_dir,
        deployment_key,
        deploy_artifact: Some(archive_path),
        _hold: Some(temp),
    })
}

fn detect_bundle_root(extracted_root: &Path) -> PathBuf {
    if extracted_root.join("greentic.demo.yaml").exists() {
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
    if dirs.len() == 1 && dirs[0].join("greentic.demo.yaml").exists() {
        return dirs.remove(0);
    }
    extracted_root.to_path_buf()
}

fn map_remote_bundle_ref(reference: &str) -> Result<String, String> {
    if let Some(rest) = reference.strip_prefix("oci://") {
        return Ok(rest.to_string());
    }
    if let Some(rest) = reference.strip_prefix("repo://") {
        return map_registry_target(rest, env::var("GREENTIC_REPO_REGISTRY_BASE").ok()).ok_or_else(
            || {
                format!(
                    "repo:// reference {reference} requires GREENTIC_REPO_REGISTRY_BASE to map to OCI"
                )
            },
        );
    }
    if let Some(rest) = reference.strip_prefix("store://") {
        return map_registry_target(rest, env::var("GREENTIC_STORE_REGISTRY_BASE").ok()).ok_or_else(
            || {
                format!(
                    "store:// reference {reference} requires GREENTIC_STORE_REGISTRY_BASE to map to OCI"
                )
            },
        );
    }
    Err(format!(
        "unsupported bundle scheme for {reference}; expected local path, file://, oci://, repo://, or store://"
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

fn run_install(sub_matches: &ArgMatches, debug: bool, locale: &str) -> i32 {
    println!("{}", t(locale, "gtc.install.public_mode"));

    let preflight_status = ensure_install_prereqs(debug, locale);
    if preflight_status != 0 {
        return preflight_status;
    }

    let public_args = vec!["install".to_string(), "tools".to_string()];
    let public_status = passthrough(DEV_BIN, &public_args, debug, locale);
    if public_status != 0 {
        return public_status;
    }

    let tenant = sub_matches
        .get_one::<String>("tenant")
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());

    let Some(tenant) = tenant else {
        return 0;
    };

    println!(
        "{}",
        tf(
            locale,
            "gtc.install.tenant_mode",
            &[("tenant", tenant.as_str())]
        )
    );

    let cli_key = sub_matches.get_one::<String>("key").cloned();
    let key = match resolve_tenant_key(cli_key, &tenant, locale) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };

    let adapter = match build_adapter() {
        Ok(adapter) => adapter,
        Err(err) => {
            eprintln!(
                "{}: {err}",
                t(locale, "gtc.err.distribution_client_missing")
            );
            return 1;
        }
    };

    let manifest_ref = format!(
        "oci://ghcr.io/greentic-biz/customers-tools/{tenant}:latest",
        tenant = tenant
    );

    let manifest_bytes = match adapter.pull_bytes(&manifest_ref, &key) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!(
                "{}: {err}",
                tf(
                    locale,
                    "gtc.err.pull_failed",
                    &[("oci", manifest_ref.as_str())]
                )
            );
            return 1;
        }
    };

    let manifest: InstallManifest = match serde_json::from_slice(&manifest_bytes) {
        Ok(manifest) => manifest,
        Err(err) => {
            eprintln!("{}: {err}", t(locale, "gtc.err.invalid_manifest"));
            return 1;
        }
    };

    let cargo_bin_dir = match resolve_cargo_bin_dir() {
        Ok(path) => path,
        Err(err) => {
            eprintln!("{}: {err}", t(locale, "gtc.err.install_dir"));
            return 1;
        }
    };

    let artifacts_root = match resolve_artifacts_root() {
        Ok(path) => path,
        Err(err) => {
            eprintln!("{}: {err}", t(locale, "gtc.err.install_dir"));
            return 1;
        }
    };

    let mut any_failed = false;

    for item in manifest.items {
        let result = install_manifest_item(
            adapter.as_ref(),
            &key,
            &item,
            &cargo_bin_dir,
            &artifacts_root,
            locale,
        );
        match result {
            Ok(()) => {
                println!(
                    "{}",
                    tf(
                        locale,
                        "gtc.install.item_ok",
                        &[("kind", item.kind.as_str()), ("name", item.name.as_str())]
                    )
                );
            }
            Err(err) => {
                any_failed = true;
                eprintln!(
                    "{}: {err}",
                    tf(
                        locale,
                        "gtc.install.item_fail",
                        &[("kind", item.kind.as_str()), ("name", item.name.as_str())]
                    )
                );
            }
        }
    }

    if any_failed {
        eprintln!("{}", t(locale, "gtc.install.summary_failed"));
        1
    } else {
        println!("{}", t(locale, "gtc.install.summary_ok"));
        0
    }
}

fn ensure_install_prereqs(debug: bool, locale: &str) -> i32 {
    let installed_binstall = detect_binstall_version(debug, locale);
    let latest_binstall = latest_binstall_version(debug, locale);

    let needs_binstall_install = match (installed_binstall.as_deref(), latest_binstall.as_deref()) {
        (Some(installed), Some(latest)) => semver_compare(installed, latest).is_lt(),
        (None, _) => true,
        (Some(_), None) => true,
    };

    if needs_binstall_install {
        let install_binstall_args = vec![
            "install".to_string(),
            "cargo-binstall".to_string(),
            "--locked".to_string(),
        ];
        let status = run_cargo(&install_binstall_args, debug, locale);
        if status != 0 {
            return status;
        }
    }

    for package in [DEV_BIN, OP_BIN, BUNDLE_BIN] {
        let binstall_args = vec![
            "binstall".to_string(),
            "-y".to_string(),
            "--version".to_string(),
            "0.4".to_string(),
            package.to_string(),
        ];
        let status = run_cargo(&binstall_args, debug, locale);
        if status != 0 {
            return status;
        }
    }

    0
}

fn detect_binstall_version(debug: bool, locale: &str) -> Option<String> {
    let output = run_cargo_capture(&["binstall", "--version"], debug, locale)?;
    if !output.status.success() {
        return None;
    }
    parse_first_semver(&String::from_utf8_lossy(&output.stdout))
        .or_else(|| parse_first_semver(&String::from_utf8_lossy(&output.stderr)))
}

fn latest_binstall_version(debug: bool, locale: &str) -> Option<String> {
    let output = run_cargo_capture(&["search", "cargo-binstall", "--limit", "1"], debug, locale)?;
    if !output.status.success() {
        return None;
    }
    parse_first_semver(&String::from_utf8_lossy(&output.stdout))
}

fn parse_first_semver(text: &str) -> Option<String> {
    for token in text.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '.' || ch == '-')) {
        if token.chars().all(|ch| ch.is_ascii_digit() || ch == '.')
            && token.split('.').count() >= 2
            && token
                .split('.')
                .all(|part| !part.is_empty() && part.chars().all(|ch| ch.is_ascii_digit()))
        {
            return Some(token.to_string());
        }
    }
    None
}

fn semver_compare(a: &str, b: &str) -> std::cmp::Ordering {
    let pa = parse_numeric_version(a);
    let pb = parse_numeric_version(b);
    let max = pa.len().max(pb.len());
    for i in 0..max {
        let av = *pa.get(i).unwrap_or(&0);
        let bv = *pb.get(i).unwrap_or(&0);
        match av.cmp(&bv) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

fn parse_numeric_version(raw: &str) -> Vec<u64> {
    raw.split('.')
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect()
}

fn run_cargo_capture(args: &[&str], debug: bool, locale: &str) -> Option<std::process::Output> {
    if debug {
        eprintln!("{} cargo {:?}", t(locale, "gtc.debug.exec"), args);
    }
    ProcessCommand::new("cargo")
        .args(args)
        .env("GREENTIC_LOCALE", locale)
        .output()
        .ok()
}

fn run_cargo(args: &[String], debug: bool, locale: &str) -> i32 {
    if debug {
        eprintln!("{} cargo {:?}", t(locale, "gtc.debug.exec"), args);
    }

    match ProcessCommand::new("cargo")
        .args(args)
        .env("GREENTIC_LOCALE", locale)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
    {
        Ok(status) => status.code().unwrap_or(1),
        Err(err) => {
            eprintln!("{}: {err}", t(locale, "gtc.err.exec_failed"));
            1
        }
    }
}

fn run_wizard_menu(debug: bool, locale: &str) -> i32 {
    println!(
        "{}",
        t_or(locale, "gtc.wizard.title", "Greentic Developer Wizard")
    );
    println!();
    println!(
        "1) {}",
        t_or(
            locale,
            "gtc.wizard.option.pack",
            "Build / Update a Pack (flows + components)"
        )
    );
    println!(
        "2) {}",
        t_or(
            locale,
            "gtc.wizard.option.bundle",
            "Build / Update a Production Bundle"
        )
    );
    println!("0) {}", t_or(locale, "gtc.wizard.option.exit", "Exit"));
    println!();

    loop {
        print!(
            "{} ",
            t_or(locale, "gtc.wizard.prompt.select", "Select option:")
        );
        if io::stdout().flush().is_err() {
            return 1;
        }

        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            return 1;
        }

        match input.trim() {
            "1" => {
                let args = vec![
                    "wizard".to_string(),
                    "--locale".to_string(),
                    locale.to_string(),
                ];
                return passthrough(PACK_BIN, &args, debug, locale);
            }
            "2" => {
                let bundle = loop {
                    let Some(path) = prompt_bundle_path(locale) else {
                        continue;
                    };
                    if Path::new(&path).exists() {
                        eprintln!(
                            "{}",
                            tf(
                                locale,
                                "gtc.wizard.err.bundle_exists",
                                &[("path", path.as_str())]
                            )
                        );
                        continue;
                    }
                    break path;
                };
                let args = vec![
                    "demo".to_string(),
                    "new".to_string(),
                    bundle,
                    "--locale".to_string(),
                    locale.to_string(),
                ];
                return passthrough(OP_BIN, &args, debug, locale);
            }
            "0" => return 0,
            _ => {}
        }
    }
}

fn prompt_bundle_path(locale: &str) -> Option<String> {
    print!(
        "{} ",
        t_or(
            locale,
            "gtc.wizard.prompt.bundle_path",
            "Bundle directory (e.g. ./mybundle):"
        )
    );
    if io::stdout().flush().is_err() {
        return None;
    }

    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        return None;
    }

    let value = input.trim().to_string();
    if value.is_empty() {
        eprintln!(
            "{}",
            t_or(
                locale,
                "gtc.wizard.err.bundle_path_required",
                "Bundle path is required."
            )
        );
        return None;
    }
    Some(value)
}

fn install_manifest_item(
    adapter: &dyn dist::DistAdapter,
    key: &str,
    item: &ManifestItem,
    cargo_bin_dir: &Path,
    artifacts_root: &Path,
    locale: &str,
) -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let staged = temp.path().join("staged");
    fs::create_dir_all(&staged).map_err(|e| e.to_string())?;

    adapter
        .pull_to_dir(&item.oci, key, &staged)
        .map_err(|e| format!("{}: {e}", t(locale, "gtc.err.pull_failed")))?;

    match item.kind {
        ArtifactKind::Tool => install_tool_artifact(&staged, cargo_bin_dir, &item.name),
        ArtifactKind::Component => {
            install_non_tool_artifact(artifacts_root, "components", item, &staged)
        }
        ArtifactKind::Pack => install_non_tool_artifact(artifacts_root, "packs", item, &staged),
        ArtifactKind::Bundle => install_non_tool_artifact(artifacts_root, "bundles", item, &staged),
    }
}

fn install_non_tool_artifact(
    artifacts_root: &Path,
    folder: &str,
    item: &ManifestItem,
    staged: &Path,
) -> Result<(), String> {
    let target = artifacts_root.join(folder).join(&item.name);
    if target.exists() {
        fs::remove_dir_all(&target).map_err(|e| e.to_string())?;
    }
    fs::create_dir_all(&target).map_err(|e| e.to_string())?;
    expand_into_target(staged, &target)
}

fn install_tool_artifact(
    staged: &Path,
    cargo_bin_dir: &Path,
    fallback_name: &str,
) -> Result<(), String> {
    fs::create_dir_all(cargo_bin_dir).map_err(|e| e.to_string())?;

    let expanded = tempfile::tempdir().map_err(|e| e.to_string())?;
    expand_into_target(staged, expanded.path())?;

    let mut candidates = gather_tool_candidates(expanded.path())?;
    if candidates.is_empty() {
        let fallback =
            find_first_file(expanded.path())?.ok_or_else(|| "no tool binary found".to_string())?;
        candidates.push((fallback_name.to_string(), fallback));
    }

    for (name_hint, source) in candidates {
        let file_name = source
            .file_name()
            .and_then(|v| v.to_str())
            .map(|v| v.to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or(name_hint);

        let target = cargo_bin_dir.join(file_name);
        fs::copy(&source, &target).map_err(|e| e.to_string())?;
        set_executable_if_unix(&target)?;
    }

    Ok(())
}

fn gather_tool_candidates(root: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let files = list_files_recursive(root)?;
    let mut out = Vec::new();

    for file in files {
        let rel = file.strip_prefix(root).unwrap_or(&file);
        let in_bin_dir = rel.components().any(|c| c.as_os_str() == "bin");
        let file_name = match file.file_name().and_then(|v| v.to_str()) {
            Some(v) => v,
            None => continue,
        };

        let looks_tool_name = file_name == "gtc"
            || file_name.starts_with("greentic-")
            || file_name.ends_with(".exe")
            || file_name.ends_with(".cmd")
            || file_name.ends_with(".bat");

        if in_bin_dir || looks_tool_name {
            out.push((file_name.to_string(), file));
        }
    }

    Ok(out)
}

fn find_first_file(root: &Path) -> Result<Option<PathBuf>, String> {
    let mut files = list_files_recursive(root)?;
    files.sort();
    Ok(files.into_iter().next())
}

fn list_files_recursive(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    recurse_files(root, &mut out)?;
    Ok(out)
}

fn recurse_files(root: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.is_dir() {
            recurse_files(&path, out)?;
        } else if path.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

fn expand_into_target(source_dir: &Path, target_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(target_dir).map_err(|e| e.to_string())?;

    let files = list_files_recursive(source_dir)?;
    for file in files {
        let data = fs::read(&file).map_err(|e| e.to_string())?;

        if looks_like_zip(&data) {
            extract_zip_bytes(&data, target_dir)?;
            continue;
        }

        if looks_like_gzip(&data) && extract_targz_bytes(&data, target_dir).is_ok() {
            continue;
        }

        if extract_tar_bytes(&data, target_dir).is_ok() {
            continue;
        }

        let name = file
            .file_name()
            .ok_or_else(|| "invalid filename".to_string())?;
        fs::copy(&file, target_dir.join(name)).map_err(|e| e.to_string())?;
    }

    Ok(())
}

fn looks_like_zip(data: &[u8]) -> bool {
    data.len() >= 4 && &data[0..4] == b"PK\x03\x04"
}

fn looks_like_gzip(data: &[u8]) -> bool {
    data.len() >= 2 && data[0] == 0x1F && data[1] == 0x8B
}

fn extract_zip_bytes(data: &[u8], out_dir: &Path) -> Result<(), String> {
    let cursor = std::io::Cursor::new(data);
    let mut zip = zip::ZipArchive::new(cursor).map_err(|e| e.to_string())?;

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let Some(path) = entry.enclosed_name().map(|p| p.to_path_buf()) else {
            continue;
        };
        let target = safe_join(out_dir, &path)?;
        if entry.is_dir() {
            fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out = fs::File::create(&target).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
        set_executable_if_unix(&target)?;
    }

    Ok(())
}

fn extract_targz_bytes(data: &[u8], out_dir: &Path) -> Result<(), String> {
    let cursor = std::io::Cursor::new(data);
    let decoder = flate2::read::GzDecoder::new(cursor);
    let mut archive = tar::Archive::new(decoder);
    extract_tar_archive(&mut archive, out_dir)
}

fn extract_tar_bytes(data: &[u8], out_dir: &Path) -> Result<(), String> {
    let cursor = std::io::Cursor::new(data);
    let mut archive = tar::Archive::new(cursor);
    extract_tar_archive(&mut archive, out_dir)
}

fn extract_tar_archive<R: Read>(
    archive: &mut tar::Archive<R>,
    out_dir: &Path,
) -> Result<(), String> {
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path().map_err(|e| e.to_string())?.to_path_buf();
        let target = safe_join(out_dir, &path)?;

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        entry.unpack(&target).map_err(|e| e.to_string())?;
        if target.is_file() {
            set_executable_if_unix(&target)?;
        }
    }

    Ok(())
}

fn safe_join(base: &Path, rel: &Path) -> Result<PathBuf, String> {
    let mut clean = PathBuf::new();
    for comp in rel.components() {
        match comp {
            Component::Normal(v) => clean.push(v),
            Component::CurDir => {}
            _ => return Err("archive entry has unsafe path".to_string()),
        }
    }
    Ok(base.join(clean))
}

#[cfg(unix)]
fn set_executable_if_unix(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = fs::metadata(path).map_err(|e| e.to_string())?;
    let mut perms = metadata.permissions();
    let mode = perms.mode();
    perms.set_mode(mode | 0o755);
    fs::set_permissions(path, perms).map_err(|e| e.to_string())
}

#[cfg(not(unix))]
fn set_executable_if_unix(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn resolve_cargo_bin_dir() -> Result<PathBuf, String> {
    if let Ok(cargo_home) = env::var("CARGO_HOME")
        && !cargo_home.trim().is_empty()
    {
        return Ok(PathBuf::from(cargo_home).join("bin"));
    }

    let base = BaseDirs::new().ok_or_else(|| "failed to resolve home directory".to_string())?;
    Ok(base.home_dir().join(".cargo").join("bin"))
}

fn resolve_artifacts_root() -> Result<PathBuf, String> {
    let base = BaseDirs::new().ok_or_else(|| "failed to resolve home directory".to_string())?;
    Ok(base.home_dir().join(".greentic").join("artifacts"))
}

fn resolve_tenant_key(
    cli_key: Option<String>,
    tenant: &str,
    locale: &str,
) -> Result<String, String> {
    if let Some(key) = cli_key
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    {
        return Ok(key);
    }

    let env_name = tenant_env_var_name(tenant);
    if let Ok(key) = env::var(&env_name)
        && !key.trim().is_empty()
    {
        println!(
            "{}",
            tf(
                locale,
                "gtc.install.using_env_key",
                &[("env", env_name.as_str())]
            )
        );
        return Ok(key.trim().to_string());
    }

    let prompt = tf(locale, "gtc.install.prompt_key", &[("tenant", tenant)]);
    let key = rpassword::prompt_password(prompt).map_err(|e| e.to_string())?;
    if key.trim().is_empty() {
        return Err(t(locale, "gtc.err.key_required").into_owned());
    }
    Ok(key)
}

fn tenant_env_var_name(tenant: &str) -> String {
    let mut normalized = String::with_capacity(tenant.len());
    let mut prev_us = false;

    for ch in tenant.chars() {
        let upper = ch.to_ascii_uppercase();
        if upper.is_ascii_alphanumeric() {
            normalized.push(upper);
            prev_us = false;
        } else if !prev_us {
            normalized.push('_');
            prev_us = true;
        }
    }

    let trimmed = normalized.trim_matches('_').to_string();
    format!("GREENTIC_{}_KEY", trimmed)
}

fn passthrough(binary: &str, args: &[String], debug: bool, locale: &str) -> i32 {
    if debug {
        eprintln!("{} {} {:?}", t(locale, "gtc.debug.exec"), binary, args);
    }

    match ProcessCommand::new(binary)
        .args(args)
        .env("GREENTIC_LOCALE", locale)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
    {
        Ok(status) => status.code().unwrap_or(1),
        Err(err) => {
            if err.kind() == std::io::ErrorKind::NotFound {
                match binary {
                    DEV_BIN => eprintln!("{}", t(locale, "gtc.err.bin_missing_dev")),
                    OP_BIN => eprintln!("{}", t(locale, "gtc.err.bin_missing_op")),
                    SETUP_BIN => eprintln!("{}", t(locale, "gtc.err.bin_missing_setup")),
                    _ => eprintln!("{}", t(locale, "gtc.err.exec_failed")),
                }
            } else {
                eprintln!("{}: {err}", t(locale, "gtc.err.exec_failed"));
            }
            1
        }
    }
}

fn run_doctor(locale: &str) -> i32 {
    let mut failed = false;

    for binary in [DEV_BIN, OP_BIN, SETUP_BIN] {
        match ProcessCommand::new(binary).arg("--version").output() {
            Ok(output) => {
                let status_label = if output.status.success() {
                    t(locale, "gtc.doctor.ok")
                } else {
                    t(locale, "gtc.doctor.warn")
                };
                let version = first_non_empty_line(&String::from_utf8_lossy(&output.stdout))
                    .or_else(|| first_non_empty_line(&String::from_utf8_lossy(&output.stderr)))
                    .unwrap_or_else(|| t(locale, "gtc.doctor.version_unavailable").into_owned());
                println!("{binary}: {status_label} ({version})");
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                failed = true;
                println!("{binary}: {}", t(locale, "gtc.doctor.missing"));
                match binary {
                    DEV_BIN => eprintln!("{}", t(locale, "gtc.err.bin_missing_dev")),
                    OP_BIN => eprintln!("{}", t(locale, "gtc.err.bin_missing_op")),
                    SETUP_BIN => eprintln!("{}", t(locale, "gtc.err.bin_missing_setup")),
                    _ => {}
                }
            }
            Err(err) => {
                failed = true;
                println!("{binary}: {} ({err})", t(locale, "gtc.doctor.missing"));
            }
        }
    }

    if failed { 1 } else { 0 }
}

fn first_non_empty_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToOwned::to_owned)
}

fn t(locale: &str, key: &'static str) -> Cow<'static, str> {
    Cow::Owned(i18n().translate(locale, key))
}

fn t_or(locale: &str, key: &'static str, fallback: &'static str) -> String {
    let value = t(locale, key).into_owned();
    if value == key {
        fallback.to_string()
    } else {
        value
    }
}

fn tf(locale: &str, key: &'static str, replacements: &[(&str, &str)]) -> String {
    let mut value = t(locale, key).into_owned();
    for (name, replace) in replacements {
        let token = format!("{{{name}}}");
        value = value.replace(&token, replace);
    }
    value
}

fn leak_str(value: String) -> &'static str {
    Box::leak(value.into_boxed_str())
}

fn i18n() -> &'static I18nCatalog {
    static CATALOG: OnceLock<I18nCatalog> = OnceLock::new();
    CATALOG.get_or_init(I18nCatalog::load)
}

#[derive(Debug)]
struct I18nCatalog {
    default_locale: String,
    supported: HashSet<String>,
    dictionaries: HashMap<String, HashMap<String, String>>,
}

impl I18nCatalog {
    fn load() -> Self {
        let locales: Value = serde_json::from_str(LOCALES_JSON).expect("valid locales.json");
        let default_locale = locales
            .get("default")
            .and_then(Value::as_str)
            .unwrap_or("en")
            .to_string();

        let supported = locales
            .get("supported")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(normalize_locale)
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_else(|| {
                let mut set = HashSet::new();
                set.insert(normalize_locale(&default_locale));
                set
            });

        let mut dictionaries = HashMap::new();
        for (locale, raw_json) in EMBEDDED_LOCALES {
            match parse_flat_json_map(raw_json) {
                Ok(map) => {
                    let normalized_key = normalize_locale(locale);
                    // Don't overwrite existing entries - prefer the primary locale (e.g., "en" over "en-GB")
                    // since EMBEDDED_LOCALES is sorted alphabetically, "en" comes before "en-GB"
                    dictionaries.entry(normalized_key).or_insert(map);
                }
                Err(_e) => {
                    // Silently skip invalid JSON files
                }
            }
        }

        Self {
            default_locale,
            supported,
            dictionaries,
        }
    }

    fn default_locale(&self) -> &str {
        &self.default_locale
    }

    fn normalize_or_default(&self, locale: &str) -> String {
        let normalized = normalize_locale(locale);
        if self.supported.contains(&normalized) {
            return normalized;
        }
        normalize_locale(&self.default_locale)
    }

    fn translate(&self, locale: &str, key: &str) -> String {
        let normalized = self.normalize_or_default(locale);

        if let Some(text) = self
            .dictionaries
            .get(&normalized)
            .and_then(|map| map.get(key))
            .cloned()
        {
            return text;
        }

        let default = normalize_locale(&self.default_locale);
        self.dictionaries
            .get(&default)
            .and_then(|map| map.get(key))
            .cloned()
            .unwrap_or_else(|| key.to_string())
    }
}

fn parse_flat_json_map(input: &str) -> Result<HashMap<String, String>, String> {
    let value: Value = serde_json::from_str(input).map_err(|err| err.to_string())?;
    let obj = value
        .as_object()
        .ok_or_else(|| "JSON root must be an object".to_string())?;

    let mut map = HashMap::with_capacity(obj.len());
    for (k, v) in obj {
        let s = v
            .as_str()
            .ok_or_else(|| format!("translation value for '{k}' must be a string"))?;
        map.insert(k.clone(), s.to_string());
    }
    Ok(map)
}

#[derive(Debug, Deserialize)]
struct InstallManifest {
    #[allow(dead_code)]
    schema: String,
    items: Vec<ManifestItem>,
}

#[derive(Debug, Deserialize)]
struct ManifestItem {
    kind: ArtifactKind,
    name: String,
    oci: String,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum ArtifactKind {
    Tool,
    Component,
    Pack,
    Bundle,
}

impl ArtifactKind {
    fn as_str(&self) -> &'static str {
        match self {
            ArtifactKind::Tool => "tool",
            ArtifactKind::Component => "component",
            ArtifactKind::Pack => "pack",
            ArtifactKind::Bundle => "bundle",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_operator_wizard_args, collect_tail, detect_locale, locale_from_args,
        parse_start_cli_options, parse_start_request, resolve_tenant_key, tenant_env_var_name,
    };
    use clap::{Arg, ArgMatches, Command};
    use std::path::{Path, PathBuf};

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
    fn parse_start_request_rejects_bundle_override() {
        let err = parse_start_request(
            &["--bundle".to_string(), "/tmp/other".to_string()],
            PathBuf::from("/tmp/bundle"),
        )
        .unwrap_err();
        assert!(err.contains("--bundle is managed by gtc start"));
    }

    #[test]
    fn build_operator_wizard_args_prepends_wizard_and_locale() {
        let args =
            build_operator_wizard_args(&["--answers".to_string(), "a.json".to_string()], "en");
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
    fn build_operator_wizard_args_preserves_explicit_locale() {
        let args = build_operator_wizard_args(
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
    fn parse_start_cli_options_strips_deploy_flags() {
        let options = parse_start_cli_options(&[
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
            options.start_args,
            vec!["--tenant".to_string(), "demo".to_string()]
        );
    }
}
