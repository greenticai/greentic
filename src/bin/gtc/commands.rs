use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

use clap::ArgMatches;
use serde_json::Value;
use tempfile::TempDir;

use crate::admin::{
    load_admin_registry, remove_admin_registry_entry, run_admin_access, run_admin_add_client,
    run_admin_certs, run_admin_clients, run_admin_health, run_admin_list, run_admin_remove_client,
    run_admin_status, run_admin_stop, run_admin_token, run_admin_tunnel, save_admin_registry,
    upsert_admin_registry_entry,
};
use crate::answer_resolver::{
    AnswerSourceKind, DefaultAnswerSourceLoader, classify_answers_source, load_answer_bytes,
    load_answers, parse_answers_bytes,
};
use crate::cli::build_cli;
use crate::deploy::{
    RefreshArgs, resolve_local_mutable_bundle_dir, run_refresh, run_start, run_stop,
};
use crate::docs_cmd::run_docs;
use crate::extensions::{
    run_extension_setup, run_extension_start, run_extension_wizard, take_extension_start_handoff,
};
use crate::i18n_support::i18n;
use crate::install::{run_install, run_update};
use crate::process::{passthrough, run_binary_capture, run_doctor};
use crate::release_cache::run_release_cache;
use crate::router::{
    collect_tail, detect_locale, locale_from_args, parse_raw_passthrough, passthrough_help_request,
    route_passthrough_subcommand,
};
use crate::toolchain::{installed_toolchain_label, latest_release_context_warning};

pub(super) fn run(raw_args: Vec<String>) -> i32 {
    let i18n = i18n();
    let default_install_channel = default_install_channel_for_invocation(raw_args.first());
    let invocation = raw_args.first().cloned();
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

    if matches.get_flag("version") {
        print_version();
        return 0;
    }

    match matches.subcommand() {
        Some(("version", _)) => {
            print_version();
            0
        }
        Some(("doctor", _)) => run_doctor(&locale),
        Some(("docs", sub_matches)) => run_docs(sub_matches, debug, &locale),
        Some(("release-cache", sub_matches)) => run_release_cache(sub_matches, debug, &locale),
        Some(("install", sub_matches)) => {
            run_install(sub_matches, default_install_channel, debug, &locale)
        }
        Some(("update", _)) => run_update(debug, &locale),
        Some(("help", sub_matches)) => run_help(sub_matches, &locale),
        Some(("add-admin", sub_matches)) => run_add_admin(sub_matches, &locale),
        Some(("remove-admin", sub_matches)) => run_remove_admin(sub_matches, &locale),
        Some(("admin", sub_matches)) => match sub_matches.subcommand() {
            Some(("access", access_matches)) => run_admin_access(access_matches, &locale),
            Some(("certs", cert_matches)) => run_admin_certs(cert_matches, &locale),
            Some(("token", token_matches)) => run_admin_token(token_matches, &locale),
            Some(("health", health_matches)) => run_admin_health(health_matches, &locale),
            Some(("status", status_matches)) => run_admin_status(status_matches, &locale),
            Some(("list", list_matches)) => run_admin_list(list_matches, &locale),
            Some(("admins", admins_matches)) => run_admin_clients(admins_matches, &locale),
            Some(("stop", stop_matches)) => run_admin_stop(stop_matches, &locale),
            Some(("add-client", add_matches)) => run_admin_add_client(add_matches, &locale),
            Some(("remove-client", remove_matches)) => {
                run_admin_remove_client(remove_matches, &locale)
            }
            Some(("tunnel", tunnel_matches)) => run_admin_tunnel(tunnel_matches, &locale),
            _ => {
                eprintln!(
                    "usage: gtc admin <access|certs|token|health|status|list|admins|stop|add-client|remove-client|tunnel> ..."
                );
                2
            }
        },
        Some(("start", sub_matches)) => {
            // `start` is a pure catch-all at the clap layer — every flag
            // (including gtc-internal ones) is parsed from the tail by one
            // parser, so any greentic-start flag works with or without a
            // bundle ref.
            let tail = collect_tail(sub_matches);
            match take_extension_start_handoff(&tail) {
                Ok(Some((handoff_path, rest))) => {
                    run_extension_start(&handoff_path, &rest, debug, &locale)
                }
                Ok(None) => run_start(&tail, debug, &locale),
                Err(err) => {
                    eprintln!("{err}");
                    2
                }
            }
        }
        Some(("stop", sub_matches)) => run_stop(&collect_tail(sub_matches), debug, &locale),
        Some(("deploy", deploy_matches)) => match deploy_matches.subcommand() {
            Some(("refresh-bundle-url", m)) => {
                let args = RefreshArgs {
                    bundle_ref: m
                        .get_one::<String>("bundle-ref")
                        .cloned()
                        .unwrap_or_default(),
                    cloud: m.get_one::<String>("cloud").cloned(),
                    environment: m
                        .get_one::<String>("environment")
                        .cloned()
                        .unwrap_or_else(|| "dev".to_string()),
                    presign_expires: m
                        .get_one::<String>("upload-bundle-presign-expires")
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(604800),
                };
                run_refresh(args)
            }
            _ => {
                eprintln!("usage: gtc deploy refresh-bundle-url <BUNDLE_REF>");
                2
            }
        },
        Some((name @ ("dev" | "op" | "wizard" | "setup" | "worker"), sub_matches)) => {
            let tail = collect_tail(sub_matches);
            let tail = if matches!(name, "wizard" | "setup") {
                let release_context =
                    match release_context_flags(sub_matches, &tail, default_install_channel) {
                        Ok(release_context) => release_context,
                        Err(err) => {
                            eprintln!("{err}");
                            return 2;
                        }
                    };
                if !release_context.ignore
                    && let Some(status) = check_release_context(
                        release_context.channel,
                        invocation.as_deref(),
                        debug,
                        &locale,
                    )
                {
                    if release_context.strict {
                        eprintln!("error: {status}");
                        return 1;
                    } else {
                        eprintln!("warning: {status}");
                    }
                }
                release_context.tail
            } else {
                tail
            };
            let mut answers_tempdir = None;
            let tail = if matches!(name, "wizard" | "setup") {
                match resolve_answers_args(&tail) {
                    Ok(resolved) => {
                        answers_tempdir = resolved.tempdir;
                        resolved.args
                    }
                    Err(err) => {
                        eprintln!("{err}");
                        return answers_error_exit_code(&err);
                    }
                }
            } else {
                tail
            };
            if name == "wizard" && sub_matches.get_many::<String>("extensions").is_some() {
                return run_extension_wizard(sub_matches, &tail, debug, &locale);
            }
            if name == "setup"
                && sub_matches
                    .get_one::<String>("extension-setup-handoff")
                    .is_some()
            {
                return run_extension_setup(sub_matches, &tail, debug, &locale);
            }
            let (binary, args) = route_passthrough_subcommand(name, &tail, &locale).expect("route");
            if name == "wizard" && has_schema_flag(&args) {
                if has_schema_full_flag(&args) {
                    return run_wizard_schema_full(binary, &args, debug, &locale);
                }
                return run_wizard_schema(binary, &args, debug, &locale);
            }
            let _answers_tempdir = answers_tempdir;
            passthrough(binary, &args, debug, &locale)
        }
        _ => 2,
    }
}

struct ResolvedAnswersArgs {
    args: Vec<String>,
    tempdir: Option<TempDir>,
}

fn resolve_answers_args(args: &[String]) -> gtc::error::GtcResult<ResolvedAnswersArgs> {
    let loader = DefaultAnswerSourceLoader;
    let mut rewritten = Vec::with_capacity(args.len());
    let mut tempdir: Option<TempDir> = None;
    let mut index = 0usize;
    let mut materialized_count = 0usize;

    while index < args.len() {
        let arg = &args[index];
        if arg == "--answers" {
            let Some(source) = args.get(index + 1) else {
                return Err(gtc::error::GtcError::invalid_data(
                    "answers source",
                    "--answers requires a value",
                ));
            };
            rewritten.push(arg.clone());
            rewritten.push(resolve_answers_arg_value(
                source,
                &loader,
                &mut tempdir,
                &mut materialized_count,
            )?);
            index += 2;
            continue;
        }

        if let Some(source) = arg.strip_prefix("--answers=") {
            let resolved =
                resolve_answers_arg_value(source, &loader, &mut tempdir, &mut materialized_count)?;
            rewritten.push(format!("--answers={resolved}"));
            index += 1;
            continue;
        }

        rewritten.push(arg.clone());
        index += 1;
    }

    Ok(ResolvedAnswersArgs {
        args: rewritten,
        tempdir,
    })
}

fn resolve_answers_arg_value(
    source: &str,
    loader: &DefaultAnswerSourceLoader,
    tempdir: &mut Option<TempDir>,
    materialized_count: &mut usize,
) -> gtc::error::GtcResult<String> {
    let kind = classify_answers_source(source)?;
    match kind {
        AnswerSourceKind::LocalPath | AnswerSourceKind::FileUrl | AnswerSourceKind::Http => {
            let _answers = load_answers(source)?;
            Ok(source.to_string())
        }
        AnswerSourceKind::Distributor => {
            let bytes = load_answer_bytes(source, loader)?;
            let _answers = parse_answers_bytes(source, &bytes)?;
            if tempdir.is_none() {
                *tempdir = Some(tempfile::tempdir().map_err(|err| {
                    gtc::error::GtcError::io("failed to create temporary answers directory", err)
                })?);
            }
            let dir = tempdir.as_ref().expect("temporary answers directory");
            let path = dir
                .path()
                .join(format!("answers-{materialized_count}.json"));
            *materialized_count += 1;
            fs::write(&path, bytes).map_err(|err| {
                gtc::error::GtcError::io(format!("failed to write {}", path.display()), err)
            })?;
            Ok(path.display().to_string())
        }
    }
}

fn answers_error_exit_code(err: &gtc::error::GtcError) -> i32 {
    if matches!(err, gtc::error::GtcError::InvalidData { context, .. } if context == "answers source")
    {
        2
    } else {
        1
    }
}

pub(super) fn default_install_channel_for_invocation(invocation: Option<&String>) -> &'static str {
    let Some(invocation) = invocation else {
        return "stable";
    };
    let Some(file_name) = Path::new(invocation)
        .file_stem()
        .and_then(|value| value.to_str())
    else {
        return "stable";
    };
    if file_name.ends_with("-dev") {
        "dev"
    } else if file_name.ends_with("-rnd") {
        "rnd"
    } else {
        "stable"
    }
}

struct ReleaseContextFlags {
    channel: &'static str,
    strict: bool,
    ignore: bool,
    tail: Vec<String>,
}

fn release_context_flags(
    matches: &ArgMatches,
    tail: &[String],
    default_channel: &'static str,
) -> Result<ReleaseContextFlags, String> {
    let mut strict = matches.get_flag("strict-release-context");
    let mut ignore = matches.get_flag("ignore-release-context");
    let mut forwarded = Vec::with_capacity(tail.len());

    for arg in tail {
        match arg.as_str() {
            "--strict-release-context" => strict = true,
            "--ignore-release-context" => ignore = true,
            _ => forwarded.push(arg.clone()),
        }
    }

    if strict && ignore {
        return Err(
            "--strict-release-context cannot be used with --ignore-release-context".to_string(),
        );
    }

    Ok(ReleaseContextFlags {
        channel: default_channel,
        strict,
        ignore,
        tail: forwarded,
    })
}

fn check_release_context(
    channel: &str,
    invocation: Option<&str>,
    debug: bool,
    locale: &str,
) -> Option<String> {
    let install_command = install_command_for_invocation(invocation);
    match latest_release_context_warning(channel, &install_command, debug, locale) {
        Ok(Some(warning)) => Some(warning),
        Ok(None) => None,
        Err(err) => Some(format!(
            "failed to verify Greentic toolchain release context for channel '{channel}': {err}. Run `{install_command} install` to refresh the local release context."
        )),
    }
}

fn install_command_for_invocation(invocation: Option<&str>) -> String {
    invocation
        .and_then(|value| Path::new(value).file_stem())
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("gtc")
        .to_string()
}

fn print_version() {
    println!("gtc {}", env!("CARGO_PKG_VERSION"));
    println!(
        "Greentic toolchain release: {}",
        installed_toolchain_label()
    );
}

fn run_wizard_schema(binary: &str, args: &[String], debug: bool, locale: &str) -> i32 {
    let raw = match run_binary_capture(binary, args, debug, locale) {
        Ok(raw) => raw,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    let mut schema: Value = match serde_json::from_str(&raw) {
        Ok(schema) => schema,
        Err(err) => {
            eprintln!("invalid wizard schema JSON from {binary}: {err}");
            return 1;
        }
    };
    rewrite_nested_schema_refs(&mut schema);
    match serde_json::to_string_pretty(&schema) {
        Ok(rendered) => {
            println!("{rendered}");
            0
        }
        Err(err) => {
            eprintln!("failed to render wizard schema JSON: {err}");
            1
        }
    }
}

fn has_schema_flag(args: &[String]) -> bool {
    args.iter()
        .any(|arg| arg == "--schema" || arg.starts_with("--schema="))
}

fn has_schema_full_flag(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "--schema=full")
}

fn strip_schema_full(args: &[String]) -> Vec<String> {
    args.iter()
        .map(|arg| {
            if arg == "--schema=full" {
                "--schema".to_string()
            } else {
                arg.clone()
            }
        })
        .collect()
}

fn run_wizard_schema_full(binary: &str, args: &[String], debug: bool, locale: &str) -> i32 {
    let stripped = strip_schema_full(args);

    let launcher_raw = match run_binary_capture(binary, &stripped, debug, locale) {
        Ok(raw) => raw,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    let mut launcher: Value = match serde_json::from_str(&launcher_raw) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("invalid wizard schema JSON from {binary}: {err}");
            return 1;
        }
    };
    rewrite_nested_schema_refs(&mut launcher);

    let component =
        capture_companion_schema(super::COMPONENT_BIN, debug, locale).unwrap_or(Value::Null);
    let flow = capture_companion_schema(super::FLOW_BIN, debug, locale).unwrap_or(Value::Null);

    let aggregated = serde_json::json!({
        "launcher": launcher,
        "component": component,
        "flow": flow,
    });

    match serde_json::to_string_pretty(&aggregated) {
        Ok(rendered) => {
            println!("{rendered}");
            0
        }
        Err(err) => {
            eprintln!("failed to render wizard schema JSON: {err}");
            1
        }
    }
}

fn capture_companion_schema(binary: &str, debug: bool, locale: &str) -> Option<Value> {
    let args = vec!["wizard".to_string(), "--schema".to_string()];
    match run_binary_capture(binary, &args, debug, locale) {
        Ok(raw) => match serde_json::from_str(&raw) {
            Ok(value) => Some(value),
            Err(err) => {
                eprintln!("warning: companion schema from {binary} is not valid JSON: {err}");
                None
            }
        },
        Err(err) => {
            eprintln!("warning: companion schema from {binary} unavailable: {err}");
            None
        }
    }
}

fn rewrite_nested_schema_refs(schema: &mut Value) {
    rewrite_nested_schema_refs_at(schema, &[], &[], &[], false);
}

fn rewrite_nested_schema_refs_at(
    value: &mut Value,
    value_path: &[String],
    defs_path: &[String],
    def_names: &[String],
    nested_defs: bool,
) {
    match value {
        Value::Object(object) => {
            let mut current_defs_path = defs_path.to_vec();
            let mut current_def_names = def_names.to_vec();
            let mut current_nested_defs = nested_defs;
            if let Some(Value::Object(defs)) = object.get("$defs") {
                current_defs_path = value_path.to_vec();
                current_defs_path.push("$defs".to_string());
                current_def_names = defs.keys().cloned().collect();
                current_nested_defs = current_defs_path != ["$defs".to_string()];
            }

            if current_nested_defs
                && let Some(Value::String(reference)) = object.get_mut("$ref")
                && let Some(rewritten) =
                    rewrite_local_ref(reference, &current_defs_path, &current_def_names)
            {
                *reference = rewritten;
            }

            for (key, child) in object.iter_mut() {
                let mut child_path = value_path.to_vec();
                child_path.push(key.clone());
                rewrite_nested_schema_refs_at(
                    child,
                    &child_path,
                    &current_defs_path,
                    &current_def_names,
                    current_nested_defs,
                );
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter_mut().enumerate() {
                let mut item_path = value_path.to_vec();
                item_path.push(index.to_string());
                rewrite_nested_schema_refs_at(item, &item_path, defs_path, def_names, nested_defs);
            }
        }
        _ => {}
    }
}

fn rewrite_local_ref(
    reference: &str,
    defs_path: &[String],
    def_names: &[String],
) -> Option<String> {
    let remaining = reference.strip_prefix("#/$defs/")?;
    let (name, suffix) = remaining.split_once('/').unwrap_or((remaining, ""));
    if !def_names.iter().any(|def_name| def_name == name) {
        return None;
    }
    let mut pointer = format!("#/{}", json_pointer_path(defs_path));
    pointer.push('/');
    pointer.push_str(&escape_json_pointer_segment(name));
    if !suffix.is_empty() {
        pointer.push('/');
        pointer.push_str(suffix);
    }
    Some(pointer)
}

fn json_pointer_path(path: &[String]) -> String {
    path.iter()
        .map(|segment| escape_json_pointer_segment(segment))
        .collect::<Vec<_>>()
        .join("/")
}

fn escape_json_pointer_segment(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

fn run_help(sub_matches: &ArgMatches, locale: &str) -> i32 {
    let path: Vec<String> = sub_matches
        .get_many::<String>("command")
        .map(|values| values.cloned().collect())
        .unwrap_or_default();
    let mut cmd = build_cli(locale);

    for segment in &path {
        let Some(next) = cmd.find_subcommand(segment).cloned() else {
            eprintln!(
                "{}: {}",
                crate::i18n_support::t(locale, "gtc.help.err.unknown_command"),
                segment
            );
            return 2;
        };
        cmd = next;
    }

    if let Err(err) = cmd.print_help() {
        eprintln!(
            "{}: {err}",
            crate::i18n_support::t(locale, "gtc.err.exec_failed")
        );
        return 1;
    }
    if let Err(err) = writeln!(io::stdout()) {
        eprintln!(
            "{}: {err}",
            crate::i18n_support::t(locale, "gtc.err.exec_failed")
        );
        return 1;
    }
    0
}

pub(super) fn run_add_admin(sub_matches: &ArgMatches, _locale: &str) -> i32 {
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

pub(super) fn run_remove_admin(sub_matches: &ArgMatches, _locale: &str) -> i32 {
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

#[cfg(test)]
mod tests {
    use super::{run_add_admin, run_remove_admin};
    use crate::cli::build_cli;
    use std::fs;

    #[test]
    fn add_admin_reports_missing_public_key_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cli = build_cli("en");
        let matches = cli
            .try_get_matches_from([
                "gtc",
                "add-admin",
                dir.path().to_str().expect("bundle path"),
                "--cn",
                "demo-cn",
                "--public-key-file",
                dir.path()
                    .join("missing.pem")
                    .to_str()
                    .expect("missing key path"),
            ])
            .expect("matches");
        let sub = matches.subcommand().expect("subcommand").1;

        assert_eq!(run_add_admin(sub, "en"), 1);
    }

    #[test]
    fn add_admin_rejects_empty_public_key_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let key = dir.path().join("pub.pem");
        fs::write(&key, " \n").expect("write");

        let cli = build_cli("en");
        let matches = cli
            .try_get_matches_from([
                "gtc",
                "add-admin",
                dir.path().to_str().expect("bundle path"),
                "--cn",
                "demo-cn",
                "--public-key-file",
                key.to_str().expect("key path"),
            ])
            .expect("matches");
        let sub = matches.subcommand().expect("subcommand").1;

        assert_eq!(run_add_admin(sub, "en"), 1);
    }

    #[test]
    fn remove_admin_requires_selector() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cli = build_cli("en");
        let matches = cli
            .try_get_matches_from(["gtc", "remove-admin", dir.path().to_str().expect("bundle")])
            .expect("matches");
        let sub = matches.subcommand().expect("subcommand").1;

        assert_eq!(run_remove_admin(sub, "en"), 2);
    }

    #[test]
    fn remove_admin_reports_missing_entry() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cli = build_cli("en");
        let matches = cli
            .try_get_matches_from([
                "gtc",
                "remove-admin",
                dir.path().to_str().expect("bundle"),
                "--cn",
                "demo-cn",
            ])
            .expect("matches");
        let sub = matches.subcommand().expect("subcommand").1;

        assert_eq!(run_remove_admin(sub, "en"), 1);
    }
}
