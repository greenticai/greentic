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
    let mut idx = 1usize;
    while idx < raw_args.len() {
        let arg = &raw_args[idx];
        if arg == "--locale" {
            idx += 2;
            continue;
        }
        if arg == "--debug-router" || arg.starts_with("--locale=") {
            idx += 1;
            continue;
        }
        if arg.starts_with('-') {
            idx += 1;
            continue;
        }

        return Some(RawPassthrough {
            subcommand: arg.clone(),
            tail: raw_args[idx + 1..].to_vec(),
        });
    }

    None
}

pub fn rewrite_legacy_op_args(args: &[String]) -> Vec<String> {
    let Some(first) = args.first() else {
        return args.to_vec();
    };

    match first.as_str() {
        "setup" => {
            let mut has_tenant = false;
            let mut has_team = false;
            for arg in &args[1..] {
                if !arg.starts_with("--") {
                    continue;
                }
                if flag_matches(arg, "tenant") {
                    has_tenant = true;
                } else if flag_matches(arg, "team") {
                    has_team = true;
                }
            }

            let mut out = Vec::with_capacity(
                2 + args.len() - 1 + if has_tenant { 0 } else { 2 } + if has_team { 0 } else { 2 },
            );
            out.push("demo".to_string());
            out.push("setup".to_string());
            out.extend_from_slice(&args[1..]);
            if !has_tenant {
                out.push("--tenant".to_string());
                out.push("default".to_string());
            }
            if !has_team {
                out.push("--team".to_string());
                out.push("default".to_string());
            }
            out
        }
        "start" => {
            let mut has_tenant = false;
            let mut has_team = false;
            let mut has_cloudflared = false;
            for arg in &args[1..] {
                if !arg.starts_with("--") {
                    continue;
                }
                if flag_matches(arg, "tenant") {
                    has_tenant = true;
                } else if flag_matches(arg, "team") {
                    has_team = true;
                } else if flag_matches(arg, "cloudflared") {
                    has_cloudflared = true;
                }
            }

            let mut out = Vec::with_capacity(
                2 + args.len() - 1
                    + if has_tenant { 0 } else { 2 }
                    + if has_team { 0 } else { 2 }
                    + if has_cloudflared { 0 } else { 2 },
            );
            out.push("demo".to_string());
            out.push("start".to_string());
            out.extend_from_slice(&args[1..]);
            if !has_tenant {
                out.push("--tenant".to_string());
                out.push("default".to_string());
            }
            if !has_team {
                out.push("--team".to_string());
                out.push("default".to_string());
            }
            if !has_cloudflared {
                out.push("--cloudflared".to_string());
                out.push("off".to_string());
            }
            out
        }
        _ => args.to_vec(),
    }
}

fn flag_matches(arg: &str, flag: &str) -> bool {
    arg.strip_prefix("--").is_some_and(|rest| {
        rest == flag
            || rest
                .strip_prefix(flag)
                .is_some_and(|suffix| suffix.starts_with('='))
    })
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
    args.iter().any(|arg| flag_matches(arg, flag))
}

pub fn detect_locale(
    raw_args: &[String],
    default_locale: &str,
    env_locale: Option<&str>,
) -> String {
    if let Some(cli_locale) = locale_from_args(raw_args) {
        return normalize_locale(&cli_locale);
    }

    let env_locale_owned = env_locale
        .map(|value| value.to_string())
        .or_else(|| GtcConfig::from_env().locale_override());

    let selected = select_locale_with_sources(
        None,
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

    let digest = hasher.finalize();
    let mut out = String::with_capacity("sha256:".len() + digest.len() * 2);
    out.push_str("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    Ok(out)
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
        let file_type = entry.file_type().map_err(|err| err.to_string())?;
        if file_type.is_symlink() {
            continue;
        }
        let relative_path = path.strip_prefix(root).map_err(|err| err.to_string())?;
        let relative = relative_path.to_string_lossy();
        if file_type.is_dir() {
            if std::path::MAIN_SEPARATOR == '\\' {
                out.push(format!("dir:{}", relative.replace('\\', "/")));
            } else {
                out.push(format!("dir:{relative}"));
            }
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
        if std::path::MAIN_SEPARATOR == '\\' {
            out.push(format!(
                "file:{}:{}:{modified}",
                relative.replace('\\', "/"),
                metadata.len()
            ));
        } else {
            out.push(format!("file:{relative}:{}:{modified}", metadata.len()));
        }
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
