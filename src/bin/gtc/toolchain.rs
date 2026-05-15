use std::collections::{BTreeMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use clap::ArgMatches;
use directories::BaseDirs;
use greentic_distributor_client::{
    CachePolicy, DistClient, DistOptions, ReleaseArtifactKind, ResolvePolicy,
};
use gtc::error::{GtcError, GtcResult};
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, AUTHORIZATION, WWW_AUTHENTICATE};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::i18n_support::{t, tf};
use super::install::{run_cargo, run_cargo_capture};
use super::process::run_binary_capture;

const TOOLCHAIN_MANIFEST_SCHEMA: &str = "greentic.toolchain-manifest.v1";
const INSTALLED_TOOLCHAIN_SCHEMA: &str = "greentic.installed-toolchain.v1";
const TOOLCHAIN_MANIFEST_MEDIA_TYPE: &str = "application/vnd.greentic.toolchain.manifest.v1+json";
const DEFAULT_GHCR_PREFIX: &str = "ghcr.io/greenticai/greentic-versions/gtc";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ToolchainManifest {
    pub schema: String,
    pub toolchain: String,
    pub version: String,
    pub channel: Option<String>,
    pub created_at: Option<String>,
    pub packages: Vec<ToolchainPackage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension_packs: Option<Vec<ToolchainArtifactRef>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub components: Option<Vec<ToolchainArtifactRef>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ToolchainPackage {
    #[serde(rename = "crate")]
    pub crate_name: String,
    pub bins: Vec<String>,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ToolchainArtifactRef {
    pub id: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedManifest {
    pub source: String,
    pub source_kind: String,
    pub digest: Option<String>,
    pub manifest: ToolchainManifest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct InstalledToolchain {
    pub schema: String,
    pub source_kind: String,
    pub source: String,
    pub resolved_digest: Option<String>,
    pub channel: Option<String>,
    pub version: String,
    pub installed_at: String,
    pub packages: Vec<ToolchainPackage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InstalledReleaseArtifacts {
    pub release: String,
    pub channel: String,
    pub index_path: PathBuf,
    pub packs: Vec<InstalledReleaseArtifact>,
    pub components: Vec<InstalledReleaseArtifact>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InstalledReleaseArtifact {
    pub reference: String,
    pub version: String,
    pub digest: String,
    pub canonical_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolchainInstallOptions {
    pub source: ToolchainSource,
    pub force: bool,
    pub dry_run: bool,
    pub phases: ToolchainInstallPhases,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ToolchainSource {
    Channel(String),
    Release { release: String, channel: String },
    LocalManifest(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ToolchainInstallPhases {
    pub binaries: bool,
    pub packs: bool,
    pub components: bool,
}

impl ToolchainInstallPhases {
    pub(crate) fn all() -> Self {
        Self {
            binaries: true,
            packs: true,
            components: true,
        }
    }

    pub(crate) fn any_artifacts(self) -> bool {
        self.packs || self.components
    }
}

impl ToolchainInstallOptions {
    pub(crate) fn from_matches(matches: &ArgMatches, default_channel: &str) -> GtcResult<Self> {
        let source = if let Some(path) = matches.get_one::<String>("manifest") {
            ToolchainSource::LocalManifest(PathBuf::from(path))
        } else if let Some(release) = matches
            .get_one::<String>("release")
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            let channel = matches
                .get_one::<String>("channel")
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .unwrap_or(default_channel);
            ToolchainSource::Release {
                release: release.to_string(),
                channel: channel.to_string(),
            }
        } else {
            let channel = matches
                .get_one::<String>("channel")
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .unwrap_or(default_channel);
            ToolchainSource::Channel(channel.to_string())
        };

        let selected_binaries = matches.get_flag("install-binaries-only");
        let selected_packs = matches.get_flag("install-packs-only");
        let selected_components = matches.get_flag("install-components-only");
        let phases = if selected_binaries || selected_packs || selected_components {
            ToolchainInstallPhases {
                binaries: selected_binaries,
                packs: selected_packs,
                components: selected_components,
            }
        } else {
            ToolchainInstallPhases::all()
        };

        Ok(Self {
            source,
            force: matches.get_flag("force"),
            dry_run: matches.get_flag("dry-run"),
            phases,
        })
    }
}

pub(crate) fn run_toolchain_install(
    options: ToolchainInstallOptions,
    debug: bool,
    locale: &str,
) -> i32 {
    let resolved = match resolve_toolchain_manifest(&options.source, debug, locale) {
        Ok(resolved) => resolved,
        Err(err) => {
            eprintln!("{}: {err}", t(locale, "gtc.err.invalid_toolchain_manifest"));
            return 1;
        }
    };

    println!(
        "{}",
        tf(
            locale,
            "gtc.install.toolchain.resolving",
            &[("source", resolved.source.as_str())]
        )
    );
    if let Some(digest) = resolved.digest.as_deref() {
        println!("  digest: {digest}");
    }

    if !options.force
        && !options.dry_run
        && options.phases.binaries
        && !has_selected_release_artifacts(&resolved.manifest, options.phases)
        && let Ok(Some(installed)) = read_installed_toolchain()
        && installed.resolved_digest == resolved.digest
        && resolved.digest.is_some()
    {
        println!("{}", t(locale, "gtc.install.toolchain.up_to_date"));
        return 0;
    }

    if options.dry_run {
        println!("{}", t(locale, "gtc.install.toolchain.dry_run"));
        if options.phases.binaries {
            for package in &resolved.manifest.packages {
                let version = match resolve_toolchain_package_version(package, debug, locale) {
                    Ok(version) => version,
                    Err(err) => {
                        eprintln!("{err}");
                        return 1;
                    }
                };
                for bin in &package.bins {
                    let args = toolchain_binstall_args_for_version(package, bin, &version);
                    println!("cargo {}", args.join(" "));
                }
            }
        }
        if options.phases.packs {
            for item in resolved.manifest.extension_packs.iter().flatten() {
                println!("prefetch pack {}:{}", item.id, item.version);
            }
        }
        if options.phases.components {
            for item in resolved.manifest.components.iter().flatten() {
                println!("prefetch component {}:{}", item.id, item.version);
            }
        }
        return 0;
    }

    if options.phases.binaries {
        let install_status = install_toolchain_manifest(&resolved, options.force, debug, locale);
        if install_status != 0 {
            return install_status;
        }
    }

    let release_context = match release_context_from_resolved(&options.source, &resolved.manifest) {
        Ok(ctx) => ctx,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    if options.phases.any_artifacts()
        && let Some(ctx) = release_context
    {
        if let Err(err) = prefetch_release_artifacts_and_write_index(
            &resolved.manifest,
            &ctx,
            options.phases,
            options.force,
        ) {
            eprintln!("failed to prefetch release artifacts: {err}");
            return 1;
        }
        if let Err(err) = write_current_release_context(&ctx) {
            eprintln!("failed to write current release context: {err}");
            return 1;
        }
    }

    if options.phases.binaries {
        let state = installed_state_from_resolved(&resolved);
        if let Err(err) = write_installed_toolchain(&state) {
            eprintln!(
                "{}: {err}",
                t(locale, "gtc.install.toolchain.state_write_failed")
            );
            return 1;
        }
    }

    0
}

pub(crate) fn install_toolchain_manifest(
    resolved: &ResolvedManifest,
    force: bool,
    debug: bool,
    locale: &str,
) -> i32 {
    for package in &resolved.manifest.packages {
        let status = install_toolchain_package(package, force, debug, locale);
        if status != 0 {
            return status;
        }
    }
    0
}

pub(crate) fn install_toolchain_package(
    package: &ToolchainPackage,
    force: bool,
    debug: bool,
    locale: &str,
) -> i32 {
    let version = match resolve_toolchain_package_version(package, debug, locale) {
        Ok(version) => version,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    for bin in &package.bins {
        if !force && installed_binary_version_matches(bin, &version, debug, locale) {
            println!("Installed {bin} binary already matches {version}; skipping.");
            continue;
        }
        println!(
            "{}",
            tf(
                locale,
                "gtc.install.toolchain.installing_package",
                &[
                    ("crate", package.crate_name.as_str()),
                    ("bin", bin.as_str())
                ]
            )
        );
        let args = toolchain_binstall_args_for_version(package, bin, &version);
        let status = run_cargo(&args, debug, locale);
        if status != 0 {
            eprintln!(
                "{}",
                tf(
                    locale,
                    "gtc.install.toolchain.item_fail",
                    &[
                        ("crate", package.crate_name.as_str()),
                        ("bin", bin.as_str())
                    ]
                )
            );
            return status;
        }
        println!(
            "{}",
            tf(
                locale,
                "gtc.install.toolchain.item_ok",
                &[
                    ("crate", package.crate_name.as_str()),
                    ("bin", bin.as_str())
                ]
            )
        );
    }
    0
}

fn installed_binary_version_matches(bin: &str, version: &str, debug: bool, locale: &str) -> bool {
    if is_latest_version(version) {
        return false;
    }
    let args = vec!["--version".to_string()];
    let Ok(output) = run_binary_capture(bin, &args, debug, locale) else {
        return false;
    };
    version_output_matches(&output, version)
}

fn version_output_matches(output: &str, expected: &str) -> bool {
    output
        .split_whitespace()
        .map(|token| {
            token.trim_matches(|ch: char| {
                matches!(
                    ch,
                    ',' | ';' | ':' | '(' | ')' | '[' | ']' | '{' | '}' | '"' | '\''
                )
            })
        })
        .any(|token| token == expected || token.strip_prefix('v') == Some(expected))
}

#[cfg(test)]
pub(crate) fn toolchain_binstall_args(package: &ToolchainPackage, bin: &str) -> Vec<String> {
    toolchain_binstall_args_for_version(package, bin, &package.version)
}

fn toolchain_binstall_args_for_version(
    package: &ToolchainPackage,
    bin: &str,
    version: &str,
) -> Vec<String> {
    let mut args = vec![
        "binstall".to_string(),
        "-y".to_string(),
        "--locked".to_string(),
        "--force".to_string(),
        package.crate_name.clone(),
    ];
    if !is_latest_version(version) {
        args.push("--version".to_string());
        args.push(version.to_string());
    }
    args.push("--bin".to_string());
    args.push(bin.to_string());
    args
}

fn resolve_toolchain_package_version(
    package: &ToolchainPackage,
    debug: bool,
    locale: &str,
) -> GtcResult<String> {
    if !is_latest_version(&package.version) {
        return Ok(package.version.clone());
    }
    latest_crate_search_version(&package.crate_name, debug, locale).ok_or_else(|| {
        GtcError::message(format!(
            "failed to resolve latest crates.io version for {}",
            package.crate_name
        ))
    })
}

fn latest_crate_search_version(package: &str, debug: bool, locale: &str) -> Option<String> {
    let output = run_cargo_capture(&["search", package, "--limit", "1"], debug, locale)?;
    if !output.status.success() {
        return None;
    }
    parse_cargo_search_version(&String::from_utf8_lossy(&output.stdout), package)
}

fn parse_cargo_search_version(output: &str, package: &str) -> Option<String> {
    for line in output.lines() {
        let Some((name, rest)) = line.split_once('=') else {
            continue;
        };
        if name.trim() != package {
            continue;
        }
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix('"') else {
            continue;
        };
        let Some((version, _)) = rest.split_once('"') else {
            continue;
        };
        if !version.trim().is_empty() {
            return Some(version.to_string());
        }
    }
    None
}

pub(crate) fn resolve_toolchain_manifest(
    source: &ToolchainSource,
    debug: bool,
    locale: &str,
) -> GtcResult<ResolvedManifest> {
    if !matches!(source, ToolchainSource::LocalManifest(_))
        && let Some(path) = env::var_os("GTC_TOOLCHAIN_MANIFEST_PATH")
    {
        let mut resolved = resolve_local_manifest(Path::new(&path))?;
        resolved.source_kind = match source {
            ToolchainSource::Channel(_) => "channel".to_string(),
            ToolchainSource::Release { .. } => "release".to_string(),
            ToolchainSource::LocalManifest(_) => "local".to_string(),
        };
        if let Some(reference) = toolchain_source_ref(source) {
            resolved.source = reference;
        }
        return Ok(resolved);
    }

    match source {
        ToolchainSource::LocalManifest(path) => resolve_local_manifest(path),
        ToolchainSource::Channel(_) | ToolchainSource::Release { .. } => {
            let reference = toolchain_source_ref(source)
                .ok_or_else(|| GtcError::message("local manifests do not have GHCR refs"))?;
            let mut resolved = resolve_ghcr_manifest(&reference, debug, locale)?;
            resolved.source_kind = match source {
                ToolchainSource::Channel(_) => "channel".to_string(),
                ToolchainSource::Release { .. } => "release".to_string(),
                ToolchainSource::LocalManifest(_) => "local".to_string(),
            };
            Ok(resolved)
        }
    }
}

pub(crate) fn resolve_local_manifest(path: &Path) -> GtcResult<ResolvedManifest> {
    let bytes = fs::read(path).map_err(|err| {
        GtcError::message(format!(
            "failed to read toolchain manifest {}: {err}",
            path.display()
        ))
    })?;
    let manifest: ToolchainManifest = serde_json::from_slice(&bytes).map_err(|err| {
        GtcError::json(
            format!("failed to parse toolchain manifest {}", path.display()),
            err,
        )
    })?;
    validate_toolchain_manifest(&manifest)?;
    Ok(ResolvedManifest {
        source: path.display().to_string(),
        source_kind: "local".to_string(),
        digest: Some(sha256_bytes(&bytes)),
        manifest,
    })
}

pub(crate) fn resolve_ghcr_manifest(
    reference: &str,
    debug: bool,
    locale: &str,
) -> GtcResult<ResolvedManifest> {
    let parsed = GhcrReference::parse(reference)?;
    let client = Client::builder()
        .user_agent(format!("gtc/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|err| GtcError::message(format!("failed to create GHCR client: {err}")))?;
    let manifest_url = format!(
        "https://ghcr.io/v2/{}/manifests/{}",
        parsed.repository, parsed.reference
    );
    if debug {
        eprintln!("{} GET {}", t(locale, "gtc.debug.exec"), manifest_url);
    }
    let manifest_response = send_ghcr_get(
        &client,
        &manifest_url,
        Some(
            "application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json",
        ),
    )?;
    let digest = manifest_response
        .headers()
        .get("Docker-Content-Digest")
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    let manifest_text = manifest_response
        .text()
        .map_err(|err| GtcError::message(format!("failed to read OCI manifest: {err}")))?;
    let manifest_payload: OciManifest = serde_json::from_str(&manifest_text).map_err(|err| {
        GtcError::json(format!("failed to parse OCI manifest for {reference}"), err)
    })?;
    if manifest_payload.layers.len() != 1 {
        return Err(GtcError::message(format!(
            "toolchain artifact {reference} must contain exactly one manifest layer"
        )));
    }
    let layer = &manifest_payload.layers[0];
    if layer.media_type != TOOLCHAIN_MANIFEST_MEDIA_TYPE {
        return Err(GtcError::message(format!(
            "toolchain artifact {reference} has unsupported layer media type '{}'",
            layer.media_type
        )));
    }
    let blob_url = format!(
        "https://ghcr.io/v2/{}/blobs/{}",
        parsed.repository, layer.digest
    );
    if debug {
        eprintln!("{} GET {}", t(locale, "gtc.debug.exec"), blob_url);
    }
    let bytes = send_ghcr_get(&client, &blob_url, Some(TOOLCHAIN_MANIFEST_MEDIA_TYPE))?
        .bytes()
        .map_err(|err| GtcError::message(format!("failed to read GHCR manifest blob: {err}")))?;
    let manifest: ToolchainManifest = serde_json::from_slice(&bytes).map_err(|err| {
        GtcError::json(
            format!("failed to parse toolchain manifest from {reference}"),
            err,
        )
    })?;
    validate_toolchain_manifest(&manifest)?;
    Ok(ResolvedManifest {
        source: reference.to_string(),
        source_kind: "ghcr".to_string(),
        digest,
        manifest,
    })
}

pub(crate) fn toolchain_source_ref(source: &ToolchainSource) -> Option<String> {
    match source {
        ToolchainSource::Channel(channel) => Some(format!("{DEFAULT_GHCR_PREFIX}:{channel}")),
        ToolchainSource::Release { release, .. } => {
            Some(format!("{DEFAULT_GHCR_PREFIX}:{release}"))
        }
        ToolchainSource::LocalManifest(_) => None,
    }
}

pub(crate) fn validate_toolchain_manifest(manifest: &ToolchainManifest) -> GtcResult<()> {
    if manifest.schema != TOOLCHAIN_MANIFEST_SCHEMA {
        return Err(GtcError::message(format!(
            "unsupported toolchain manifest schema '{}'",
            manifest.schema
        )));
    }
    if manifest.toolchain != "gtc" {
        return Err(GtcError::message(format!(
            "unsupported toolchain '{}'",
            manifest.toolchain
        )));
    }
    if manifest.packages.is_empty() {
        return Err(GtcError::message(
            "toolchain manifest must include at least one package",
        ));
    }
    let mut seen = HashSet::new();
    for package in &manifest.packages {
        if package.crate_name.trim().is_empty() {
            return Err(GtcError::message("toolchain package is missing crate"));
        }
        if package.version.trim().is_empty() {
            return Err(GtcError::message(format!(
                "toolchain package '{}' is missing version",
                package.crate_name
            )));
        }
        if package.bins.is_empty() {
            return Err(GtcError::message(format!(
                "toolchain package '{}' is missing bins",
                package.crate_name
            )));
        }
        for bin in &package.bins {
            if bin.trim().is_empty() {
                return Err(GtcError::message(format!(
                    "toolchain package '{}' contains an empty bin",
                    package.crate_name
                )));
            }
            let key = (package.crate_name.clone(), bin.clone());
            if !seen.insert(key) {
                return Err(GtcError::message(format!(
                    "duplicate toolchain entry '{}:{}'",
                    package.crate_name, bin
                )));
            }
        }
    }
    validate_toolchain_artifact_refs("extension_packs", manifest.extension_packs.as_deref())?;
    validate_toolchain_artifact_refs("components", manifest.components.as_deref())?;
    Ok(())
}

fn validate_toolchain_artifact_refs(
    section: &str,
    refs: Option<&[ToolchainArtifactRef]>,
) -> GtcResult<()> {
    let Some(refs) = refs else {
        return Ok(());
    };
    let mut seen = HashSet::new();
    for item in refs {
        if item.id.trim().is_empty() {
            return Err(GtcError::message(format!("{section} contains an empty id")));
        }
        if item.version.trim().is_empty() {
            return Err(GtcError::message(format!(
                "{section} entry '{}' is missing version",
                item.id
            )));
        }
        if !seen.insert(item.id.clone()) {
            return Err(GtcError::message(format!(
                "{section} contains duplicate id '{}'",
                item.id
            )));
        }
    }
    Ok(())
}

pub(crate) fn is_latest_version(version: &str) -> bool {
    version == "latest"
}

pub(crate) fn installed_toolchain_path() -> GtcResult<PathBuf> {
    if let Some(root) = env::var_os("GTC_TOOLCHAIN_STATE_DIR") {
        return Ok(PathBuf::from(root).join("installed.json"));
    }
    let base =
        BaseDirs::new().ok_or_else(|| GtcError::message("failed to resolve home directory"))?;
    Ok(base
        .home_dir()
        .join(".greentic")
        .join("toolchain")
        .join("installed.json"))
}

pub(crate) fn current_release_context_path() -> GtcResult<PathBuf> {
    if let Some(root) = env::var_os("GTC_RELEASE_STATE_DIR") {
        return Ok(PathBuf::from(root).join("current.json"));
    }
    let base =
        BaseDirs::new().ok_or_else(|| GtcError::message("failed to resolve home directory"))?;
    Ok(base
        .home_dir()
        .join(".greentic")
        .join("releases")
        .join("current.json"))
}

pub(crate) fn read_installed_toolchain() -> GtcResult<Option<InstalledToolchain>> {
    let path = installed_toolchain_path()?;
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = fs::read(&path).map_err(|err| {
        GtcError::message(format!(
            "failed to read installed toolchain state {}: {err}",
            path.display()
        ))
    })?;
    let state = serde_json::from_slice(&bytes).map_err(|err| {
        GtcError::json(
            format!(
                "failed to parse installed toolchain state {}",
                path.display()
            ),
            err,
        )
    })?;
    Ok(Some(state))
}

pub(crate) fn installed_toolchain_label() -> String {
    match read_installed_toolchain() {
        Ok(Some(state)) => {
            let mut label = state.version;
            if let Some(channel) = state.channel.filter(|value| !value.trim().is_empty()) {
                label.push_str(&format!(" ({channel})"));
            }
            if let Some(digest) = state
                .resolved_digest
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                label.push_str(&format!(" [{digest}]"));
            }
            label
        }
        Ok(None) => "not installed".to_string(),
        Err(err) => format!("unknown ({err})"),
    }
}

pub(crate) fn installed_release_artifacts() -> GtcResult<Option<InstalledReleaseArtifacts>> {
    let Some(installed) = read_installed_toolchain()? else {
        return Ok(None);
    };
    let channel = installed
        .channel
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("stable");
    let Some(release_channel) = parse_release_channel(channel) else {
        return Err(GtcError::message(format!(
            "release channel '{channel}' is not supported; use stable, dev, or rnd"
        )));
    };
    let ctx = ReleaseResolutionContext {
        release: installed.version.clone(),
        channel: release_channel,
    };
    let cache_root = DistOptions::default().cache_dir;
    let index_path = release_index_path(&cache_root, &ctx)?;
    let Some(index) = read_release_index_if_exists(&index_path)? else {
        return Ok(Some(InstalledReleaseArtifacts {
            release: installed.version,
            channel: channel.to_string(),
            index_path,
            packs: Vec::new(),
            components: Vec::new(),
        }));
    };

    let mut packs = Vec::new();
    let mut components = Vec::new();
    for (reference, entry) in index.refs {
        let artifact = InstalledReleaseArtifact {
            reference: reference.clone(),
            version: entry.version,
            digest: entry.digest,
            canonical_ref: entry.canonical_ref,
        };
        if reference.contains("/packs/") {
            packs.push(artifact);
        } else if reference.contains("/components/") {
            components.push(artifact);
        }
    }

    Ok(Some(InstalledReleaseArtifacts {
        release: installed.version,
        channel: channel.to_string(),
        index_path,
        packs,
        components,
    }))
}

pub(crate) fn latest_release_context_warning(
    expected_channel: &str,
    install_command: &str,
    debug: bool,
    locale: &str,
) -> GtcResult<Option<String>> {
    let Some(installed) = read_installed_toolchain()? else {
        return Ok(Some(format!(
            "Greentic toolchain release context is not installed for channel '{expected_channel}'. Run `{install_command} install` to install the latest {expected_channel} release."
        )));
    };

    let installed_channel = installed.channel.as_deref().unwrap_or("unknown");
    if installed_channel != expected_channel {
        return Ok(Some(format!(
            "Greentic toolchain release context is on channel '{installed_channel}', but this launcher expects '{expected_channel}'. Run `{install_command} install` to install the latest {expected_channel} release."
        )));
    }

    let source = ToolchainSource::Channel(expected_channel.to_string());
    let latest = resolve_toolchain_manifest(&source, debug, locale)?;
    let installed_matches_latest = match (
        installed.resolved_digest.as_deref(),
        latest.digest.as_deref(),
    ) {
        (Some(installed_digest), Some(latest_digest)) => installed_digest == latest_digest,
        _ => installed.version == latest.manifest.version,
    };

    if installed_matches_latest {
        return Ok(None);
    }

    Ok(Some(format!(
        "Greentic toolchain release context is {} ({installed_channel}), but the latest {expected_channel} release is {}. Run `{install_command} install` to upgrade.",
        installed.version, latest.manifest.version
    )))
}

pub(crate) fn write_installed_toolchain(state: &InstalledToolchain) -> GtcResult<()> {
    let path = installed_toolchain_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            GtcError::message(format!(
                "failed to create toolchain state dir {}: {err}",
                parent.display()
            ))
        })?;
    }
    let bytes = serde_json::to_vec_pretty(state)
        .map_err(|err| GtcError::json("failed to encode installed toolchain state", err))?;
    fs::write(&path, bytes).map_err(|err| {
        GtcError::message(format!(
            "failed to write installed toolchain state {}: {err}",
            path.display()
        ))
    })
}

pub(crate) fn installed_state_from_resolved(resolved: &ResolvedManifest) -> InstalledToolchain {
    InstalledToolchain {
        schema: INSTALLED_TOOLCHAIN_SCHEMA.to_string(),
        source_kind: resolved.source_kind.clone(),
        source: resolved.source.clone(),
        resolved_digest: resolved.digest.clone(),
        channel: resolved.manifest.channel.clone(),
        version: resolved.manifest.version.clone(),
        installed_at: installed_at_now(),
        packages: resolved.manifest.packages.clone(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct CurrentReleaseContext {
    release: String,
    channel: ReleaseChannel,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum ReleaseChannel {
    Stable,
    Dev,
    Rnd,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReleaseResolutionContext {
    release: String,
    channel: ReleaseChannel,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ReleaseIndex {
    schema: String,
    release: String,
    channel: ReleaseChannel,
    refs: BTreeMap<String, ReleaseIndexEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ReleaseIndexEntry {
    version: String,
    digest: String,
    canonical_ref: String,
}

fn write_current_release_context(ctx: &ReleaseResolutionContext) -> GtcResult<()> {
    let current = CurrentReleaseContext {
        release: ctx.release.clone(),
        channel: ctx.channel.clone(),
    };
    write_json_atomic(&current_release_context_path()?, &current)
}

fn release_context_from_resolved(
    source: &ToolchainSource,
    manifest: &ToolchainManifest,
) -> GtcResult<Option<ReleaseResolutionContext>> {
    match source {
        ToolchainSource::Release { release, channel } => Ok(Some(ReleaseResolutionContext {
            release: release.clone(),
            channel: parse_release_channel(channel).ok_or_else(|| {
                GtcError::message(format!(
                    "release channel '{channel}' is not supported; use stable, dev, or rnd"
                ))
            })?,
        })),
        ToolchainSource::Channel(_) | ToolchainSource::LocalManifest(_) => {
            let Some(channel) = manifest.channel.as_deref() else {
                return Ok(None);
            };
            Ok(Some(ReleaseResolutionContext {
                release: manifest.version.clone(),
                channel: parse_release_channel(channel).ok_or_else(|| {
                    GtcError::message(format!(
                        "release channel '{channel}' is not supported; use stable, dev, or rnd"
                    ))
                })?,
            }))
        }
    }
}

fn parse_release_channel(channel: &str) -> Option<ReleaseChannel> {
    match channel {
        "stable" => Some(ReleaseChannel::Stable),
        "dev" => Some(ReleaseChannel::Dev),
        "rnd" => Some(ReleaseChannel::Rnd),
        _ => None,
    }
}

fn release_channel_name(channel: &ReleaseChannel) -> &'static str {
    match channel {
        ReleaseChannel::Stable => "stable",
        ReleaseChannel::Dev => "dev",
        ReleaseChannel::Rnd => "rnd",
    }
}

fn prefetch_release_artifacts_and_write_index(
    manifest: &ToolchainManifest,
    ctx: &ReleaseResolutionContext,
    phases: ToolchainInstallPhases,
    force: bool,
) -> GtcResult<()> {
    let refs = release_artifact_refs(manifest, phases);
    if refs.is_empty() {
        return Ok(());
    }

    let options = DistOptions::default();
    let cache_root = options.cache_dir.clone();
    let client = DistClient::new(options);
    let existing_index = read_release_index_if_exists(&release_index_path(&cache_root, ctx)?)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| GtcError::message(format!("failed to create runtime: {err}")))?;
    let mut index_refs = BTreeMap::new();

    for release_ref in refs {
        let kind_label = release_artifact_kind_label(release_ref.kind);
        let item = release_ref.item;
        let repo = artifact_repo(&item.id)?;
        let mutable_ref = format!("{}:{}", repo, release_channel_name(&ctx.channel));
        let version_ref = format!("{repo}:{}", item.version);
        if !force
            && let Some(entry) = cached_release_index_entry(
                &client,
                existing_index.as_ref(),
                &mutable_ref,
                &item.version,
            )
        {
            println!(
                "Cached {kind_label} {}:{} already matches {}; skipping.",
                item.id, item.version, entry.digest
            );
            index_refs.insert(mutable_ref, entry);
            continue;
        }
        println!(
            "Prefetching {kind_label} {}:{} ({version_ref})",
            item.id, item.version
        );
        let resolved = if let Some(prefetch_ref) = mock_prefetch_source_ref(&version_ref)? {
            let source = client.parse_source(&prefetch_ref).map_err(|err| {
                GtcError::message(format!("failed to parse {prefetch_ref}: {err}"))
            })?;
            let descriptor = runtime
                .block_on(client.resolve(source, ResolvePolicy))
                .map_err(|err| {
                    GtcError::message(format!("failed to resolve {version_ref}: {err}"))
                })?;
            runtime
                .block_on(client.fetch(&descriptor, CachePolicy))
                .map_err(|err| GtcError::message(format!("failed to fetch {version_ref}: {err}")))?
        } else {
            runtime
                .block_on(client.prefetch_release_artifact(
                    release_ref.kind,
                    &version_ref,
                    CachePolicy,
                ))
                .map_err(|err| {
                    GtcError::message(format!("failed to prefetch {version_ref}: {err}"))
                })?
        };
        let entry = client
            .stat_cache(&resolved.descriptor.digest)
            .map_err(|err| {
                GtcError::message(format!(
                    "failed to verify cached artifact {}: {err}",
                    resolved.descriptor.digest
                ))
            })?;
        let blob_path = cache_blob_path(&cache_root, &entry.digest)?;
        let entry_path = cache_entry_path(&cache_root, &entry.digest)?;
        if !blob_path.is_file() {
            return Err(GtcError::message(format!(
                "cached artifact blob missing for {} at {}",
                entry.digest,
                blob_path.display()
            )));
        }
        if !entry_path.is_file() {
            return Err(GtcError::message(format!(
                "cached artifact entry missing for {} at {}",
                entry.digest,
                entry_path.display()
            )));
        }
        let digest = entry.digest.clone();
        println!(
            "Prefetched {kind_label} {}:{} -> {digest}",
            item.id, item.version
        );
        index_refs.insert(
            mutable_ref,
            ReleaseIndexEntry {
                version: item.version.clone(),
                digest: digest.clone(),
                canonical_ref: format!("oci://{repo}@{digest}"),
            },
        );
    }

    let index = ReleaseIndex {
        schema: "greentic.release-index.v1".to_string(),
        release: ctx.release.clone(),
        channel: ctx.channel.clone(),
        refs: index_refs,
    };
    write_json_atomic(&release_index_path(&cache_root, ctx)?, &index)
}

fn read_release_index_if_exists(path: &Path) -> GtcResult<Option<ReleaseIndex>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(GtcError::message(format!(
                "failed to read release index {}: {err}",
                path.display()
            )));
        }
    };
    serde_json::from_slice(&bytes).map(Some).map_err(|err| {
        GtcError::json(
            format!("failed to parse release index {}", path.display()),
            err,
        )
    })
}

fn cached_release_index_entry(
    client: &DistClient,
    index: Option<&ReleaseIndex>,
    mutable_ref: &str,
    version: &str,
) -> Option<ReleaseIndexEntry> {
    let entry = index?.refs.get(mutable_ref)?;
    if entry.version != version {
        return None;
    }
    if canonical_ref_digest(&entry.canonical_ref).as_deref() != Some(entry.digest.as_str()) {
        return None;
    }
    client.open_cached(&entry.digest).ok()?;
    Some(entry.clone())
}

fn canonical_ref_digest(canonical_ref: &str) -> Option<String> {
    let (_, digest) = canonical_ref.rsplit_once('@')?;
    Some(if digest.starts_with("sha256:") {
        digest.to_string()
    } else {
        format!("sha256:{digest}")
    })
}

fn release_artifact_refs(
    manifest: &ToolchainManifest,
    phases: ToolchainInstallPhases,
) -> Vec<ReleaseArtifactRef> {
    let mut refs = Vec::new();
    if phases.packs {
        refs.extend(
            manifest
                .extension_packs
                .iter()
                .flat_map(|items| items.iter())
                .cloned()
                .map(|item| ReleaseArtifactRef {
                    kind: ReleaseArtifactKind::Pack,
                    item,
                }),
        );
    }
    if phases.components {
        refs.extend(
            manifest
                .components
                .iter()
                .flat_map(|items| items.iter())
                .cloned()
                .map(|item| ReleaseArtifactRef {
                    kind: ReleaseArtifactKind::Component,
                    item,
                }),
        );
    }
    refs
}

struct ReleaseArtifactRef {
    kind: ReleaseArtifactKind,
    item: ToolchainArtifactRef,
}

fn release_artifact_kind_label(kind: ReleaseArtifactKind) -> &'static str {
    match kind {
        ReleaseArtifactKind::Pack => "pack",
        ReleaseArtifactKind::Component => "component",
    }
}

fn has_selected_release_artifacts(
    manifest: &ToolchainManifest,
    phases: ToolchainInstallPhases,
) -> bool {
    (phases.packs
        && manifest
            .extension_packs
            .as_deref()
            .is_some_and(|items| !items.is_empty()))
        || (phases.components
            && manifest
                .components
                .as_deref()
                .is_some_and(|items| !items.is_empty()))
}

fn artifact_repo(id: &str) -> GtcResult<String> {
    let trimmed = id.trim().trim_start_matches('/');
    if trimmed.is_empty()
        || trimmed.contains("..")
        || trimmed.split('/').any(|segment| segment.is_empty())
    {
        return Err(GtcError::message(format!("invalid artifact id '{id}'")));
    }
    Ok(format!("ghcr.io/greenticai/{trimmed}"))
}

fn mock_prefetch_source_ref(version_ref: &str) -> GtcResult<Option<String>> {
    let Some(root) = env::var_os("GTC_RELEASE_ARTIFACT_MOCK_ROOT") else {
        return Ok(None);
    };
    let root = PathBuf::from(root);
    let index_path = root.join("index.json");
    let raw = fs::read_to_string(&index_path).map_err(|err| {
        GtcError::message(format!("failed to read {}: {err}", index_path.display()))
    })?;
    let index: BTreeMap<String, String> = serde_json::from_str(&raw)
        .map_err(|err| GtcError::json(format!("failed to parse {}", index_path.display()), err))?;
    let rel = index.get(version_ref).ok_or_else(|| {
        GtcError::message(format!(
            "mock release artifact index missing mapping for {version_ref}"
        ))
    })?;
    Ok(Some(root.join(rel).display().to_string()))
}

fn release_index_path(cache_root: &Path, ctx: &ReleaseResolutionContext) -> GtcResult<PathBuf> {
    if ctx.release.trim().is_empty()
        || Path::new(&ctx.release).components().count() != 1
        || ctx.release.contains(std::path::MAIN_SEPARATOR)
    {
        return Err(GtcError::message(
            "release context release must be a single path segment",
        ));
    }
    Ok(cache_root
        .join("release-index")
        .join("v1")
        .join(release_channel_name(&ctx.channel))
        .join(format!("{}.json", ctx.release)))
}

fn cache_blob_path(cache_root: &Path, digest: &str) -> GtcResult<PathBuf> {
    Ok(cache_artifact_dir(cache_root, digest)?.join("blob"))
}

fn cache_entry_path(cache_root: &Path, digest: &str) -> GtcResult<PathBuf> {
    Ok(cache_artifact_dir(cache_root, digest)?.join("entry.json"))
}

fn cache_artifact_dir(cache_root: &Path, digest: &str) -> GtcResult<PathBuf> {
    let hex = digest
        .strip_prefix("sha256:")
        .ok_or_else(|| GtcError::message(format!("unsupported artifact digest '{digest}'")))?;
    if hex.len() != 64 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(GtcError::message(format!(
            "invalid artifact digest '{digest}'"
        )));
    }
    let (prefix, rest) = hex.split_at(2);
    Ok(cache_root
        .join("artifacts")
        .join("sha256")
        .join(prefix)
        .join(rest))
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> GtcResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            GtcError::message(format!("failed to create {}: {err}", parent.display()))
        })?;
    }
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|err| GtcError::json(format!("failed to encode {}", path.display()), err))?;
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    fs::write(&tmp, bytes)
        .map_err(|err| GtcError::message(format!("failed to write {}: {err}", tmp.display())))?;
    fs::rename(&tmp, path).map_err(|err| {
        let _ = fs::remove_file(&tmp);
        GtcError::message(format!(
            "failed to replace {} with {}: {err}",
            path.display(),
            tmp.display()
        ))
    })
}

fn send_ghcr_get(
    client: &Client,
    url: &str,
    accept: Option<&str>,
) -> GtcResult<reqwest::blocking::Response> {
    let mut request = client.get(url);
    if let Some(accept) = accept {
        request = request.header(ACCEPT, accept);
    }
    let response = request
        .try_clone()
        .ok_or_else(|| GtcError::message("failed to clone GHCR request"))?
        .send()
        .map_err(|err| GtcError::message(format!("failed to fetch GHCR artifact: {err}")))?;
    if response.status() != reqwest::StatusCode::UNAUTHORIZED {
        return response
            .error_for_status()
            .map_err(|err| GtcError::message(format!("failed to fetch GHCR artifact: {err}")));
    }
    let Some(challenge) = response
        .headers()
        .get(WWW_AUTHENTICATE)
        .and_then(|value| value.to_str().ok())
    else {
        return Err(GtcError::message("GHCR authentication challenge missing"));
    };
    let token = fetch_bearer_token(client, challenge)?;
    let mut authed = client
        .get(url)
        .header(AUTHORIZATION, format!("Bearer {token}"));
    if let Some(accept) = accept {
        authed = authed.header(ACCEPT, accept);
    }
    authed
        .send()
        .map_err(|err| GtcError::message(format!("failed to fetch GHCR artifact: {err}")))?
        .error_for_status()
        .map_err(|err| GtcError::message(format!("failed to fetch GHCR artifact: {err}")))
}

fn fetch_bearer_token(client: &Client, challenge: &str) -> GtcResult<String> {
    let params = parse_bearer_challenge(challenge)?;
    let realm = params
        .get("realm")
        .ok_or_else(|| GtcError::message("GHCR auth challenge missing realm"))?;
    let mut request = client.get(realm);
    let has_credentials = if let Some((username, token)) = ghcr_credentials() {
        request = request.basic_auth(username, Some(token));
        true
    } else {
        false
    };
    if let Some(service) = params.get("service") {
        request = request.query(&[("service", service)]);
    }
    if let Some(scope) = params.get("scope") {
        request = request.query(&[("scope", scope)]);
    }
    let response_text = request
        .send()
        .map_err(|err| GtcError::message(format!("failed to fetch GHCR auth token: {err}")))?
        .error_for_status()
        .map_err(|err| {
            if err.status() == Some(reqwest::StatusCode::FORBIDDEN) {
                if has_credentials {
                    GtcError::message(
                        "failed to fetch GHCR auth token: token was rejected; ensure it has read:packages access to ghcr.io/greenticai/greentic-versions/gtc",
                    )
                } else {
                    GtcError::message(
                        "failed to fetch GHCR auth token: anonymous access was denied; set GHCR_TOKEN or GITHUB_TOKEN with read:packages access",
                    )
                }
            } else {
                GtcError::message(format!("failed to fetch GHCR auth token: {err}"))
            }
        })?
        .text()
        .map_err(|err| {
            GtcError::message(format!("failed to read GHCR auth token response: {err}"))
        })?;
    let response: BearerTokenResponse = serde_json::from_str(&response_text)
        .map_err(|err| GtcError::json("failed to parse GHCR auth token response", err))?;
    response
        .token
        .or(response.access_token)
        .ok_or_else(|| GtcError::message("GHCR auth token response did not include a token"))
}

fn ghcr_credentials() -> Option<(String, String)> {
    let token = env::var("GHCR_TOKEN")
        .ok()
        .or_else(|| env::var("GITHUB_TOKEN").ok())
        .or_else(|| env::var("CR_PAT").ok())?;
    let username = env::var("GHCR_USERNAME")
        .ok()
        .or_else(|| env::var("GITHUB_ACTOR").ok())
        .or_else(|| env::var("USER").ok())
        .unwrap_or_else(|| "gtc".to_string());
    Some((username, token))
}

fn parse_bearer_challenge(
    challenge: &str,
) -> GtcResult<std::collections::BTreeMap<String, String>> {
    let raw = challenge
        .strip_prefix("Bearer ")
        .ok_or_else(|| GtcError::message("unsupported GHCR authentication challenge"))?;
    let mut params = std::collections::BTreeMap::new();
    for part in raw.split(',') {
        let Some((key, value)) = part.trim().split_once('=') else {
            continue;
        };
        params.insert(key.to_string(), value.trim_matches('"').to_string());
    }
    Ok(params)
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::from("sha256:");
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn installed_at_now() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    format!("unix:{seconds}")
}

#[derive(Debug)]
struct GhcrReference {
    repository: String,
    reference: String,
}

impl GhcrReference {
    fn parse(raw: &str) -> GtcResult<Self> {
        let without_registry = raw
            .strip_prefix("ghcr.io/")
            .ok_or_else(|| GtcError::message(format!("unsupported GHCR reference '{raw}'")))?;
        let Some((repository, reference)) = without_registry.rsplit_once(':') else {
            return Err(GtcError::message(format!(
                "GHCR reference '{raw}' is missing a tag"
            )));
        };
        Ok(Self {
            repository: repository.to_string(),
            reference: reference.to_string(),
        })
    }
}

#[derive(Debug, Deserialize)]
struct OciManifest {
    layers: Vec<OciLayer>,
}

#[derive(Debug, Deserialize)]
struct OciLayer {
    #[serde(rename = "mediaType")]
    media_type: String,
    digest: String,
}

#[derive(Debug, Deserialize)]
struct BearerTokenResponse {
    token: Option<String>,
    access_token: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn pinned_manifest() -> ToolchainManifest {
        ToolchainManifest {
            schema: TOOLCHAIN_MANIFEST_SCHEMA.to_string(),
            toolchain: "gtc".to_string(),
            version: "1.0.4".to_string(),
            channel: Some("stable".to_string()),
            created_at: None,
            packages: vec![
                ToolchainPackage {
                    crate_name: "greentic-dev".to_string(),
                    bins: vec!["greentic-dev".to_string()],
                    version: "0.5.9".to_string(),
                },
                ToolchainPackage {
                    crate_name: "greentic-runner".to_string(),
                    bins: vec![
                        "greentic-runner".to_string(),
                        "greentic-runner-cli".to_string(),
                    ],
                    version: "0.5.10".to_string(),
                },
            ],
            extension_packs: None,
            components: None,
        }
    }

    #[test]
    fn parses_pinned_toolchain_manifest() {
        let raw = r#"{
            "schema":"greentic.toolchain-manifest.v1",
            "toolchain":"gtc",
            "version":"1.0.4",
            "channel":"stable",
            "packages":[{"crate":"greentic-dev","bins":["greentic-dev"],"version":"0.5.9"}]
        }"#;
        let manifest: ToolchainManifest = serde_json::from_str(raw).expect("manifest");
        validate_toolchain_manifest(&manifest).expect("valid");
        assert_eq!(manifest.packages[0].crate_name, "greentic-dev");
    }

    #[test]
    fn parses_latest_toolchain_manifest() {
        let raw = r#"{
            "schema":"greentic.toolchain-manifest.v1",
            "toolchain":"gtc",
            "version":"dev",
            "channel":"dev",
            "packages":[{"crate":"greentic-flow","bins":["greentic-flow"],"version":"latest"}]
        }"#;
        let manifest: ToolchainManifest = serde_json::from_str(raw).expect("manifest");
        validate_toolchain_manifest(&manifest).expect("valid");
        assert!(is_latest_version(&manifest.packages[0].version));
    }

    #[test]
    fn rejects_manifest_with_wrong_schema() {
        let mut manifest = pinned_manifest();
        manifest.schema = "wrong".to_string();
        assert!(validate_toolchain_manifest(&manifest).is_err());
    }

    #[test]
    fn rejects_manifest_with_wrong_toolchain() {
        let mut manifest = pinned_manifest();
        manifest.toolchain = "other".to_string();
        assert!(validate_toolchain_manifest(&manifest).is_err());
    }

    #[test]
    fn rejects_manifest_with_duplicate_crate_bin_entries() {
        let mut manifest = pinned_manifest();
        manifest.packages.push(ToolchainPackage {
            crate_name: "greentic-dev".to_string(),
            bins: vec!["greentic-dev".to_string()],
            version: "0.5.10".to_string(),
        });
        assert!(validate_toolchain_manifest(&manifest).is_err());
    }

    #[test]
    fn toolchain_source_maps_unrestricted_channel_to_ghcr_ref() {
        assert_eq!(
            toolchain_source_ref(&ToolchainSource::Channel("demo".to_string())).as_deref(),
            Some("ghcr.io/greenticai/greentic-versions/gtc:demo")
        );
    }

    #[test]
    fn toolchain_source_maps_release_to_ghcr_ref() {
        assert_eq!(
            toolchain_source_ref(&ToolchainSource::Release {
                release: "1.0.5".to_string(),
                channel: "stable".to_string(),
            })
            .as_deref(),
            Some("ghcr.io/greenticai/greentic-versions/gtc:1.0.5")
        );
    }

    #[test]
    fn toolchain_binstall_args_for_pinned_package() {
        let package = ToolchainPackage {
            crate_name: "greentic-start".to_string(),
            bins: vec!["greentic-start".to_string()],
            version: "0.5.8".to_string(),
        };
        assert_eq!(
            toolchain_binstall_args(&package, "greentic-start"),
            vec![
                "binstall",
                "-y",
                "--locked",
                "--force",
                "greentic-start",
                "--version",
                "0.5.8",
                "--bin",
                "greentic-start"
            ]
        );
    }

    #[test]
    fn toolchain_binstall_args_for_latest_package() {
        let package = ToolchainPackage {
            crate_name: "greentic-flow".to_string(),
            bins: vec!["greentic-flow".to_string()],
            version: "latest".to_string(),
        };
        assert_eq!(
            toolchain_binstall_args(&package, "greentic-flow"),
            vec![
                "binstall",
                "-y",
                "--locked",
                "--force",
                "greentic-flow",
                "--bin",
                "greentic-flow"
            ]
        );
    }

    #[test]
    fn local_manifest_uses_file_digest() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("manifest.json");
        fs::write(
            &path,
            serde_json::to_vec(&pinned_manifest()).expect("encode manifest"),
        )
        .expect("write manifest");
        let resolved = resolve_local_manifest(&path).expect("resolve");
        assert!(
            resolved
                .digest
                .as_deref()
                .unwrap_or_default()
                .starts_with("sha256:")
        );
        assert_eq!(resolved.source_kind, "local");
    }

    #[test]
    fn parse_release_channel_recognizes_stable_dev_rnd() {
        assert!(matches!(
            parse_release_channel("stable"),
            Some(ReleaseChannel::Stable)
        ));
        assert!(matches!(
            parse_release_channel("dev"),
            Some(ReleaseChannel::Dev)
        ));
        assert!(matches!(
            parse_release_channel("rnd"),
            Some(ReleaseChannel::Rnd)
        ));
        assert!(parse_release_channel("ga").is_none());
    }

    #[test]
    fn release_channel_name_round_trips() {
        assert_eq!(release_channel_name(&ReleaseChannel::Stable), "stable");
        assert_eq!(release_channel_name(&ReleaseChannel::Dev), "dev");
        assert_eq!(release_channel_name(&ReleaseChannel::Rnd), "rnd");
    }

    #[test]
    fn artifact_repo_rejects_invalid_ids_and_accepts_valid_ones() {
        assert_eq!(
            artifact_repo("packs/sample/foo").expect("ok"),
            "ghcr.io/greenticai/packs/sample/foo"
        );
        // Leading slash is stripped but the rest must remain non-empty.
        assert_eq!(
            artifact_repo("/packs/foo").expect("ok"),
            "ghcr.io/greenticai/packs/foo"
        );
        assert!(artifact_repo("").is_err());
        assert!(artifact_repo("packs//foo").is_err());
        assert!(artifact_repo("packs/../foo").is_err());
    }

    #[test]
    fn canonical_ref_digest_normalizes_with_or_without_sha_prefix() {
        let raw_digest = "0".repeat(64);
        let canonical = format!("oci://repo@{raw_digest}");
        assert_eq!(
            canonical_ref_digest(&canonical).as_deref(),
            Some(format!("sha256:{raw_digest}").as_str())
        );
        let with_prefix = format!("oci://repo@sha256:{raw_digest}");
        assert_eq!(
            canonical_ref_digest(&with_prefix).as_deref(),
            Some(format!("sha256:{raw_digest}").as_str())
        );
        assert!(canonical_ref_digest("not-canonical").is_none());
    }

    #[test]
    fn parse_bearer_challenge_collects_quoted_params_and_rejects_unsupported() {
        let parsed = parse_bearer_challenge(
            "Bearer realm=\"https://example/token\",service=\"ghcr.io\",scope=\"repo:foo:pull\"",
        )
        .expect("parsed");
        assert_eq!(
            parsed.get("realm").map(String::as_str),
            Some("https://example/token")
        );
        assert_eq!(parsed.get("service").map(String::as_str), Some("ghcr.io"));
        assert_eq!(
            parsed.get("scope").map(String::as_str),
            Some("repo:foo:pull")
        );

        assert!(parse_bearer_challenge("Basic realm=\"example\"").is_err());
    }

    #[test]
    fn ghcr_reference_parse_returns_repository_and_tag() {
        let parsed = GhcrReference::parse("ghcr.io/greenticai/example:stable").expect("ref");
        assert_eq!(parsed.repository, "greenticai/example");
        assert_eq!(parsed.reference, "stable");

        assert!(GhcrReference::parse("docker.io/example:tag").is_err());
        assert!(GhcrReference::parse("ghcr.io/greenticai/example").is_err());
    }

    #[test]
    fn installed_toolchain_path_honors_state_dir_env() {
        let dir = tempdir().expect("tempdir");
        let old = env::var_os("GTC_TOOLCHAIN_STATE_DIR");
        unsafe {
            env::set_var("GTC_TOOLCHAIN_STATE_DIR", dir.path());
        }
        let path = installed_toolchain_path().expect("path");
        unsafe {
            match old {
                Some(value) => env::set_var("GTC_TOOLCHAIN_STATE_DIR", value),
                None => env::remove_var("GTC_TOOLCHAIN_STATE_DIR"),
            }
        }
        assert_eq!(path, dir.path().join("installed.json"));
    }

    #[test]
    fn current_release_context_path_honors_state_dir_env() {
        let dir = tempdir().expect("tempdir");
        let old = env::var_os("GTC_RELEASE_STATE_DIR");
        unsafe {
            env::set_var("GTC_RELEASE_STATE_DIR", dir.path());
        }
        let path = current_release_context_path().expect("path");
        unsafe {
            match old {
                Some(value) => env::set_var("GTC_RELEASE_STATE_DIR", value),
                None => env::remove_var("GTC_RELEASE_STATE_DIR"),
            }
        }
        assert_eq!(path, dir.path().join("current.json"));
    }

    #[test]
    fn write_then_read_installed_toolchain_roundtrips_through_state_dir_env() {
        let dir = tempdir().expect("tempdir");
        let old = env::var_os("GTC_TOOLCHAIN_STATE_DIR");
        unsafe {
            env::set_var("GTC_TOOLCHAIN_STATE_DIR", dir.path());
        }
        let resolved = ResolvedManifest {
            source: "ghcr.io/greenticai/greentic-versions/gtc:1.0.4".to_string(),
            source_kind: "channel".to_string(),
            digest: Some("sha256:deadbeef".to_string()),
            manifest: pinned_manifest(),
        };
        let state = installed_state_from_resolved(&resolved);
        write_installed_toolchain(&state).expect("write");
        let read = read_installed_toolchain().expect("read").expect("present");
        assert_eq!(read.version, state.version);
        assert_eq!(read.resolved_digest.as_deref(), Some("sha256:deadbeef"));

        let label = installed_toolchain_label();
        assert!(label.contains(&state.version));
        unsafe {
            match old {
                Some(value) => env::set_var("GTC_TOOLCHAIN_STATE_DIR", value),
                None => env::remove_var("GTC_TOOLCHAIN_STATE_DIR"),
            }
        }
    }

    #[test]
    fn installed_toolchain_label_reports_not_installed_when_state_absent() {
        let dir = tempdir().expect("tempdir");
        let old = env::var_os("GTC_TOOLCHAIN_STATE_DIR");
        unsafe {
            env::set_var("GTC_TOOLCHAIN_STATE_DIR", dir.path());
        }
        let label = installed_toolchain_label();
        unsafe {
            match old {
                Some(value) => env::set_var("GTC_TOOLCHAIN_STATE_DIR", value),
                None => env::remove_var("GTC_TOOLCHAIN_STATE_DIR"),
            }
        }
        assert_eq!(label, "not installed");
    }

    #[test]
    fn has_selected_release_artifacts_respects_phase_selection() {
        let mut manifest = pinned_manifest();
        manifest.extension_packs = Some(vec![ToolchainArtifactRef {
            id: "packs/sample".to_string(),
            version: "0.1.0".to_string(),
        }]);
        assert!(has_selected_release_artifacts(
            &manifest,
            ToolchainInstallPhases::all()
        ));
        assert!(!has_selected_release_artifacts(
            &manifest,
            ToolchainInstallPhases {
                binaries: true,
                packs: false,
                components: false,
            }
        ));
    }

    #[test]
    fn validate_toolchain_artifact_refs_rejects_duplicates_and_empty_values() {
        let dup = vec![
            ToolchainArtifactRef {
                id: "packs/a".to_string(),
                version: "0.1".to_string(),
            },
            ToolchainArtifactRef {
                id: "packs/a".to_string(),
                version: "0.2".to_string(),
            },
        ];
        assert!(validate_toolchain_artifact_refs("extension_packs", Some(dup.as_slice())).is_err());

        let empty_version = vec![ToolchainArtifactRef {
            id: "packs/a".to_string(),
            version: String::new(),
        }];
        assert!(
            validate_toolchain_artifact_refs("extension_packs", Some(empty_version.as_slice()))
                .is_err()
        );

        let empty_id = vec![ToolchainArtifactRef {
            id: String::new(),
            version: "0.1".to_string(),
        }];
        assert!(
            validate_toolchain_artifact_refs("extension_packs", Some(empty_id.as_slice())).is_err()
        );

        // None is accepted.
        validate_toolchain_artifact_refs("extension_packs", None).expect("none ok");
    }

    #[test]
    fn release_index_path_rejects_path_separators_in_release() {
        let cache_root = std::path::PathBuf::from("/tmp/cache");
        let ctx = ReleaseResolutionContext {
            release: "../escape".to_string(),
            channel: ReleaseChannel::Stable,
        };
        assert!(release_index_path(&cache_root, &ctx).is_err());

        let ctx_ok = ReleaseResolutionContext {
            release: "1.0.4".to_string(),
            channel: ReleaseChannel::Stable,
        };
        let path = release_index_path(&cache_root, &ctx_ok).expect("path");
        assert!(
            path.ends_with(
                std::path::Path::new("release-index")
                    .join("v1")
                    .join("stable")
                    .join("1.0.4.json")
            )
        );
    }

    #[test]
    fn cache_artifact_dir_validates_digest_shape() {
        let cache_root = std::path::PathBuf::from("/tmp/cache");
        let digest = format!("sha256:{}", "ab".repeat(32));
        let dir = cache_artifact_dir(&cache_root, &digest).expect("ok");
        assert!(dir.starts_with("/tmp/cache/artifacts/sha256"));

        let blob = cache_blob_path(&cache_root, &digest).expect("blob");
        assert!(blob.ends_with("blob"));
        let entry = cache_entry_path(&cache_root, &digest).expect("entry");
        assert!(entry.ends_with("entry.json"));

        assert!(cache_artifact_dir(&cache_root, "deadbeef").is_err());
        assert!(cache_artifact_dir(&cache_root, "sha256:bad").is_err());
        let bad_hex = format!("sha256:{}", "zz".repeat(32));
        assert!(cache_artifact_dir(&cache_root, &bad_hex).is_err());
    }

    #[test]
    fn version_output_matches_handles_punctuation_and_v_prefix() {
        assert!(super::version_output_matches("greentic 0.1.2", "0.1.2"));
        assert!(super::version_output_matches("greentic v0.1.2", "0.1.2"));
        assert!(super::version_output_matches(
            "greentic-dev (0.1.2)",
            "0.1.2"
        ));
        assert!(!super::version_output_matches("greentic 0.1.3", "0.1.2"));
    }

    #[test]
    fn parse_cargo_search_version_extracts_first_match_and_skips_other_packages() {
        let raw = "greentic-flow = \"0.1.0\"  # description\nother-pkg = \"9.9.9\"";
        assert_eq!(
            parse_cargo_search_version(raw, "greentic-flow").as_deref(),
            Some("0.1.0")
        );
        assert!(parse_cargo_search_version("blank-output", "greentic-flow").is_none());
    }

    #[test]
    fn mock_prefetch_source_ref_returns_path_when_env_points_at_index() {
        let dir = tempdir().expect("tempdir");
        let index_path = dir.path().join("index.json");
        let target = dir.path().join("artifacts").join("file.bin");
        fs::create_dir_all(target.parent().expect("parent")).expect("mkdir");
        fs::write(&target, b"fixture").expect("write target");
        fs::write(
            &index_path,
            r#"{"ghcr.io/example:1.0.0":"artifacts/file.bin","misc":"value"}"#,
        )
        .expect("write index");

        let old = env::var_os("GTC_RELEASE_ARTIFACT_MOCK_ROOT");
        unsafe {
            env::set_var("GTC_RELEASE_ARTIFACT_MOCK_ROOT", dir.path());
        }
        let resolved = mock_prefetch_source_ref("ghcr.io/example:1.0.0")
            .expect("ok")
            .expect("some");
        let missing = mock_prefetch_source_ref("ghcr.io/example:9.9.9");
        unsafe {
            match old {
                Some(value) => env::set_var("GTC_RELEASE_ARTIFACT_MOCK_ROOT", value),
                None => env::remove_var("GTC_RELEASE_ARTIFACT_MOCK_ROOT"),
            }
        }
        assert_eq!(PathBuf::from(&resolved), target);
        assert!(missing.is_err());
    }

    #[test]
    fn mock_prefetch_source_ref_returns_none_without_env() {
        let old = env::var_os("GTC_RELEASE_ARTIFACT_MOCK_ROOT");
        unsafe {
            env::remove_var("GTC_RELEASE_ARTIFACT_MOCK_ROOT");
        }
        let resolved = mock_prefetch_source_ref("ghcr.io/example:1.0.0").expect("ok");
        unsafe {
            if let Some(value) = old {
                env::set_var("GTC_RELEASE_ARTIFACT_MOCK_ROOT", value);
            }
        }
        assert!(resolved.is_none());
    }

    #[test]
    fn write_json_atomic_creates_parent_directories_and_replaces_file() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("nested").join("dir").join("file.json");
        let value = pinned_manifest();
        write_json_atomic(&path, &value).expect("write");
        let raw = fs::read_to_string(&path).expect("read");
        let decoded: ToolchainManifest = serde_json::from_str(&raw).expect("decode");
        assert_eq!(decoded, value);
    }

    #[test]
    fn install_options_from_matches_supports_explicit_release_and_local_manifest() {
        let cmd = || {
            clap::Command::new("install")
                .arg(clap::Arg::new("manifest").long("manifest").num_args(1))
                .arg(clap::Arg::new("release").long("release").num_args(1))
                .arg(clap::Arg::new("channel").long("channel").num_args(1))
                .arg(
                    clap::Arg::new("force")
                        .long("force")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    clap::Arg::new("dry-run")
                        .long("dry-run")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    clap::Arg::new("install-binaries-only")
                        .long("install-binaries-only")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    clap::Arg::new("install-packs-only")
                        .long("install-packs-only")
                        .action(clap::ArgAction::SetTrue),
                )
                .arg(
                    clap::Arg::new("install-components-only")
                        .long("install-components-only")
                        .action(clap::ArgAction::SetTrue),
                )
        };

        let release_matches = cmd()
            .try_get_matches_from([
                "install",
                "--release",
                "1.0.4",
                "--channel",
                "stable",
                "--dry-run",
            ])
            .expect("matches");
        let release_options =
            ToolchainInstallOptions::from_matches(&release_matches, "stable").expect("options");
        assert!(matches!(
            release_options.source,
            ToolchainSource::Release { ref release, ref channel } if release == "1.0.4" && channel == "stable"
        ));
        assert!(release_options.dry_run);
        assert!(release_options.phases.binaries);

        let dir = tempdir().expect("tempdir");
        let manifest_path = dir.path().join("manifest.json");
        fs::write(&manifest_path, "{}").expect("manifest");
        let manifest_matches = cmd()
            .try_get_matches_from([
                "install",
                "--manifest",
                manifest_path.to_str().expect("utf8"),
                "--install-packs-only",
            ])
            .expect("matches");
        let manifest_options =
            ToolchainInstallOptions::from_matches(&manifest_matches, "stable").expect("options");
        assert!(matches!(
            manifest_options.source,
            ToolchainSource::LocalManifest(ref path) if path == &manifest_path
        ));
        assert!(!manifest_options.phases.binaries);
        assert!(manifest_options.phases.packs);
        assert!(!manifest_options.phases.components);

        let channel_matches = cmd().try_get_matches_from(["install"]).expect("matches");
        let channel_options =
            ToolchainInstallOptions::from_matches(&channel_matches, "dev").expect("options");
        assert!(matches!(
            channel_options.source,
            ToolchainSource::Channel(ref channel) if channel == "dev"
        ));
    }

    #[test]
    fn toolchain_install_phases_any_artifacts_reflects_selection() {
        let none = ToolchainInstallPhases {
            binaries: true,
            packs: false,
            components: false,
        };
        assert!(!none.any_artifacts());
        let packs = ToolchainInstallPhases {
            binaries: false,
            packs: true,
            components: false,
        };
        assert!(packs.any_artifacts());
        let components = ToolchainInstallPhases {
            binaries: false,
            packs: false,
            components: true,
        };
        assert!(components.any_artifacts());
    }

    #[test]
    fn release_context_from_resolved_returns_none_for_channel_without_manifest_channel() {
        let mut manifest = pinned_manifest();
        manifest.channel = None;
        let source = ToolchainSource::Channel("stable".to_string());
        let result = release_context_from_resolved(&source, &manifest).expect("ok");
        assert!(result.is_none());
    }

    #[test]
    fn release_context_from_resolved_uses_manifest_channel_for_local() {
        let mut manifest = pinned_manifest();
        manifest.channel = Some("dev".to_string());
        let source = ToolchainSource::LocalManifest(std::path::PathBuf::from("/tmp/m.json"));
        let ctx = release_context_from_resolved(&source, &manifest)
            .expect("ok")
            .expect("some");
        assert_eq!(ctx.release, manifest.version);
        assert!(matches!(ctx.channel, ReleaseChannel::Dev));
    }

    #[test]
    fn release_context_from_resolved_rejects_unsupported_channel_label() {
        let mut manifest = pinned_manifest();
        manifest.channel = Some("ga".to_string());
        let source = ToolchainSource::Release {
            release: "1.0.4".to_string(),
            channel: "ga".to_string(),
        };
        assert!(release_context_from_resolved(&source, &manifest).is_err());
    }
}
