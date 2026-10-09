//! Installing a toolchain package straight from a release archive the manifest
//! names, instead of through `cargo binstall`.
//!
//! `cargo binstall` resolves every version through the crates.io index, so a
//! toolchain manifest could only ever pin a version crates.io carries. That
//! stopped being a safe assumption on 2026-09-08, when crates.io locked the
//! account that publishes every Greentic crate: dev builds kept attaching their
//! archives to GitHub releases, but none of them reached the index, and the dev
//! channel froze on whatever it pinned the day before.
//!
//! A package that carries `artifacts` names, per target, the archive URL and its
//! sha256 — the same shape the manifest's `gtc` field already uses for
//! self-update, and for the same reason: a name the publisher states cannot
//! drift from the name the publisher used. When the running target has an
//! entry, the archive is fetched, verified, and exactly the package's `bins` are
//! installed into the cargo bin directory `cargo binstall` would have used. A
//! package without an entry for this target installs through `cargo binstall`
//! exactly as before, so every manifest published so far keeps its meaning.
//!
//! There is deliberately no fallback from a failed artifact install to
//! `cargo binstall`. A manifest names an artifact precisely when the version is
//! not reachable any other way, and quietly installing whatever crates.io has
//! instead would report success for a binary the manifest never pinned.

use std::fs;
use std::path::{Path, PathBuf};

use gtc::error::{GtcError, GtcResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::archive::{
    extract_targz_bytes, extract_zip_bytes, looks_like_gzip, looks_like_zip, set_executable_if_unix,
};
use super::install::{fetch_https_bytes, list_files_recursive};

/// One release archive of a toolchain package, as named by whoever published it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PackageArtifactRef {
    /// Rust target triple the archive was built for, e.g. `x86_64-unknown-linux-gnu`.
    pub target: String,
    /// `https://` URL of a `.tgz` or `.zip` archive (`file://` is accepted for
    /// local manifests and tests).
    pub url: String,
    /// Hex sha256 of the archive, WITHOUT a `sha256:` prefix — the same shape as
    /// `GtcArtifactRef::sha256`.
    pub sha256: String,
}

/// The artifact `artifacts` names for `target`, if it names one.
pub(crate) fn artifact_for_target<'a>(
    artifacts: Option<&'a [PackageArtifactRef]>,
    target: &str,
) -> Option<&'a PackageArtifactRef> {
    artifacts?.iter().find(|artifact| artifact.target == target)
}

/// Structural checks for one package's `artifacts`, run with the rest of
/// manifest validation so a malformed entry is refused before anything is
/// downloaded or installed.
pub(crate) fn validate_package_artifacts(
    crate_name: &str,
    version: &str,
    artifacts: &[PackageArtifactRef],
) -> GtcResult<()> {
    if super::toolchain::is_latest_version(version) {
        // An archive is one concrete build. Pairing it with `latest` would
        // make the manifest claim a version it cannot know.
        return Err(GtcError::message(format!(
            "toolchain package '{crate_name}' names release artifacts but pins no version"
        )));
    }
    let mut targets = std::collections::HashSet::new();
    for artifact in artifacts {
        if artifact.target.trim().is_empty() {
            return Err(GtcError::message(format!(
                "toolchain package '{crate_name}' has an artifact with no target"
            )));
        }
        if !targets.insert(artifact.target.as_str()) {
            return Err(GtcError::message(format!(
                "toolchain package '{crate_name}' names more than one artifact for {}",
                artifact.target
            )));
        }
        if !is_hex_sha256(&artifact.sha256) {
            return Err(GtcError::message(format!(
                "toolchain package '{crate_name}' artifact for {} has an invalid sha256 \
                 (expected 64 hex characters without a prefix)",
                artifact.target
            )));
        }
        if !(artifact.url.starts_with("https://") || artifact.url.starts_with("file://")) {
            return Err(GtcError::message(format!(
                "toolchain package '{crate_name}' artifact for {} must be an https:// URL: {}",
                artifact.target, artifact.url
            )));
        }
    }
    Ok(())
}

fn is_hex_sha256(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

/// Download `artifact`, verify it, and install `bins` from it into `bin_dir`.
///
/// Every bin is located in the extracted archive BEFORE any of them is written,
/// so an archive missing one of them installs nothing rather than half a
/// package. Returns the installed paths.
pub(crate) fn install_package_artifact(
    artifact: &PackageArtifactRef,
    bins: &[String],
    bin_dir: &Path,
    locale: &str,
) -> GtcResult<Vec<PathBuf>> {
    let bytes = fetch_artifact_bytes(&artifact.url, locale)?;
    verify_artifact_sha256(&bytes, &artifact.sha256, &artifact.url)?;
    install_bins_from_archive(&bytes, bins, bin_dir)
}

fn fetch_artifact_bytes(url: &str, locale: &str) -> GtcResult<Vec<u8>> {
    if let Some(path) = url.strip_prefix("file://") {
        return fs::read(path).map_err(|e| GtcError::io(format!("failed to read {path}"), e));
    }
    if url.starts_with("https://") {
        // No credential: a toolchain archive is a public release asset, and
        // `fetch_https_bytes` follows GitHub's redirect to its object storage
        // the same way self-update does.
        return fetch_https_bytes(url, "", locale, "application/octet-stream");
    }
    Err(GtcError::invalid_data(
        "toolchain artifact URL",
        format!("unsupported scheme for {url}"),
    ))
}

fn verify_artifact_sha256(bytes: &[u8], expected_hex: &str, url: &str) -> GtcResult<()> {
    let digest = Sha256::digest(bytes);
    let mut actual = String::with_capacity(64);
    for byte in digest {
        actual.push_str(&format!("{byte:02x}"));
    }
    if actual.eq_ignore_ascii_case(expected_hex) {
        return Ok(());
    }
    Err(GtcError::invalid_data(
        format!("integrity check for {url}"),
        format!(
            "expected sha256:{}, got sha256:{actual}",
            expected_hex.to_lowercase()
        ),
    ))
}

pub(crate) fn install_bins_from_archive(
    bytes: &[u8],
    bins: &[String],
    bin_dir: &Path,
) -> GtcResult<Vec<PathBuf>> {
    let extract_dir = tempfile::tempdir()
        .map_err(|e| GtcError::io("failed to create artifact extract directory", e))?;
    if looks_like_gzip(bytes) {
        extract_targz_bytes(bytes, extract_dir.path())?;
    } else if looks_like_zip(bytes) {
        extract_zip_bytes(bytes, extract_dir.path())?;
    } else {
        return Err(GtcError::invalid_data(
            "toolchain artifact",
            "not a .tgz or .zip archive",
        ));
    }

    let files = list_files_recursive(extract_dir.path())?;
    let mut located = Vec::with_capacity(bins.len());
    for bin in bins {
        located.push((bin, locate_bin(&files, bin)?));
    }

    fs::create_dir_all(bin_dir)
        .map_err(|e| GtcError::io(format!("failed to create {}", bin_dir.display()), e))?;
    let mut installed = Vec::with_capacity(located.len());
    for (bin, source) in located {
        let file_name = source
            .file_name()
            .ok_or_else(|| GtcError::message(format!("invalid file name for {bin}")))?;
        let target = bin_dir.join(file_name);
        replace_file(&source, &target)?;
        installed.push(target);
    }
    Ok(installed)
}

/// The single file in the archive that IS `bin`.
///
/// Matched by exact file name (plus `.exe`), never by prefix: a dev archive
/// ships `greentic-start-dev` beside a README and a LICENSE, and a prefix match
/// is how the wrong file gets installed under a tool's name. Ambiguity is an
/// error for the same reason.
fn locate_bin(files: &[PathBuf], bin: &str) -> GtcResult<PathBuf> {
    let windows_name = format!("{bin}.exe");
    let matches: Vec<&PathBuf> = files
        .iter()
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name == bin || name == windows_name)
        })
        .collect();
    match matches.as_slice() {
        [only] => Ok((*only).clone()),
        [] => Err(GtcError::invalid_data(
            "toolchain artifact",
            format!("the archive contains no binary named {bin}"),
        )),
        _ => Err(GtcError::invalid_data(
            "toolchain artifact",
            format!("the archive contains more than one binary named {bin}"),
        )),
    }
}

/// Write `source` to `target` through a sibling temp file and a rename, so an
/// interrupted install never leaves a truncated binary where a working one was.
fn replace_file(source: &Path, target: &Path) -> GtcResult<()> {
    let file_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| GtcError::message(format!("invalid install path {}", target.display())))?;
    let staging = target.with_file_name(format!(".{file_name}.gtc-install-{}", std::process::id()));
    fs::copy(source, &staging).map_err(|e| {
        GtcError::io(
            format!(
                "failed to stage {} -> {}",
                source.display(),
                staging.display()
            ),
            e,
        )
    })?;
    if let Err(err) = set_executable_if_unix(&staging) {
        let _ = fs::remove_file(&staging);
        return Err(err);
    }
    // Windows refuses to rename over an existing file.
    #[cfg(windows)]
    if target.exists() {
        fs::remove_file(target)
            .map_err(|e| GtcError::io(format!("failed to replace {}", target.display()), e))?;
    }
    fs::rename(&staging, target).map_err(|e| {
        let _ = fs::remove_file(&staging);
        GtcError::io(
            format!(
                "failed to install {} -> {}",
                staging.display(),
                target.display()
            ),
            e,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tgz_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::default(),
        ));
        for (path, data) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, path, *data)
                .expect("append tar entry");
        }
        builder
            .into_inner()
            .expect("finish tar")
            .finish()
            .expect("finish gzip")
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn artifact(target: &str, url: &str, sha256: &str) -> PackageArtifactRef {
        PackageArtifactRef {
            target: target.to_string(),
            url: url.to_string(),
            sha256: sha256.to_string(),
        }
    }

    #[test]
    fn artifact_for_target_picks_the_running_target_only() {
        let artifacts = vec![
            artifact("x86_64-unknown-linux-gnu", "https://a", &"a".repeat(64)),
            artifact("aarch64-apple-darwin", "https://b", &"b".repeat(64)),
        ];
        assert_eq!(
            artifact_for_target(Some(&artifacts), "aarch64-apple-darwin").map(|a| a.url.as_str()),
            Some("https://b")
        );
        assert!(artifact_for_target(Some(&artifacts), "x86_64-pc-windows-msvc").is_none());
        assert!(artifact_for_target(None, "x86_64-unknown-linux-gnu").is_none());
    }

    #[test]
    fn validation_refuses_what_could_not_be_installed_safely() {
        let good = artifact(
            "x86_64-unknown-linux-gnu",
            "https://x/y.tgz",
            &"0".repeat(64),
        );
        validate_package_artifacts("greentic-start-dev", "1.2.3", std::slice::from_ref(&good))
            .expect("valid");

        assert!(validate_package_artifacts("p", "latest", std::slice::from_ref(&good)).is_err());
        assert!(
            validate_package_artifacts("p", "1.2.3", &[good.clone(), good.clone()]).is_err(),
            "two artifacts for one target"
        );
        let prefixed = artifact(
            &good.target,
            &good.url,
            &format!("sha256:{}", "0".repeat(64)),
        );
        assert!(validate_package_artifacts("p", "1.2.3", &[prefixed]).is_err());
        let short = artifact(&good.target, &good.url, "abc");
        assert!(validate_package_artifacts("p", "1.2.3", &[short]).is_err());
        let plain_http = artifact(&good.target, "http://x/y.tgz", &good.sha256);
        assert!(validate_package_artifacts("p", "1.2.3", &[plain_http]).is_err());
        let no_target = artifact(" ", &good.url, &good.sha256);
        assert!(validate_package_artifacts("p", "1.2.3", &[no_target]).is_err());
    }

    #[test]
    fn installs_exactly_the_named_bins_from_a_verified_archive() {
        let archive = tgz_with(&[
            (
                "greentic-start-dev-v1.2.3-x86_64-unknown-linux-gnu/greentic-start-dev",
                b"#!bin",
            ),
            (
                "greentic-start-dev-v1.2.3-x86_64-unknown-linux-gnu/README.md",
                b"readme",
            ),
            (
                "greentic-start-dev-v1.2.3-x86_64-unknown-linux-gnu/greentic-start-dev.sha256",
                b"x",
            ),
        ]);
        let work = tempfile::tempdir().expect("tempdir");
        let archive_path = work.path().join("pkg.tgz");
        fs::write(&archive_path, &archive).expect("write archive");
        let bin_dir = work.path().join("bin");
        let entry = artifact(
            "x86_64-unknown-linux-gnu",
            &format!("file://{}", archive_path.display()),
            &sha256_hex(&archive),
        );

        let installed =
            install_package_artifact(&entry, &["greentic-start-dev".to_string()], &bin_dir, "en")
                .expect("install");

        assert_eq!(installed, vec![bin_dir.join("greentic-start-dev")]);
        assert_eq!(
            fs::read(bin_dir.join("greentic-start-dev")).expect("read"),
            b"#!bin"
        );
        let mut names: Vec<_> = fs::read_dir(&bin_dir)
            .expect("read bin dir")
            .map(|entry| entry.expect("entry").file_name())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec!["greentic-start-dev"],
            "no README, sidecar or staging file"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(bin_dir.join("greentic-start-dev"))
                .expect("stat")
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111, "installed binary is executable");
        }
    }

    #[test]
    fn a_checksum_mismatch_installs_nothing() {
        let archive = tgz_with(&[("pkg/greentic-flow-dev", b"#!bin")]);
        let work = tempfile::tempdir().expect("tempdir");
        let archive_path = work.path().join("pkg.tgz");
        fs::write(&archive_path, &archive).expect("write archive");
        let bin_dir = work.path().join("bin");
        let entry = artifact(
            "x86_64-unknown-linux-gnu",
            &format!("file://{}", archive_path.display()),
            &"0".repeat(64),
        );

        let err =
            install_package_artifact(&entry, &["greentic-flow-dev".to_string()], &bin_dir, "en")
                .expect_err("mismatch must fail");

        assert!(err.to_string().contains("integrity check"), "{err}");
        assert!(
            !bin_dir.exists(),
            "nothing written on a failed verification"
        );
    }

    #[test]
    fn a_missing_bin_installs_none_of_the_package() {
        let archive = tgz_with(&[("pkg/greentic-runner-dev", b"#!bin")]);
        let bin_dir = tempfile::tempdir().expect("tempdir");

        let err = install_bins_from_archive(
            &archive,
            &[
                "greentic-runner-dev".to_string(),
                "greentic-runner-cli".to_string(),
            ],
            bin_dir.path(),
        )
        .expect_err("missing bin must fail");

        assert!(err.to_string().contains("greentic-runner-cli"), "{err}");
        assert!(
            fs::read_dir(bin_dir.path()).expect("read").next().is_none(),
            "the bin that WAS present must not be installed on its own"
        );
    }

    #[test]
    fn an_ambiguous_bin_is_refused() {
        let archive = tgz_with(&[
            ("a/greentic-pack-dev", b"one"),
            ("b/greentic-pack-dev", b"two"),
        ]);
        let bin_dir = tempfile::tempdir().expect("tempdir");
        let err =
            install_bins_from_archive(&archive, &["greentic-pack-dev".to_string()], bin_dir.path())
                .expect_err("ambiguous must fail");
        assert!(err.to_string().contains("more than one"), "{err}");
    }

    #[test]
    fn a_prefix_match_is_not_the_bin() {
        let archive = tgz_with(&[("pkg/greentic-pack-dev-helper", b"nope")]);
        let bin_dir = tempfile::tempdir().expect("tempdir");
        assert!(
            install_bins_from_archive(&archive, &["greentic-pack-dev".to_string()], bin_dir.path())
                .is_err()
        );
    }

    #[test]
    fn reinstalling_replaces_the_previous_binary() {
        let bin_dir = tempfile::tempdir().expect("tempdir");
        fs::write(bin_dir.path().join("greentic-setup-dev"), b"old").expect("seed");
        let archive = tgz_with(&[("pkg/greentic-setup-dev", b"new")]);
        install_bins_from_archive(
            &archive,
            &["greentic-setup-dev".to_string()],
            bin_dir.path(),
        )
        .expect("install");
        assert_eq!(
            fs::read(bin_dir.path().join("greentic-setup-dev")).expect("read"),
            b"new"
        );
    }

    #[test]
    fn a_non_archive_is_refused() {
        let bin_dir = tempfile::tempdir().expect("tempdir");
        assert!(
            install_bins_from_archive(b"plain text", &["x".to_string()], bin_dir.path()).is_err()
        );
    }
}
