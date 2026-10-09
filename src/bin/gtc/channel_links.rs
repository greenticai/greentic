//! The side-by-side layout for the dev channel.
//!
//! The dev channel installs every companion under a `-dev` name
//! (`greentic-start-dev`, `greentic-runner-dev`, ...) into the same bin
//! directory stable uses, so the two toolchains sit beside each other instead
//! of replacing each other. Tools that resolve a companion by its CANONICAL
//! name — greentic-designer, greentic-pack, a script — therefore find the
//! stable build, or nothing, even on a machine whose dev toolchain is fully
//! installed.
//!
//! Renaming the dev binaries to canonical names in that shared directory would
//! clobber the stable toolchain, which operators deliberately keep unsuffixed.
//! So the canonical names live in a directory of their own:
//!
//! ```text
//! <toolchain state dir>/channels/dev/bin/greentic-start -> <cargo bin>/greentic-start-dev
//! ```
//!
//! That directory is never put on `PATH` by gtc. `gtc channel-env` prints the
//! shell lines that select it — and the `GREENTIC_*_BIN` overrides pointing at
//! the `-dev` binaries — for whichever process the operator chooses.
//!
//! The location is the toolchain state directory (`~/.greentic/toolchain`, or
//! `GTC_TOOLCHAIN_STATE_DIR`) because that is where gtc already records what
//! the toolchain install did. `~/.greentic/bin` is deliberately NOT used:
//! greentic-designer owns `~/.greentic/bin/<id>/current` as its managed-binary
//! pointer, and a directory of links there would share a namespace with it.
//!
//! The links mirror what is actually installed rather than what a manifest
//! lists: every `greentic-<name>-dev` binary in the cargo bin directory gets a
//! `greentic-<name>` link, and a link whose binary is gone is removed. Stable
//! installs never write to this directory's targets, so running
//! `gtc install` (stable) re-syncs the dev links without touching either
//! toolchain's binaries.

use std::fs;
use std::path::{Path, PathBuf};

use clap::ArgMatches;
use gtc::error::{GtcError, GtcResult};

use super::i18n_support::{t_or, tf_or};
use super::process::resolve_cargo_bin_dir;
use super::toolchain::installed_toolchain_path;

/// The only channel whose binaries carry a name suffix today.
pub(crate) const DEV_CHANNEL: &str = "dev";
const DEV_SUFFIX: &str = "-dev";
const COMPANION_PREFIX: &str = "greentic-";

/// `GREENTIC_*_BIN` override honoured for each canonical companion name, by
/// gtc itself (`src/config.rs`) and by greentic-designer's binary registry.
///
/// An explicit table rather than a derivation: `greentic-deploy-platform`
/// reads `GREENTIC_PLATFORM_BIN`, and emitting a variable nobody reads would
/// read as a selection that does nothing. A companion absent from this table
/// is still selected through the `PATH` link.
const BIN_ENV_VARS: &[(&str, &str)] = &[
    ("greentic-bundle", "GREENTIC_BUNDLE_BIN"),
    ("greentic-component", "GREENTIC_COMPONENT_BIN"),
    ("greentic-deploy-platform", "GREENTIC_PLATFORM_BIN"),
    ("greentic-deployer", "GREENTIC_DEPLOYER_BIN"),
    ("greentic-dev", "GREENTIC_DEV_BIN"),
    ("greentic-dw", "GREENTIC_DW_BIN"),
    ("greentic-flow", "GREENTIC_FLOW_BIN"),
    ("greentic-operator", "GREENTIC_OPERATOR_BIN"),
    ("greentic-pack", "GREENTIC_PACK_BIN"),
    ("greentic-runner", "GREENTIC_RUNNER_BIN"),
    ("greentic-secrets", "GREENTIC_SECRETS_BIN"),
    ("greentic-setup", "GREENTIC_SETUP_BIN"),
    ("greentic-sorx", "GREENTIC_SORX_BIN"),
    ("greentic-start", "GREENTIC_START_BIN"),
];

/// One installed dev-channel binary and the canonical name it stands in for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DevBinary {
    pub canonical: String,
    pub path: PathBuf,
}

/// What a sync changed, by canonical name.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct LinkSync {
    pub created: Vec<String>,
    pub updated: Vec<String>,
    pub removed: Vec<String>,
    /// Names occupied by something that is not a link. Never overwritten: the
    /// directory is gtc's, but a file an operator put there is not.
    pub skipped: Vec<String>,
}

impl LinkSync {
    pub(crate) fn changed(&self) -> bool {
        !(self.created.is_empty() && self.updated.is_empty() && self.removed.is_empty())
    }
}

/// `<state dir>/channels/<channel>/bin`, where `<state dir>` is the directory
/// holding `installed.json`.
pub(crate) fn channel_links_dir(channel: &str) -> GtcResult<PathBuf> {
    let state_file = installed_toolchain_path()?;
    let state_dir = state_file
        .parent()
        .ok_or_else(|| GtcError::message("toolchain state path has no parent directory"))?;
    Ok(links_dir_under(state_dir, channel))
}

fn links_dir_under(state_dir: &Path, channel: &str) -> PathBuf {
    state_dir.join("channels").join(channel).join("bin")
}

/// The canonical name a dev-channel binary file stands in for, or `None` when
/// the file is not one.
///
/// `greentic-dev` itself is the STABLE developer CLI and maps to nothing — its
/// dev build is `greentic-dev-dev`. `gtc-dev` maps to nothing either: a `gtc`
/// link would run the dev launcher under the stable name, and gtc picks its
/// channel and its companions' names from the name it was invoked as.
pub(crate) fn canonical_name_for(file_name: &str) -> Option<String> {
    let exe_suffix = std::env::consts::EXE_SUFFIX;
    let stem = if exe_suffix.is_empty() {
        file_name
    } else {
        file_name.strip_suffix(exe_suffix)?
    };
    let canonical = stem.strip_suffix(DEV_SUFFIX)?;
    let name = canonical.strip_prefix(COMPANION_PREFIX)?;
    let well_formed = !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
        && !name.starts_with('-')
        && !name.ends_with('-');
    well_formed.then(|| canonical.to_string())
}

/// Every dev-channel binary installed in `bin_dir`, sorted by canonical name.
/// An absent directory is an empty install, not an error.
pub(crate) fn discover_dev_binaries(bin_dir: &Path) -> GtcResult<Vec<DevBinary>> {
    let entries = match fs::read_dir(bin_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => {
            return Err(GtcError::io(
                format!("failed to read {}", bin_dir.display()),
                err,
            ));
        }
    };
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry
            .map_err(|err| GtcError::io(format!("failed to read {}", bin_dir.display()), err))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(canonical) = entry.file_name().to_str().and_then(canonical_name_for) else {
            continue;
        };
        found.push(DevBinary { canonical, path });
    }
    found.sort_by(|a, b| a.canonical.cmp(&b.canonical));
    Ok(found)
}

/// Make `links_dir` hold exactly one link per `binaries` entry, named by its
/// canonical name and pointing at its path. Links for anything else are
/// removed; non-link entries are left alone and reported.
///
/// The directory is created only when there is something to link, so a
/// machine that never installed the dev channel gets no `channels/` tree.
#[cfg(unix)]
pub(crate) fn sync_links(links_dir: &Path, binaries: &[DevBinary]) -> GtcResult<LinkSync> {
    use std::os::unix::fs::symlink;

    let mut sync = LinkSync::default();
    if binaries.is_empty() && !links_dir.exists() {
        return Ok(sync);
    }
    fs::create_dir_all(links_dir)
        .map_err(|err| GtcError::io(format!("failed to create {}", links_dir.display()), err))?;

    for binary in binaries {
        let link = links_dir.join(&binary.canonical);
        match fs::symlink_metadata(&link) {
            Ok(meta) if meta.file_type().is_symlink() => {
                let current = fs::read_link(&link).map_err(|err| {
                    GtcError::io(format!("failed to read link {}", link.display()), err)
                })?;
                if current == binary.path {
                    continue;
                }
                // Replace through a rename so a concurrent resolver sees the
                // old link or the new one, never a missing name.
                let staging =
                    links_dir.join(format!(".{}.tmp-{}", binary.canonical, std::process::id()));
                let _ = fs::remove_file(&staging);
                symlink(&binary.path, &staging).map_err(|err| {
                    GtcError::io(format!("failed to create link {}", staging.display()), err)
                })?;
                fs::rename(&staging, &link).map_err(|err| {
                    let _ = fs::remove_file(&staging);
                    GtcError::io(format!("failed to replace link {}", link.display()), err)
                })?;
                sync.updated.push(binary.canonical.clone());
            }
            Ok(_) => sync.skipped.push(binary.canonical.clone()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                symlink(&binary.path, &link).map_err(|err| {
                    GtcError::io(format!("failed to create link {}", link.display()), err)
                })?;
                sync.created.push(binary.canonical.clone());
            }
            Err(err) => {
                return Err(GtcError::io(
                    format!("failed to inspect {}", link.display()),
                    err,
                ));
            }
        }
    }

    let wanted: std::collections::HashSet<&str> =
        binaries.iter().map(|b| b.canonical.as_str()).collect();
    let entries = fs::read_dir(links_dir)
        .map_err(|err| GtcError::io(format!("failed to read {}", links_dir.display()), err))?;
    for entry in entries {
        let entry = entry
            .map_err(|err| GtcError::io(format!("failed to read {}", links_dir.display()), err))?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if wanted.contains(name.as_str()) {
            continue;
        }
        let is_link = entry
            .file_type()
            .map(|kind| kind.is_symlink())
            .unwrap_or(false);
        if !is_link {
            continue;
        }
        fs::remove_file(entry.path()).map_err(|err| {
            GtcError::io(
                format!("failed to remove stale link {}", entry.path().display()),
                err,
            )
        })?;
        if !name.starts_with('.') {
            sync.removed.push(name);
        }
    }
    sync.removed.sort();
    Ok(sync)
}

/// Refresh the dev channel's canonical-name links from the cargo bin
/// directory. Called after every toolchain install that touched binaries.
///
/// Best-effort by design: the binaries are installed either way, and a link
/// directory that could not be written is reported, never turned into a failed
/// install.
pub(crate) fn sync_dev_channel_links(locale: &str) {
    let bin_dir = match resolve_cargo_bin_dir() {
        Ok(dir) => dir,
        Err(err) => {
            eprintln!("warning: dev channel links not refreshed: {err}");
            return;
        }
    };
    let binaries = match discover_dev_binaries(&bin_dir) {
        Ok(binaries) => binaries,
        Err(err) => {
            eprintln!("warning: dev channel links not refreshed: {err}");
            return;
        }
    };
    let links_dir = match channel_links_dir(DEV_CHANNEL) {
        Ok(dir) => dir,
        Err(err) => {
            eprintln!("warning: dev channel links not refreshed: {err}");
            return;
        }
    };
    report_sync(
        locale,
        &links_dir,
        &binaries,
        refresh(&links_dir, &binaries),
    );
}

#[cfg(unix)]
fn refresh(links_dir: &Path, binaries: &[DevBinary]) -> Option<GtcResult<LinkSync>> {
    Some(sync_links(links_dir, binaries))
}

/// No link mechanism outside unix: creating a symlink on Windows needs a
/// privilege most accounts lack, and a copy would silently go stale the next
/// time the `-dev` binary is replaced. `gtc channel-env` still selects the dev
/// binaries there, through the `GREENTIC_*_BIN` overrides.
#[cfg(not(unix))]
fn refresh(_links_dir: &Path, _binaries: &[DevBinary]) -> Option<GtcResult<LinkSync>> {
    None
}

fn report_sync(
    locale: &str,
    links_dir: &Path,
    binaries: &[DevBinary],
    outcome: Option<GtcResult<LinkSync>>,
) {
    let dir = links_dir.display().to_string();
    match outcome {
        None if !binaries.is_empty() => println!(
            "{}",
            t_or(
                locale,
                "gtc.channel_links.unsupported_platform",
                "Canonical-name links for the dev channel are not created on this platform; \
                 run `gtc channel-env --channel dev` to select the -dev binaries through \
                 GREENTIC_*_BIN instead."
            )
        ),
        None => {}
        Some(Err(err)) => eprintln!(
            "{}",
            tf_or(
                locale,
                "gtc.channel_links.sync_failed",
                "warning: dev channel links in {dir} were not refreshed: {error}",
                &[("dir", dir.as_str()), ("error", err.to_string().as_str())],
            )
        ),
        Some(Ok(sync)) => {
            for name in &sync.skipped {
                eprintln!(
                    "{}",
                    tf_or(
                        locale,
                        "gtc.channel_links.skipped",
                        "warning: {dir}/{name} is not a link; left it in place",
                        &[("dir", dir.as_str()), ("name", name.as_str())],
                    )
                );
            }
            if sync.changed() {
                let count = binaries.len().to_string();
                println!(
                    "{}",
                    tf_or(
                        locale,
                        "gtc.channel_links.synced",
                        "Dev channel canonical-name links: {count} in {dir} (not on PATH; \
                         `gtc channel-env --channel dev` prints how to select them).",
                        &[("count", count.as_str()), ("dir", dir.as_str())],
                    )
                );
            }
        }
    }
}

/// Shell dialect for [`render_activation`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shell {
    Sh,
    Fish,
    Pwsh,
}

impl Shell {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "sh" | "bash" | "zsh" => Some(Self::Sh),
            "fish" => Some(Self::Fish),
            "powershell" | "pwsh" => Some(Self::Pwsh),
            _ => None,
        }
    }

    fn platform_default() -> Self {
        if cfg!(windows) { Self::Pwsh } else { Self::Sh }
    }
}

pub(crate) fn env_var_for(canonical: &str) -> Option<&'static str> {
    BIN_ENV_VARS
        .iter()
        .find(|(name, _)| *name == canonical)
        .map(|(_, var)| *var)
}

/// The shell lines that select the dev channel for the current shell.
///
/// `links_dir` is `None` when there is no link directory to put on `PATH`
/// (non-unix, or none created). The `GREENTIC_*_BIN` lines are emitted either
/// way, because a `PATH` entry alone loses to anything a resolver checks
/// before `PATH` — greentic-designer, for one, consults its own managed-binary
/// pointer first.
pub(crate) fn render_activation(
    shell: Shell,
    links_dir: Option<&Path>,
    binaries: &[DevBinary],
) -> String {
    let mut out = String::new();
    out.push_str("# Greentic dev channel: canonical companion names -> -dev binaries.\n");
    out.push_str("# Applies to this shell only; the stable toolchain on PATH is untouched.\n");
    if let Some(dir) = links_dir {
        let dir = dir.display().to_string();
        match shell {
            Shell::Sh => out.push_str(&format!("export PATH={}:\"$PATH\"\n", sh_quote(&dir))),
            Shell::Fish => out.push_str(&format!("set -gx PATH {} $PATH\n", fish_quote(&dir))),
            Shell::Pwsh => out.push_str(&format!(
                "$env:PATH = {} + [IO.Path]::PathSeparator + $env:PATH\n",
                ps_quote(&dir)
            )),
        }
    }
    for binary in binaries {
        let Some(var) = env_var_for(&binary.canonical) else {
            continue;
        };
        let value = binary.path.display().to_string();
        match shell {
            Shell::Sh => out.push_str(&format!("export {var}={}\n", sh_quote(&value))),
            Shell::Fish => out.push_str(&format!("set -gx {var} {}\n", fish_quote(&value))),
            Shell::Pwsh => out.push_str(&format!("$env:{var} = {}\n", ps_quote(&value))),
        }
    }
    out
}

fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn fish_quote(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn ps_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// `gtc channel-env`: refresh the dev links, then print the activation lines
/// on stdout (everything else goes to stderr, so `eval "$(gtc channel-env)"`
/// only ever evaluates the snippet).
pub(crate) fn run_channel_env(matches: &ArgMatches, locale: &str) -> i32 {
    let channel = matches
        .get_one::<String>("channel")
        .map(String::as_str)
        .unwrap_or(DEV_CHANNEL);
    if channel != DEV_CHANNEL {
        eprintln!(
            "{}",
            tf_or(
                locale,
                "gtc.channel_env.unsupported_channel",
                "channel '{channel}' has no side-by-side layout: only the dev channel installs \
                 suffixed names. Stable companions already carry their canonical names on PATH.",
                &[("channel", channel)],
            )
        );
        return 2;
    }
    let shell = match matches.get_one::<String>("shell") {
        Some(value) => match Shell::parse(value) {
            Some(shell) => shell,
            None => {
                eprintln!(
                    "{}",
                    tf_or(
                        locale,
                        "gtc.channel_env.unsupported_shell",
                        "unsupported shell '{shell}'; use sh, fish, or powershell",
                        &[("shell", value.as_str())],
                    )
                );
                return 2;
            }
        },
        None => Shell::platform_default(),
    };

    let bin_dir = match resolve_cargo_bin_dir() {
        Ok(dir) => dir,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    let binaries = match discover_dev_binaries(&bin_dir) {
        Ok(binaries) => binaries,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    if binaries.is_empty() {
        let dir = bin_dir.display().to_string();
        eprintln!(
            "{}",
            tf_or(
                locale,
                "gtc.channel_env.nothing_installed",
                "no dev-channel binaries (greentic-*-dev) found in {dir}; run \
                 `gtc install --channel dev` (or `gtc-dev install`) first.",
                &[("dir", dir.as_str())],
            )
        );
        return 1;
    }
    let links_dir = match channel_links_dir(DEV_CHANNEL) {
        Ok(dir) => dir,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    let path_entry = match refresh(&links_dir, &binaries) {
        Some(Ok(sync)) => {
            for name in &sync.skipped {
                eprintln!(
                    "warning: {}/{name} is not a link; left it in place",
                    links_dir.display()
                );
            }
            Some(links_dir.as_path())
        }
        Some(Err(err)) => {
            eprintln!(
                "warning: dev channel links in {} were not refreshed ({err}); \
                 selecting the dev binaries through GREENTIC_*_BIN only",
                links_dir.display()
            );
            None
        }
        None => None,
    };
    print!("{}", render_activation(shell, path_entry, &binaries));
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn touch(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, b"#!/bin/sh\n").expect("write binary");
        path
    }

    #[test]
    fn canonical_names_strip_only_the_dev_suffix_of_companions() {
        let exe = std::env::consts::EXE_SUFFIX;
        let name = |base: &str| format!("{base}{exe}");
        assert_eq!(
            canonical_name_for(&name("greentic-start-dev")).as_deref(),
            Some("greentic-start")
        );
        assert_eq!(
            canonical_name_for(&name("greentic-dev-dev")).as_deref(),
            Some("greentic-dev")
        );
        assert_eq!(
            canonical_name_for(&name("greentic-deploy-platform-dev")).as_deref(),
            Some("greentic-deploy-platform")
        );
        // The stable developer CLI merely ends in "-dev".
        assert_eq!(canonical_name_for(&name("greentic-dev")), None);
        // The launcher itself is never linked under the stable name.
        assert_eq!(canonical_name_for(&name("gtc-dev")), None);
        // Stable companions and backups are not dev binaries.
        assert_eq!(canonical_name_for(&name("greentic-start")), None);
        assert_eq!(canonical_name_for("greentic-start-dev.prev"), None);
    }

    #[test]
    fn discovery_finds_dev_binaries_and_ignores_stable_ones() {
        let bin = tempdir().expect("tempdir");
        let exe = std::env::consts::EXE_SUFFIX;
        let start = touch(bin.path(), &format!("greentic-start-dev{exe}"));
        let runner = touch(bin.path(), &format!("greentic-runner-dev{exe}"));
        touch(bin.path(), &format!("greentic-start{exe}"));
        touch(bin.path(), &format!("greentic-dev{exe}"));
        touch(bin.path(), &format!("gtc-dev{exe}"));
        fs::create_dir(bin.path().join(format!("greentic-dir-dev{exe}"))).expect("dir");

        let found = discover_dev_binaries(bin.path()).expect("discover");
        assert_eq!(
            found,
            vec![
                DevBinary {
                    canonical: "greentic-runner".to_string(),
                    path: runner
                },
                DevBinary {
                    canonical: "greentic-start".to_string(),
                    path: start
                },
            ]
        );
    }

    #[test]
    fn discovery_of_a_missing_bin_dir_is_empty() {
        let root = tempdir().expect("tempdir");
        let found = discover_dev_binaries(&root.path().join("absent")).expect("discover");
        assert!(found.is_empty());
    }

    #[test]
    fn links_dir_lives_under_the_toolchain_state_dir() {
        let state = tempdir().expect("tempdir");
        assert_eq!(
            links_dir_under(state.path(), DEV_CHANNEL),
            state.path().join("channels").join("dev").join("bin")
        );
    }

    #[cfg(unix)]
    #[test]
    fn sync_creates_updates_and_removes_links_without_touching_binaries() {
        let bin = tempdir().expect("bin");
        let state = tempdir().expect("state");
        let links = links_dir_under(state.path(), DEV_CHANNEL);
        let stable = touch(bin.path(), "greentic-start");
        let start = touch(bin.path(), "greentic-start-dev");
        let runner = touch(bin.path(), "greentic-runner-dev");

        // First install: both links created, pointing at the -dev binaries.
        let found = discover_dev_binaries(bin.path()).expect("discover");
        let sync = sync_links(&links, &found).expect("sync");
        assert_eq!(sync.created, vec!["greentic-runner", "greentic-start"]);
        assert_eq!(fs::read_link(links.join("greentic-start")).unwrap(), start);
        assert_eq!(
            fs::read_link(links.join("greentic-runner")).unwrap(),
            runner
        );

        // Re-running is a no-op.
        let again = sync_links(&links, &found).expect("sync again");
        assert!(!again.changed(), "{again:?}");

        // A binary that moved is re-pointed; one that disappeared is unlinked.
        let moved = tempdir().expect("moved");
        let start_moved = touch(moved.path(), "greentic-start-dev");
        let next = vec![DevBinary {
            canonical: "greentic-start".to_string(),
            path: start_moved.clone(),
        }];
        let sync = sync_links(&links, &next).expect("sync update");
        assert_eq!(sync.updated, vec!["greentic-start"]);
        assert_eq!(sync.removed, vec!["greentic-runner"]);
        assert_eq!(
            fs::read_link(links.join("greentic-start")).unwrap(),
            start_moved
        );
        assert!(fs::symlink_metadata(links.join("greentic-runner")).is_err());

        // Nothing installed any more: every link goes, the directory stays.
        let sync = sync_links(&links, &[]).expect("sync empty");
        assert_eq!(sync.removed, vec!["greentic-start"]);
        assert!(links.is_dir());

        // The stable binary and the -dev binaries were never modified.
        assert!(stable.is_file() && start.is_file() && runner.is_file());
        assert!(
            !fs::symlink_metadata(&stable)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[cfg(unix)]
    #[test]
    fn sync_with_nothing_to_link_creates_no_directory() {
        let state = tempdir().expect("state");
        let links = links_dir_under(state.path(), DEV_CHANNEL);
        let sync = sync_links(&links, &[]).expect("sync");
        assert_eq!(sync, LinkSync::default());
        assert!(!state.path().join("channels").exists());
    }

    #[cfg(unix)]
    #[test]
    fn sync_never_overwrites_a_file_that_is_not_a_link() {
        let bin = tempdir().expect("bin");
        let state = tempdir().expect("state");
        let links = links_dir_under(state.path(), DEV_CHANNEL);
        fs::create_dir_all(&links).expect("links dir");
        let own = touch(&links, "greentic-start");
        let stray = touch(&links, "notes.txt");
        let found = vec![DevBinary {
            canonical: "greentic-start".to_string(),
            path: touch(bin.path(), "greentic-start-dev"),
        }];

        let sync = sync_links(&links, &found).expect("sync");
        assert_eq!(sync.skipped, vec!["greentic-start"]);
        assert!(sync.removed.is_empty());
        assert_eq!(fs::read(&own).unwrap(), b"#!/bin/sh\n");
        assert!(stray.is_file());
    }

    fn sample_binaries() -> Vec<DevBinary> {
        vec![
            DevBinary {
                canonical: "greentic-deploy-platform".to_string(),
                path: PathBuf::from("/home/op/.cargo/bin/greentic-deploy-platform-dev"),
            },
            DevBinary {
                canonical: "greentic-gui".to_string(),
                path: PathBuf::from("/home/op/.cargo/bin/greentic-gui-dev"),
            },
            DevBinary {
                canonical: "greentic-start".to_string(),
                path: PathBuf::from("/home/o'p/.cargo/bin/greentic-start-dev"),
            },
        ]
    }

    #[test]
    fn sh_activation_prepends_the_links_and_exports_known_overrides() {
        let out = render_activation(
            Shell::Sh,
            Some(Path::new("/home/op/.greentic/toolchain/channels/dev/bin")),
            &sample_binaries(),
        );
        let lines: Vec<&str> = out.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(
            lines,
            vec![
                "export PATH='/home/op/.greentic/toolchain/channels/dev/bin':\"$PATH\"",
                "export GREENTIC_PLATFORM_BIN='/home/op/.cargo/bin/greentic-deploy-platform-dev'",
                "export GREENTIC_START_BIN='/home/o'\\''p/.cargo/bin/greentic-start-dev'",
            ]
        );
    }

    #[test]
    fn activation_without_a_links_dir_still_selects_through_overrides() {
        let out = render_activation(Shell::Pwsh, None, &sample_binaries());
        assert!(!out.contains("$env:PATH"), "{out}");
        assert!(out.contains(
            "$env:GREENTIC_PLATFORM_BIN = '/home/op/.cargo/bin/greentic-deploy-platform-dev'"
        ));
        assert!(
            out.contains("$env:GREENTIC_START_BIN = '/home/o''p/.cargo/bin/greentic-start-dev'")
        );
    }

    #[test]
    fn fish_activation_uses_set_gx() {
        let out = render_activation(Shell::Fish, Some(Path::new("/l")), &sample_binaries());
        assert!(out.contains("set -gx PATH '/l' $PATH\n"), "{out}");
        assert!(
            out.contains(
                "set -gx GREENTIC_START_BIN '/home/o\\'p/.cargo/bin/greentic-start-dev'\n"
            )
        );
    }

    #[test]
    fn every_gtc_companion_has_an_override_in_the_table() {
        use crate::{
            BUNDLE_BIN, COMPONENT_BIN, DEPLOYER_BIN, DEV_BIN, DW_BIN, FLOW_BIN, OP_BIN, PACK_BIN,
            PLATFORM_BIN, RUNNER_BIN, SECRETS_BIN, SETUP_BIN, START_BIN,
        };
        for name in [
            BUNDLE_BIN,
            COMPONENT_BIN,
            DEPLOYER_BIN,
            DEV_BIN,
            DW_BIN,
            FLOW_BIN,
            OP_BIN,
            PACK_BIN,
            PLATFORM_BIN,
            RUNNER_BIN,
            SECRETS_BIN,
            SETUP_BIN,
            START_BIN,
        ] {
            assert!(
                env_var_for(name).is_some(),
                "{name} has no GREENTIC_*_BIN entry"
            );
        }
    }

    #[test]
    fn shell_names_parse() {
        assert_eq!(Shell::parse("bash"), Some(Shell::Sh));
        assert_eq!(Shell::parse("fish"), Some(Shell::Fish));
        assert_eq!(Shell::parse("pwsh"), Some(Shell::Pwsh));
        assert_eq!(Shell::parse("tcsh"), None);
    }
}
