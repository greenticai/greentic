use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use clap::ArgMatches;
use directories::BaseDirs;
use gtc::error::{GtcError, GtcResult};
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, AUTHORIZATION, WWW_AUTHENTICATE};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::i18n_support::{t, tf};
use super::install::{run_cargo, run_cargo_capture};

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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ToolchainPackage {
    #[serde(rename = "crate")]
    pub crate_name: String,
    pub bins: Vec<String>,
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
pub(crate) struct ToolchainInstallOptions {
    pub source: ToolchainSource,
    pub force: bool,
    pub dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ToolchainSource {
    Channel(String),
    Release(String),
    LocalManifest(PathBuf),
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
            ToolchainSource::Release(release.to_string())
        } else {
            let channel = matches
                .get_one::<String>("channel")
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .unwrap_or(default_channel);
            ToolchainSource::Channel(channel.to_string())
        };

        Ok(Self {
            source,
            force: matches.get_flag("force"),
            dry_run: matches.get_flag("dry-run"),
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
        && let Ok(Some(installed)) = read_installed_toolchain()
        && installed.resolved_digest == resolved.digest
        && resolved.digest.is_some()
    {
        println!("{}", t(locale, "gtc.install.toolchain.up_to_date"));
        return 0;
    }

    if options.dry_run {
        println!("{}", t(locale, "gtc.install.toolchain.dry_run"));
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
        return 0;
    }

    let install_status = install_toolchain_manifest(&resolved, debug, locale);
    if install_status != 0 {
        return install_status;
    }

    let state = installed_state_from_resolved(&resolved);
    if let Err(err) = write_installed_toolchain(&state) {
        eprintln!(
            "{}: {err}",
            t(locale, "gtc.install.toolchain.state_write_failed")
        );
        return 1;
    }

    0
}

pub(crate) fn install_toolchain_manifest(
    resolved: &ResolvedManifest,
    debug: bool,
    locale: &str,
) -> i32 {
    for package in &resolved.manifest.packages {
        let status = install_toolchain_package(package, debug, locale);
        if status != 0 {
            return status;
        }
    }
    0
}

pub(crate) fn install_toolchain_package(
    package: &ToolchainPackage,
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
            ToolchainSource::Release(_) => "release".to_string(),
            ToolchainSource::LocalManifest(_) => "local".to_string(),
        };
        if let Some(reference) = toolchain_source_ref(source) {
            resolved.source = reference;
        }
        return Ok(resolved);
    }

    match source {
        ToolchainSource::LocalManifest(path) => resolve_local_manifest(path),
        ToolchainSource::Channel(_) | ToolchainSource::Release(_) => {
            let reference = toolchain_source_ref(source)
                .ok_or_else(|| GtcError::message("local manifests do not have GHCR refs"))?;
            let mut resolved = resolve_ghcr_manifest(&reference, debug, locale)?;
            resolved.source_kind = match source {
                ToolchainSource::Channel(_) => "channel".to_string(),
                ToolchainSource::Release(_) => "release".to_string(),
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
        ToolchainSource::Release(release) => Some(format!("{DEFAULT_GHCR_PREFIX}:{release}")),
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
    use crate::tests::env_test_lock;
    use clap::{Arg, ArgAction, Command};
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
            toolchain_source_ref(&ToolchainSource::Release("1.0.5".to_string())).as_deref(),
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
    fn toolchain_install_options_default_to_stable_channel() {
        let matches = Command::new("toolchain")
            .arg(Arg::new("manifest").long("manifest"))
            .arg(Arg::new("release").long("release"))
            .arg(Arg::new("channel").long("channel"))
            .arg(Arg::new("force").long("force").action(ArgAction::SetTrue))
            .arg(
                Arg::new("dry-run")
                    .long("dry-run")
                    .action(ArgAction::SetTrue),
            )
            .get_matches_from(["toolchain"]);

        let options = ToolchainInstallOptions::from_matches(&matches, "stable").expect("options");
        assert_eq!(
            options.source,
            ToolchainSource::Channel("stable".to_string())
        );
        assert!(!options.force);
        assert!(!options.dry_run);
    }

    #[test]
    fn toolchain_install_options_prefer_manifest_over_release_and_channel() {
        let matches = Command::new("toolchain")
            .arg(Arg::new("manifest").long("manifest"))
            .arg(Arg::new("release").long("release"))
            .arg(Arg::new("channel").long("channel"))
            .arg(Arg::new("force").long("force").action(ArgAction::SetTrue))
            .arg(
                Arg::new("dry-run")
                    .long("dry-run")
                    .action(ArgAction::SetTrue),
            )
            .get_matches_from([
                "toolchain",
                "--manifest",
                "/tmp/manifest.json",
                "--release",
                "1.2.3",
                "--channel",
                "dev",
                "--force",
                "--dry-run",
            ]);

        let options = ToolchainInstallOptions::from_matches(&matches, "stable").expect("options");
        assert_eq!(
            options.source,
            ToolchainSource::LocalManifest(PathBuf::from("/tmp/manifest.json"))
        );
        assert!(options.force);
        assert!(options.dry_run);
    }

    #[test]
    fn parse_cargo_search_version_requires_matching_crate_and_quoted_version() {
        assert_eq!(
            parse_cargo_search_version(
                "other = \"1.0.0\"\ngreentic-dev = \"0.6.0\" # demo",
                "greentic-dev"
            )
            .as_deref(),
            Some("0.6.0")
        );
        assert_eq!(
            parse_cargo_search_version("greentic-dev = latest", "greentic-dev"),
            None
        );
    }

    #[test]
    fn validate_toolchain_manifest_rejects_missing_bins_and_empty_bin_names() {
        let mut missing_bins = pinned_manifest();
        missing_bins.packages[0].bins.clear();
        assert!(validate_toolchain_manifest(&missing_bins).is_err());

        let mut empty_bin = pinned_manifest();
        empty_bin.packages[0].bins = vec!["".to_string()];
        assert!(validate_toolchain_manifest(&empty_bin).is_err());
    }

    #[test]
    fn resolve_toolchain_manifest_uses_env_override_for_channel_sources() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("manifest.json");
        fs::write(
            &path,
            serde_json::to_vec(&pinned_manifest()).expect("encode manifest"),
        )
        .expect("write manifest");

        unsafe {
            env::set_var("GTC_TOOLCHAIN_MANIFEST_PATH", &path);
        }
        let resolved = resolve_toolchain_manifest(
            &ToolchainSource::Channel("stable".to_string()),
            false,
            "en",
        )
        .expect("resolve");
        unsafe {
            env::remove_var("GTC_TOOLCHAIN_MANIFEST_PATH");
        }

        assert_eq!(resolved.source_kind, "channel");
        assert_eq!(
            resolved.source,
            "ghcr.io/greenticai/greentic-versions/gtc:stable"
        );
        assert!(
            resolved
                .digest
                .as_deref()
                .unwrap_or("")
                .starts_with("sha256:")
        );
    }

    #[test]
    fn installed_toolchain_round_trip_uses_override_state_dir() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempdir().expect("tempdir");
        unsafe {
            env::set_var("GTC_TOOLCHAIN_STATE_DIR", dir.path());
        }
        let state = installed_state_from_resolved(&ResolvedManifest {
            source: "demo".to_string(),
            source_kind: "local".to_string(),
            digest: Some("sha256:abc".to_string()),
            manifest: pinned_manifest(),
        });

        write_installed_toolchain(&state).expect("write state");
        let loaded = read_installed_toolchain()
            .expect("read state")
            .expect("installed state");
        unsafe {
            env::remove_var("GTC_TOOLCHAIN_STATE_DIR");
        }

        assert_eq!(loaded.schema, INSTALLED_TOOLCHAIN_SCHEMA);
        assert_eq!(loaded.source, "demo");
        assert_eq!(loaded.resolved_digest.as_deref(), Some("sha256:abc"));
    }

    #[test]
    fn parse_bearer_challenge_extracts_realm_service_and_scope() {
        let parsed = parse_bearer_challenge(
            "Bearer realm=\"https://ghcr.io/token\",service=\"ghcr.io\",scope=\"repo:demo:pull\"",
        )
        .expect("challenge");
        assert_eq!(
            parsed.get("realm").map(String::as_str),
            Some("https://ghcr.io/token")
        );
        assert_eq!(parsed.get("service").map(String::as_str), Some("ghcr.io"));
        assert_eq!(
            parsed.get("scope").map(String::as_str),
            Some("repo:demo:pull")
        );
    }

    #[test]
    fn ghcr_reference_parse_rejects_missing_tag() {
        assert!(GhcrReference::parse("ghcr.io/greenticai/greentic-versions/gtc").is_err());
    }
}
