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
use greentic_distributor_client::{DistClient, DistOptions, save_login_default};
use greentic_i18n::{normalize_locale, select_locale_with_sources};
use greentic_start::{
    CloudflaredModeArg, NatsModeArg, NgrokModeArg, RestartTarget, StartRequest, StopRequest,
    run_start_request, run_stop_request,
};
use greentic_types::decode_pack_manifest;
use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose, SanType,
};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_yaml::Value as YamlValue;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use zip::ZipArchive;

const DEV_BIN: &str = "greentic-dev";

const OP_BIN: &str = "greentic-operator";
const BUNDLE_BIN: &str = "greentic-bundle";
const DEPLOYER_BIN: &str = "greentic-deployer";
const SETUP_BIN: &str = "greentic-setup";
const DEFAULT_OPERATOR_IMAGE_DIGEST: &str =
    "sha256:fa1c82477a03dc2642fdadf5f8d5dc818cb7ed99905b50e53fbab7fec763a8eb";
const EMBEDDED_TERRAFORM_GTPACK: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/deployer/terraform.gtpack"
));

const LOCALES_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/i18n/locales.json"
));
include!(concat!(env!("OUT_DIR"), "/embedded_i18n.rs"));

fn main() {
    let raw_args: Vec<String> = env::args().collect();
    let exit_code = run(raw_args);
    std::process::exit(exit_code);
}

fn run(raw_args: Vec<String>) -> i32 {
    let i18n = i18n();
    let cli_locale = locale_from_args(&raw_args);
    let locale = detect_locale(&raw_args, i18n.default_locale());
    let raw_passthrough = parse_raw_passthrough(&raw_args);

    if let Some((binary, args)) =
        passthrough_help_request(raw_passthrough.as_ref(), &cli_locale, &locale)
    {
        let debug = raw_args.iter().any(|arg| arg == "--debug-router");
        return passthrough(binary, &args, debug, &locale);
    }

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
        Some(("add-admin", sub_matches)) => run_add_admin(sub_matches, &locale),
        Some(("remove-admin", sub_matches)) => run_remove_admin(sub_matches, &locale),
        Some(("start", sub_matches)) => run_start(sub_matches, debug, &locale),
        Some(("stop", sub_matches)) => run_stop(sub_matches, debug, &locale),
        Some(("dev", sub_matches)) => {
            let tail = collect_tail(sub_matches);
            let (binary, args) =
                route_passthrough_subcommand("dev", &tail, &locale).expect("dev route");
            passthrough(binary, &args, debug, &locale)
        }
        Some(("op", sub_matches)) => {
            let tail = collect_tail(sub_matches);
            let (binary, args) =
                route_passthrough_subcommand("op", &tail, &locale).expect("op route");
            passthrough(binary, &args, debug, &locale)
        }
        Some(("wizard", sub_matches)) => {
            let tail = collect_tail(sub_matches);
            let (binary, args) =
                route_passthrough_subcommand("wizard", &tail, &locale).expect("wizard route");
            passthrough(binary, &args, debug, &locale)
        }
        Some(("setup", sub_matches)) => {
            let tail = collect_tail(sub_matches);
            let (binary, args) =
                route_passthrough_subcommand("setup", &tail, &locale).expect("setup route");
            passthrough(binary, &args, debug, &locale)
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
            Command::new("add-admin")
                .about("Register an admin client certificate identity for a local bundle.")
                .arg(
                    Arg::new("bundle-ref")
                        .value_name("BUNDLE_REF")
                        .required(true)
                        .help("Local bundle directory to update."),
                )
                .arg(
                    Arg::new("cn")
                        .long("cn")
                        .value_name("CLIENT_CN")
                        .required(true)
                        .num_args(1)
                        .help("Client certificate Common Name allowed to access the admin API."),
                )
                .arg(
                    Arg::new("name")
                        .long("name")
                        .value_name("ADMIN_NAME")
                        .num_args(1)
                        .help("Optional human-readable admin label."),
                )
                .arg(
                    Arg::new("public-key-file")
                        .long("public-key-file")
                        .value_name("PATH")
                        .required(true)
                        .num_args(1)
                        .help("PEM/OpenSSH public key file for this admin."),
                ),
        )
        .subcommand(
            Command::new("remove-admin")
                .about("Remove an admin client certificate identity from a local bundle.")
                .arg(
                    Arg::new("bundle-ref")
                        .value_name("BUNDLE_REF")
                        .required(true)
                        .help("Local bundle directory to update."),
                )
                .arg(
                    Arg::new("cn")
                        .long("cn")
                        .value_name("CLIENT_CN")
                        .num_args(1)
                        .conflicts_with("name")
                        .help("Client certificate Common Name to remove."),
                )
                .arg(
                    Arg::new("name")
                        .long("name")
                        .value_name("ADMIN_NAME")
                        .num_args(1)
                        .conflicts_with("cn")
                        .help("Admin label to remove."),
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
                .arg(
                    Arg::new("deploy-bundle-source")
                        .long("deploy-bundle-source")
                        .value_name("BUNDLE_SOURCE")
                        .num_args(1)
                        .help("Override the remote bundle source passed to cloud deployers (for example https://.../bundle.gtbundle)."),
                )
                .arg(cmd_args.clone()),
        )
        .subcommand(
            Command::new("stop")
                .about(t_or(
                    locale,
                    "gtc.cmd.stop.about",
                    "Stop a bundle runtime or destroy a deployed environment.",
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

#[derive(Debug, Clone)]
struct RawPassthrough {
    subcommand: String,
    tail: Vec<String>,
}

fn parse_raw_passthrough(raw_args: &[String]) -> Option<RawPassthrough> {
    let mut iter = raw_args.iter().skip(1).peekable();

    while let Some(arg) = iter.next() {
        if arg == "--locale" {
            iter.next();
            continue;
        }
        if arg == "--debug-router" {
            continue;
        }
        if arg.starts_with("--locale=") {
            continue;
        }
        if arg.starts_with('-') {
            continue;
        }

        let subcommand = arg.to_string();
        let tail = iter.cloned().collect();
        return Some(RawPassthrough { subcommand, tail });
    }

    None
}

fn passthrough_help_request(
    raw: Option<&RawPassthrough>,
    _cli_locale: &Option<String>,
    locale: &str,
) -> Option<(&'static str, Vec<String>)> {
    let raw = raw?;
    if !raw.tail.iter().any(|arg| arg == "--help" || arg == "-h") {
        return None;
    }

    route_passthrough_subcommand(&raw.subcommand, &raw.tail, locale)
}

fn route_passthrough_subcommand(
    subcommand: &str,
    tail: &[String],
    locale: &str,
) -> Option<(&'static str, Vec<String>)> {
    match subcommand {
        "dev" => Some((DEV_BIN, tail.to_vec())),
        "op" => Some((OP_BIN, rewrite_legacy_op_args(tail))),
        "setup" => Some((SETUP_BIN, tail.to_vec())),
        "wizard" => Some((DEV_BIN, build_wizard_args(tail, locale))),
        _ => None,
    }
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

fn build_wizard_args(args: &[String], locale: &str) -> Vec<String> {
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
    Gcp,
    Azure,
}

impl StartTarget {
    fn as_str(self) -> &'static str {
        match self {
            StartTarget::Runtime => "runtime",
            StartTarget::SingleVm => "single-vm",
            StartTarget::Aws => "aws",
            StartTarget::Gcp => "gcp",
            StartTarget::Azure => "azure",
        }
    }
}

#[derive(Debug, Deserialize)]
struct DeploymentTargetsDocument {
    targets: Vec<DeploymentTargetRecord>,
}

#[derive(Debug, Deserialize)]
struct DeploymentTargetRecord {
    target: String,
    provider_pack: Option<String>,
    default: Option<bool>,
}

#[derive(Debug)]
struct StartCliOptions {
    start_args: Vec<String>,
    explicit_target: Option<StartTarget>,
    environment: Option<String>,
    provider_pack: Option<PathBuf>,
    app_pack: Option<PathBuf>,
    deploy_bundle_source: Option<String>,
}

#[derive(Debug)]
struct StopCliOptions {
    stop_args: Vec<String>,
    explicit_target: Option<StartTarget>,
    environment: Option<String>,
    provider_pack: Option<PathBuf>,
    app_pack: Option<PathBuf>,
    destroy: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct StartDeploymentState {
    target: String,
    bundle_fingerprint: String,
    bundle_ref: String,
    deployed_at_epoch_s: u64,
    artifact_path: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
struct AdminRegistryDocument {
    admins: Vec<AdminRegistryEntry>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
struct AdminRegistryEntry {
    name: Option<String>,
    client_cn: String,
    public_key: String,
    added_at_epoch_s: u64,
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
    let resolved = match resolve_bundle_reference(bundle_ref, locale) {
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
    let mut request =
        match parse_start_request(&cli_options.start_args, resolved.bundle_dir.clone()) {
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
    if request.admin {
        match ensure_admin_certs_ready(&resolved.bundle_dir, request.admin_certs_dir.as_deref()) {
            Ok(cert_dir) => request.admin_certs_dir = Some(cert_dir),
            Err(err) => {
                eprintln!(
                    "{}: {err}",
                    t_or(
                        locale,
                        "gtc.start.err.admin_certs_failed",
                        "failed to prepare admin certificates"
                    )
                );
                return 1;
            }
        }
    }
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
    println!("Selected deployment target: {}", target.as_str());
    println!("Bundle source: {}", bundle_ref);
    println!("Resolved bundle dir: {}", resolved.bundle_dir.display());
    if target != StartTarget::Runtime {
        println!("Deployment mode: deploy via {} target", target.as_str());
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
    println!("Deployment mode: local runtime");
    println!(
        "Starting tenant={} team={}",
        request.tenant.as_deref().unwrap_or("default"),
        request.team.as_deref().unwrap_or("default")
    );
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

fn run_add_admin(sub_matches: &ArgMatches, _locale: &str) -> i32 {
    let Some(bundle_ref) = sub_matches.get_one::<String>("bundle-ref") else {
        eprintln!("missing bundle ref");
        return 2;
    };
    let Some(client_cn) = sub_matches.get_one::<String>("cn") else {
        eprintln!("missing --cn");
        return 2;
    };
    let Some(public_key_file) = sub_matches.get_one::<String>("public-key-file") else {
        eprintln!("missing --public-key-file");
        return 2;
    };

    let bundle_dir = match resolve_local_mutable_bundle_dir(bundle_ref) {
        Ok(path) => path,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    let public_key = match fs::read_to_string(public_key_file) {
        Ok(value) => value.trim().to_string(),
        Err(err) => {
            eprintln!("failed to read public key file {}: {err}", public_key_file);
            return 1;
        }
    };
    if public_key.is_empty() {
        eprintln!("public key file is empty: {}", public_key_file);
        return 1;
    }

    let mut registry = match load_admin_registry(&bundle_dir) {
        Ok(doc) => doc,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    let name = sub_matches.get_one::<String>("name").cloned();
    upsert_admin_registry_entry(&mut registry, name.clone(), client_cn.clone(), public_key);
    if let Err(err) = save_admin_registry(&bundle_dir, &registry) {
        eprintln!("{err}");
        return 1;
    }

    println!(
        "admin added: cn={} name={} bundle={}",
        client_cn,
        name.as_deref().unwrap_or(""),
        bundle_dir.display()
    );
    0
}

fn run_remove_admin(sub_matches: &ArgMatches, _locale: &str) -> i32 {
    let Some(bundle_ref) = sub_matches.get_one::<String>("bundle-ref") else {
        eprintln!("missing bundle ref");
        return 2;
    };
    let selector_cn = sub_matches.get_one::<String>("cn").cloned();
    let selector_name = sub_matches.get_one::<String>("name").cloned();
    if selector_cn.is_none() && selector_name.is_none() {
        eprintln!("either --cn or --name is required");
        return 2;
    }

    let bundle_dir = match resolve_local_mutable_bundle_dir(bundle_ref) {
        Ok(path) => path,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    let mut registry = match load_admin_registry(&bundle_dir) {
        Ok(doc) => doc,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    let removed = remove_admin_registry_entry(
        &mut registry,
        selector_cn.as_deref(),
        selector_name.as_deref(),
    );
    if !removed {
        eprintln!("no matching admin entry found");
        return 1;
    }
    if let Err(err) = save_admin_registry(&bundle_dir, &registry) {
        eprintln!("{err}");
        return 1;
    }

    println!(
        "admin removed: cn={} name={} bundle={}",
        selector_cn.as_deref().unwrap_or(""),
        selector_name.as_deref().unwrap_or(""),
        bundle_dir.display()
    );
    0
}

fn run_stop(sub_matches: &ArgMatches, debug: bool, locale: &str) -> i32 {
    let Some(bundle_ref) = sub_matches.get_one::<String>("bundle-ref") else {
        eprintln!(
            "{}",
            t_or(
                locale,
                "gtc.stop.err.bundle_required",
                "bundle ref is required"
            )
        );
        return 2;
    };
    let tail = collect_tail(sub_matches);
    let cli_options = match parse_stop_cli_options(&tail) {
        Ok(value) => value,
        Err(err) => {
            eprintln!(
                "{}: {err}",
                t_or(
                    locale,
                    "gtc.stop.err.invalid_args",
                    "invalid stop arguments"
                )
            );
            return 2;
        }
    };
    let resolved = match resolve_bundle_reference(bundle_ref, locale) {
        Ok(value) => value,
        Err(err) => {
            eprintln!(
                "{}: {err}",
                t_or(
                    locale,
                    "gtc.stop.err.resolve_failed",
                    "failed to resolve bundle"
                )
            );
            return 1;
        }
    };
    let request = match parse_stop_request(&cli_options.stop_args, resolved.bundle_dir.clone()) {
        Ok(value) => value,
        Err(err) => {
            eprintln!(
                "{}: {err}",
                t_or(
                    locale,
                    "gtc.stop.err.invalid_args",
                    "invalid stop arguments"
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
                        "gtc.stop.err.target_select_failed",
                        "failed to choose deployment target"
                    )
                );
                return 2;
            }
        };
    match target {
        StartTarget::Runtime => {
            if cli_options.destroy {
                eprintln!(
                    "{}",
                    t_or(
                        locale,
                        "gtc.stop.err.runtime_destroy_unsupported",
                        "--destroy is not supported for runtime target"
                    )
                );
                return 2;
            }
            if debug {
                eprintln!(
                    "{} gtc-stop-lib bundle={:?} tenant={} team={}",
                    t(locale, "gtc.debug.exec"),
                    request.bundle,
                    request.tenant,
                    request.team
                );
            }
            match run_stop_request(request) {
                Ok(()) => 0,
                Err(err) => {
                    eprintln!(
                        "{}: {err}",
                        t_or(locale, "gtc.stop.err.run_failed", "failed to stop bundle")
                    );
                    1
                }
            }
        }
        _ => {
            if !cli_options.destroy {
                eprintln!(
                    "{}",
                    t_or(
                        locale,
                        "gtc.stop.err.destroy_required",
                        "deployed targets currently require --destroy; stop without destroy is not implemented"
                    )
                );
                return 2;
            }
            match destroy_bundle_deployment(
                bundle_ref,
                &resolved,
                &request,
                &cli_options,
                target,
                debug,
                locale,
            ) {
                Ok(()) => 0,
                Err(err) => {
                    eprintln!(
                        "{}: {err}",
                        t_or(
                            locale,
                            "gtc.stop.err.destroy_failed",
                            "failed to destroy deployed bundle"
                        )
                    );
                    1
                }
            }
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
                    .map_err(|err| err.to_string())?
                    .as_secs(),
                artifact_path: Some(artifact_path.display().to_string()),
            };
            save_deployment_state(&state_path, &state)?;
            Ok(())
        }
        StartTarget::Aws | StartTarget::Gcp | StartTarget::Azure => {
            println!("Applying cloud deployment target: {}", target.as_str());
            // For cloud targets, local deployment state is not authoritative
            // enough to prove the remote infrastructure still exists. Re-apply
            // on each start so the deployer reconciles remote state.
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

fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|err| {
        format!(
            "failed to read artifact {} for sha256: {err}",
            path.display()
        )
    })?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

fn resolve_remote_deploy_bundle_source_override() -> Option<String> {
    std::env::var("GREENTIC_DEPLOY_BUNDLE_SOURCE")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
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

fn admin_registry_path(bundle_dir: &Path) -> PathBuf {
    bundle_dir
        .join(".greentic")
        .join("admin")
        .join("admins.json")
}

fn load_admin_registry(bundle_dir: &Path) -> Result<AdminRegistryDocument, String> {
    let path = admin_registry_path(bundle_dir);
    if !path.exists() {
        return Ok(AdminRegistryDocument { admins: Vec::new() });
    }
    let raw = fs::read_to_string(&path)
        .map_err(|err| format!("failed to read admin registry {}: {err}", path.display()))?;
    serde_json::from_str(&raw)
        .map_err(|err| format!("failed to parse admin registry {}: {err}", path.display()))
}

fn save_admin_registry(bundle_dir: &Path, registry: &AdminRegistryDocument) -> Result<(), String> {
    let path = admin_registry_path(bundle_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            format!(
                "failed to create admin registry dir {}: {err}",
                parent.display()
            )
        })?;
    }
    let raw = serde_json::to_vec_pretty(registry)
        .map_err(|err| format!("failed to serialize admin registry: {err}"))?;
    fs::write(&path, raw)
        .map_err(|err| format!("failed to write admin registry {}: {err}", path.display()))
}

fn upsert_admin_registry_entry(
    registry: &mut AdminRegistryDocument,
    name: Option<String>,
    client_cn: String,
    public_key: String,
) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0);
    if let Some(existing) = registry
        .admins
        .iter_mut()
        .find(|entry| entry.client_cn == client_cn)
    {
        existing.name = name;
        existing.public_key = public_key;
        existing.added_at_epoch_s = now;
        return;
    }
    registry.admins.push(AdminRegistryEntry {
        name,
        client_cn,
        public_key,
        added_at_epoch_s: now,
    });
    registry
        .admins
        .sort_by(|left, right| left.client_cn.cmp(&right.client_cn));
}

fn remove_admin_registry_entry(
    registry: &mut AdminRegistryDocument,
    client_cn: Option<&str>,
    name: Option<&str>,
) -> bool {
    let before = registry.admins.len();
    registry.admins.retain(|entry| {
        if let Some(client_cn) = client_cn {
            return entry.client_cn != client_cn;
        }
        if let Some(name) = name {
            return entry.name.as_deref() != Some(name);
        }
        true
    });
    before != registry.admins.len()
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
        format!(
            "failed to write deployment spec {}: {err}",
            spec_path.display()
        )
    })?;
    Ok(spec_path)
}

fn resolve_admin_cert_dir(bundle_dir: &Path) -> PathBuf {
    for candidate in [
        bundle_dir.join(".greentic").join("admin").join("certs"),
        bundle_dir.join("certs"),
    ] {
        if candidate.join("ca.crt").exists()
            && candidate.join("server.crt").exists()
            && candidate.join("server.key").exists()
        {
            return candidate;
        }
    }
    PathBuf::from("/etc/greentic/admin")
}

fn ensure_admin_certs_ready(
    bundle_dir: &Path,
    explicit_dir: Option<&Path>,
) -> Result<PathBuf, String> {
    if let Some(explicit_dir) = explicit_dir {
        ensure_admin_cert_dir_contents(explicit_dir)?;
        return Ok(explicit_dir.to_path_buf());
    }

    let bundle_local = bundle_dir.join(".greentic").join("admin").join("certs");
    if has_admin_server_certs(&bundle_local) {
        ensure_admin_cert_dir_contents(&bundle_local)?;
        return Ok(bundle_local);
    }

    generate_dev_admin_cert_bundle(&bundle_local)?;
    Ok(bundle_local)
}

fn has_admin_server_certs(cert_dir: &Path) -> bool {
    cert_dir.join("ca.crt").exists()
        && cert_dir.join("server.crt").exists()
        && cert_dir.join("server.key").exists()
}

fn ensure_admin_cert_dir_contents(cert_dir: &Path) -> Result<(), String> {
    for required in ["ca.crt", "server.crt", "server.key"] {
        let path = cert_dir.join(required);
        if !path.exists() {
            return Err(format!(
                "required admin TLS file missing: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn generate_dev_admin_cert_bundle(cert_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(cert_dir).map_err(|err| {
        format!(
            "failed to create admin cert dir {}: {err}",
            cert_dir.display()
        )
    })?;

    let mut ca_params = CertificateParams::new(Vec::<String>::new())
        .map_err(|err| format!("failed to create admin CA params: {err}"))?;
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "greentic-admin-ca");
    ca_params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::CrlSign,
    ];
    let ca_key = KeyPair::generate().map_err(|err| format!("failed to generate CA key: {err}"))?;
    let ca_issuer = CertifiedIssuer::self_signed(ca_params, ca_key)
        .map_err(|err| format!("failed to generate CA certificate: {err}"))?;

    let mut server_params = CertificateParams::new(vec!["localhost".to_string()])
        .map_err(|err| format!("failed to create admin server cert params: {err}"))?;
    server_params
        .distinguished_name
        .push(DnType::CommonName, "greentic-admin-server");
    server_params.subject_alt_names.push(SanType::IpAddress(
        "127.0.0.1".parse().expect("static localhost ip"),
    ));
    server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    server_params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment,
    ];
    let server_key =
        KeyPair::generate().map_err(|err| format!("failed to generate server key: {err}"))?;
    let server_cert = server_params
        .signed_by(&server_key, &*ca_issuer)
        .map_err(|err| format!("failed to generate server certificate: {err}"))?;

    let mut client_params = CertificateParams::new(Vec::<String>::new())
        .map_err(|err| format!("failed to create admin client cert params: {err}"))?;
    client_params
        .distinguished_name
        .push(DnType::CommonName, "local-admin");
    client_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    client_params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment,
    ];
    let client_key =
        KeyPair::generate().map_err(|err| format!("failed to generate client key: {err}"))?;
    let client_cert = client_params
        .signed_by(&client_key, &*ca_issuer)
        .map_err(|err| format!("failed to generate client certificate: {err}"))?;

    fs::write(cert_dir.join("ca.crt"), ca_issuer.pem()).map_err(|err| {
        format!(
            "failed to write {}: {err}",
            cert_dir.join("ca.crt").display()
        )
    })?;
    fs::write(cert_dir.join("ca.key"), ca_issuer.key().serialize_pem()).map_err(|err| {
        format!(
            "failed to write {}: {err}",
            cert_dir.join("ca.key").display()
        )
    })?;
    fs::write(cert_dir.join("server.crt"), server_cert.pem()).map_err(|err| {
        format!(
            "failed to write {}: {err}",
            cert_dir.join("server.crt").display()
        )
    })?;
    fs::write(cert_dir.join("server.key"), server_key.serialize_pem()).map_err(|err| {
        format!(
            "failed to write {}: {err}",
            cert_dir.join("server.key").display()
        )
    })?;
    fs::write(cert_dir.join("client.crt"), client_cert.pem()).map_err(|err| {
        format!(
            "failed to write {}: {err}",
            cert_dir.join("client.crt").display()
        )
    })?;
    fs::write(cert_dir.join("client.key"), client_key.serialize_pem()).map_err(|err| {
        format!(
            "failed to write {}: {err}",
            cert_dir.join("client.key").display()
        )
    })?;
    Ok(())
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
    let bundle_artifact = prepare_deployable_bundle_artifact(resolved, debug, locale)?;
    let bundle_digest = sha256_file(&bundle_artifact)?;
    let remote_override = cli_options
        .deploy_bundle_source
        .clone()
        .or_else(resolve_remote_deploy_bundle_source_override);
    validate_cloud_deploy_inputs(
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
    print_cloud_deploy_contract_hint(target);
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
    run_binary_checked(
        DEPLOYER_BIN,
        &args,
        debug,
        locale,
        "multi-target deploy apply",
    )
}

fn print_cloud_deploy_contract_hint(target: StartTarget) {
    println!("Cloud deploy contract:");
    println!("  required remote bundle source:");
    println!("    --deploy-bundle-source https://.../bundle.gtbundle");
    match target {
        StartTarget::Aws => {
            println!("  required external Terraform vars:");
            println!("    GREENTIC_DEPLOY_TERRAFORM_VAR_REMOTE_STATE_BACKEND");
            println!("  optional Terraform vars:");
            println!("    GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE_DIGEST");
            println!("      default: {DEFAULT_OPERATOR_IMAGE_DIGEST}");
            println!("    GREENTIC_DEPLOY_TERRAFORM_VAR_DNS_NAME (personalized mode only)");
            println!("  internal AWS bootstrap now handles:");
            println!("    admin TLS server secrets");
        }
        StartTarget::Gcp | StartTarget::Azure => {
            println!("  additional target-specific Terraform vars may still be required via:");
            println!("    GREENTIC_DEPLOY_TERRAFORM_VAR_*");
        }
        StartTarget::SingleVm | StartTarget::Runtime => {}
    }
}

fn validate_cloud_deploy_inputs(
    target: StartTarget,
    remote_bundle_source: Option<&str>,
    bundle_dir: &Path,
    locale: &str,
) -> Result<(), String> {
    require_tool_in_path(
        "terraform",
        "install terraform and make sure it is available in PATH",
    )?;
    validate_public_base_url_for_static_routes(bundle_dir)?;
    match target {
        StartTarget::Aws => {
            ensure_cloud_credentials(target, locale)?;
            ensure_target_terraform_inputs(target)?;
            let remote_bundle_source = remote_bundle_source.ok_or_else(|| {
                "aws deploy requires a remote bundle source; pass --deploy-bundle-source https://.../bundle.gtbundle or set GREENTIC_DEPLOY_BUNDLE_SOURCE".to_string()
            })?;
            if !is_remote_bundle_source(remote_bundle_source) {
                return Err(format!(
                    "aws deploy requires a remote bundle source, got local path: {remote_bundle_source}"
                ));
            }
            validate_bundle_registry_mapping_env(remote_bundle_source)?;
            Ok(())
        }
        StartTarget::Gcp | StartTarget::Azure => {
            ensure_cloud_credentials(target, locale)?;
            ensure_target_terraform_inputs(target)?;
            let remote_bundle_source = remote_bundle_source.ok_or_else(|| {
                format!(
                    "{} deploy requires a remote bundle source; pass --deploy-bundle-source https://.../bundle.gtbundle or set GREENTIC_DEPLOY_BUNDLE_SOURCE",
                    target.as_str()
                )
            })?;
            if !is_remote_bundle_source(remote_bundle_source) {
                return Err(format!(
                    "{} deploy requires a remote bundle source, got local path: {remote_bundle_source}",
                    target.as_str()
                ));
            }
            validate_bundle_registry_mapping_env(remote_bundle_source)?;
            Ok(())
        }
        StartTarget::SingleVm | StartTarget::Runtime => Ok(()),
    }
}

fn validate_public_base_url_for_static_routes(bundle_dir: &Path) -> Result<(), String> {
    if !bundle_declares_static_routes(bundle_dir)? {
        return Ok(());
    }
    Ok(())
}

fn bundle_declares_static_routes(bundle_dir: &Path) -> Result<bool, String> {
    for root in [bundle_dir.join("providers"), bundle_dir.join("packs")] {
        if !root.exists() {
            continue;
        }
        if dir_declares_static_routes(&root)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn dir_declares_static_routes(root: &Path) -> Result<bool, String> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries =
            fs::read_dir(&dir).map_err(|err| format!("failed to read {}: {err}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|err| format!("failed to read dir entry: {err}"))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|err| format!("failed to stat {}: {err}", path.display()))?;
            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("gtpack") {
                continue;
            }
            if pack_declares_static_routes(&path)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn pack_declares_static_routes(path: &Path) -> Result<bool, String> {
    const EXT_STATIC_ROUTES_V1: &str = "greentic.static-routes.v1";
    let file =
        fs::File::open(path).map_err(|err| format!("failed to open {}: {err}", path.display()))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|err| format!("failed to open zip archive {}: {err}", path.display()))?;
    let mut manifest_entry = archive
        .by_name("manifest.cbor")
        .map_err(|err| format!("failed to open manifest.cbor in {}: {err}", path.display()))?;
    let mut bytes = Vec::new();
    manifest_entry
        .read_to_end(&mut bytes)
        .map_err(|err| format!("failed to read manifest.cbor in {}: {err}", path.display()))?;
    let manifest = decode_pack_manifest(&bytes).map_err(|err| {
        format!(
            "failed to decode pack manifest in {}: {err}",
            path.display()
        )
    })?;
    Ok(manifest
        .extensions
        .as_ref()
        .is_some_and(|extensions| extensions.contains_key(EXT_STATIC_ROUTES_V1)))
}

fn validate_bundle_registry_mapping_env(bundle_source: &str) -> Result<(), String> {
    if bundle_source.starts_with("repo://") {
        require_env_var("GREENTIC_REPO_REGISTRY_BASE")?;
    }
    if bundle_source.starts_with("store://") {
        require_env_var("GREENTIC_STORE_REGISTRY_BASE")?;
    }
    Ok(())
}

fn append_bundle_registry_args(args: &mut Vec<String>, bundle_source: &str) -> Result<(), String> {
    if bundle_source.starts_with("repo://") {
        let value = env::var("GREENTIC_REPO_REGISTRY_BASE").map_err(|_| {
            "missing required environment variable GREENTIC_REPO_REGISTRY_BASE".to_string()
        })?;
        if value.trim().is_empty() {
            return Err("GREENTIC_REPO_REGISTRY_BASE must not be empty".to_string());
        }
        args.push("--repo-registry-base".to_string());
        args.push(value);
    }
    if bundle_source.starts_with("store://") {
        let value = env::var("GREENTIC_STORE_REGISTRY_BASE").map_err(|_| {
            "missing required environment variable GREENTIC_STORE_REGISTRY_BASE".to_string()
        })?;
        if value.trim().is_empty() {
            return Err("GREENTIC_STORE_REGISTRY_BASE must not be empty".to_string());
        }
        args.push("--store-registry-base".to_string());
        args.push(value);
    }
    Ok(())
}

fn is_remote_bundle_source(value: &str) -> bool {
    matches_remote_bundle_ref(value)
}

fn matches_remote_bundle_ref(value: &str) -> bool {
    value.starts_with("http://")
        || value.starts_with("https://")
        || value.starts_with("oci://")
        || value.starts_with("repo://")
        || value.starts_with("store://")
}

fn require_env_var(name: &str) -> Result<(), String> {
    match env::var(name) {
        Ok(value) if !value.trim().is_empty() => Ok(()),
        _ => Err(format!("missing required environment variable: {name}")),
    }
}

fn missing_cloud_credentials_error(names: &[&str], help: &str) -> String {
    format!(
        "missing cloud credentials; {}. Expected one of: {}",
        help,
        names.join(", ")
    )
}

fn ensure_cloud_credentials(target: StartTarget, locale: &str) -> Result<(), String> {
    let (names, help) = match target {
        StartTarget::Aws => (
            &[
                "AWS_ACCESS_KEY_ID",
                "AWS_PROFILE",
                "AWS_DEFAULT_PROFILE",
                "AWS_WEB_IDENTITY_TOKEN_FILE",
            ][..],
            "set AWS credentials (for example AWS_ACCESS_KEY_ID/AWS_SECRET_ACCESS_KEY or AWS_PROFILE)",
        ),
        StartTarget::Azure => (
            &[
                "ARM_CLIENT_ID",
                "ARM_USE_OIDC",
                "AZURE_CLIENT_ID",
                "AZURE_TENANT_ID",
                "AZURE_SUBSCRIPTION_ID",
            ][..],
            "set Azure credentials (for example ARM_CLIENT_ID/ARM_TENANT_ID/ARM_SUBSCRIPTION_ID or the corresponding AZURE_* variables)",
        ),
        StartTarget::Gcp => (
            &[
                "GOOGLE_APPLICATION_CREDENTIALS",
                "GOOGLE_OAUTH_ACCESS_TOKEN",
                "CLOUDSDK_AUTH_ACCESS_TOKEN",
            ][..],
            "set GCP credentials (for example GOOGLE_APPLICATION_CREDENTIALS or CLOUDSDK_AUTH_ACCESS_TOKEN)",
        ),
        StartTarget::SingleVm | StartTarget::Runtime => return Ok(()),
    };
    if cloud_credentials_satisfied(target) {
        return Ok(());
    }
    if !can_prompt_interactively() {
        return Err(missing_cloud_credentials_error(names, help));
    }
    let _ = locale;
    println!(
        "Cloud credentials for {} are missing. gtc can collect them for this run.",
        target.as_str()
    );
    match target {
        StartTarget::Aws => prompt_aws_credentials()?,
        StartTarget::Azure => prompt_azure_credentials()?,
        StartTarget::Gcp => prompt_gcp_credentials()?,
        StartTarget::SingleVm | StartTarget::Runtime => {}
    }
    if cloud_credentials_satisfied(target) {
        Ok(())
    } else {
        Err(missing_cloud_credentials_error(names, help))
    }
}

fn ensure_target_terraform_inputs(target: StartTarget) -> Result<(), String> {
    let requirements: &[(&str, bool, Option<&str>)] = match target {
        StartTarget::Aws => &[(
            "GREENTIC_DEPLOY_TERRAFORM_VAR_REMOTE_STATE_BACKEND",
            true,
            None,
        )],
        StartTarget::Gcp => &[
            ("GREENTIC_DEPLOY_TERRAFORM_VAR_GCP_PROJECT_ID", true, None),
            (
                "GREENTIC_DEPLOY_TERRAFORM_VAR_GCP_REGION",
                true,
                Some("us-central1"),
            ),
        ],
        StartTarget::Azure => &[
            (
                "GREENTIC_DEPLOY_TERRAFORM_VAR_AZURE_KEY_VAULT_ID",
                true,
                None,
            ),
            (
                "GREENTIC_DEPLOY_TERRAFORM_VAR_AZURE_LOCATION",
                true,
                Some("westeurope"),
            ),
        ],
        StartTarget::SingleVm | StartTarget::Runtime => &[],
    };
    if requirements.is_empty() {
        return Ok(());
    }
    let missing: Vec<_> = requirements
        .iter()
        .filter(|(name, required, _)| *required && !env_var_present(name))
        .copied()
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    if !can_prompt_interactively() {
        return Err(format!(
            "missing required deployment configuration: {}",
            missing
                .iter()
                .map(|(name, _, _)| *name)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    println!(
        "Additional {} deployment inputs are required for this run.",
        target.as_str()
    );
    for (name, _, default) in missing {
        let prompt = match name {
            "GREENTIC_DEPLOY_TERRAFORM_VAR_REMOTE_STATE_BACKEND" => {
                "Terraform remote state backend:"
            }
            "GREENTIC_DEPLOY_TERRAFORM_VAR_GCP_PROJECT_ID" => "GCP project ID:",
            "GREENTIC_DEPLOY_TERRAFORM_VAR_GCP_REGION" => "GCP region:",
            "GREENTIC_DEPLOY_TERRAFORM_VAR_AZURE_KEY_VAULT_ID" => "Azure Key Vault resource ID:",
            "GREENTIC_DEPLOY_TERRAFORM_VAR_AZURE_LOCATION" => "Azure location:",
            _ => name,
        };
        let value = prompt_value_with_default(prompt, default)?;
        unsafe {
            env::set_var(name, value);
        }
    }
    Ok(())
}

fn cloud_credentials_satisfied(target: StartTarget) -> bool {
    match target {
        StartTarget::Aws => {
            env_var_present("AWS_PROFILE")
                || env_var_present("AWS_DEFAULT_PROFILE")
                || env_var_present("AWS_WEB_IDENTITY_TOKEN_FILE")
                || (env_var_present("AWS_ACCESS_KEY_ID")
                    && env_var_present("AWS_SECRET_ACCESS_KEY"))
        }
        StartTarget::Azure => {
            (env_var_present("ARM_CLIENT_ID")
                && env_var_present("ARM_TENANT_ID")
                && env_var_present("ARM_SUBSCRIPTION_ID")
                && (env_var_present("ARM_CLIENT_SECRET") || env_var_present("ARM_USE_OIDC")))
                || (env_var_present("AZURE_CLIENT_ID")
                    && env_var_present("AZURE_TENANT_ID")
                    && env_var_present("AZURE_SUBSCRIPTION_ID"))
        }
        StartTarget::Gcp => {
            env_var_present("GOOGLE_APPLICATION_CREDENTIALS")
                || env_var_present("GOOGLE_OAUTH_ACCESS_TOKEN")
                || env_var_present("CLOUDSDK_AUTH_ACCESS_TOKEN")
        }
        StartTarget::SingleVm | StartTarget::Runtime => true,
    }
}

fn env_var_present(name: &str) -> bool {
    env::var(name)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}

fn can_prompt_interactively() -> bool {
    if cfg!(test) {
        return false;
    }
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

fn prompt_aws_credentials() -> Result<(), String> {
    let mode = prompt_choice(
        "Select AWS credential input mode:",
        &[
            "Access key pair",
            "AWS profile",
            "Web identity token file",
            "Abort",
        ],
    )?;
    match mode {
        0 => {
            let access_key_id = prompt_non_empty("AWS access key ID:")?;
            let secret_access_key = prompt_secret("AWS secret access key:")?;
            let session_token = prompt_optional_secret("AWS session token (optional):")?;
            let default_region = prompt_optional("AWS default region (optional):")?;
            unsafe {
                env::set_var("AWS_ACCESS_KEY_ID", access_key_id);
                env::set_var("AWS_SECRET_ACCESS_KEY", secret_access_key);
                if let Some(value) = session_token {
                    env::set_var("AWS_SESSION_TOKEN", value);
                }
                if let Some(value) = default_region {
                    env::set_var("AWS_DEFAULT_REGION", value);
                }
            }
        }
        1 => {
            let profile = prompt_non_empty("AWS profile:")?;
            let default_region = prompt_optional("AWS default region (optional):")?;
            unsafe {
                env::set_var("AWS_PROFILE", profile);
                if let Some(value) = default_region {
                    env::set_var("AWS_DEFAULT_REGION", value);
                }
            }
        }
        2 => {
            let token_file = prompt_non_empty("AWS web identity token file:")?;
            let role_arn = prompt_optional("AWS role ARN (optional):")?;
            unsafe {
                env::set_var("AWS_WEB_IDENTITY_TOKEN_FILE", token_file);
                if let Some(value) = role_arn {
                    env::set_var("AWS_ROLE_ARN", value);
                }
            }
        }
        _ => return Err("cloud deploy aborted before AWS credentials were configured".to_string()),
    }
    Ok(())
}

fn prompt_azure_credentials() -> Result<(), String> {
    let mode = prompt_choice(
        "Select Azure credential input mode:",
        &["ARM service principal", "Azure OIDC", "Abort"],
    )?;
    match mode {
        0 => {
            let subscription_id = prompt_non_empty("Azure subscription ID:")?;
            let tenant_id = prompt_non_empty("Azure tenant ID:")?;
            let client_id = prompt_non_empty("Azure client ID:")?;
            let client_secret = prompt_secret("Azure client secret:")?;
            unsafe {
                env::set_var("ARM_SUBSCRIPTION_ID", subscription_id);
                env::set_var("ARM_TENANT_ID", tenant_id);
                env::set_var("ARM_CLIENT_ID", client_id);
                env::set_var("ARM_CLIENT_SECRET", client_secret);
            }
        }
        1 => {
            let subscription_id = prompt_non_empty("Azure subscription ID:")?;
            let tenant_id = prompt_non_empty("Azure tenant ID:")?;
            let client_id = prompt_non_empty("Azure client ID:")?;
            unsafe {
                env::set_var("ARM_SUBSCRIPTION_ID", subscription_id);
                env::set_var("ARM_TENANT_ID", tenant_id);
                env::set_var("ARM_CLIENT_ID", client_id);
                env::set_var("ARM_USE_OIDC", "true");
            }
        }
        _ => {
            return Err(
                "cloud deploy aborted before Azure credentials were configured".to_string(),
            );
        }
    }
    Ok(())
}

fn prompt_gcp_credentials() -> Result<(), String> {
    let mode = prompt_choice(
        "Select GCP credential input mode:",
        &["Service account credentials file", "Access token", "Abort"],
    )?;
    match mode {
        0 => {
            let credentials_file = prompt_non_empty("GOOGLE_APPLICATION_CREDENTIALS path:")?;
            unsafe {
                env::set_var("GOOGLE_APPLICATION_CREDENTIALS", credentials_file);
            }
        }
        1 => {
            let access_token = prompt_secret("GCP access token:")?;
            unsafe {
                env::set_var("CLOUDSDK_AUTH_ACCESS_TOKEN", access_token);
            }
        }
        _ => return Err("cloud deploy aborted before GCP credentials were configured".to_string()),
    }
    Ok(())
}

fn prompt_choice(prompt: &str, options: &[&str]) -> Result<usize, String> {
    println!("{prompt}");
    for (idx, option) in options.iter().enumerate() {
        println!("{} ) {}", idx + 1, option);
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
        .map_err(|_| "invalid selection".to_string())?;
    let idx = choice.saturating_sub(1);
    if idx < options.len() {
        Ok(idx)
    } else {
        Err("invalid selection".to_string())
    }
}

fn prompt_non_empty(prompt: &str) -> Result<String, String> {
    loop {
        let value = prompt_optional(prompt)?;
        if let Some(value) = value {
            return Ok(value);
        }
        println!("A value is required.");
    }
}

fn prompt_optional(prompt: &str) -> Result<Option<String>, String> {
    print!("{prompt} ");
    io::stdout().flush().map_err(|err| err.to_string())?;
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(|err| err.to_string())?;
    let trimmed = input.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_string()))
    }
}

fn prompt_value_with_default(prompt: &str, default: Option<&str>) -> Result<String, String> {
    loop {
        match default {
            Some(default) => {
                print!("{prompt} [{default}] ");
            }
            None => {
                print!("{prompt} ");
            }
        }
        io::stdout().flush().map_err(|err| err.to_string())?;
        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .map_err(|err| err.to_string())?;
        let trimmed = input.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
        if let Some(default) = default {
            return Ok(default.to_string());
        }
        println!("A value is required.");
    }
}

fn prompt_secret(prompt: &str) -> Result<String, String> {
    loop {
        let value = rpassword::prompt_password(prompt).map_err(|err| err.to_string())?;
        if !value.trim().is_empty() {
            return Ok(value);
        }
        println!("A value is required.");
    }
}

fn prompt_optional_secret(prompt: &str) -> Result<Option<String>, String> {
    let value = rpassword::prompt_password(prompt).map_err(|err| err.to_string())?;
    if value.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(value))
    }
}

fn require_tool_in_path(binary: &str, help: &str) -> Result<(), String> {
    if binary_in_path(binary) {
        Ok(())
    } else {
        Err(format!(
            "required tool `{binary}` not found in PATH; {help}"
        ))
    }
}

fn binary_in_path(binary: &str) -> bool {
    env::var_os("PATH")
        .map(|path| {
            env::split_paths(&path).any(|dir| resolve_binary_in_dir(&dir, binary).is_some())
        })
        .unwrap_or(false)
}

fn destroy_bundle_deployment(
    bundle_ref: &str,
    resolved: &StartBundleResolution,
    request: &StopRequest,
    cli_options: &StopCliOptions,
    target: StartTarget,
    debug: bool,
    locale: &str,
) -> Result<(), String> {
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
        StartTarget::Runtime => Err("runtime target cannot be destroyed via deployer".to_string()),
    }
}

fn stop_request_to_start_request(
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

fn load_or_prepare_single_vm_artifact(
    resolved: &StartBundleResolution,
    request: &StopRequest,
    debug: bool,
    locale: &str,
) -> Result<PathBuf, String> {
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

fn run_multi_target_deployer_destroy(
    resolved: &StartBundleResolution,
    request: &StopRequest,
    cli_options: &StopCliOptions,
    target: StartTarget,
    debug: bool,
    locale: &str,
) -> Result<(), String> {
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
    run_binary_checked(
        DEPLOYER_BIN,
        &args,
        debug,
        locale,
        "multi-target deploy destroy",
    )
}

fn run_single_vm_destroy(spec_path: &Path, debug: bool, locale: &str) -> Result<(), String> {
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

fn remove_deployment_state_file(deployment_key: &str, target: StartTarget) -> Result<(), String> {
    let path = deployment_state_path(deployment_key, target)?;
    if !path.exists() {
        return Ok(());
    }
    fs::remove_file(&path).map_err(|err| {
        format!(
            "failed to remove deployment state {}: {err}",
            path.display()
        )
    })
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
    let command = resolve_binary_command(binary);
    let mut process = ProcessCommand::new(&command);
    process.args(args).env("GREENTIC_LOCALE", locale);
    apply_default_deploy_env(&mut process);
    let output = process
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
    let command = resolve_binary_command(binary);
    let mut process = ProcessCommand::new(&command);
    process
        .args(args)
        .env("GREENTIC_LOCALE", locale)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    apply_default_deploy_env(&mut process);
    process
        .status()
        .map_err(|err| format!("failed to execute {binary}: {err}"))
}

fn apply_default_deploy_env(process: &mut ProcessCommand) {
    if env::var_os("GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE_DIGEST").is_none() {
        process.env(
            "GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE_DIGEST",
            DEFAULT_OPERATOR_IMAGE_DIGEST,
        );
    }
}

fn fingerprint_bundle_dir(bundle_dir: &Path) -> Result<String, String> {
    let mut files = Vec::new();
    collect_bundle_entries(bundle_dir, bundle_dir, &mut files)?;
    files.sort();
    Ok(normalize_bundle_fingerprint(&files.join("\n")))
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

fn normalize_bundle_fingerprint(raw: &str) -> String {
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
        admin: false,
        admin_port: 8443,
        admin_certs_dir: None,
        admin_allowed_clients: Vec::new(),
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
            "--admin" => request.admin = true,
            "--admin-port" => {
                idx += 1;
                request.admin_port = required_value(tail, idx, "--admin-port")?
                    .parse()
                    .map_err(|_| "invalid --admin-port".to_string())?;
            }
            "--admin-certs-dir" => {
                idx += 1;
                request.admin_certs_dir = Some(PathBuf::from(required_value(
                    tail,
                    idx,
                    "--admin-certs-dir",
                )?));
            }
            "--admin-allowed-clients" => {
                idx += 1;
                let value = required_value(tail, idx, "--admin-allowed-clients")?;
                request.admin_allowed_clients.extend(
                    value
                        .split(',')
                        .filter(|part| !part.is_empty())
                        .map(|part| part.to_string()),
                );
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
                } else if other == "--admin" {
                    request.admin = true;
                } else if let Some(value) = other.strip_prefix("--admin-port=") {
                    request.admin_port = value
                        .parse()
                        .map_err(|_| "invalid --admin-port value".to_string())?;
                } else if let Some(value) = other.strip_prefix("--admin-certs-dir=") {
                    request.admin_certs_dir = Some(PathBuf::from(value));
                } else if let Some(value) = other.strip_prefix("--admin-allowed-clients=") {
                    request.admin_allowed_clients.extend(
                        value
                            .split(',')
                            .filter(|part| !part.is_empty())
                            .map(|part| part.to_string()),
                    );
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

fn parse_stop_request(tail: &[String], bundle_dir: PathBuf) -> Result<StopRequest, String> {
    let mut request = StopRequest {
        bundle: Some(bundle_dir.display().to_string()),
        state_dir: None,
        tenant: "demo".to_string(),
        team: "default".to_string(),
    };

    let mut idx = 0usize;
    while idx < tail.len() {
        let arg = &tail[idx];
        match arg.as_str() {
            "--tenant" => {
                idx += 1;
                request.tenant = required_value(tail, idx, "--tenant")?;
            }
            "--team" => {
                idx += 1;
                request.team = required_value(tail, idx, "--team")?;
            }
            "--state-dir" => {
                idx += 1;
                request.state_dir = Some(PathBuf::from(required_value(tail, idx, "--state-dir")?));
            }
            "--bundle" => {
                return Err(
                    "--bundle is managed by gtc stop; pass the bundle ref as the main argument"
                        .to_string(),
                );
            }
            other => {
                if let Some(value) = other.strip_prefix("--tenant=") {
                    request.tenant = value.to_string();
                } else if let Some(value) = other.strip_prefix("--team=") {
                    request.team = value.to_string();
                } else if let Some(value) = other.strip_prefix("--state-dir=") {
                    request.state_dir = Some(PathBuf::from(value));
                } else if other.starts_with("--bundle=") {
                    return Err(
                        "--bundle is managed by gtc stop; pass the bundle ref as the main argument"
                            .to_string(),
                    );
                } else {
                    return Err(format!("unsupported stop argument: {other}"));
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
    let mut deploy_bundle_source = None;
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
            "--deploy-bundle-source" => {
                idx += 1;
                deploy_bundle_source = Some(required_value(tail, idx, "--deploy-bundle-source")?);
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
                } else if let Some(value) = arg.strip_prefix("--deploy-bundle-source=") {
                    deploy_bundle_source = Some(value.to_string());
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
        deploy_bundle_source,
    })
}

fn parse_stop_cli_options(tail: &[String]) -> Result<StopCliOptions, String> {
    let mut stop_args = Vec::new();
    let mut explicit_target = None;
    let mut environment = None;
    let mut provider_pack = None;
    let mut app_pack = None;
    let mut destroy = false;
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
            "--destroy" => destroy = true,
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
                    stop_args.push(arg.clone());
                }
            }
        }
        idx += 1;
    }
    Ok(StopCliOptions {
        stop_args,
        explicit_target,
        environment,
        provider_pack,
        app_pack,
        destroy,
    })
}

fn parse_start_target(value: &str) -> Result<StartTarget, String> {
    match value.trim() {
        "runtime" | "local" => Ok(StartTarget::Runtime),
        "single-vm" | "single_vm" => Ok(StartTarget::SingleVm),
        "aws" => Ok(StartTarget::Aws),
        "gcp" => Ok(StartTarget::Gcp),
        "azure" => Ok(StartTarget::Azure),
        other => Err(format!(
            "unsupported --target value {other}; expected runtime, single-vm, aws, gcp, or azure"
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
    if let Some(default_target) = load_default_deployment_target(bundle_dir)? {
        return Ok(default_target);
    }
    let mut deploy_targets = detect_bundle_deployment_targets(bundle_dir)?;
    deploy_targets.sort_by_key(|value| match value {
        StartTarget::Aws => 0,
        StartTarget::Gcp => 1,
        StartTarget::Azure => 2,
        StartTarget::SingleVm => 3,
        StartTarget::Runtime => 4,
    });
    deploy_targets.dedup();
    if deploy_targets.is_empty() {
        return Ok(StartTarget::Runtime);
    }
    if deploy_targets.len() == 1 {
        return Ok(deploy_targets[0]);
    }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(format!(
            "multiple start targets are available ({}); rerun with --target",
            deploy_targets
                .iter()
                .map(|value| value.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    prompt_start_target(&deploy_targets, locale)
}

fn detect_bundle_deployment_targets(bundle_dir: &Path) -> Result<Vec<StartTarget>, String> {
    if let Some(explicit_targets) = load_explicit_deployment_targets(bundle_dir)? {
        return Ok(explicit_targets);
    }
    Ok(Vec::new())
}

fn load_explicit_deployment_targets(bundle_dir: &Path) -> Result<Option<Vec<StartTarget>>, String> {
    let Some(doc) = load_deployment_targets_document(bundle_dir)? else {
        return Ok(None);
    };
    let mut targets = Vec::new();
    for record in doc.targets {
        let target = parse_start_target(&record.target)?;
        targets.push(target);
    }
    Ok(Some(targets))
}

fn load_default_deployment_target(bundle_dir: &Path) -> Result<Option<StartTarget>, String> {
    let Some(doc) = load_deployment_targets_document(bundle_dir)? else {
        return Ok(None);
    };
    for record in doc.targets {
        if record.default.unwrap_or(false) {
            return Ok(Some(parse_start_target(&record.target)?));
        }
    }
    Ok(None)
}

fn load_deployment_targets_document(
    bundle_dir: &Path,
) -> Result<Option<DeploymentTargetsDocument>, String> {
    let path = bundle_dir.join(".greentic").join("deployment-targets.json");
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let doc: DeploymentTargetsDocument = serde_json::from_str(&raw)
        .map_err(|err| format!("failed to parse {}: {err}", path.display()))?;
    Ok(Some(doc))
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
    if let Some(path) = resolve_target_provider_pack_from_metadata(bundle_dir, target)? {
        return Ok(path);
    }
    if let Some(path) = resolve_canonical_target_provider_pack(target) {
        return Ok(path);
    }
    Err(format!(
        "no deployer provider pack found for target {}; define deployment_targets metadata or install greentic-deployer with dist packs",
        target.as_str(),
    ))
}

fn resolve_canonical_target_provider_pack(target: StartTarget) -> Option<PathBuf> {
    let filename = canonical_target_provider_pack_filename(target)?;
    let deployer_bin = resolve_companion_binary(DEPLOYER_BIN)?;
    resolve_canonical_target_provider_pack_from(Some(deployer_bin.as_path()), filename)
}

fn resolve_canonical_target_provider_pack_from(
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

fn canonical_target_provider_pack_filename(target: StartTarget) -> Option<&'static str> {
    match target {
        StartTarget::Aws | StartTarget::Gcp | StartTarget::Azure => Some("terraform.gtpack"),
        StartTarget::Runtime | StartTarget::SingleVm => None,
    }
}

fn resolve_target_provider_pack_from_metadata(
    bundle_dir: &Path,
    target: StartTarget,
) -> Result<Option<PathBuf>, String> {
    let Some(doc) = load_deployment_targets_document(bundle_dir)? else {
        return Ok(None);
    };
    for record in doc.targets {
        let parsed_target = parse_start_target(&record.target)?;
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

fn resolve_deploy_app_pack_path(
    bundle_dir: &Path,
    override_path: Option<&PathBuf>,
) -> Result<PathBuf, String> {
    if let Some(path) = override_path {
        return Ok(path.clone());
    }
    if let Some(path) = resolve_app_pack_path_from_bundle_metadata(bundle_dir)? {
        return Ok(path);
    }
    let default_pack_ref = bundle_dir.join("default.gtpack");
    if default_pack_ref.exists() {
        let raw = fs::read_to_string(&default_pack_ref).map_err(|err| {
            format!(
                "failed to read default pack reference {}: {err}",
                default_pack_ref.display()
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
        _ => Err(
            "cloud deployment requires a canonical app pack; set bundle.yaml app_packs, add default.gtpack, or pass --app-pack explicitly"
                .to_string(),
        ),
    }
}

fn resolve_app_pack_path_from_bundle_metadata(
    bundle_dir: &Path,
) -> Result<Option<PathBuf>, String> {
    let bundle_path = bundle_dir.join("bundle.yaml");
    if !bundle_path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&bundle_path)
        .map_err(|err| format!("failed to read {}: {err}", bundle_path.display()))?;
    let doc: YamlValue = serde_yaml::from_str(&raw)
        .map_err(|err| format!("failed to parse {}: {err}", bundle_path.display()))?;
    let Some(app_packs) = doc.get("app_packs").and_then(YamlValue::as_sequence) else {
        return Ok(None);
    };
    let Some(reference) = app_packs.first().and_then(YamlValue::as_str) else {
        return Ok(None);
    };
    let candidate = if Path::new(reference).is_absolute() {
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

fn resolve_bundle_reference(
    reference: &str,
    locale: &str,
) -> Result<StartBundleResolution, String> {
    let trimmed = reference.trim();
    if trimmed.is_empty() {
        return Err("bundle reference is empty".to_string());
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
        .map_err(|e| format!("failed to fetch remote bundle {trimmed}: {e}"))?;
    resolve_archive_bundle_path(fetched, sanitize_identifier(&mapped))
}

fn resolve_local_mutable_bundle_dir(reference: &str) -> Result<PathBuf, String> {
    let Some(path) = parse_local_bundle_ref(reference) else {
        return Err("admin registry updates require a local bundle directory path".to_string());
    };
    if !path.exists() {
        return Err(format!("bundle path does not exist: {}", path.display()));
    }
    if !path.is_dir() {
        return Err(format!(
            "admin registry updates require a local bundle directory, got: {}",
            path.display()
        ));
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
        "unsupported bundle scheme for {reference}; expected local path, file://, http(s)://, oci://, repo://, or store://"
    ))
}

fn download_https_bundle_to_tempfile(url: &str, locale: &str) -> Result<PathBuf, String> {
    let bytes = fetch_https_bytes(url, "", locale, "application/octet-stream")?;
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let file_name = url_file_name(url).unwrap_or_else(|| "bundle.gtbundle".to_string());
    let path = temp.path().join(file_name);
    fs::write(&path, bytes).map_err(|e| e.to_string())?;
    if path.extension().and_then(|value| value.to_str()) != Some("gtbundle") {
        return Err(format!(
            "remote bundle URL must point to a .gtbundle archive: {url}"
        ));
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

    if let Err(err) = ensure_deployer_dist_pack(debug) {
        eprintln!(
            "{}: {err}",
            tf(
                locale,
                "gtc.install.item_fail",
                &[("kind", "asset"), ("name", "terraform.gtpack")]
            )
        );
        return 1;
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

    let manifest_url = match resolve_tenant_manifest_url(&tenant, &key, locale) {
        Ok(url) => url,
        Err(err) => {
            eprintln!("{}: {err}", t(locale, "gtc.err.pull_failed"));
            return 1;
        }
    };

    let manifest_bytes = match fetch_download_bytes_with_auth(&manifest_url, &key, locale) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!(
                "{}: {err}",
                tf(
                    locale,
                    "gtc.err.pull_failed",
                    &[("oci", manifest_url.as_str())]
                )
            );
            return 1;
        }
    };

    let manifest: TenantInstallManifest = match serde_json::from_slice(&manifest_bytes) {
        Ok(manifest) => manifest,
        Err(err) => {
            eprintln!("{}: {err}", t(locale, "gtc.err.invalid_manifest"));
            return 1;
        }
    };

    if manifest.schema_version != "1" {
        eprintln!(
            "{}: unsupported schema_version '{}'",
            t(locale, "gtc.err.invalid_manifest"),
            manifest.schema_version
        );
        return 1;
    }

    if manifest.tenant != tenant {
        eprintln!(
            "{}: tenant '{}' does not match requested tenant '{}'",
            t(locale, "gtc.err.invalid_manifest"),
            manifest.tenant,
            tenant
        );
        return 1;
    }

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
    let current_os = match current_install_os() {
        Ok(value) => value,
        Err(code) => return code,
    };
    let current_arch = match current_install_arch() {
        Ok(value) => value,
        Err(code) => return code,
    };

    for tool in manifest.tools {
        let result = install_tenant_tool_reference(
            &tool,
            &tenant,
            &key,
            &current_os,
            &current_arch,
            &cargo_bin_dir,
            locale,
        );
        match result {
            Ok(()) => {
                println!(
                    "{}",
                    tf(
                        locale,
                        "gtc.install.item_ok",
                        &[("kind", "tool"), ("name", tool.id.as_str())]
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
                        &[("kind", "tool"), ("name", tool.id.as_str())]
                    )
                );
            }
        }
    }

    for doc in manifest.docs {
        let result = install_tenant_doc_reference(&doc, &tenant, &key, &artifacts_root, locale);
        match result {
            Ok(paths) => {
                println!(
                    "{}",
                    tf(
                        locale,
                        "gtc.install.item_ok",
                        &[("kind", "doc"), ("name", doc.id.as_str())]
                    )
                );
                for path in paths {
                    println!("  -> {}", path.display());
                }
            }
            Err(err) => {
                any_failed = true;
                eprintln!(
                    "{}: {err}",
                    tf(
                        locale,
                        "gtc.install.item_fail",
                        &[("kind", "doc"), ("name", doc.id.as_str())]
                    )
                );
            }
        }
    }

    for asset in manifest.store_assets {
        let result = install_store_asset_reference(&asset, &tenant, &key, &artifacts_root, locale);
        match result {
            Ok(paths) => {
                println!(
                    "{}",
                    tf(
                        locale,
                        "gtc.install.item_ok",
                        &[("kind", "store asset"), ("name", asset.id.as_str())]
                    )
                );
                for path in paths {
                    println!("  -> {}", path.display());
                }
            }
            Err(err) => {
                any_failed = true;
                eprintln!(
                    "{}: {err}",
                    tf(
                        locale,
                        "gtc.install.item_fail",
                        &[("kind", "store asset"), ("name", asset.id.as_str())]
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

    for package in [DEV_BIN, OP_BIN, BUNDLE_BIN, SETUP_BIN, DEPLOYER_BIN] {
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

fn ensure_deployer_dist_pack(debug: bool) -> Result<(), String> {
    let cargo_bin_dir = resolve_cargo_bin_dir()?;
    let dist_dir = cargo_bin_dir.join("dist");
    let target = dist_dir.join("terraform.gtpack");
    if target.is_file() {
        return Ok(());
    }

    fs::create_dir_all(&dist_dir).map_err(|e| e.to_string())?;
    fs::write(&target, EMBEDDED_TERRAFORM_GTPACK).map_err(|e| e.to_string())?;

    if debug {
        eprintln!("installed deployer pack at {}", target.display());
    }

    Ok(())
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

fn install_tenant_tool_reference(
    tool_ref: &TenantManifestReference,
    _tenant: &str,
    key: &str,
    current_os: &str,
    current_arch: &str,
    cargo_bin_dir: &Path,
    locale: &str,
) -> Result<(), String> {
    let tool: ToolManifest = fetch_json_with_auth(&tool_ref.url, key, locale)?;
    let target = tool
        .install
        .targets
        .iter()
        .find(|target| target.os == current_os && target.arch == current_arch)
        .ok_or_else(|| {
            format!(
                "no install target for tool '{}' on {current_os}/{current_arch}",
                tool.id
            )
        })?;

    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let staged = temp.path().join("staged");
    fs::create_dir_all(&staged).map_err(|e| e.to_string())?;
    download_url_into_dir(
        &target.url,
        key,
        &staged,
        Some(&tool.install.binary_name),
        locale,
    )?;
    install_tool_artifact(&staged, cargo_bin_dir, &tool.install.binary_name)
}

fn install_tenant_doc_reference(
    doc_ref: &TenantManifestReference,
    _tenant: &str,
    key: &str,
    artifacts_root: &Path,
    locale: &str,
) -> Result<Vec<PathBuf>, String> {
    let manifest: DocManifest = fetch_json_with_auth(&doc_ref.url, key, locale)?;
    let mut installed = Vec::new();
    for doc in manifest.entries()? {
        if doc.download_file_name.contains('/')
            || doc.download_file_name.contains('\\')
            || doc.download_file_name.is_empty()
        {
            return Err(format!(
                "invalid doc file name '{}'",
                doc.download_file_name
            ));
        }

        let docs_root = artifacts_root.join("docs");
        fs::create_dir_all(&docs_root).map_err(|e| e.to_string())?;
        let target = safe_join(&docs_root, Path::new(&doc.default_relative_path))?
            .join(&doc.download_file_name);
        download_url_to_path(&doc.source.url, key, &target, locale)?;
        installed.push(target);
    }
    Ok(installed)
}

fn install_store_asset_reference(
    asset_ref: &TenantManifestReference,
    tenant: &str,
    key: &str,
    artifacts_root: &Path,
    locale: &str,
) -> Result<Vec<PathBuf>, String> {
    let manifest: StoreAssetManifest = fetch_json_with_auth(&asset_ref.url, key, locale)?;
    let mut installed = Vec::new();

    for item in manifest.items {
        let resolved = rewrite_store_tenant_placeholder(&item, tenant);
        installed.push(install_store_asset_item(
            &resolved,
            tenant,
            key,
            artifacts_root,
            locale,
        )?);
    }
    Ok(installed)
}

fn install_store_asset_item(
    store_url: &str,
    tenant: &str,
    key: &str,
    artifacts_root: &Path,
    locale: &str,
) -> Result<PathBuf, String> {
    save_store_login(tenant, key)?;
    let client = DistClient::new(DistOptions::default());
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("failed to build tokio runtime: {e}"))?;
    let artifact = rt
        .block_on(client.download_store_artifact(store_url))
        .map_err(|e| format!("{}: {e}", t(locale, "gtc.err.pull_failed")))?;
    let file_name = store_asset_file_name(store_url)
        .ok_or_else(|| format!("unable to derive filename from {store_url}"))?;
    let target = store_asset_target_path(artifacts_root, &file_name)?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(&target, artifact.bytes)
        .map_err(|e| format!("failed to write {}: {e}", target.display()))?;
    Ok(target)
}

fn download_url_into_dir(
    url: &str,
    key: &str,
    target_dir: &Path,
    fallback_name: Option<&str>,
    locale: &str,
) -> Result<PathBuf, String> {
    let file_name = url_file_name(url)
        .filter(|value| !value.is_empty())
        .or_else(|| fallback_name.map(|value| value.to_string()))
        .ok_or_else(|| format!("unable to derive file name from {url}"))?;
    let target = target_dir.join(file_name);
    download_url_to_path(url, key, &target, locale)?;
    Ok(target)
}

fn download_url_to_path(url: &str, key: &str, target: &Path, locale: &str) -> Result<(), String> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let bytes = fetch_download_bytes_with_auth(url, key, locale)?;
    let mut file = fs::File::create(target).map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    Ok(())
}

fn fetch_json_with_auth<T: serde::de::DeserializeOwned>(
    url: &str,
    key: &str,
    locale: &str,
) -> Result<T, String> {
    let bytes = fetch_json_bytes_with_auth(url, key, locale)?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

fn fetch_json_bytes_with_auth(url: &str, key: &str, locale: &str) -> Result<Vec<u8>, String> {
    if let Some(path) = file_url_path(url) {
        return fs::read(&path).map_err(|e| format!("failed to read {}: {e}", path.display()));
    }

    match url.split_once("://").map(|(scheme, _)| scheme) {
        Some("http") | Some("https") => {
            if let Some(asset_url) = resolve_github_release_asset_api_url(url, key, locale)? {
                fetch_asset_bytes(&asset_url, key, locale)
            } else {
                fetch_https_json_or_file_bytes(url, key, locale)
            }
        }
        _ => Err(format!("unsupported download URL scheme for {url}")),
    }
}

fn fetch_download_bytes_with_auth(url: &str, key: &str, locale: &str) -> Result<Vec<u8>, String> {
    if let Some(path) = file_url_path(url) {
        return fs::read(&path).map_err(|e| format!("failed to read {}: {e}", path.display()));
    }

    match url.split_once("://").map(|(scheme, _)| scheme) {
        Some("http") | Some("https") => {
            if let Some(asset_url) = resolve_github_release_asset_api_url(url, key, locale)? {
                fetch_asset_bytes(&asset_url, key, locale)
            } else {
                fetch_https_bytes(url, key, locale, "application/octet-stream")
            }
        }
        _ => Err(format!("unsupported download URL scheme for {url}")),
    }
}

fn fetch_https_json_or_file_bytes(url: &str, key: &str, locale: &str) -> Result<Vec<u8>, String> {
    fetch_https_bytes(url, key, locale, "application/vnd.github+json")
}

fn fetch_asset_bytes(url: &str, key: &str, locale: &str) -> Result<Vec<u8>, String> {
    fetch_https_bytes(url, key, locale, "application/octet-stream")
}

fn fetch_https_bytes(url: &str, key: &str, locale: &str, accept: &str) -> Result<Vec<u8>, String> {
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("failed to create HTTP client: {e}"))?;

    let mut current = reqwest::Url::parse(url).map_err(|e| format!("invalid URL {url}: {e}"))?;
    for _ in 0..10 {
        let response = client
            .get(current.clone())
            .header("Accept", accept)
            .header("Authorization", format!("Bearer {key}"))
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", format!("gtc/{}", env!("CARGO_PKG_VERSION")))
            .send()
            .map_err(|e| format!("{}: {e}", t(locale, "gtc.err.pull_failed")))?;

        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .ok_or_else(|| format!("redirect missing Location header for {}", current))?
                .to_str()
                .map_err(|e| format!("invalid redirect Location for {}: {e}", current))?;
            current = current
                .join(location)
                .map_err(|e| format!("invalid redirect target {location}: {e}"))?;
            continue;
        }

        if !response.status().is_success() {
            return Err(format!(
                "{}: HTTP {} for {}",
                t(locale, "gtc.err.pull_failed"),
                response.status(),
                current
            ));
        }

        return response
            .bytes()
            .map(|bytes| bytes.to_vec())
            .map_err(|e| e.to_string());
    }

    Err(format!("too many redirects while fetching {url}"))
}

fn save_store_login(tenant: &str, token: &str) -> Result<(), String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("failed to build tokio runtime: {e}"))?;
    rt.block_on(save_login_default(tenant, token))
        .map_err(|e| e.to_string())
}

fn rewrite_store_tenant_placeholder(url: &str, tenant: &str) -> String {
    url.replace("/{tenant}/", &format!("/{tenant}/"))
}

fn resolve_tenant_manifest_url(tenant: &str, key: &str, locale: &str) -> Result<String, String> {
    if let Ok(template) = env::var("GTC_TENANT_MANIFEST_URL_TEMPLATE") {
        return Ok(template.replace("{tenant}", tenant));
    }
    let release = fetch_github_release("greentic-biz", "customers-tools", "latest", key, locale)?;
    let asset_name = format!("{tenant}.json");
    release
        .assets
        .into_iter()
        .find(|asset| asset.name == asset_name)
        .map(|asset| asset.url)
        .ok_or_else(|| format!("tenant manifest asset '{asset_name}' not found in latest release"))
}

fn resolve_github_release_asset_api_url(
    url: &str,
    key: &str,
    locale: &str,
) -> Result<Option<String>, String> {
    let parsed = match reqwest::Url::parse(url) {
        Ok(parsed) => parsed,
        Err(_) => return Ok(None),
    };
    if parsed.scheme() != "https" || parsed.host_str() != Some("github.com") {
        return Ok(None);
    }

    let segments = match parsed.path_segments() {
        Some(segments) => segments.collect::<Vec<_>>(),
        None => return Ok(None),
    };
    if segments.len() != 6 || segments[2] != "releases" {
        return Ok(None);
    }

    let owner = segments[0];
    let repo = segments[1];
    let asset_name = segments[5];

    let tag = match (segments[3], segments[4]) {
        ("latest", "download") => "latest",
        ("download", tag) => tag,
        _ => return Ok(None),
    };

    let release = fetch_github_release(owner, repo, tag, key, locale)?;
    Ok(release
        .assets
        .into_iter()
        .find(|asset| asset.name == asset_name)
        .map(|asset| asset.url))
}

fn fetch_github_release(
    owner: &str,
    repo: &str,
    tag: &str,
    key: &str,
    locale: &str,
) -> Result<GithubRelease, String> {
    let url = if tag == "latest" {
        format!("https://api.github.com/repos/{owner}/{repo}/releases/latest")
    } else {
        format!("https://api.github.com/repos/{owner}/{repo}/releases/tags/{tag}")
    };
    let bytes = fetch_https_json_or_file_bytes(&url, key, locale)?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

fn store_asset_file_name(store_url: &str) -> Option<String> {
    let trimmed = store_url.trim_start_matches("store://");
    let last = trimmed.rsplit('/').next()?;
    Some(last.split(':').next().unwrap_or(last).to_string())
}

fn store_asset_target_path(artifacts_root: &Path, file_name: &str) -> Result<PathBuf, String> {
    let rel = if file_name.ends_with(".gtpack") {
        PathBuf::from("packs").join(file_name)
    } else if file_name.ends_with(".gtbundle") {
        PathBuf::from("bundles").join(file_name)
    } else if file_name.ends_with(".wasm") {
        PathBuf::from("components").join(file_name)
    } else {
        PathBuf::from("store_assets").join(file_name)
    };
    safe_join(artifacts_root, &rel)
}

fn url_file_name(url: &str) -> Option<String> {
    let trimmed = url.trim_end_matches('/');
    trimmed
        .rsplit('/')
        .next()
        .map(|segment| segment.split('?').next().unwrap_or(segment))
        .filter(|segment| !segment.is_empty())
        .map(|segment| segment.to_string())
}

fn current_install_os() -> Result<String, i32> {
    match env::consts::OS {
        "linux" | "macos" | "windows" => Ok(env::consts::OS.to_string()),
        other => {
            eprintln!("unsupported install OS '{other}'");
            Err(1)
        }
    }
}

fn current_install_arch() -> Result<String, i32> {
    if let Some(runtime) = detect_runtime_install_arch()
        && let Some(normalized) = normalize_install_arch(&runtime)
    {
        return Ok(normalized.to_string());
    }

    if let Some(normalized) = normalize_install_arch(env::consts::ARCH) {
        return Ok(normalized.to_string());
    }

    eprintln!(
        "unsupported install architecture '{}' (runtime) / '{}' (build)",
        detect_runtime_install_arch().unwrap_or_else(|| "unknown".to_string()),
        env::consts::ARCH
    );
    Err(1)
}

fn detect_runtime_install_arch() -> Option<String> {
    if cfg!(target_os = "macos")
        && let Some(value) = query_command_trimmed("sysctl", &["-n", "hw.optional.arm64"])
    {
        match value.as_str() {
            "1" => return Some("arm64".to_string()),
            "0" => return Some("x86_64".to_string()),
            _ => {}
        }
    }

    if cfg!(windows) {
        return env::var("PROCESSOR_ARCHITEW6432")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| env::var("PROCESSOR_ARCHITECTURE").ok())
            .map(|v| v.trim().to_string());
    }

    query_command_trimmed("uname", &["-m"])
}

fn query_command_trimmed(command: &str, args: &[&str]) -> Option<String> {
    ProcessCommand::new(command)
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn file_url_path(url: &str) -> Option<PathBuf> {
    let path = url.strip_prefix("file://")?;
    if path.is_empty() {
        return None;
    }
    Some(PathBuf::from(path))
}

fn normalize_install_arch(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "x86_64" | "amd64" => Some("x86_64"),
        "aarch64" | "arm64" => Some("aarch64"),
        _ => None,
    }
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

        if looks_like_squashfs(&data) {
            extract_squashfs_file(&file, target_dir)?;
            continue;
        }

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

fn looks_like_squashfs(data: &[u8]) -> bool {
    data.len() >= 4 && &data[0..4] == b"hsqs"
}

fn extract_squashfs_file(path: &Path, out_dir: &Path) -> Result<(), String> {
    let output = ProcessCommand::new("unsquashfs")
        .arg("-no-progress")
        .arg("-dest")
        .arg(out_dir)
        .arg(path)
        .output()
        .map_err(|e| format!("failed to run unsquashfs for {}: {e}", path.display()))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    Err(format!(
        "failed to extract squashfs bundle {}: {}{}{}",
        path.display(),
        stdout.trim(),
        if !stdout.trim().is_empty() && !stderr.trim().is_empty() {
            " "
        } else {
            ""
        },
        stderr.trim()
    ))
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
    loop {
        let key = rpassword::prompt_password(&prompt).map_err(|e| e.to_string())?;
        if !key.trim().is_empty() {
            return Ok(key);
        }
        eprintln!("{}", t(locale, "gtc.err.key_required"));
    }
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

    let command = resolve_binary_command(binary);

    match ProcessCommand::new(&command)
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
                    DEV_BIN => print_missing_dev_message(locale),
                    OP_BIN => print_missing_op_message(locale),
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

    for binary in [DEV_BIN, OP_BIN, BUNDLE_BIN, SETUP_BIN, DEPLOYER_BIN] {
        let command = resolve_binary_command(binary);
        match ProcessCommand::new(&command).arg("--version").output() {
            Ok(output) => {
                let status_label = if output.status.success() {
                    t(locale, "gtc.doctor.ok")
                } else {
                    t(locale, "gtc.doctor.warn")
                };
                let version = first_non_empty_line(&String::from_utf8_lossy(&output.stdout))
                    .or_else(|| first_non_empty_line(&String::from_utf8_lossy(&output.stderr)))
                    .unwrap_or_else(|| t(locale, "gtc.doctor.version_unavailable").into_owned());
                println!("{binary}: {status_label} ({version}) [{}]", command);
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                failed = true;
                println!(
                    "{binary}: {} [{}]",
                    t(locale, "gtc.doctor.missing"),
                    command
                );
                match binary {
                    DEV_BIN => print_missing_dev_message(locale),
                    OP_BIN => print_missing_op_message(locale),
                    BUNDLE_BIN => eprintln!(
                        "{}",
                        t_or(
                            locale,
                            "gtc.err.bin_missing_bundle",
                            "greentic-bundle not found in PATH. Install with: cargo install greentic-bundle",
                        )
                    ),
                    SETUP_BIN => eprintln!("{}", t(locale, "gtc.err.bin_missing_setup")),
                    DEPLOYER_BIN => eprintln!(
                        "{}",
                        t_or(
                            locale,
                            "gtc.err.bin_missing_deployer",
                            "greentic-deployer not found in PATH. Install with: cargo install greentic-deployer",
                        )
                    ),
                    _ => {}
                }
            }
            Err(err) => {
                failed = true;
                println!(
                    "{binary}: {} ({err}) [{}]",
                    t(locale, "gtc.doctor.missing"),
                    command
                );
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

fn print_missing_dev_message(locale: &str) {
    eprintln!("{}", t(locale, "gtc.err.bin_missing_dev"));
    eprintln!(
        "{}",
        t_or(
            locale,
            "gtc.err.bin_missing_dev_install_hint",
            "Run `gtc install` first."
        )
    );
}

fn print_missing_op_message(locale: &str) {
    eprintln!("{}", t(locale, "gtc.err.bin_missing_op"));
    eprintln!(
        "{}",
        t_or(
            locale,
            "gtc.err.bin_missing_op_install_hint",
            "Run `gtc install` first."
        )
    );
}

fn resolve_binary_command(binary: &str) -> String {
    if let Some(path) = resolve_companion_binary(binary) {
        return path.display().to_string();
    }
    match binary {
        DEV_BIN => env::var("GREENTIC_DEV_BIN").unwrap_or_else(|_| binary.to_string()),
        _ => binary.to_string(),
    }
}

fn resolve_companion_binary(binary: &str) -> Option<PathBuf> {
    resolve_companion_binary_from(env::current_exe().ok().as_deref(), binary)
}

fn resolve_companion_binary_from(current_exe: Option<&Path>, binary: &str) -> Option<PathBuf> {
    let env_override = companion_binary_env_override(binary);
    if let Some(path) = env_override {
        return Some(PathBuf::from(path));
    }

    if let Some(current_exe) = current_exe {
        let exe_dir = current_exe.parent()?;
        if let Some(sibling) = resolve_binary_in_dir(exe_dir, binary) {
            return Some(sibling);
        }

        if let Some(workspace_candidate) = resolve_workspace_local_binary(current_exe, binary) {
            return Some(workspace_candidate);
        }
    }

    if let Ok(cargo_bin_dir) = resolve_cargo_bin_dir()
        && let Some(cargo_candidate) = resolve_binary_in_dir(&cargo_bin_dir, binary)
    {
        return Some(cargo_candidate);
    }
    None
}

fn resolve_binary_in_dir(dir: &Path, binary: &str) -> Option<PathBuf> {
    binary_file_candidates(dir, binary)
        .into_iter()
        .find(|candidate| candidate.is_file())
}

fn binary_file_candidates(dir: &Path, binary: &str) -> Vec<PathBuf> {
    let mut candidates = vec![dir.join(binary)];
    let exe_suffix = env::consts::EXE_SUFFIX;
    if !exe_suffix.is_empty() && !binary.ends_with(exe_suffix) {
        candidates.push(dir.join(format!("{binary}{exe_suffix}")));
    }
    candidates
}

fn resolve_workspace_local_binary(current_exe: &Path, binary: &str) -> Option<PathBuf> {
    let repo_dir = current_exe.parent()?.parent()?.parent()?;
    let workspace_root = repo_dir.parent()?;
    let repo_name = match binary {
        DEV_BIN => "greentic-dev",
        OP_BIN => "greentic-operator",
        BUNDLE_BIN => "greentic-bundle",
        DEPLOYER_BIN => "greentic-deployer",
        SETUP_BIN => "greentic-setup",
        _ => return None,
    };
    let candidate = workspace_root
        .join(repo_name)
        .join("target")
        .join("debug")
        .as_path()
        .to_path_buf();
    resolve_binary_in_dir(&candidate, binary)
}

fn companion_binary_env_override(binary: &str) -> Option<std::ffi::OsString> {
    match binary {
        DEV_BIN => env::var_os("GREENTIC_DEV_BIN"),
        OP_BIN => env::var_os("GREENTIC_OPERATOR_BIN"),
        BUNDLE_BIN => env::var_os("GREENTIC_BUNDLE_BIN"),
        DEPLOYER_BIN => env::var_os("GREENTIC_DEPLOYER_BIN"),
        SETUP_BIN => env::var_os("GREENTIC_SETUP_BIN"),
        _ => None,
    }
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
struct TenantInstallManifest {
    #[serde(rename = "$schema")]
    #[allow(dead_code)]
    schema: Option<String>,
    schema_version: String,
    tenant: String,
    tools: Vec<TenantManifestReference>,
    docs: Vec<TenantManifestReference>,
    #[serde(default)]
    store_assets: Vec<TenantManifestReference>,
}

#[derive(Debug, Deserialize)]
struct TenantManifestReference {
    id: String,
    url: String,
}

#[derive(Debug, Deserialize)]
struct ToolManifest {
    #[allow(dead_code)]
    schema_version: String,
    id: String,
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    description: String,
    install: ToolInstallManifest,
    #[allow(dead_code)]
    docs: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ToolInstallManifest {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    kind: String,
    binary_name: String,
    targets: Vec<ToolInstallTarget>,
}

#[derive(Debug, Deserialize)]
struct ToolInstallTarget {
    os: String,
    arch: String,
    url: String,
    #[allow(dead_code)]
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct DocManifest {
    #[allow(dead_code)]
    schema_version: String,
    #[allow(dead_code)]
    id: String,
    title: Option<String>,
    source: Option<DocSource>,
    download_file_name: Option<String>,
    default_relative_path: Option<String>,
    docs: Option<Vec<DocManifestEntry>>,
}

#[derive(Debug, Deserialize, Clone)]
struct DocManifestEntry {
    #[allow(dead_code)]
    title: String,
    source: DocSource,
    download_file_name: String,
    default_relative_path: String,
}

#[derive(Debug, Deserialize, Clone)]
struct DocSource {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    kind: String,
    url: String,
}

#[derive(Debug, Deserialize)]
struct StoreAssetManifest {
    #[allow(dead_code)]
    schema_version: String,
    items: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    assets: Vec<GithubReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubReleaseAsset {
    url: String,
    name: String,
}

impl DocManifest {
    fn entries(self) -> Result<Vec<DocManifestEntry>, String> {
        if let Some(entries) = self.docs {
            return Ok(entries);
        }
        match (
            self.title,
            self.source,
            self.download_file_name,
            self.default_relative_path,
        ) {
            (Some(title), Some(source), Some(download_file_name), Some(default_relative_path)) => {
                Ok(vec![DocManifestEntry {
                    title,
                    source,
                    download_file_name,
                    default_relative_path,
                }])
            }
            _ => Err("doc manifest must contain either top-level doc fields or docs[]".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AdminRegistryDocument, DEV_BIN, StartBundleResolution, StartTarget, admin_registry_path,
        build_wizard_args, collect_tail, detect_bundle_root, detect_locale,
        ensure_admin_certs_ready, fingerprint_bundle_dir, locale_from_args,
        normalize_bundle_fingerprint, normalize_install_arch, parse_start_cli_options,
        parse_start_request, parse_stop_cli_options, parse_stop_request,
        remove_admin_registry_entry, resolve_admin_cert_dir,
        resolve_canonical_target_provider_pack_from, resolve_companion_binary_from,
        resolve_deploy_app_pack_path, resolve_local_mutable_bundle_dir,
        resolve_target_provider_pack, resolve_tenant_key, rewrite_store_tenant_placeholder,
        route_passthrough_subcommand, save_admin_registry, select_start_target,
        tenant_env_var_name, upsert_admin_registry_entry, validate_cloud_deploy_inputs,
        write_single_vm_spec,
    };
    use clap::{Arg, ArgMatches, Command};
    use std::env;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};
    use tempfile::{TempDir, tempdir};

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
    fn route_passthrough_subcommand_routes_wizard_to_greentic_dev() {
        let tail = vec!["--help".to_string()];
        let (binary, args) =
            route_passthrough_subcommand("wizard", &tail, "en").expect("wizard route");

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
        let temp = tempfile::tempdir().expect("tempdir");
        let cargo_home = temp.path().join("cargo-home");
        let cargo_bin = cargo_home.join("bin");
        let cargo_binary = cargo_bin.join(DEV_BIN);
        std::fs::create_dir_all(&cargo_bin).expect("mkdir cargo bin");
        std::fs::write(&cargo_binary, b"").expect("write cargo binary");

        unsafe {
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
        let temp = tempfile::tempdir().expect("tempdir");
        let exe_dir = temp.path().join("bin");
        std::fs::create_dir_all(&exe_dir).expect("mkdir");
        let current_exe = exe_dir.join("gtc");
        let sibling = exe_dir.join(DEV_BIN);
        std::fs::write(&current_exe, b"").expect("write gtc");
        std::fs::write(&sibling, b"").expect("write companion");

        let resolved =
            resolve_companion_binary_from(Some(current_exe.as_path()), DEV_BIN).expect("path");
        assert_eq!(resolved, sibling);
    }

    #[test]
    fn resolve_companion_binary_falls_back_to_workspace_local_binary() {
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
    fn validate_cloud_deploy_inputs_accepts_aws_remote_bundle_when_required_envs_present() {
        let _guard = env_test_lock().lock().unwrap();
        let _path_guard = temp_path_with_binary("terraform");
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
    fn validate_cloud_deploy_inputs_rejects_local_bundle_for_aws() {
        let _guard = env_test_lock().lock().unwrap();
        let _path_guard = temp_path_with_binary("terraform");
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
        .unwrap_err();

        unsafe {
            env::remove_var("GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE_DIGEST");
            env::remove_var("GREENTIC_DEPLOY_TERRAFORM_VAR_REMOTE_STATE_BACKEND");
        }
        clear_aws_credential_env();

        assert!(err.contains("aws deploy requires a remote bundle source"));
    }

    #[test]
    fn validate_cloud_deploy_inputs_does_not_accept_partial_aws_access_key_env() {
        let _guard = env_test_lock().lock().unwrap();
        let _path_guard = temp_path_with_binary("terraform");
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
        .unwrap_err();

        unsafe {
            env::remove_var("GREENTIC_DEPLOY_TERRAFORM_VAR_REMOTE_STATE_BACKEND");
        }
        clear_aws_credential_env();

        assert!(err.contains("missing cloud credentials"));
    }

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

    fn env_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct PathGuard {
        _temp_dir: tempfile::TempDir,
        original: Option<std::ffi::OsString>,
    }

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

    #[test]
    fn write_single_vm_spec_uses_bundle_local_server_certs() {
        let dir = tempfile::tempdir().expect("tempdir");
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

        let spec_path =
            write_single_vm_spec("demo-bundle", &resolved, &request, &artifact_path).expect("spec");
        let spec = std::fs::read_to_string(&spec_path).expect("read spec");

        assert!(spec.contains("source: 'file://"));
        assert!(spec.contains(&artifact_path.display().to_string()));
        assert!(spec.contains("certFile: '"));
        assert!(spec.contains(".greentic/admin/certs/server.crt"));
        assert!(spec.contains("keyFile: '"));
        assert!(spec.contains(".greentic/admin/certs/server.key"));
        assert!(!spec.contains("client.crt"));
        assert!(!spec.contains("client.key"));
    }

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
        std::fs::write(dir.path().join("packs/cards-demo.gtpack"), "fixture")
            .expect("write app pack");

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
}
