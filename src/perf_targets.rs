use std::fs;
use std::io::{BufReader, Read};
use std::path::Path;
use std::time::UNIX_EPOCH;

use crate::config::GtcConfig;
use greentic_i18n::{normalize_locale, select_locale_with_sources};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct RawPassthrough {
    pub subcommand: String,
    pub tail: Vec<String>,
}

pub fn parse_raw_passthrough(raw_args: &[String]) -> Option<RawPassthrough> {
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

pub fn rewrite_legacy_op_args(args: &[String]) -> Vec<String> {
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

pub fn ensure_flag_value(args: &mut Vec<String>, flag: &str, value: &str) {
    if has_flag(args, flag) {
        return;
    }
    args.push(format!("--{flag}"));
    if !value.is_empty() {
        args.push(value.to_string());
    }
}

pub fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| {
        arg.strip_prefix("--").is_some_and(|rest| {
            rest == flag
                || rest
                    .strip_prefix(flag)
                    .is_some_and(|suffix| suffix.starts_with('='))
        })
    })
}

pub fn detect_locale(
    raw_args: &[String],
    default_locale: &str,
    env_locale: Option<&str>,
) -> String {
    let cli_locale = locale_from_args(raw_args);
    let env_locale_owned = env_locale
        .map(|value| value.to_string())
        .or_else(|| GtcConfig::from_env().locale_override());

    let selected = select_locale_with_sources(
        cli_locale.as_deref(),
        Some(default_locale),
        env_locale_owned.as_deref(),
        None,
    );

    normalize_locale(&selected)
}

pub fn locale_from_args(raw_args: &[String]) -> Option<String> {
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

pub fn sha256_file(path: &Path) -> Result<String, String> {
    let file = fs::File::open(path).map_err(|err| {
        format!(
            "failed to read artifact {} for sha256: {err}",
            path.display()
        )
    })?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];

    loop {
        let n = reader.read(&mut buf).map_err(|err| {
            format!(
                "failed to read artifact {} for sha256: {err}",
                path.display()
            )
        })?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    Ok(format!("sha256:{:x}", hasher.finalize()))
}

pub fn collect_bundle_entries(
    root: &Path,
    dir: &Path,
    out: &mut Vec<String>,
) -> Result<(), String> {
    for entry in fs::read_dir(dir)
        .map_err(|err| format!("failed to read bundle directory {}: {err}", dir.display()))?
    {
        let entry = entry.map_err(|err| err.to_string())?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|err| err.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let file_type = entry.file_type().map_err(|err| err.to_string())?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
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

#[cfg(test)]
mod tests {
    use super::{
        collect_bundle_entries, detect_locale, has_flag, parse_raw_passthrough,
        rewrite_legacy_op_args, sha256_file,
    };
    use proptest::prelude::*;
    use std::fs;

    #[test]
    fn has_flag_matches_plain_and_equals_forms() {
        let args = vec![
            "--locale=en".to_string(),
            "--debug".to_string(),
            "--team=default".to_string(),
        ];
        assert!(has_flag(&args, "locale"));
        assert!(has_flag(&args, "debug"));
        assert!(has_flag(&args, "team"));
    }

    #[test]
    fn has_flag_rejects_prefix_collisions() {
        let args = vec!["--locale-extra".to_string()];
        assert!(!has_flag(&args, "locale"));
    }

    #[test]
    fn rewrite_legacy_op_args_adds_expected_defaults() {
        let args = vec!["start".to_string(), "--foo".to_string(), "bar".to_string()];
        let rewritten = rewrite_legacy_op_args(&args);
        assert_eq!(rewritten[0], "demo");
        assert!(rewritten.iter().any(|arg| arg == "--tenant"));
        assert!(rewritten.iter().any(|arg| arg == "--team"));
        assert!(rewritten.iter().any(|arg| arg == "--cloudflared"));
    }

    #[test]
    fn parse_raw_passthrough_skips_global_flags() {
        let raw = vec![
            "gtc".to_string(),
            "--locale".to_string(),
            "fr".to_string(),
            "--debug-router".to_string(),
            "wizard".to_string(),
            "--help".to_string(),
        ];
        let parsed = parse_raw_passthrough(&raw).expect("passthrough");
        assert_eq!(parsed.subcommand, "wizard");
        assert_eq!(parsed.tail, vec!["--help".to_string()]);
    }

    #[test]
    fn detect_locale_prefers_cli_value() {
        let raw = vec![
            "gtc".to_string(),
            "--locale=nl".to_string(),
            "doctor".to_string(),
        ];
        assert_eq!(detect_locale(&raw, "en", Some("de")), "nl");
    }

    #[test]
    fn sha256_file_matches_known_digest() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("artifact.bin");
        fs::write(&path, b"hello world").expect("write");
        assert_eq!(
            sha256_file(&path).expect("digest"),
            "sha256:b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn sha256_file_is_streaming_not_fs_read_based() {
        let source = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/perf_targets.rs"));
        let sha_fn = source
            .split("pub fn sha256_file")
            .nth(1)
            .expect("sha256_file present");
        let body = sha_fn
            .split("pub fn collect_bundle_entries")
            .next()
            .expect("sha256_file body");

        assert!(body.contains("BufReader::new"));
        assert!(body.contains("reader.read(&mut buf)"));
        assert!(!body.contains("fs::read(path)"));
    }

    #[test]
    fn collect_bundle_entries_walks_tiny_tree() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("nested");
        fs::create_dir_all(&nested).expect("mkdir");
        fs::write(nested.join("file.txt"), b"hi").expect("write");
        let mut out = Vec::new();
        collect_bundle_entries(dir.path(), dir.path(), &mut out).expect("walk");
        assert!(out.iter().any(|line| line == "dir:nested"));
        assert!(
            out.iter()
                .any(|line| line.starts_with("file:nested/file.txt:2:"))
        );
    }

    proptest! {
        #[test]
        fn has_flag_only_matches_exact_long_flag_suffixes(
            suffix in "[A-Za-z0-9_-]{1,12}"
        ) {
            let exact = vec![format!("--locale={suffix}")];
            prop_assert!(has_flag(&exact, "locale"));

            let prefixed = vec![format!("--locale{suffix}")];
            prop_assert!(!has_flag(&prefixed, "locale"));

            let different = vec![format!("--other={suffix}")];
            prop_assert!(!has_flag(&different, "locale"));
        }

        #[test]
        fn parse_raw_passthrough_returns_first_non_flag_subcommand(
            cmd in "[a-z][a-z0-9-]{0,10}",
            tail1 in "[a-z0-9-]{0,8}",
            tail2 in "[a-z0-9-]{0,8}"
        ) {
            let mut raw = vec![
                "gtc".to_string(),
                "--locale".to_string(),
                "en".to_string(),
                "--debug-router".to_string(),
                cmd.clone(),
            ];
            if !tail1.is_empty() {
                raw.push(tail1.clone());
            }
            if !tail2.is_empty() {
                raw.push(tail2.clone());
            }

            let parsed = parse_raw_passthrough(&raw).expect("subcommand should be parsed");
            prop_assert_eq!(parsed.subcommand, cmd);
            let expected_tail: Vec<String> = raw.into_iter().skip(5).collect();
            prop_assert_eq!(parsed.tail, expected_tail);
        }
    }
}
