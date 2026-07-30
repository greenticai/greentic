//! Toolchain version + checksum bookkeeping in the shared `system.log`.
//!
//! Before gtc hands off to a companion binary (`greentic-setup`,
//! `greentic-start`, …) it appends a record to the unified `system.log` naming
//! gtc's own version + sha256 and the resolved companion's version + sha256.
//! This gives a per-run audit trail of exactly which binaries participated,
//! which makes cross-binary bug hunting much easier.
//!
//! Records are written to two places when both are available: always to the
//! stable `~/.greentic/logs/system.log`, and additionally to the bundle/run
//! `<log_dir>/system.log` when one can be resolved from the invocation args —
//! mirroring greentic-start's own log-dir resolution so gtc's lines interleave
//! with that run's runtime/operator lines.
//!
//! Everything here is strictly best-effort: any failure to hash or write is
//! swallowed so bookkeeping can never break an actual command. In particular it
//! never spawns a companion to read its `--version`: that would be an
//! observable extra invocation (routing tests assert exactly what a companion
//! receives), so versions come from gtc's own recorded installed-toolchain
//! state and the sha256 is the authoritative build fingerprint.

use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use directories::BaseDirs;
use gtc::perf_targets::sha256_file;

/// gtc's own package version, embedded at compile time.
const GTC_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Tracing-style target used for these records so they line up with the
/// `operator_log` records greentic-start writes into the same file.
const LOG_TARGET: &str = "gtc.toolchain";

/// Dedup key marking that gtc's own self line has already been emitted this run.
const GTC_SELF_KEY: &str = "\0gtc-self";

/// Per-process set of dedup keys already recorded, so a single gtc run that
/// touches the same companion multiple times (e.g. a `--schema` probe then the
/// real hand-off) only emits one snapshot line for it — and gtc's own line only
/// once.
fn recorded() -> &'static Mutex<HashSet<String>> {
    static RECORDED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    RECORDED.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Insert `key` into the per-process record set, returning `true` the first
/// time it is seen and `false` on every subsequent call (or on lock poisoning,
/// which suppresses further writes rather than risking duplicates).
fn record_once(key: &str) -> bool {
    match recorded().lock() {
        Ok(mut seen) => seen.insert(key.to_string()),
        Err(_) => false,
    }
}

/// Record a toolchain snapshot for `binary`, just resolved on disk to
/// `command`, as gtc is about to invoke it.
///
/// Only real `greentic-*` companion binaries are snapshotted; absolute helper
/// paths used by tests/probes (e.g. `/bin/sh`) are ignored. The first snapshot
/// of a run is prefixed with gtc's own version + checksum line. The bundle/run
/// log dir (when any) is resolved from gtc's own process arguments.
pub(super) fn record_invocation(binary: &str, command: &str) {
    if !binary.starts_with("greentic-") {
        return;
    }
    if !record_once(binary) {
        return;
    }

    let mut lines = Vec::with_capacity(2);
    if record_once(GTC_SELF_KEY) {
        lines.push(gtc_self_line());
    }
    lines.push(companion_line(binary, command));

    let argv: Vec<String> = std::env::args().skip(1).collect();
    for dir in target_log_dirs(&argv) {
        append_lines(&dir, &lines);
    }
}

/// Build the `gtc <version> <sha256> (<path>)` self-description line.
fn gtc_self_line() -> String {
    let exe = std::env::current_exe().ok();
    let checksum = exe
        .as_deref()
        .and_then(|path| sha256_file(path).ok())
        .unwrap_or_else(|| "sha256:unavailable".to_string());
    let path = exe
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "gtc".to_string());
    format!("gtc {GTC_VERSION} {checksum} ({path})")
}

/// Build the `<binary> <version> <sha256> (<path>)` line for a companion binary.
///
/// The version comes from gtc's recorded installed-toolchain state — never by
/// spawning the binary, which would be an observable extra invocation. When the
/// version is not recorded (dev/override runs with no installed toolchain), the
/// sha256 alone identifies the exact build, so the version reads
/// `version:unrecorded`.
fn companion_line(binary: &str, command: &str) -> String {
    let checksum =
        sha256_file(Path::new(command)).unwrap_or_else(|_| "sha256:unavailable".to_string());
    let version = installed_versions()
        .get(binary)
        .cloned()
        .unwrap_or_else(|| "version:unrecorded".to_string());
    format!("{binary} {version} {checksum} ({command})")
}

/// Per-process cache mapping a companion binary name to the version gtc has
/// recorded for it in its installed-toolchain state (`installed.json`). Built
/// once and reused; empty when no toolchain is installed (dev/override runs), in
/// which case companion lines carry the checksum alone.
fn installed_versions() -> &'static HashMap<String, String> {
    static VERSIONS: OnceLock<HashMap<String, String>> = OnceLock::new();
    VERSIONS.get_or_init(|| {
        let mut map = HashMap::new();
        if let Ok(Some(installed)) = crate::toolchain::read_installed_toolchain() {
            for package in installed.packages {
                for bin in package.bins {
                    map.insert(bin, package.version.clone());
                }
            }
        }
        map
    })
}

/// The `system.log`-parent directories to write to: always the stable home log
/// dir, plus a bundle/run log dir when one is resolvable from `args`.
fn target_log_dirs(args: &[String]) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = system_log_home_dir() {
        dirs.push(home);
    }
    if let Some(bundle_dir) = resolve_bundle_log_dir(args)
        && !dirs.contains(&bundle_dir)
    {
        dirs.push(bundle_dir);
    }
    dirs
}

/// The stable, always-written log directory. Honors the `GTC_SYSTEM_LOG_DIR`
/// override (used by tests to keep the real home clean, and by ops to redirect
/// gtc's bookkeeping); otherwise defaults to `~/.greentic/logs`, matching the
/// home fallback `operator_log` itself uses for `system.log`.
fn system_log_home_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("GTC_SYSTEM_LOG_DIR").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    // In-crate unit tests exercise the companion resolvers in-process without a
    // sandboxed HOME, so a bare `cargo test` must never append to the
    // developer's real ~/.greentic/logs. Setting GTC_SYSTEM_LOG_DIR (above) is
    // how tests and integration harnesses opt into a temp dir instead.
    if cfg!(test) {
        return None;
    }
    BaseDirs::new().map(|base| base.home_dir().join(".greentic").join("logs"))
}

/// Resolve the bundle/run log directory from `args`, mirroring greentic-start's
/// own resolution: an explicit `--log-dir <dir>` wins, otherwise the first
/// bundle-looking positional directory is treated as the run root and logged
/// into as `<bundle>/logs`. Returns `None` when neither is present (in which
/// case only the stable home log is written).
fn resolve_bundle_log_dir(args: &[String]) -> Option<PathBuf> {
    for (i, arg) in args.iter().enumerate() {
        if arg == "--log-dir" {
            if let Some(dir) = args.get(i + 1) {
                return Some(PathBuf::from(dir));
            }
        } else if let Some(dir) = arg.strip_prefix("--log-dir=") {
            return Some(PathBuf::from(dir));
        }
    }

    // Skip flag values so `--state-dir ./state` doesn't get mistaken for a
    // bundle positional; only a bundle-marker-bearing directory qualifies.
    let mut prev_consumes_value = false;
    for arg in args {
        if arg.starts_with('-') {
            prev_consumes_value = !arg.contains('=');
            continue;
        }
        if prev_consumes_value {
            prev_consumes_value = false;
            continue;
        }
        let path = Path::new(arg);
        if looks_like_bundle_dir(path) {
            return Some(path.join("logs"));
        }
    }
    None
}

/// A directory counts as a bundle root when it carries one of the usual bundle
/// markers. Deliberately stricter than a bare `is_dir()` check so an unrelated
/// positional path never diverts the log into the wrong place; the cost of a
/// false negative is only that the run is logged to the home log alone.
fn looks_like_bundle_dir(path: &Path) -> bool {
    const MARKERS: [&str; 5] = [
        "greentic.demo.yaml",
        "manifest.json",
        "bundle.json",
        "packs",
        "state",
    ];
    path.is_dir() && MARKERS.iter().any(|marker| path.join(marker).exists())
}

/// Append `lines` to `<dir>/system.log`, creating the directory and file as
/// needed. Each line is wrapped in the shared `operator_log` record shape so it
/// reads cleanly alongside greentic-start's own entries.
fn append_lines(dir: &Path, lines: &[String]) {
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("system.log"))
    else {
        return;
    };
    for line in lines {
        let _ = file.write_all(format_record(line).as_bytes());
    }
    let _ = file.flush();
}

/// Wrap `message` in the same `{rfc3339} [{Level}] {target} - {message}` shape
/// `operator_log` uses, so gtc's records interleave readably in `system.log`.
fn format_record(message: &str) -> String {
    let timestamp = chrono::Utc::now().to_rfc3339();
    format!("{timestamp} [Info] {LOG_TARGET} - {message}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_once_dedupes_per_key() {
        // A fresh, isolated set for the assertion (the module-global set is
        // shared, so exercise the primitive directly on distinct keys).
        assert!(record_once("syslog-test::alpha"));
        assert!(!record_once("syslog-test::alpha"));
        assert!(record_once("syslog-test::beta"));
    }

    #[test]
    fn companion_line_uses_unrecorded_when_no_version_and_hashes_the_file() {
        // No installed toolchain state is guaranteed in the test env, so the
        // version falls back to `version:unrecorded`; the line must still name
        // the binary, a sha256, and the resolved path.
        let dir = tempfile::tempdir().expect("tempdir");
        let bin = dir.path().join("greentic-fake");
        std::fs::write(&bin, b"fake-binary-bytes").expect("write fake binary");
        let line = companion_line("greentic-fake", &bin.display().to_string());
        assert!(line.starts_with("greentic-fake "), "line: {line}");
        assert!(line.contains("sha256:"), "line: {line}");
        assert!(
            line.contains(&bin.display().to_string()),
            "line should carry the resolved path: {line}"
        );
    }

    #[test]
    fn resolve_bundle_log_dir_prefers_explicit_flag() {
        let args = vec![
            "start".to_string(),
            "--log-dir".to_string(),
            "/var/run/greentic/logs".to_string(),
        ];
        assert_eq!(
            resolve_bundle_log_dir(&args),
            Some(PathBuf::from("/var/run/greentic/logs"))
        );

        let joined = vec!["--log-dir=/tmp/gt/logs".to_string()];
        assert_eq!(
            resolve_bundle_log_dir(&joined),
            Some(PathBuf::from("/tmp/gt/logs"))
        );
    }

    #[test]
    fn resolve_bundle_log_dir_uses_bundle_marker_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bundle = dir.path().join("mybundle");
        std::fs::create_dir_all(&bundle).expect("bundle dir");
        std::fs::write(bundle.join("greentic.demo.yaml"), "tenant: demo\n").expect("marker");

        let args = vec![
            "start".to_string(),
            bundle.to_string_lossy().to_string(),
            "--verbose".to_string(),
        ];
        assert_eq!(resolve_bundle_log_dir(&args), Some(bundle.join("logs")));
    }

    #[test]
    fn resolve_bundle_log_dir_ignores_flag_values_and_plain_dirs() {
        let dir = tempfile::tempdir().expect("tempdir");
        // An existing dir with no bundle markers must not be picked up, even as
        // a flag value.
        let plain = dir.path().join("state");
        std::fs::create_dir_all(&plain).expect("state dir");

        let args = vec![
            "start".to_string(),
            "--state-dir".to_string(),
            plain.to_string_lossy().to_string(),
        ];
        assert_eq!(resolve_bundle_log_dir(&args), None);
    }

    #[test]
    fn looks_like_bundle_dir_requires_a_marker() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(!looks_like_bundle_dir(dir.path()));
        std::fs::create_dir_all(dir.path().join("packs")).expect("packs");
        assert!(looks_like_bundle_dir(dir.path()));
    }

    #[test]
    fn append_lines_writes_wrapped_records() {
        let dir = tempfile::tempdir().expect("tempdir");
        append_lines(
            dir.path(),
            &["gtc 1.1.8 sha256:abcd (/opt/gtc)".to_string()],
        );
        let contents =
            std::fs::read_to_string(dir.path().join("system.log")).expect("read system.log");
        assert!(contents.contains("[Info] gtc.toolchain -"));
        assert!(contents.contains("gtc 1.1.8 sha256:abcd (/opt/gtc)"));
    }

    #[test]
    fn record_invocation_ignores_non_greentic_binaries() {
        // `/bin/sh` and friends must never be snapshotted; nothing to assert on
        // the filesystem, but this must not panic or write.
        record_invocation("/bin/sh", "/bin/sh");
    }
}
