use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use clap::ArgMatches;
use gtc::error::{GtcError, GtcResult};
use gtc::start_stop_parsing::{parse_start_request, parse_stop_request};
use serde::Deserialize;

use super::bundle_resolution::resolve_bundle_reference;
use super::cloud_deploy::{destroy_deployment, ensure_started_or_deployed};
use super::prepared_bundle::{prepare_bundle_for_start, print_prepared_bundle_debug};
use super::{ChildProcessEnv, StartCliOptions, StartTarget, StopCliOptions};
use crate::START_BIN;
use crate::admin::ensure_admin_certs_ready;
use crate::i18n_support::t_or;
use crate::process::{run_binary_checked, run_binary_checked_with_target_and_env};
use crate::router::collect_tail;

pub(crate) fn run_start(sub_matches: &ArgMatches, debug: bool, locale: &str) -> i32 {
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
    run_start_with_bundle_ref_and_tail(bundle_ref, &tail, debug, locale)
}

pub(crate) fn run_start_with_bundle_ref_and_tail(
    bundle_ref: &str,
    tail: &[String],
    debug: bool,
    locale: &str,
) -> i32 {
    let cli_options = match parse_start_cli_options(tail) {
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
    let prepared = match prepare_bundle_for_start(bundle_ref, &resolved, debug, locale) {
        Ok(value) => value,
        Err(err) => {
            eprintln!(
                "{}: {err}",
                t_or(
                    locale,
                    "gtc.start.err.prepare_failed",
                    "failed to prepare bundle"
                )
            );
            return 1;
        }
    };
    request.bundle = Some(prepared.prepared_root.display().to_string());
    if request.admin {
        if request
            .admin_certs_dir
            .as_deref()
            .is_some_and(|path| path.starts_with(&resolved.bundle_dir))
        {
            request.admin_certs_dir = None;
        }
        match ensure_admin_certs_ready(&prepared.prepared_root, request.admin_certs_dir.as_deref())
        {
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
    print_prepared_bundle_debug(&prepared, target, None, None);
    if target != StartTarget::Runtime {
        println!("Deployment mode: deploy via {} target", target.as_str());
        let deploy_result = ensure_started_or_deployed(
            bundle_ref,
            &resolved,
            &prepared,
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
        request.tenant.as_deref().unwrap_or("demo"),
        request.team.as_deref().unwrap_or("default")
    );
    let args = request.to_runtime_start_args(locale);
    let runtime_env = local_runtime_secret_env(&resolved.bundle_dir);
    match run_binary_checked_with_target_and_env(
        START_BIN,
        &args,
        debug,
        locale,
        "start bundle",
        None,
        runtime_env.as_ref(),
    ) {
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

fn local_runtime_secret_env(bundle_dir: &Path) -> Option<ChildProcessEnv> {
    let dev_secrets = bundle_dir
        .join(".greentic")
        .join("dev")
        .join(".dev.secrets.env");
    if !dev_secrets.is_file() {
        return None;
    }
    let mut env = ChildProcessEnv::new();
    env.set(
        "GREENTIC_DEV_SECRETS_PATH",
        dev_secrets.display().to_string(),
    );
    Some(env)
}

pub(crate) fn run_stop(sub_matches: &ArgMatches, debug: bool, locale: &str) -> i32 {
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
            let args = request.to_runtime_stop_args(locale);
            match run_binary_checked(START_BIN, &args, debug, locale, "stop bundle") {
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
            match destroy_deployment(
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

pub(crate) fn parse_start_cli_options(tail: &[String]) -> GtcResult<StartCliOptions> {
    let mut start_args = Vec::new();
    let mut explicit_target = None;
    let mut environment = None;
    let mut provider_pack = None;
    let mut app_pack = None;
    let mut deploy_bundle_source = None;
    let mut upload_bundle: Option<String> = None;
    let mut upload_bundle_presign_expires: Option<u64> = None;
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
            "--upload-bundle" => {
                idx += 1;
                upload_bundle = Some(required_value(tail, idx, "--upload-bundle")?);
            }
            "--upload-bundle-presign-expires" => {
                idx += 1;
                let raw = required_value(tail, idx, "--upload-bundle-presign-expires")?;
                upload_bundle_presign_expires = Some(raw.parse::<u64>().map_err(|e| {
                    GtcError::message(format!("invalid --upload-bundle-presign-expires: {e}"))
                })?);
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
                } else if let Some(value) = arg.strip_prefix("--upload-bundle=") {
                    upload_bundle = Some(value.to_string());
                } else if let Some(value) = arg.strip_prefix("--upload-bundle-presign-expires=") {
                    upload_bundle_presign_expires = Some(value.parse::<u64>().map_err(|e| {
                        GtcError::message(format!("invalid --upload-bundle-presign-expires: {e}"))
                    })?);
                } else {
                    start_args.push(arg.clone());
                }
            }
        }
        idx += 1;
    }
    if upload_bundle.is_some() && deploy_bundle_source.is_some() {
        return Err(GtcError::message(
            "--upload-bundle and --deploy-bundle-source are mutually exclusive; pick one"
                .to_string(),
        ));
    }
    Ok(StartCliOptions {
        start_args,
        explicit_target,
        environment,
        provider_pack,
        app_pack,
        deploy_bundle_source,
        upload_bundle,
        upload_bundle_presign_expires,
    })
}

pub(crate) fn parse_stop_cli_options(tail: &[String]) -> GtcResult<StopCliOptions> {
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

fn parse_start_target(value: &str) -> GtcResult<StartTarget> {
    match value.trim() {
        "runtime" | "local" => Ok(StartTarget::Runtime),
        "aws" => Ok(StartTarget::Aws),
        "gcp" => Ok(StartTarget::Gcp),
        "azure" => Ok(StartTarget::Azure),
        other => Err(GtcError::message(format!(
            "unsupported --target value {other}; expected runtime, aws, gcp, or azure"
        ))),
    }
}

pub(crate) fn select_start_target(
    bundle_dir: &Path,
    explicit_target: Option<StartTarget>,
    locale: &str,
) -> GtcResult<StartTarget> {
    select_start_target_with_mode(
        bundle_dir,
        explicit_target,
        locale,
        io::stdin().is_terminal() && io::stdout().is_terminal(),
    )
}

fn select_start_target_with_mode(
    bundle_dir: &Path,
    explicit_target: Option<StartTarget>,
    locale: &str,
    interactive: bool,
) -> GtcResult<StartTarget> {
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
        StartTarget::Runtime => 3,
    });
    deploy_targets.dedup();
    if deploy_targets.is_empty() {
        return Ok(StartTarget::Runtime);
    }
    if deploy_targets.len() == 1 {
        return Ok(deploy_targets[0]);
    }
    if !interactive {
        return Err(GtcError::message(format!(
            "multiple start targets are available ({}); rerun with --target",
            deploy_targets
                .iter()
                .map(|value| value.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    prompt_start_target(&deploy_targets, locale)
}

fn detect_bundle_deployment_targets(bundle_dir: &Path) -> GtcResult<Vec<StartTarget>> {
    if let Some(explicit_targets) = load_explicit_deployment_targets(bundle_dir)? {
        return Ok(explicit_targets);
    }
    Ok(Vec::new())
}

#[derive(Debug, Deserialize)]
struct DeploymentTargetsDocument {
    targets: Vec<DeploymentTargetRecord>,
}

#[derive(Debug, Deserialize)]
struct DeploymentTargetRecord {
    target: String,
    default: Option<bool>,
}

fn load_explicit_deployment_targets(bundle_dir: &Path) -> GtcResult<Option<Vec<StartTarget>>> {
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

fn load_default_deployment_target(bundle_dir: &Path) -> GtcResult<Option<StartTarget>> {
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

fn prompt_start_target(targets: &[StartTarget], locale: &str) -> GtcResult<StartTarget> {
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
    io::stdout()
        .flush()
        .map_err(|err| GtcError::message(err.to_string()))?;
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(|err| GtcError::message(err.to_string()))?;
    let choice = input
        .trim()
        .parse::<usize>()
        .map_err(|_| GtcError::message("invalid target selection"))?;
    targets
        .get(choice.saturating_sub(1))
        .copied()
        .ok_or_else(|| GtcError::message("invalid target selection"))
}

fn required_value(args: &[String], idx: usize, flag: &str) -> GtcResult<String> {
    let flag_name = flag.to_string();
    args.get(idx)
        .cloned()
        .ok_or_else(|| GtcError::message(format!("missing value for {flag_name}")))
}

#[cfg(test)]
mod tests {
    use super::{
        load_default_deployment_target, parse_start_cli_options, parse_start_request,
        parse_stop_cli_options, parse_stop_request, select_start_target,
        select_start_target_with_mode,
    };
    use crate::deploy::StartTarget;
    use gtc::start_stop_parsing::{
        CloudflaredModeArg, NatsModeArg, NgrokModeArg, RestartTarget, StartRequest, StopRequest,
    };
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn parse_start_cli_options_extracts_deploy_specific_flags() {
        let opts = parse_start_cli_options(&[
            "--target=aws".to_string(),
            "--environment".to_string(),
            "prod".to_string(),
            "--provider-pack=./terraform.gtpack".to_string(),
            "--tenant".to_string(),
            "demo".to_string(),
        ])
        .expect("opts");

        assert_eq!(opts.explicit_target, Some(StartTarget::Aws));
        assert_eq!(opts.environment.as_deref(), Some("prod"));
        assert_eq!(
            opts.provider_pack.as_deref(),
            Some(PathBuf::from("./terraform.gtpack").as_path())
        );
        assert_eq!(
            opts.start_args,
            vec!["--tenant".to_string(), "demo".to_string()]
        );
    }

    #[test]
    fn parse_stop_cli_options_extracts_destroy_flag() {
        let opts = parse_stop_cli_options(&[
            "--destroy".to_string(),
            "--target=aws".to_string(),
            "--team".to_string(),
            "ops".to_string(),
        ])
        .expect("opts");

        assert!(opts.destroy);
        assert_eq!(opts.explicit_target, Some(StartTarget::Aws));
        assert_eq!(
            opts.stop_args,
            vec!["--team".to_string(), "ops".to_string()]
        );
    }

    #[test]
    fn parse_start_request_supports_equals_flags() {
        let request = parse_start_request(
            &[
                "--tenant=demo".to_string(),
                "--team=ops".to_string(),
                "--admin".to_string(),
                "--admin-allowed-clients=a,b".to_string(),
            ],
            PathBuf::from("/tmp/bundle"),
        )
        .expect("request");

        assert_eq!(request.tenant.as_deref(), Some("demo"));
        assert_eq!(request.team.as_deref(), Some("ops"));
        assert!(request.admin);
        assert_eq!(
            request.admin_allowed_clients,
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn parse_stop_request_supports_state_dir_and_defaults() {
        let request = parse_stop_request(
            &["--state-dir=/tmp/state".to_string()],
            PathBuf::from("/tmp/bundle"),
        )
        .expect("request");

        assert_eq!(request.bundle.as_deref(), Some("/tmp/bundle"));
        assert_eq!(request.tenant, "demo");
        assert_eq!(request.team, "default");
        assert_eq!(
            request.state_dir.as_deref(),
            Some(PathBuf::from("/tmp/state").as_path())
        );
    }

    #[test]
    fn build_runtime_start_args_serializes_request() {
        let request = StartRequest {
            bundle: Some("/tmp/bundle".to_string()),
            tenant: Some("demo".to_string()),
            team: Some("ops".to_string()),
            no_nats: false,
            no_browser: true,
            nats: NatsModeArg::External,
            nats_url: Some("nats://demo".to_string()),
            config: Some(PathBuf::from("/tmp/config.yaml")),
            cloudflared: CloudflaredModeArg::Off,
            cloudflared_binary: Some(PathBuf::from("/tmp/cloudflared")),
            ngrok: NgrokModeArg::On,
            ngrok_binary: Some(PathBuf::from("/tmp/ngrok")),
            runner_binary: Some(PathBuf::from("/tmp/runner")),
            restart: vec![RestartTarget::Gateway, RestartTarget::Nats],
            log_dir: Some(PathBuf::from("/tmp/logs")),
            verbose: true,
            quiet: false,
            admin: true,
            admin_port: 9443,
            admin_certs_dir: Some(PathBuf::from("/tmp/admin-certs")),
            admin_allowed_clients: vec!["ops".to_string(), "local".to_string()],
            tunnel_explicit: true,
        };

        let args = request.to_runtime_start_args("fr");
        assert_eq!(args[0], "--locale");
        assert_eq!(args[1], "fr");
        assert_eq!(args[2], "start");
        assert!(args.contains(&"--bundle".to_string()));
        assert!(args.contains(&"/tmp/bundle".to_string()));
        assert!(args.contains(&"--nats".to_string()));
        assert!(args.contains(&"external".to_string()));
        assert!(args.contains(&"--cloudflared".to_string()));
        assert!(args.contains(&"off".to_string()));
        assert!(args.contains(&"--ngrok".to_string()));
        assert!(args.contains(&"on".to_string()));
        assert!(args.contains(&"--verbose".to_string()));
        assert!(args.contains(&"--admin".to_string()));
        assert!(args.contains(&"--no-browser".to_string()));
        assert!(args.contains(&"ops,local".to_string()));
        assert!(args.contains(&"gateway,nats".to_string()));
    }

    #[test]
    fn build_runtime_stop_args_serializes_request() {
        let request = StopRequest {
            bundle: Some("/tmp/bundle".to_string()),
            state_dir: Some(PathBuf::from("/tmp/state")),
            tenant: "demo".to_string(),
            team: "ops".to_string(),
        };

        let args = request.to_runtime_stop_args("en");
        assert_eq!(
            args,
            vec![
                "--locale".to_string(),
                "en".to_string(),
                "stop".to_string(),
                "--bundle".to_string(),
                "/tmp/bundle".to_string(),
                "--state-dir".to_string(),
                "/tmp/state".to_string(),
                "--tenant".to_string(),
                "demo".to_string(),
                "--team".to_string(),
                "ops".to_string(),
            ]
        );
    }

    #[test]
    fn load_default_deployment_target_reads_metadata_default() {
        let dir = tempfile::tempdir().expect("tempdir");
        let greentic = dir.path().join(".greentic");
        fs::create_dir_all(&greentic).expect("mkdir");
        fs::write(
            greentic.join("deployment-targets.json"),
            r#"{"targets":[{"target":"aws"},{"target":"gcp","default":true}]}"#,
        )
        .expect("write");

        let target = load_default_deployment_target(dir.path()).expect("target");
        assert_eq!(target, Some(StartTarget::Gcp));
    }

    #[test]
    fn select_start_target_prefers_explicit_target_over_metadata() {
        let dir = tempfile::tempdir().expect("tempdir");
        let greentic = dir.path().join(".greentic");
        fs::create_dir_all(&greentic).expect("mkdir");
        fs::write(
            greentic.join("deployment-targets.json"),
            r#"{"targets":[{"target":"gcp","default":true}]}"#,
        )
        .expect("write");

        let target = select_start_target(dir.path(), Some(StartTarget::Aws), "en").expect("target");
        assert_eq!(target, StartTarget::Aws);
    }

    #[test]
    fn parse_start_request_rejects_unknown_argument() {
        let err = parse_start_request(&["--mystery".to_string()], PathBuf::from("/tmp/bundle"))
            .unwrap_err();
        assert!(err.contains("unsupported start argument"));
    }

    #[test]
    fn parse_stop_request_rejects_bundle_override() {
        let err = parse_stop_request(
            &["--bundle".to_string(), "/tmp/other".to_string()],
            PathBuf::from("/tmp/bundle"),
        )
        .unwrap_err();
        assert!(err.contains("--bundle is managed by gtc stop"));
    }

    #[test]
    fn select_start_target_errors_for_multiple_targets_without_tty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let greentic = dir.path().join(".greentic");
        fs::create_dir_all(&greentic).expect("mkdir");
        fs::write(
            greentic.join("deployment-targets.json"),
            r#"{"targets":[{"target":"aws"},{"target":"gcp"}]}"#,
        )
        .expect("write");

        let err = select_start_target_with_mode(dir.path(), None, "en", false).unwrap_err();
        assert!(err.contains("multiple start targets are available"));
    }

    #[test]
    fn parse_start_request_rejects_invalid_admin_port() {
        let err = parse_start_request(
            &["--admin-port=abc".to_string()],
            PathBuf::from("/tmp/bundle"),
        )
        .unwrap_err();
        assert!(err.contains("invalid --admin-port"));
    }

    #[test]
    fn load_default_deployment_target_returns_none_when_metadata_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = load_default_deployment_target(dir.path()).expect("target");
        assert_eq!(target, None);
    }

    // PR-08 Phase 5: regression coverage. Local `gtc start` reads the dev
    // envelope from `<bundle>/.greentic/dev/.dev.secrets.env` via
    // `GREENTIC_DEV_SECRETS_PATH`. The bake-into-bundle change must not
    // disturb that contract — the envelope still lives in the source bundle
    // directory, and `local_runtime_secret_env` still points the runner
    // subprocess at it.

    #[test]
    fn local_runtime_secret_env_points_at_bundle_envelope_when_present() {
        let dir = tempfile::tempdir().expect("tempdir");
        let envelope = dir
            .path()
            .join(".greentic")
            .join("dev")
            .join(".dev.secrets.env");
        fs::create_dir_all(envelope.parent().expect("parent")).expect("mkdir");
        fs::write(&envelope, "SECRET=value\n").expect("envelope");

        let env = super::local_runtime_secret_env(dir.path()).expect("env present");
        let entry = env
            .vars
            .iter()
            .find(|(key, _)| key == "GREENTIC_DEV_SECRETS_PATH")
            .expect("GREENTIC_DEV_SECRETS_PATH must be set");
        assert_eq!(entry.1.as_str(), envelope.display().to_string());
    }

    #[test]
    fn local_runtime_secret_env_returns_none_when_envelope_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        // No `.greentic/dev/.dev.secrets.env` file created on purpose.
        assert!(super::local_runtime_secret_env(dir.path()).is_none());
    }
}
