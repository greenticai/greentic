use clap::{Arg, ArgMatches};
use gtc::perf_targets::{
    RawPassthrough, detect_locale as perf_detect_locale, has_flag as perf_has_flag,
    parse_raw_passthrough as perf_parse_raw_passthrough,
    rewrite_legacy_op_args as perf_rewrite_legacy_op_args,
};

use super::{DEV_BIN, OP_BIN, SETUP_BIN};
use crate::extensions::has_extension_flags;
use crate::i18n_support::i18n;

#[cfg(test)]
const RAW_PASSTHROUGH_GLOBAL_FLAG_SPECS: &[(&str, bool)] = &[
    ("debug-router", false),
    ("help", false),
    ("locale", true),
    ("version", false),
];

pub(super) fn passthrough_args() -> Arg {
    Arg::new("args")
        .num_args(0..)
        .trailing_var_arg(true)
        .allow_hyphen_values(true)
}

pub(super) fn collect_tail(matches: &ArgMatches) -> Vec<String> {
    matches
        .get_many::<String>("args")
        .map(|vals| vals.cloned().collect())
        .unwrap_or_default()
}

pub(super) fn parse_raw_passthrough(raw_args: &[String]) -> Option<RawPassthrough> {
    perf_parse_raw_passthrough(raw_args)
}

#[cfg(test)]
pub(super) fn raw_passthrough_global_flag_specs() -> &'static [(&'static str, bool)] {
    RAW_PASSTHROUGH_GLOBAL_FLAG_SPECS
}

pub(super) fn passthrough_help_request(
    raw: Option<&RawPassthrough>,
    _cli_locale: &Option<String>,
    locale: &str,
) -> Option<(&'static str, Vec<String>)> {
    let raw = raw?;
    if raw.subcommand == "wizard" && has_extension_flags(&raw.tail) {
        return None;
    }
    if !raw.tail.iter().any(|arg| arg == "--help" || arg == "-h") {
        return None;
    }

    route_passthrough_subcommand(&raw.subcommand, &raw.tail, locale)
}

pub(super) fn route_passthrough_subcommand(
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
    perf_rewrite_legacy_op_args(args)
}

pub(super) fn build_wizard_args(args: &[String], locale: &str) -> Vec<String> {
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
    perf_has_flag(args, flag)
}

pub(super) fn detect_locale(raw_args: &[String], default_locale: &str) -> String {
    let selected = perf_detect_locale(raw_args, default_locale, None);
    i18n().normalize_or_default(&selected)
}

pub(super) fn locale_from_args(raw_args: &[String]) -> Option<String> {
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
