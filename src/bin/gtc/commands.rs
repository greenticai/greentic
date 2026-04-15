use std::env;
use std::fs;
use std::io::{self, Write};

use clap::ArgMatches;

use crate::admin::{
    load_admin_registry, remove_admin_registry_entry, run_admin_tunnel, save_admin_registry,
    upsert_admin_registry_entry,
};
use crate::cli::build_cli;
use crate::deploy::{resolve_local_mutable_bundle_dir, run_start, run_stop};
use crate::i18n_support::i18n;
use crate::install::{run_install, run_update};
use crate::process::{passthrough, run_doctor};
use crate::router::{
    collect_tail, detect_locale, locale_from_args, parse_raw_passthrough, passthrough_help_request,
    route_passthrough_subcommand,
};

pub(super) fn run(raw_args: Vec<String>) -> i32 {
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
        Some(("update", _)) => run_update(debug, &locale),
        Some(("help", sub_matches)) => run_help(sub_matches, &locale),
        Some(("add-admin", sub_matches)) => run_add_admin(sub_matches, &locale),
        Some(("remove-admin", sub_matches)) => run_remove_admin(sub_matches, &locale),
        Some(("admin", sub_matches)) => match sub_matches.subcommand() {
            Some(("tunnel", tunnel_matches)) => run_admin_tunnel(tunnel_matches, &locale),
            _ => {
                eprintln!(
                    "{}",
                    crate::i18n_support::t(&locale, "gtc.admin.usage.tunnel")
                );
                2
            }
        },
        Some(("start", sub_matches)) => run_start(sub_matches, debug, &locale),
        Some(("stop", sub_matches)) => run_stop(sub_matches, debug, &locale),
        Some((name @ ("dev" | "op" | "wizard" | "setup"), sub_matches)) => {
            let tail = collect_tail(sub_matches);
            let (binary, args) = route_passthrough_subcommand(name, &tail, &locale).expect("route");
            passthrough(binary, &args, debug, &locale)
        }
        _ => 2,
    }
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
