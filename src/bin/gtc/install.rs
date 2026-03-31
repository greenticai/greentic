use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};

use clap::ArgMatches;
use directories::BaseDirs;
use greentic_distributor_client::{DistClient, DistOptions, save_login_default};
use gtc::config::GtcConfig;
use gtc::error::{GtcError, GtcResult};
use reqwest::blocking::Client;
use serde::Deserialize;

use super::archive::{
    extract_squashfs_file, extract_tar_bytes, extract_targz_bytes, extract_zip_bytes,
    looks_like_gzip, looks_like_squashfs, looks_like_zip, safe_join, set_executable_if_unix,
};
use super::i18n_support::{t, tf};
use super::process::{passthrough, resolve_cargo_bin_dir};
use super::{
    BUNDLE_BIN, DEPLOYER_BIN, DEV_BIN, EMBEDDED_TERRAFORM_GTPACK, OP_BIN, SETUP_BIN, sha256_file,
};

pub(super) fn run_install(sub_matches: &ArgMatches, debug: bool, locale: &str) -> i32 {
    println!("{}", t(locale, "gtc.install.public_mode"));

    let preflight_status = ensure_install_prereqs(debug, locale);
    if preflight_status != 0 {
        return preflight_status;
    }

    let public_args = vec!["install".to_string(), "tools".to_string()];
    let public_status = passthrough(DEV_BIN, &public_args, debug, locale);
    if public_status != 0 {
        return public_status;
    }

    if let Err(err) = ensure_deployer_dist_pack(debug) {
        eprintln!(
            "{}: {err}",
            tf(
                locale,
                "gtc.install.item_fail",
                &[("kind", "asset"), ("name", "terraform.gtpack")]
            )
        );
        return 1;
    }

    let tenant = sub_matches
        .get_one::<String>("tenant")
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());

    let Some(tenant) = tenant else {
        return 0;
    };

    println!(
        "{}",
        tf(
            locale,
            "gtc.install.tenant_mode",
            &[("tenant", tenant.as_str())]
        )
    );

    let cli_key = sub_matches.get_one::<String>("key").cloned();
    let key = match resolve_tenant_key(cli_key, &tenant, locale) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };

    let manifest_url = match resolve_tenant_manifest_url(&tenant, &key, locale) {
        Ok(url) => url,
        Err(err) => {
            eprintln!("{}: {err}", t(locale, "gtc.err.pull_failed"));
            return 1;
        }
    };

    let manifest_bytes = match fetch_download_bytes_with_auth(&manifest_url, &key, locale) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!(
                "{}: {err}",
                tf(
                    locale,
                    "gtc.err.pull_failed",
                    &[("oci", manifest_url.as_str())]
                )
            );
            return 1;
        }
    };

    let manifest: TenantInstallManifest = match serde_json::from_slice(&manifest_bytes) {
        Ok(manifest) => manifest,
        Err(err) => {
            eprintln!("{}: {err}", t(locale, "gtc.err.invalid_manifest"));
            return 1;
        }
    };

    if manifest.schema_version != "1" {
        eprintln!(
            "{}: unsupported schema_version '{}'",
            t(locale, "gtc.err.invalid_manifest"),
            manifest.schema_version
        );
        return 1;
    }

    if manifest.tenant != tenant {
        eprintln!(
            "{}: tenant '{}' does not match requested tenant '{}'",
            t(locale, "gtc.err.invalid_manifest"),
            manifest.tenant,
            tenant
        );
        return 1;
    }

    let cargo_bin_dir = match resolve_cargo_bin_dir() {
        Ok(path) => path,
        Err(err) => {
            eprintln!("{}: {err}", t(locale, "gtc.err.install_dir"));
            return 1;
        }
    };

    let artifacts_root = match resolve_artifacts_root() {
        Ok(path) => path,
        Err(err) => {
            eprintln!("{}: {err}", t(locale, "gtc.err.install_dir"));
            return 1;
        }
    };

    let mut any_failed = false;
    let current_os = match current_install_os() {
        Ok(value) => value,
        Err(code) => return code,
    };
    let current_arch = match current_install_arch() {
        Ok(value) => value,
        Err(code) => return code,
    };

    for tool in manifest.tools {
        let result = install_tenant_tool_reference(
            &tool,
            &tenant,
            &key,
            &current_os,
            &current_arch,
            &cargo_bin_dir,
            locale,
        );
        match result {
            Ok(()) => {
                println!(
                    "{}",
                    tf(
                        locale,
                        "gtc.install.item_ok",
                        &[("kind", "tool"), ("name", tool.id.as_str())]
                    )
                );
            }
            Err(err) => {
                any_failed = true;
                eprintln!(
                    "{}: {err}",
                    tf(
                        locale,
                        "gtc.install.item_fail",
                        &[("kind", "tool"), ("name", tool.id.as_str())]
                    )
                );
            }
        }
    }

    for doc in manifest.docs {
        let result = install_tenant_doc_reference(&doc, &tenant, &key, &artifacts_root, locale);
        match result {
            Ok(paths) => {
                println!(
                    "{}",
                    tf(
                        locale,
                        "gtc.install.item_ok",
                        &[("kind", "doc"), ("name", doc.id.as_str())]
                    )
                );
                for path in paths {
                    println!("  -> {}", path.display());
                }
            }
            Err(err) => {
                any_failed = true;
                eprintln!(
                    "{}: {err}",
                    tf(
                        locale,
                        "gtc.install.item_fail",
                        &[("kind", "doc"), ("name", doc.id.as_str())]
                    )
                );
            }
        }
    }

    for asset in manifest.store_assets {
        let result = install_store_asset_reference(&asset, &tenant, &key, &artifacts_root, locale);
        match result {
            Ok(paths) => {
                println!(
                    "{}",
                    tf(
                        locale,
                        "gtc.install.item_ok",
                        &[("kind", "store asset"), ("name", asset.id.as_str())]
                    )
                );
                for path in paths {
                    println!("  -> {}", path.display());
                }
            }
            Err(err) => {
                any_failed = true;
                eprintln!(
                    "{}: {err}",
                    tf(
                        locale,
                        "gtc.install.item_fail",
                        &[("kind", "store asset"), ("name", asset.id.as_str())]
                    )
                );
            }
        }
    }

    if any_failed {
        eprintln!("{}", t(locale, "gtc.install.summary_failed"));
        1
    } else {
        println!("{}", t(locale, "gtc.install.summary_ok"));
        0
    }
}

pub(super) fn run_update(debug: bool, locale: &str) -> i32 {
    println!("{}", t(locale, "gtc.update.start"));

    if !is_binstall_available(debug, locale) {
        eprintln!("{}", t(locale, "gtc.update.binstall_missing"));
        return 1;
    }

    let mut any_failed = false;

    for package in [DEV_BIN, OP_BIN, BUNDLE_BIN, SETUP_BIN, DEPLOYER_BIN] {
        println!(
            "{}",
            tf(locale, "gtc.update.updating", &[("package", package)])
        );

        let binstall_args = vec![
            "binstall".to_string(),
            "-y".to_string(),
            "--force".to_string(),
            "--version".to_string(),
            "0.4".to_string(),
            package.to_string(),
        ];
        let status = run_cargo(&binstall_args, debug, locale);
        if status != 0 {
            any_failed = true;
            eprintln!(
                "{}",
                tf(locale, "gtc.update.item_fail", &[("package", package)])
            );
        } else {
            println!(
                "{}",
                tf(locale, "gtc.update.item_ok", &[("package", package)])
            );
        }
    }

    let tools_args = vec!["install".to_string(), "tools".to_string()];
    let tools_status = passthrough(DEV_BIN, &tools_args, debug, locale);
    if tools_status != 0 {
        any_failed = true;
    }

    if any_failed {
        eprintln!("{}", t(locale, "gtc.update.summary_failed"));
        1
    } else {
        println!("{}", t(locale, "gtc.update.summary_ok"));
        0
    }
}

fn ensure_install_prereqs(debug: bool, locale: &str) -> i32 {
    let installed_binstall = detect_binstall_version(debug, locale);
    let latest_binstall = latest_binstall_version(debug, locale);

    let needs_binstall_install = match (installed_binstall.as_deref(), latest_binstall.as_deref()) {
        (Some(installed), Some(latest)) => semver_compare(installed, latest).is_lt(),
        (None, _) => true,
        (Some(_), None) => true,
    };

    if needs_binstall_install {
        let install_binstall_args = vec![
            "install".to_string(),
            "cargo-binstall".to_string(),
            "--locked".to_string(),
        ];
        let status = run_cargo(&install_binstall_args, debug, locale);
        if status != 0 {
            return status;
        }
    }

    for package in [DEV_BIN, OP_BIN, BUNDLE_BIN, SETUP_BIN, DEPLOYER_BIN] {
        let binstall_args = vec![
            "binstall".to_string(),
            "-y".to_string(),
            "--version".to_string(),
            "0.4".to_string(),
            package.to_string(),
        ];
        let status = run_cargo(&binstall_args, debug, locale);
        if status != 0 {
            return status;
        }
    }

    0
}

fn ensure_deployer_dist_pack(debug: bool) -> GtcResult<()> {
    let cargo_bin_dir = resolve_cargo_bin_dir()?;
    let dist_dir = cargo_bin_dir.join("dist");
    let target = dist_dir.join("terraform.gtpack");
    fs::create_dir_all(&dist_dir)
        .map_err(|e| GtcError::io(format!("failed to create {}", dist_dir.display()), e))?;

    let needs_write = match fs::read(&target) {
        Ok(existing) => existing != EMBEDDED_TERRAFORM_GTPACK,
        Err(_) => true,
    };
    if !needs_write {
        return Ok(());
    }

    fs::write(&target, EMBEDDED_TERRAFORM_GTPACK)
        .map_err(|e| GtcError::io(format!("failed to write {}", target.display()), e))?;

    if debug {
        eprintln!("updated deployer pack at {}", target.display());
    }

    Ok(())
}

fn is_binstall_available(debug: bool, locale: &str) -> bool {
    if let Some(output) = run_cargo_capture(&["binstall", "-V"], debug, locale)
        && output.status.success()
    {
        return true;
    }
    detect_binstall_version(debug, locale).is_some()
}

fn detect_binstall_version(debug: bool, locale: &str) -> Option<String> {
    let output = run_cargo_capture(&["binstall", "--version"], debug, locale)?;
    if !output.status.success() {
        return None;
    }
    parse_first_semver(&String::from_utf8_lossy(&output.stdout))
        .or_else(|| parse_first_semver(&String::from_utf8_lossy(&output.stderr)))
}

fn latest_binstall_version(debug: bool, locale: &str) -> Option<String> {
    let output = run_cargo_capture(&["search", "cargo-binstall", "--limit", "1"], debug, locale)?;
    if !output.status.success() {
        return None;
    }
    parse_first_semver(&String::from_utf8_lossy(&output.stdout))
}

fn parse_first_semver(text: &str) -> Option<String> {
    for token in text.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '.' || ch == '-')) {
        if token.chars().all(|ch| ch.is_ascii_digit() || ch == '.')
            && token.split('.').count() >= 2
            && token
                .split('.')
                .all(|part| !part.is_empty() && part.chars().all(|ch| ch.is_ascii_digit()))
        {
            return Some(token.to_string());
        }
    }
    None
}

fn semver_compare(a: &str, b: &str) -> std::cmp::Ordering {
    let pa = parse_numeric_version(a);
    let pb = parse_numeric_version(b);
    let max = pa.len().max(pb.len());
    for i in 0..max {
        let av = *pa.get(i).unwrap_or(&0);
        let bv = *pb.get(i).unwrap_or(&0);
        match av.cmp(&bv) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

fn parse_numeric_version(raw: &str) -> Vec<u64> {
    raw.split('.')
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect()
}

fn run_cargo_capture(args: &[&str], debug: bool, locale: &str) -> Option<std::process::Output> {
    if debug {
        eprintln!("{} cargo {:?}", t(locale, "gtc.debug.exec"), args);
    }
    ProcessCommand::new("cargo")
        .args(args)
        .env("GREENTIC_LOCALE", locale)
        .output()
        .ok()
}

fn run_cargo(args: &[String], debug: bool, locale: &str) -> i32 {
    if debug {
        eprintln!("{} cargo {:?}", t(locale, "gtc.debug.exec"), args);
    }

    match ProcessCommand::new("cargo")
        .args(args)
        .env("GREENTIC_LOCALE", locale)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
    {
        Ok(status) => status.code().unwrap_or(1),
        Err(err) => {
            eprintln!("{}: {err}", t(locale, "gtc.err.exec_failed"));
            1
        }
    }
}

fn install_tenant_tool_reference(
    tool_ref: &TenantManifestReference,
    _tenant: &str,
    key: &str,
    current_os: &str,
    current_arch: &str,
    cargo_bin_dir: &Path,
    locale: &str,
) -> GtcResult<()> {
    let tool: ToolManifest = fetch_json_with_auth(&tool_ref.url, key, locale)?;
    let target = tool
        .install
        .targets
        .iter()
        .find(|target| target.os == current_os && target.arch == current_arch)
        .ok_or_else(|| {
            GtcError::message(format!(
                "no install target for tool '{}' on {current_os}/{current_arch}",
                tool.id
            ))
        })?;

    let temp = tempfile::tempdir().map_err(|e| GtcError::message(e.to_string()))?;
    let staged = temp.path().join("staged");
    fs::create_dir_all(&staged)
        .map_err(|e| GtcError::io(format!("failed to create {}", staged.display()), e))?;
    let staged_artifact = download_url_into_dir(
        &target.url,
        key,
        &staged,
        Some(&tool.install.binary_name),
        locale,
    )?;
    verify_sha256_digest(&staged_artifact, &target.sha256)?;
    install_tool_artifact(&staged, cargo_bin_dir, &tool.install.binary_name)
}

pub(super) fn normalize_expected_sha256(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.starts_with("sha256:") {
        trimmed.to_string()
    } else {
        format!("sha256:{trimmed}")
    }
}

pub(super) fn verify_sha256_digest(path: &Path, expected: &str) -> GtcResult<()> {
    let actual = sha256_file(path).map_err(GtcError::message)?;
    let expected = normalize_expected_sha256(expected);
    if actual == expected {
        return Ok(());
    }
    Err(GtcError::invalid_data(
        format!("integrity check for {}", path.display()),
        format!("expected {expected}, got {actual}"),
    ))
}

fn install_tenant_doc_reference(
    doc_ref: &TenantManifestReference,
    _tenant: &str,
    key: &str,
    artifacts_root: &Path,
    locale: &str,
) -> GtcResult<Vec<PathBuf>> {
    let manifest: DocManifest = fetch_json_with_auth(&doc_ref.url, key, locale)?;
    let mut installed = Vec::new();
    for doc in manifest.entries()? {
        if doc.download_file_name.contains('/')
            || doc.download_file_name.contains('\\')
            || doc.download_file_name.is_empty()
        {
            return Err(GtcError::message(format!(
                "invalid doc file name '{}'",
                doc.download_file_name
            )));
        }

        let docs_root = artifacts_root.join("docs");
        fs::create_dir_all(&docs_root)
            .map_err(|e| GtcError::io(format!("failed to create {}", docs_root.display()), e))?;
        let target = safe_join(&docs_root, Path::new(&doc.default_relative_path))
            .map_err(|e| GtcError::message(e.to_string()))?
            .join(&doc.download_file_name);
        download_url_to_path(&doc.source.url, key, &target, locale)?;
        installed.push(target);
    }
    Ok(installed)
}

fn install_store_asset_reference(
    asset_ref: &TenantManifestReference,
    tenant: &str,
    key: &str,
    artifacts_root: &Path,
    locale: &str,
) -> GtcResult<Vec<PathBuf>> {
    let manifest: StoreAssetManifest = fetch_json_with_auth(&asset_ref.url, key, locale)?;
    let mut installed = Vec::new();

    for item in manifest.items {
        let resolved = rewrite_store_tenant_placeholder(&item, tenant);
        installed.push(install_store_asset_item(
            &resolved,
            tenant,
            key,
            artifacts_root,
            locale,
        )?);
    }
    Ok(installed)
}

fn install_store_asset_item(
    store_url: &str,
    tenant: &str,
    key: &str,
    artifacts_root: &Path,
    locale: &str,
) -> GtcResult<PathBuf> {
    save_store_login(tenant, key)?;
    let client = DistClient::new(DistOptions::default());
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| GtcError::message(format!("failed to build tokio runtime: {e}")))?;
    let artifact = rt
        .block_on(client.download_store_artifact(store_url))
        .map_err(|e| GtcError::message(format!("{}: {e}", t(locale, "gtc.err.pull_failed"))))?;
    let file_name = store_asset_file_name(store_url)
        .ok_or_else(|| GtcError::message(format!("unable to derive filename from {store_url}")))?;
    let target = store_asset_target_path(artifacts_root, &file_name)?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| GtcError::io(format!("failed to create {}", parent.display()), e))?;
    }
    fs::write(&target, artifact.bytes)
        .map_err(|e| GtcError::io(format!("failed to write {}", target.display()), e))?;
    Ok(target)
}

fn download_url_into_dir(
    url: &str,
    key: &str,
    target_dir: &Path,
    fallback_name: Option<&str>,
    locale: &str,
) -> GtcResult<PathBuf> {
    let file_name = url_file_name(url)
        .filter(|value| !value.is_empty())
        .or_else(|| fallback_name.map(|value| value.to_string()))
        .ok_or_else(|| {
            GtcError::invalid_data(
                "download URL",
                format!("unable to derive file name from {url}"),
            )
        })?;
    let target = target_dir.join(file_name);
    download_url_to_path(url, key, &target, locale)?;
    Ok(target)
}

fn download_url_to_path(url: &str, key: &str, target: &Path, locale: &str) -> GtcResult<()> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| GtcError::io(format!("failed to create {}", parent.display()), e))?;
    }
    let bytes = fetch_download_bytes_with_auth(url, key, locale)?;
    let mut file = fs::File::create(target)
        .map_err(|e| GtcError::io(format!("failed to create {}", target.display()), e))?;
    file.write_all(&bytes)
        .map_err(|e| GtcError::io(format!("failed to write {}", target.display()), e))?;
    Ok(())
}

fn fetch_json_with_auth<T: serde::de::DeserializeOwned>(
    url: &str,
    key: &str,
    locale: &str,
) -> GtcResult<T> {
    let bytes = fetch_json_bytes_with_auth(url, key, locale)?;
    serde_json::from_slice(&bytes)
        .map_err(|e| GtcError::json(format!("failed to parse JSON from {url}"), e))
}

fn fetch_json_bytes_with_auth(url: &str, key: &str, locale: &str) -> GtcResult<Vec<u8>> {
    if let Some(path) = file_url_path(url) {
        return fs::read(&path)
            .map_err(|e| GtcError::io(format!("failed to read {}", path.display()), e));
    }

    match url.split_once("://").map(|(scheme, _)| scheme) {
        Some("http") | Some("https") => {
            if let Some(asset_url) = resolve_github_release_asset_api_url(url, key, locale)? {
                fetch_asset_bytes(&asset_url, key, locale)
            } else {
                fetch_https_json_or_file_bytes(url, key, locale)
            }
        }
        _ => Err(GtcError::invalid_data(
            "download URL",
            format!("unsupported scheme for {url}"),
        )),
    }
}

fn fetch_download_bytes_with_auth(url: &str, key: &str, locale: &str) -> GtcResult<Vec<u8>> {
    if let Some(path) = file_url_path(url) {
        return fs::read(&path)
            .map_err(|e| GtcError::io(format!("failed to read {}", path.display()), e));
    }

    match url.split_once("://").map(|(scheme, _)| scheme) {
        Some("http") | Some("https") => {
            if let Some(asset_url) = resolve_github_release_asset_api_url(url, key, locale)? {
                fetch_asset_bytes(&asset_url, key, locale)
            } else {
                fetch_https_bytes(url, key, locale, "application/octet-stream")
            }
        }
        _ => Err(GtcError::invalid_data(
            "download URL",
            format!("unsupported scheme for {url}"),
        )),
    }
}

fn fetch_https_json_or_file_bytes(url: &str, key: &str, locale: &str) -> GtcResult<Vec<u8>> {
    fetch_https_bytes(url, key, locale, "application/vnd.github+json")
}

fn fetch_asset_bytes(url: &str, key: &str, locale: &str) -> GtcResult<Vec<u8>> {
    fetch_https_bytes(url, key, locale, "application/octet-stream")
}

pub(super) fn fetch_https_bytes(
    url: &str,
    key: &str,
    locale: &str,
    accept: &str,
) -> GtcResult<Vec<u8>> {
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| GtcError::message(format!("failed to create HTTP client: {e}")))?;

    let mut current = reqwest::Url::parse(url)
        .map_err(|e| GtcError::invalid_data("download URL", format!("{url}: {e}")))?;
    let original = current.clone();
    for _ in 0..10 {
        let mut request = client
            .get(current.clone())
            .header("Accept", accept)
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", format!("gtc/{}", env!("CARGO_PKG_VERSION")));
        if !key.is_empty() && should_send_auth_header(&original, &current) {
            request = request.header("Authorization", format!("Bearer {key}"));
        }
        let response = request
            .send()
            .map_err(|e| GtcError::message(format!("{}: {e}", t(locale, "gtc.err.pull_failed"))))?;

        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .ok_or_else(|| {
                    GtcError::invalid_data(
                        "redirect response",
                        format!("missing Location header for {}", current),
                    )
                })?
                .to_str()
                .map_err(|e| {
                    GtcError::invalid_data(
                        "redirect response",
                        format!("invalid Location for {}: {e}", current),
                    )
                })?;
            current = current.join(location).map_err(|e| {
                GtcError::invalid_data(
                    "redirect response",
                    format!("invalid redirect target {location}: {e}"),
                )
            })?;
            continue;
        }

        if !response.status().is_success() {
            return Err(GtcError::message(format!(
                "{}: HTTP {} for {}",
                t(locale, "gtc.err.pull_failed"),
                response.status(),
                current
            )));
        }

        return response
            .bytes()
            .map(|bytes| bytes.to_vec())
            .map_err(|e| GtcError::message(format!("failed to read response body: {e}")));
    }

    Err(GtcError::invalid_data(
        "redirect handling",
        format!("too many redirects while fetching {url}"),
    ))
}

pub(super) fn should_send_auth_header(original: &reqwest::Url, current: &reqwest::Url) -> bool {
    original.scheme() == current.scheme()
        && original.host_str() == current.host_str()
        && original.port_or_known_default() == current.port_or_known_default()
}

fn save_store_login(tenant: &str, token: &str) -> GtcResult<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| GtcError::message(format!("failed to build tokio runtime: {e}")))?;
    rt.block_on(save_login_default(tenant, token))
        .map_err(|e| GtcError::message(e.to_string()))
}

pub(super) fn rewrite_store_tenant_placeholder(url: &str, tenant: &str) -> String {
    url.replace("{tenant}", tenant)
}

fn resolve_tenant_manifest_url(tenant: &str, key: &str, locale: &str) -> GtcResult<String> {
    if let Some(template) = GtcConfig::from_env().tenant_manifest_url_template() {
        return Ok(template.replace("{tenant}", tenant));
    }
    let release = fetch_github_release("greentic-biz", "customers-tools", "latest", key, locale)?;
    let asset_name = format!("{tenant}.json");
    release
        .assets
        .into_iter()
        .find(|asset| asset.name == asset_name)
        .map(|asset| asset.url)
        .ok_or_else(|| {
            GtcError::invalid_data(
                "tenant manifest release",
                format!("asset '{asset_name}' not found in latest release"),
            )
        })
}

fn resolve_github_release_asset_api_url(
    url: &str,
    key: &str,
    locale: &str,
) -> GtcResult<Option<String>> {
    let parsed = match reqwest::Url::parse(url) {
        Ok(parsed) => parsed,
        Err(_) => return Ok(None),
    };
    if parsed.scheme() != "https" || parsed.host_str() != Some("github.com") {
        return Ok(None);
    }

    let segments = match parsed.path_segments() {
        Some(segments) => segments.collect::<Vec<_>>(),
        None => return Ok(None),
    };
    if segments.len() != 6 || segments[2] != "releases" {
        return Ok(None);
    }

    let owner = segments[0];
    let repo = segments[1];
    let asset_name = segments[5];

    let tag = match (segments[3], segments[4]) {
        ("latest", "download") => "latest",
        ("download", tag) => tag,
        _ => return Ok(None),
    };

    let release = fetch_github_release(owner, repo, tag, key, locale)?;
    Ok(release
        .assets
        .into_iter()
        .find(|asset| asset.name == asset_name)
        .map(|asset| asset.url))
}

fn fetch_github_release(
    owner: &str,
    repo: &str,
    tag: &str,
    key: &str,
    locale: &str,
) -> GtcResult<GithubRelease> {
    let url = if tag == "latest" {
        format!("https://api.github.com/repos/{owner}/{repo}/releases/latest")
    } else {
        format!("https://api.github.com/repos/{owner}/{repo}/releases/tags/{tag}")
    };
    let bytes = fetch_https_json_or_file_bytes(&url, key, locale)?;
    serde_json::from_slice(&bytes).map_err(|e| {
        GtcError::json(
            format!("failed to parse GitHub release {owner}/{repo}:{tag}"),
            e,
        )
    })
}

fn store_asset_file_name(store_url: &str) -> Option<String> {
    let trimmed = store_url.trim_start_matches("store://");
    let last = trimmed.rsplit('/').next()?;
    Some(last.split(':').next().unwrap_or(last).to_string())
}

fn store_asset_target_path(artifacts_root: &Path, file_name: &str) -> GtcResult<PathBuf> {
    let rel = if file_name.ends_with(".gtpack") {
        PathBuf::from("packs").join(file_name)
    } else if file_name.ends_with(".gtbundle") {
        PathBuf::from("bundles").join(file_name)
    } else if file_name.ends_with(".wasm") {
        PathBuf::from("components").join(file_name)
    } else {
        PathBuf::from("store_assets").join(file_name)
    };
    safe_join(artifacts_root, &rel)
}

pub(super) fn url_file_name(url: &str) -> Option<String> {
    let trimmed = url.trim_end_matches('/');
    trimmed
        .rsplit('/')
        .next()
        .map(|segment| segment.split('?').next().unwrap_or(segment))
        .filter(|segment| !segment.is_empty())
        .map(|segment| segment.to_string())
}

fn current_install_os() -> Result<String, i32> {
    match env::consts::OS {
        "linux" | "macos" | "windows" => Ok(env::consts::OS.to_string()),
        other => {
            eprintln!("unsupported install OS '{other}'");
            Err(1)
        }
    }
}

fn current_install_arch() -> Result<String, i32> {
    if let Some(runtime) = detect_runtime_install_arch()
        && let Some(normalized) = normalize_install_arch(&runtime)
    {
        return Ok(normalized.to_string());
    }

    if let Some(normalized) = normalize_install_arch(env::consts::ARCH) {
        return Ok(normalized.to_string());
    }

    eprintln!(
        "unsupported install architecture '{}' (runtime) / '{}' (build)",
        detect_runtime_install_arch().unwrap_or_else(|| "unknown".to_string()),
        env::consts::ARCH
    );
    Err(1)
}

fn detect_runtime_install_arch() -> Option<String> {
    if cfg!(target_os = "macos")
        && let Some(value) = query_command_trimmed("sysctl", &["-n", "hw.optional.arm64"])
    {
        match value.as_str() {
            "1" => return Some("arm64".to_string()),
            "0" => return Some("x86_64".to_string()),
            _ => {}
        }
    }

    if cfg!(windows) {
        return env::var("PROCESSOR_ARCHITEW6432")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| env::var("PROCESSOR_ARCHITECTURE").ok())
            .map(|v| v.trim().to_string());
    }

    query_command_trimmed("uname", &["-m"])
}

fn query_command_trimmed(command: &str, args: &[&str]) -> Option<String> {
    ProcessCommand::new(command)
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn file_url_path(url: &str) -> Option<PathBuf> {
    let path = url.strip_prefix("file://")?;
    if path.is_empty() {
        return None;
    }
    Some(PathBuf::from(path))
}

pub(super) fn normalize_install_arch(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "x86_64" | "amd64" => Some("x86_64"),
        "aarch64" | "arm64" => Some("aarch64"),
        _ => None,
    }
}

fn install_tool_artifact(
    staged: &Path,
    cargo_bin_dir: &Path,
    fallback_name: &str,
) -> GtcResult<()> {
    fs::create_dir_all(cargo_bin_dir)
        .map_err(|e| GtcError::io(format!("failed to create {}", cargo_bin_dir.display()), e))?;

    let expanded = tempfile::tempdir().map_err(|e| GtcError::message(e.to_string()))?;
    expand_into_target(staged, expanded.path())?;

    let mut candidates = gather_tool_candidates(expanded.path())?;
    if candidates.is_empty() {
        let fallback = find_first_file(expanded.path())?
            .ok_or_else(|| GtcError::message("no tool binary found"))?;
        candidates.push((fallback_name.to_string(), fallback));
    }

    for (name_hint, source) in candidates {
        let file_name = source
            .file_name()
            .and_then(|v| v.to_str())
            .map(|v| v.to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or(name_hint);

        let target = cargo_bin_dir.join(file_name);
        fs::copy(&source, &target).map_err(|e| {
            GtcError::io(
                format!(
                    "failed to install tool {} -> {}",
                    source.display(),
                    target.display()
                ),
                e,
            )
        })?;
        set_executable_if_unix(&target).map_err(|e| GtcError::message(e.to_string()))?;
    }

    Ok(())
}

fn gather_tool_candidates(root: &Path) -> GtcResult<Vec<(String, PathBuf)>> {
    let files = list_files_recursive(root)?;
    let mut out = Vec::new();

    for file in files {
        let rel = file.strip_prefix(root).unwrap_or(&file);
        let in_bin_dir = rel.components().any(|c| c.as_os_str() == "bin");
        let file_name = match file.file_name().and_then(|v| v.to_str()) {
            Some(v) => v,
            None => continue,
        };

        let looks_tool_name = file_name == "gtc"
            || file_name.starts_with("greentic-")
            || file_name.ends_with(".exe")
            || file_name.ends_with(".cmd")
            || file_name.ends_with(".bat");

        if in_bin_dir || looks_tool_name {
            out.push((file_name.to_string(), file));
        }
    }

    Ok(out)
}

fn find_first_file(root: &Path) -> GtcResult<Option<PathBuf>> {
    let mut files = list_files_recursive(root)?;
    files.sort();
    Ok(files.into_iter().next())
}

pub(super) fn list_files_recursive(root: &Path) -> GtcResult<Vec<PathBuf>> {
    let mut out = Vec::new();
    recurse_files(root, &mut out)?;
    Ok(out)
}

pub(super) fn recurse_files(root: &Path, out: &mut Vec<PathBuf>) -> GtcResult<()> {
    for entry in fs::read_dir(root)
        .map_err(|e| GtcError::io(format!("failed to read {}", root.display()), e))?
    {
        let entry = entry.map_err(|e| GtcError::message(e.to_string()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|e| GtcError::io(format!("failed to stat {}", path.display()), e))?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            recurse_files(&path, out)?;
        } else if file_type.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

pub(super) fn expand_into_target(source_dir: &Path, target_dir: &Path) -> GtcResult<()> {
    fs::create_dir_all(target_dir)
        .map_err(|e| GtcError::io(format!("failed to create {}", target_dir.display()), e))?;

    let files = list_files_recursive(source_dir)?;
    for file in files {
        let data = fs::read(&file)
            .map_err(|e| GtcError::io(format!("failed to read {}", file.display()), e))?;

        if looks_like_squashfs(&data) {
            extract_squashfs_file(&file, target_dir)
                .map_err(|e| GtcError::message(e.to_string()))?;
            continue;
        }

        if looks_like_zip(&data) {
            extract_zip_bytes(&data, target_dir).map_err(|e| GtcError::message(e.to_string()))?;
            continue;
        }

        if looks_like_gzip(&data) && extract_targz_bytes(&data, target_dir).is_ok() {
            continue;
        }

        if extract_tar_bytes(&data, target_dir).is_ok() {
            continue;
        }

        let name = file
            .file_name()
            .ok_or_else(|| GtcError::message("invalid filename"))?;
        let target = target_dir.join(name);
        fs::copy(&file, &target).map_err(|e| {
            GtcError::io(
                format!(
                    "failed to copy extracted file {} -> {}",
                    file.display(),
                    target.display()
                ),
                e,
            )
        })?;
    }

    Ok(())
}

fn resolve_artifacts_root() -> GtcResult<PathBuf> {
    let base =
        BaseDirs::new().ok_or_else(|| GtcError::message("failed to resolve home directory"))?;
    Ok(base.home_dir().join(".greentic").join("artifacts"))
}

pub(super) fn resolve_tenant_key(
    cli_key: Option<String>,
    tenant: &str,
    locale: &str,
) -> GtcResult<String> {
    if let Some(key) = cli_key
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    {
        return Ok(key);
    }

    let env_name = tenant_env_var_name(tenant);
    if let Some(key) = GtcConfig::from_env().tenant_key(tenant) {
        println!(
            "{}",
            tf(
                locale,
                "gtc.install.using_env_key",
                &[("env", env_name.as_str())]
            )
        );
        return Ok(key);
    }

    let prompt = tf(locale, "gtc.install.prompt_key", &[("tenant", tenant)]);
    loop {
        let key =
            rpassword::prompt_password(&prompt).map_err(|e| GtcError::message(e.to_string()))?;
        if !key.trim().is_empty() {
            return Ok(key);
        }
        eprintln!("{}", t(locale, "gtc.err.key_required"));
    }
}

pub(super) fn tenant_env_var_name(tenant: &str) -> String {
    let mut normalized = String::with_capacity(tenant.len());
    let mut prev_us = false;

    for ch in tenant.chars() {
        let upper = ch.to_ascii_uppercase();
        if upper.is_ascii_alphanumeric() {
            normalized.push(upper);
            prev_us = false;
        } else if !prev_us {
            normalized.push('_');
            prev_us = true;
        }
    }

    let trimmed = normalized.trim_matches('_').to_string();
    format!("GREENTIC_{}_KEY", trimmed)
}

#[derive(Debug, Deserialize)]
struct TenantInstallManifest {
    #[serde(rename = "$schema")]
    #[allow(dead_code)]
    schema: Option<String>,
    schema_version: String,
    tenant: String,
    tools: Vec<TenantManifestReference>,
    docs: Vec<TenantManifestReference>,
    #[serde(default)]
    store_assets: Vec<TenantManifestReference>,
}

#[derive(Debug, Deserialize)]
struct TenantManifestReference {
    id: String,
    url: String,
}

#[derive(Debug, Deserialize)]
struct ToolManifest {
    #[allow(dead_code)]
    schema_version: String,
    id: String,
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    description: String,
    install: ToolInstallManifest,
    #[allow(dead_code)]
    docs: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ToolInstallManifest {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    kind: String,
    binary_name: String,
    targets: Vec<ToolInstallTarget>,
}

#[derive(Debug, Deserialize)]
struct ToolInstallTarget {
    os: String,
    arch: String,
    url: String,
    #[allow(dead_code)]
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct DocManifest {
    #[allow(dead_code)]
    schema_version: String,
    #[allow(dead_code)]
    id: String,
    title: Option<String>,
    source: Option<DocSource>,
    download_file_name: Option<String>,
    default_relative_path: Option<String>,
    docs: Option<Vec<DocManifestEntry>>,
}

#[derive(Debug, Deserialize, Clone)]
struct DocManifestEntry {
    #[allow(dead_code)]
    title: String,
    source: DocSource,
    download_file_name: String,
    default_relative_path: String,
}

#[derive(Debug, Deserialize, Clone)]
struct DocSource {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    kind: String,
    url: String,
}

#[derive(Debug, Deserialize)]
struct StoreAssetManifest {
    #[allow(dead_code)]
    schema_version: String,
    items: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    assets: Vec<GithubReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubReleaseAsset {
    url: String,
    name: String,
}

impl DocManifest {
    fn entries(self) -> GtcResult<Vec<DocManifestEntry>> {
        if let Some(entries) = self.docs {
            return Ok(entries);
        }
        match (
            self.title,
            self.source,
            self.download_file_name,
            self.default_relative_path,
        ) {
            (Some(title), Some(source), Some(download_file_name), Some(default_relative_path)) => {
                Ok(vec![DocManifestEntry {
                    title,
                    source,
                    download_file_name,
                    default_relative_path,
                }])
            }
            _ => Err(GtcError::invalid_data(
                "doc manifest",
                "must contain either top-level doc fields or docs[]",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DocManifest, DocManifestEntry, DocSource, TenantManifestReference, current_install_arch,
        current_install_os, detect_binstall_version, download_url_into_dir,
        ensure_deployer_dist_pack, ensure_install_prereqs, expand_into_target,
        fetch_download_bytes_with_auth, fetch_download_bytes_with_auth as fetch_download_bytes,
        fetch_json_bytes_with_auth, fetch_json_bytes_with_auth as fetch_json_bytes,
        fetch_json_with_auth, file_url_path, gather_tool_candidates, install_tenant_doc_reference,
        install_tenant_tool_reference, install_tool_artifact, is_binstall_available,
        latest_binstall_version, list_files_recursive, normalize_expected_sha256,
        parse_first_semver, parse_numeric_version, recurse_files,
        resolve_github_release_asset_api_url, run_update, semver_compare, store_asset_file_name,
        store_asset_target_path, url_file_name,
    };
    use crate::EMBEDDED_TERRAFORM_GTPACK;
    #[cfg(unix)]
    use crate::tests::env_test_lock;
    use sha2::Digest;
    use std::cmp::Ordering;
    #[cfg(unix)]
    use std::env;
    use std::fs;
    use std::io::Write;
    use std::path::Path;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    #[cfg(unix)]
    fn write_executable(path: &Path, body: &str) {
        fs::write(path, body).expect("write script");
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod");
    }

    #[test]
    fn parse_first_semver_finds_numeric_version_tokens() {
        assert_eq!(
            parse_first_semver("cargo-binstall 1.7.0"),
            Some("1.7.0".to_string())
        );
        assert_eq!(
            parse_first_semver("cargo-binstall = \"1.8.2\""),
            Some("1.8.2".to_string())
        );
        assert_eq!(parse_first_semver("no semver here"), None);
    }

    #[test]
    fn semver_compare_handles_different_lengths() {
        assert_eq!(semver_compare("1.2.0", "1.2"), Ordering::Equal);
        assert_eq!(semver_compare("1.2.1", "1.2.0"), Ordering::Greater);
        assert_eq!(semver_compare("1.1.9", "1.2.0"), Ordering::Less);
        assert_eq!(parse_numeric_version("1.2.x"), vec![1, 2, 0]);
    }

    #[test]
    fn url_and_file_helpers_extract_expected_names() {
        assert_eq!(
            url_file_name("https://example.com/releases/demo.gtpack?download=1"),
            Some("demo.gtpack".to_string())
        );
        assert_eq!(
            file_url_path("file:///tmp/demo.json"),
            Some(Path::new("/tmp/demo.json").to_path_buf())
        );
        assert_eq!(file_url_path("https://example.com/demo.json"), None);
    }

    #[test]
    fn store_asset_helpers_route_files_into_expected_buckets() {
        let root = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            store_asset_file_name("store://tenant/packs/demo.gtpack:1.0.0"),
            Some("demo.gtpack".to_string())
        );
        assert_eq!(
            store_asset_target_path(root.path(), "demo.gtpack").expect("path"),
            root.path().join("packs").join("demo.gtpack")
        );
        assert_eq!(
            store_asset_target_path(root.path(), "demo.wasm").expect("path"),
            root.path().join("components").join("demo.wasm")
        );
        assert_eq!(
            store_asset_target_path(root.path(), "demo.txt").expect("path"),
            root.path().join("store_assets").join("demo.txt")
        );
    }

    #[test]
    fn download_helpers_support_local_file_urls() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("doc.json");
        fs::write(&source, br#"{"ok":true}"#).expect("write");

        let json = fetch_json_bytes_with_auth(&format!("file://{}", source.display()), "", "en")
            .expect("json");
        assert_eq!(json, br#"{"ok":true}"#);

        let bytes =
            fetch_download_bytes_with_auth(&format!("file://{}", source.display()), "", "en")
                .expect("bytes");
        assert_eq!(bytes, br#"{"ok":true}"#);

        let out_dir = dir.path().join("out");
        let downloaded = download_url_into_dir(
            &format!("file://{}", source.display()),
            "",
            &out_dir,
            None,
            "en",
        )
        .expect("downloaded");
        assert_eq!(downloaded, out_dir.join("doc.json"));
        assert_eq!(fs::read(downloaded).expect("read"), br#"{"ok":true}"#);
    }

    #[test]
    fn resolve_github_release_asset_api_url_returns_none_for_non_matching_urls() {
        assert_eq!(
            resolve_github_release_asset_api_url("https://example.com/demo.gtpack", "", "en")
                .expect("url"),
            None
        );
        assert_eq!(
            resolve_github_release_asset_api_url(
                "https://github.com/owner/repo/blob/main/demo",
                "",
                "en"
            )
            .expect("url"),
            None
        );
    }

    #[test]
    fn normalize_expected_sha256_keeps_existing_prefix() {
        assert_eq!(
            normalize_expected_sha256("sha256:abc"),
            "sha256:abc".to_string()
        );
    }

    #[test]
    fn current_install_platform_helpers_return_supported_values() {
        assert!(matches!(
            current_install_os().expect("os").as_str(),
            "linux" | "macos" | "windows"
        ));
        assert!(matches!(
            current_install_arch().expect("arch").as_str(),
            "x86_64" | "aarch64"
        ));
    }

    #[test]
    fn doc_manifest_entries_support_both_schema_shapes() {
        let top_level = DocManifest {
            schema_version: "1".to_string(),
            id: "docs".to_string(),
            title: Some("Guide".to_string()),
            source: Some(DocSource {
                kind: "download".to_string(),
                url: "https://example.com/guide.pdf".to_string(),
            }),
            download_file_name: Some("guide.pdf".to_string()),
            default_relative_path: Some("guides".to_string()),
            docs: None,
        };
        assert_eq!(top_level.entries().expect("entries").len(), 1);

        let nested = DocManifest {
            schema_version: "1".to_string(),
            id: "docs".to_string(),
            title: None,
            source: None,
            download_file_name: None,
            default_relative_path: None,
            docs: Some(vec![DocManifestEntry {
                title: "Guide".to_string(),
                source: DocSource {
                    kind: "download".to_string(),
                    url: "https://example.com/guide.pdf".to_string(),
                },
                download_file_name: "guide.pdf".to_string(),
                default_relative_path: "guides".to_string(),
            }]),
        };
        assert_eq!(nested.entries().expect("entries").len(), 1);
    }

    #[test]
    fn doc_manifest_entries_reject_missing_fields() {
        let manifest = DocManifest {
            schema_version: "1".to_string(),
            id: "docs".to_string(),
            title: Some("Guide".to_string()),
            source: None,
            download_file_name: Some("guide.pdf".to_string()),
            default_relative_path: Some("guides".to_string()),
            docs: None,
        };
        let err = manifest.entries().unwrap_err();
        assert!(err.to_string().contains("doc manifest"));
    }

    #[test]
    fn fetch_helpers_reject_unsupported_schemes() {
        let err = fetch_json_bytes("oci://example", "", "en").unwrap_err();
        assert!(err.to_string().contains("unsupported scheme"));

        let err = fetch_download_bytes("repo://example", "", "en").unwrap_err();
        assert!(err.to_string().contains("unsupported scheme"));
    }

    #[test]
    fn fetch_json_with_auth_reads_local_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("doc.json");
        fs::write(&source, br#"{"ok":true}"#).expect("write");

        let parsed: serde_json::Value =
            fetch_json_with_auth(&format!("file://{}", source.display()), "", "en")
                .expect("parsed");
        assert_eq!(
            parsed.get("ok").and_then(serde_json::Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn download_url_into_dir_preserves_local_file_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("payload.bin");
        fs::write(&source, b"payload").expect("write");
        let out_dir = dir.path().join("out");

        let downloaded = download_url_into_dir(
            &format!("file://{}", source.display()),
            "",
            &out_dir,
            Some("fallback.bin"),
            "en",
        )
        .expect("downloaded");
        assert_eq!(
            downloaded.file_name().and_then(|v| v.to_str()),
            Some("payload.bin")
        );
    }

    #[test]
    fn store_asset_target_path_rejects_traversal_like_names() {
        let root = tempfile::tempdir().expect("tempdir");
        let err = store_asset_target_path(root.path(), "../bad.gtpack").unwrap_err();
        assert!(err.to_string().contains("unsafe path"));
    }

    #[test]
    fn gather_tool_candidates_finds_bin_and_named_tools() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("bin")).expect("mkdir");
        fs::write(dir.path().join("bin/greentic-demo"), b"bin").expect("write");
        fs::write(dir.path().join("greentic-setup"), b"tool").expect("write");
        fs::write(dir.path().join("notes.txt"), b"ignore").expect("write");

        let candidates = gather_tool_candidates(dir.path()).expect("candidates");
        let names: Vec<_> = candidates.into_iter().map(|(name, _)| name).collect();
        assert!(names.contains(&"greentic-demo".to_string()));
        assert!(names.contains(&"greentic-setup".to_string()));
        assert!(!names.contains(&"notes.txt".to_string()));
    }

    #[test]
    fn install_tool_artifact_copies_detected_tools_into_cargo_bin_dir() {
        let staged = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(staged.path().join("bin")).expect("mkdir");
        fs::write(staged.path().join("bin/greentic-demo"), b"tool").expect("write");

        let cargo_bin = tempfile::tempdir().expect("tempdir");
        install_tool_artifact(staged.path(), cargo_bin.path(), "fallback").expect("install");
        let installed = cargo_bin.path().join("greentic-demo");
        assert!(installed.exists());
        assert_eq!(fs::read(installed).expect("read"), b"tool");
    }

    #[test]
    fn install_tool_artifact_falls_back_to_first_file_when_no_named_candidate_exists() {
        let staged = tempfile::tempdir().expect("tempdir");
        fs::write(staged.path().join("payload.bin"), b"tool").expect("write");

        let cargo_bin = tempfile::tempdir().expect("tempdir");
        install_tool_artifact(staged.path(), cargo_bin.path(), "fallback").expect("install");
        let installed = cargo_bin.path().join("payload.bin");
        assert!(installed.exists());
    }

    #[cfg(unix)]
    #[test]
    fn recurse_files_skips_symlinked_directories() {
        let dir = tempfile::tempdir().expect("tempdir");
        let real = dir.path().join("real");
        fs::create_dir_all(&real).expect("mkdir");
        fs::write(real.join("tool"), b"tool").expect("write");
        symlink(&real, dir.path().join("link")).expect("symlink");

        let mut out = Vec::new();
        recurse_files(dir.path(), &mut out).expect("recurse");

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].file_name().and_then(|v| v.to_str()), Some("tool"));
    }

    #[test]
    fn list_files_recursive_collects_nested_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("nested")).expect("mkdir");
        fs::write(dir.path().join("nested/tool"), b"tool").expect("write");

        let files = list_files_recursive(dir.path()).expect("files");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_name().and_then(|v| v.to_str()), Some("tool"));
    }

    #[test]
    fn install_tenant_doc_reference_installs_local_doc_into_docs_tree() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("guide.pdf");
        fs::write(&source, b"doc-bytes").expect("write");
        let manifest = dir.path().join("manifest.json");
        fs::write(
            &manifest,
            format!(
                r#"{{"schema_version":"1","id":"docs","title":"Guide","source":{{"type":"download","url":"file://{}"}},"download_file_name":"guide.pdf","default_relative_path":"guides"}}"#,
                source.display()
            ),
        )
        .expect("write manifest");

        let artifacts = dir.path().join("artifacts");
        let installed = install_tenant_doc_reference(
            &TenantManifestReference {
                id: "docs".to_string(),
                url: format!("file://{}", manifest.display()),
            },
            "tenant",
            "",
            &artifacts,
            "en",
        )
        .expect("install");

        assert_eq!(installed, vec![artifacts.join("docs/guides/guide.pdf")]);
        assert_eq!(fs::read(&installed[0]).expect("read"), b"doc-bytes");
    }

    #[test]
    fn install_tenant_doc_reference_rejects_unsafe_file_names() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("guide.pdf");
        fs::write(&source, b"doc-bytes").expect("write");
        let manifest = dir.path().join("manifest.json");
        fs::write(
            &manifest,
            format!(
                r#"{{"schema_version":"1","id":"docs","title":"Guide","source":{{"type":"download","url":"file://{}"}},"download_file_name":"../guide.pdf","default_relative_path":"guides"}}"#,
                source.display()
            ),
        )
        .expect("write manifest");

        let err = install_tenant_doc_reference(
            &TenantManifestReference {
                id: "docs".to_string(),
                url: format!("file://{}", manifest.display()),
            },
            "tenant",
            "",
            &dir.path().join("artifacts"),
            "en",
        )
        .unwrap_err();

        assert!(err.contains("invalid doc file name"));
    }

    #[test]
    fn expand_into_target_copies_files_and_extracts_archives() {
        let source = tempfile::tempdir().expect("tempdir");
        fs::write(source.path().join("plain.txt"), b"plain").expect("write");

        let zip_path = source.path().join("bundle.zip");
        let file = fs::File::create(&zip_path).expect("create");
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("nested/from-zip.txt", options)
            .expect("start");
        zip.write_all(b"zip-bytes").expect("write zip");
        zip.finish().expect("finish");

        let out = tempfile::tempdir().expect("tempdir");
        expand_into_target(source.path(), out.path()).expect("expand");

        assert_eq!(
            fs::read(out.path().join("plain.txt")).expect("read"),
            b"plain"
        );
        assert_eq!(
            fs::read(out.path().join("nested/from-zip.txt")).expect("read"),
            b"zip-bytes"
        );
    }

    #[test]
    fn install_tenant_tool_reference_installs_matching_local_artifact() {
        let dir = tempfile::tempdir().expect("tempdir");
        let artifact = dir.path().join("greentic-demo");
        fs::write(&artifact, b"tool-bytes").expect("write");
        let digest = format!("sha256:{:x}", sha2::Sha256::digest(b"tool-bytes"));

        let current_os = current_install_os().expect("os");
        let current_arch = current_install_arch().expect("arch");
        let manifest = dir.path().join("tool.json");
        fs::write(
            &manifest,
            format!(
                r#"{{
                    "schema_version":"1",
                    "id":"demo-tool",
                    "name":"Demo Tool",
                    "description":"fixture",
                    "install":{{
                        "type":"binary",
                        "binary_name":"greentic-demo",
                        "targets":[{{"os":"{os}","arch":"{arch}","url":"file://{url}","sha256":"{sha}"}}]
                    }},
                    "docs":[]
                }}"#,
                os = current_os,
                arch = current_arch,
                url = artifact.display(),
                sha = digest
            ),
        )
        .expect("write manifest");

        let cargo_bin = tempfile::tempdir().expect("tempdir");
        install_tenant_tool_reference(
            &TenantManifestReference {
                id: "demo-tool".to_string(),
                url: format!("file://{}", manifest.display()),
            },
            "tenant",
            "",
            &current_os,
            &current_arch,
            cargo_bin.path(),
            "en",
        )
        .expect("install");

        let installed = cargo_bin.path().join("greentic-demo");
        assert!(installed.exists());
        assert_eq!(fs::read(installed).expect("read"), b"tool-bytes");
    }

    #[test]
    fn install_tenant_tool_reference_rejects_missing_platform_target() {
        let dir = tempfile::tempdir().expect("tempdir");
        let artifact = dir.path().join("greentic-demo");
        fs::write(&artifact, b"tool-bytes").expect("write");
        let manifest = dir.path().join("tool.json");
        fs::write(
            &manifest,
            format!(
                r#"{{
                    "schema_version":"1",
                    "id":"demo-tool",
                    "name":"Demo Tool",
                    "description":"fixture",
                    "install":{{
                        "type":"binary",
                        "binary_name":"greentic-demo",
                        "targets":[{{"os":"other","arch":"other","url":"file://{}","sha256":"sha256:deadbeef"}}]
                    }},
                    "docs":[]
                }}"#,
                artifact.display()
            ),
        )
        .expect("write manifest");

        let err = install_tenant_tool_reference(
            &TenantManifestReference {
                id: "demo-tool".to_string(),
                url: format!("file://{}", manifest.display()),
            },
            "tenant",
            "",
            &current_install_os().expect("os"),
            &current_install_arch().expect("arch"),
            dir.path(),
            "en",
        )
        .unwrap_err();

        assert!(err.contains("no install target for tool"));
    }

    #[test]
    fn install_tenant_tool_reference_rejects_digest_mismatch() {
        let dir = tempfile::tempdir().expect("tempdir");
        let artifact = dir.path().join("greentic-demo");
        fs::write(&artifact, b"tool-bytes").expect("write");
        let manifest = dir.path().join("tool.json");
        let current_os = current_install_os().expect("os");
        let current_arch = current_install_arch().expect("arch");
        fs::write(
            &manifest,
            format!(
                r#"{{
                    "schema_version":"1",
                    "id":"demo-tool",
                    "name":"Demo Tool",
                    "description":"fixture",
                    "install":{{
                        "type":"binary",
                        "binary_name":"greentic-demo",
                        "targets":[{{"os":"{os}","arch":"{arch}","url":"file://{url}","sha256":"sha256:deadbeef"}}]
                    }},
                    "docs":[]
                }}"#,
                os = current_os,
                arch = current_arch,
                url = artifact.display()
            ),
        )
        .expect("write manifest");

        let err = install_tenant_tool_reference(
            &TenantManifestReference {
                id: "demo-tool".to_string(),
                url: format!("file://{}", manifest.display()),
            },
            "tenant",
            "",
            &current_os,
            &current_arch,
            dir.path(),
            "en",
        )
        .unwrap_err();

        assert!(err.to_string().contains("integrity check"));
    }

    #[cfg(unix)]
    #[test]
    fn ensure_deployer_dist_pack_writes_embedded_pack_into_cargo_home() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let original_cargo_home = env::var_os("CARGO_HOME");
        unsafe {
            env::set_var("CARGO_HOME", dir.path());
        }

        ensure_deployer_dist_pack(false).expect("write pack");
        let target = dir.path().join("bin/dist/terraform.gtpack");
        assert_eq!(fs::read(&target).expect("read"), EMBEDDED_TERRAFORM_GTPACK);

        fs::write(&target, b"stale").expect("overwrite");
        ensure_deployer_dist_pack(false).expect("rewrite pack");
        assert_eq!(fs::read(&target).expect("read"), EMBEDDED_TERRAFORM_GTPACK);

        unsafe {
            match original_cargo_home {
                Some(value) => env::set_var("CARGO_HOME", value),
                None => env::remove_var("CARGO_HOME"),
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn binstall_detection_helpers_use_fake_cargo_output() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let cargo = dir.path().join("cargo");
        write_executable(
            &cargo,
            "#!/bin/sh\nif [ \"$1\" = \"binstall\" ] && [ \"$2\" = \"-V\" ]; then\n  echo 'cargo-binstall 1.7.0'\n  exit 0\nfi\nif [ \"$1\" = \"binstall\" ] && [ \"$2\" = \"--version\" ]; then\n  echo 'cargo-binstall 1.7.0'\n  exit 0\nfi\nif [ \"$1\" = \"search\" ] && [ \"$2\" = \"cargo-binstall\" ]; then\n  echo 'cargo-binstall = \"1.8.1\"'\n  exit 0\nfi\nexit 1\n",
        );

        let original_path = env::var_os("PATH");
        unsafe {
            env::set_var("PATH", dir.path());
        }

        assert_eq!(
            detect_binstall_version(false, "en").as_deref(),
            Some("1.7.0")
        );
        assert_eq!(
            latest_binstall_version(false, "en").as_deref(),
            Some("1.8.1")
        );
        assert!(is_binstall_available(false, "en"));

        unsafe {
            match original_path {
                Some(value) => env::set_var("PATH", value),
                None => env::remove_var("PATH"),
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn ensure_install_prereqs_installs_missing_binstall_and_required_packages() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let log = dir.path().join("cargo.log");
        let cargo = dir.path().join("cargo");
        write_executable(
            &cargo,
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nif [ \"$1\" = \"binstall\" ] && [ \"$2\" = \"--version\" ]; then\n  exit 1\nfi\nif [ \"$1\" = \"search\" ] && [ \"$2\" = \"cargo-binstall\" ]; then\n  echo 'cargo-binstall = \"1.8.1\"'\n  exit 0\nfi\nexit 0\n",
                log.display()
            ),
        );

        let original_path = env::var_os("PATH");
        unsafe {
            env::set_var("PATH", dir.path());
        }

        assert_eq!(ensure_install_prereqs(false, "en"), 0);

        let logged = fs::read_to_string(log).expect("read log");
        assert!(logged.contains("search cargo-binstall --limit 1"));
        assert!(logged.contains("install cargo-binstall --locked"));
        assert!(logged.contains("binstall -y --version 0.4 greentic-dev"));
        assert!(logged.contains("binstall -y --version 0.4 greentic-operator"));
        assert!(logged.contains("binstall -y --version 0.4 greentic-bundle"));
        assert!(logged.contains("binstall -y --version 0.4 greentic-setup"));
        assert!(logged.contains("binstall -y --version 0.4 greentic-deployer"));

        unsafe {
            match original_path {
                Some(value) => env::set_var("PATH", value),
                None => env::remove_var("PATH"),
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn run_update_reports_failure_after_attempting_all_packages_and_tools() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let cargo_log = dir.path().join("cargo.log");
        let dev_log = dir.path().join("dev.log");
        let cargo = dir.path().join("cargo");
        let dev = dir.path().join("greentic-dev");

        write_executable(
            &cargo,
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nif [ \"$1\" = \"binstall\" ] && [ \"$2\" = \"-V\" ]; then\n  echo 'cargo-binstall 1.7.0'\n  exit 0\nfi\nif [ \"$1\" = \"binstall\" ] && [ \"$2\" = \"--version\" ]; then\n  echo 'cargo-binstall 1.7.0'\n  exit 0\nfi\nif [ \"$1\" = \"binstall\" ] && [ \"$6\" = \"greentic-operator\" ]; then\n  exit 9\nfi\nexit 0\n",
                cargo_log.display()
            ),
        );
        write_executable(
            &dev,
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit 0\n",
                dev_log.display()
            ),
        );

        let original_path = env::var_os("PATH");
        let original_dev_bin = env::var_os("GREENTIC_DEV_BIN");
        unsafe {
            env::set_var("PATH", dir.path());
            env::set_var("GREENTIC_DEV_BIN", &dev);
        }

        assert_eq!(run_update(false, "en"), 1);

        let cargo_logged = fs::read_to_string(cargo_log).expect("read cargo log");
        assert!(cargo_logged.contains("binstall -y --force --version 0.4 greentic-dev"));
        assert!(cargo_logged.contains("binstall -y --force --version 0.4 greentic-operator"));
        assert!(cargo_logged.contains("binstall -y --force --version 0.4 greentic-bundle"));
        assert!(cargo_logged.contains("binstall -y --force --version 0.4 greentic-setup"));
        assert!(cargo_logged.contains("binstall -y --force --version 0.4 greentic-deployer"));

        let dev_logged = fs::read_to_string(dev_log).expect("read dev log");
        assert!(dev_logged.contains("install tools"));

        unsafe {
            match original_path {
                Some(value) => env::set_var("PATH", value),
                None => env::remove_var("PATH"),
            }
            match original_dev_bin {
                Some(value) => env::set_var("GREENTIC_DEV_BIN", value),
                None => env::remove_var("GREENTIC_DEV_BIN"),
            }
        }
    }
}
