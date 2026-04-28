use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use clap::ArgMatches;
use gtc::error::{GtcError, GtcResult};
use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose, SanType,
};
use reqwest::blocking::{Client, ClientBuilder};
use reqwest::{Certificate, Identity};
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};
use serde_yaml_bw as serde_yaml;

use crate::DEPLOYER_BIN;
use crate::deploy::resolve_local_mutable_bundle_dir;
use crate::process::resolve_companion_command;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub(super) struct AdminRegistryDocument {
    pub(super) admins: Vec<AdminRegistryEntry>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub(super) struct AdminRegistryEntry {
    pub(super) name: Option<String>,
    pub(super) client_cn: String,
    pub(super) public_key: String,
    pub(super) added_at_epoch_s: u64,
}

#[derive(Debug, Deserialize)]
struct AdminAccessSummary {
    admin_public_endpoint: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MaterializedAdminCertsSummary {
    ca_cert_path: PathBuf,
    client_cert_path: PathBuf,
    client_key_path: PathBuf,
}

enum AdminAuth {
    Bearer(String),
    Mtls {
        ca_cert_path: PathBuf,
        client_cert_path: PathBuf,
        client_key_path: PathBuf,
    },
}

struct RemoteAdminContext {
    base_url: String,
    auth: AdminAuth,
}

pub(super) fn admin_registry_path(bundle_dir: &Path) -> PathBuf {
    bundle_dir
        .join(".greentic")
        .join("admin")
        .join("admins.json")
}

pub(super) fn load_admin_registry(bundle_dir: &Path) -> GtcResult<AdminRegistryDocument> {
    let path = admin_registry_path(bundle_dir);
    if !path.exists() {
        return Ok(AdminRegistryDocument { admins: Vec::new() });
    }
    let raw = fs::read_to_string(&path).map_err(|err| {
        GtcError::io(
            format!("failed to read admin registry {}", path.display()),
            err,
        )
    })?;
    serde_json::from_str(&raw).map_err(|err| {
        GtcError::json(
            format!("failed to parse admin registry {}", path.display()),
            err,
        )
    })
}

pub(super) fn save_admin_registry(
    bundle_dir: &Path,
    registry: &AdminRegistryDocument,
) -> GtcResult<()> {
    let path = admin_registry_path(bundle_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            GtcError::io(
                format!("failed to create admin registry dir {}", parent.display()),
                err,
            )
        })?;
    }
    let raw = serde_json::to_vec_pretty(registry)
        .map_err(|err| GtcError::message(format!("failed to serialize admin registry: {err}")))?;
    fs::write(&path, raw).map_err(|err| {
        GtcError::io(
            format!("failed to write admin registry {}", path.display()),
            err,
        )
    })
}

pub(super) fn upsert_admin_registry_entry(
    registry: &mut AdminRegistryDocument,
    name: Option<String>,
    client_cn: String,
    public_key: String,
) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0);
    if let Some(existing) = registry
        .admins
        .iter_mut()
        .find(|entry| entry.client_cn == client_cn)
    {
        existing.name = name;
        existing.public_key = public_key;
        existing.added_at_epoch_s = now;
        return;
    }
    registry.admins.push(AdminRegistryEntry {
        name,
        client_cn,
        public_key,
        added_at_epoch_s: now,
    });
    registry
        .admins
        .sort_by(|left, right| left.client_cn.cmp(&right.client_cn));
}

pub(super) fn remove_admin_registry_entry(
    registry: &mut AdminRegistryDocument,
    client_cn: Option<&str>,
    name: Option<&str>,
) -> bool {
    let before = registry.admins.len();
    registry.admins.retain(|entry| {
        if let Some(client_cn) = client_cn {
            return entry.client_cn != client_cn;
        }
        if let Some(name) = name {
            return entry.name.as_deref() != Some(name);
        }
        true
    });
    before != registry.admins.len()
}

pub(super) fn resolve_admin_cert_dir(bundle_dir: &Path) -> PathBuf {
    for candidate in [
        bundle_dir.join(".greentic").join("admin").join("certs"),
        bundle_dir.join("certs"),
    ] {
        if candidate.join("ca.crt").exists()
            && candidate.join("server.crt").exists()
            && candidate.join("server.key").exists()
        {
            return candidate;
        }
    }
    PathBuf::from("/etc/greentic/admin")
}

pub(super) fn ensure_admin_certs_ready(
    bundle_dir: &Path,
    explicit_dir: Option<&Path>,
) -> GtcResult<PathBuf> {
    if let Some(explicit_dir) = explicit_dir {
        ensure_admin_cert_dir_contents(explicit_dir)?;
        return Ok(explicit_dir.to_path_buf());
    }

    let bundle_local = bundle_dir.join(".greentic").join("admin").join("certs");
    if has_admin_server_certs(&bundle_local) {
        ensure_admin_cert_dir_contents(&bundle_local)?;
        return Ok(bundle_local);
    }

    generate_dev_admin_cert_bundle(&bundle_local)?;
    Ok(bundle_local)
}

fn has_admin_server_certs(cert_dir: &Path) -> bool {
    cert_dir.join("ca.crt").exists()
        && cert_dir.join("server.crt").exists()
        && cert_dir.join("server.key").exists()
}

fn ensure_admin_cert_dir_contents(cert_dir: &Path) -> GtcResult<()> {
    for required in ["ca.crt", "server.crt", "server.key"] {
        let path = cert_dir.join(required);
        if !path.exists() {
            return Err(GtcError::invalid_data(
                "admin TLS directory",
                format!("required file missing: {}", path.display()),
            ));
        }
    }
    Ok(())
}

fn generate_dev_admin_cert_bundle(cert_dir: &Path) -> GtcResult<()> {
    fs::create_dir_all(cert_dir).map_err(|err| {
        GtcError::io(
            format!("failed to create admin cert dir {}", cert_dir.display()),
            err,
        )
    })?;

    let mut ca_params = CertificateParams::new(Vec::<String>::new())
        .map_err(|err| GtcError::message(format!("failed to create admin CA params: {err}")))?;
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "greentic-admin-ca");
    ca_params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::CrlSign,
    ];
    let ca_key = KeyPair::generate()
        .map_err(|err| GtcError::message(format!("failed to generate CA key: {err}")))?;
    let ca_issuer = CertifiedIssuer::self_signed(ca_params, ca_key)
        .map_err(|err| GtcError::message(format!("failed to generate CA certificate: {err}")))?;

    let mut server_params =
        CertificateParams::new(vec!["localhost".to_string()]).map_err(|err| {
            GtcError::message(format!("failed to create admin server cert params: {err}"))
        })?;
    server_params
        .distinguished_name
        .push(DnType::CommonName, "greentic-admin-server");
    server_params.subject_alt_names.push(SanType::IpAddress(
        "127.0.0.1".parse().expect("static localhost ip"),
    ));
    server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    server_params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment,
    ];
    let server_key = KeyPair::generate()
        .map_err(|err| GtcError::message(format!("failed to generate server key: {err}")))?;
    let server_cert = server_params
        .signed_by(&server_key, &*ca_issuer)
        .map_err(|err| {
            GtcError::message(format!("failed to generate server certificate: {err}"))
        })?;

    let mut client_params = CertificateParams::new(Vec::<String>::new()).map_err(|err| {
        GtcError::message(format!("failed to create admin client cert params: {err}"))
    })?;
    client_params
        .distinguished_name
        .push(DnType::CommonName, "local-admin");
    client_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    client_params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment,
    ];
    let client_key = KeyPair::generate()
        .map_err(|err| GtcError::message(format!("failed to generate client key: {err}")))?;
    let client_cert = client_params
        .signed_by(&client_key, &*ca_issuer)
        .map_err(|err| {
            GtcError::message(format!("failed to generate client certificate: {err}"))
        })?;

    fs::write(cert_dir.join("ca.crt"), ca_issuer.pem()).map_err(|err| {
        GtcError::io(
            format!("failed to write {}", cert_dir.join("ca.crt").display()),
            err,
        )
    })?;
    write_private_key_file(&cert_dir.join("ca.key"), &ca_issuer.key().serialize_pem())?;
    fs::write(cert_dir.join("server.crt"), server_cert.pem()).map_err(|err| {
        GtcError::io(
            format!("failed to write {}", cert_dir.join("server.crt").display()),
            err,
        )
    })?;
    write_private_key_file(&cert_dir.join("server.key"), &server_key.serialize_pem())?;
    fs::write(cert_dir.join("client.crt"), client_cert.pem()).map_err(|err| {
        GtcError::io(
            format!("failed to write {}", cert_dir.join("client.crt").display()),
            err,
        )
    })?;
    write_private_key_file(&cert_dir.join("client.key"), &client_key.serialize_pem())?;
    Ok(())
}

fn write_private_key_file(path: &Path, contents: &str) -> GtcResult<()> {
    fs::write(path, contents)
        .map_err(|err| GtcError::io(format!("failed to write {}", path.display()), err))?;
    set_private_file_permissions(path)?;
    Ok(())
}

#[cfg(unix)]
fn set_private_file_permissions(path: &Path) -> GtcResult<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|err| {
        GtcError::io(
            format!("failed to set permissions on {}", path.display()),
            err,
        )
    })
}

#[cfg(not(unix))]
fn set_private_file_permissions(_path: &Path) -> GtcResult<()> {
    Ok(())
}

pub(super) fn run_admin_tunnel(sub_matches: &ArgMatches, _locale: &str) -> i32 {
    let Some(bundle_ref) = sub_matches.get_one::<String>("bundle-ref") else {
        eprintln!("missing bundle ref");
        return 2;
    };
    let target = sub_matches
        .get_one::<String>("target")
        .map(String::as_str)
        .unwrap_or("aws");
    let local_port = sub_matches
        .get_one::<String>("local-port")
        .expect("defaulted by clap");
    let container = sub_matches
        .get_one::<String>("container")
        .expect("defaulted by clap");

    if target != "aws" {
        eprintln!("admin tunnel currently supports only --target aws");
        return 2;
    }

    let bundle_dir = match resolve_local_mutable_bundle_dir(bundle_ref) {
        Ok(path) => path,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };

    let deployer_bin = resolve_companion_command(DEPLOYER_BIN);
    let status = ProcessCommand::new(&deployer_bin)
        .args([
            "aws",
            "admin-tunnel",
            "--bundle-dir",
            &bundle_dir.display().to_string(),
            "--local-port",
            local_port,
            "--container",
            container,
        ])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status();

    match status {
        Ok(status) if status.success() => 0,
        Ok(status) => {
            eprintln!("admin tunnel exited with status {status}");
            1
        }
        Err(err) => {
            eprintln!("failed to start greentic-deployer aws admin tunnel: {err}");
            1
        }
    }
}

pub(super) fn run_admin_access(sub_matches: &ArgMatches, _locale: &str) -> i32 {
    run_admin_deployer_command(sub_matches, "admin-access")
}

pub(super) fn run_admin_certs(sub_matches: &ArgMatches, _locale: &str) -> i32 {
    run_admin_deployer_command(sub_matches, "admin-certs")
}

pub(super) fn run_admin_token(sub_matches: &ArgMatches, _locale: &str) -> i32 {
    run_admin_deployer_command(sub_matches, "admin-token")
}

pub(super) fn run_admin_health(sub_matches: &ArgMatches, _locale: &str) -> i32 {
    let target = sub_matches
        .get_one::<String>("target")
        .map(String::as_str)
        .unwrap_or("aws");
    if target == "aws" {
        return run_remote_admin_request(sub_matches, reqwest::Method::GET, "/health", None);
    }
    run_admin_deployer_command(sub_matches, "admin-health")
}

pub(super) fn run_admin_status(sub_matches: &ArgMatches, _locale: &str) -> i32 {
    run_remote_admin_request(sub_matches, reqwest::Method::GET, "/status", None)
}

pub(super) fn run_admin_list(sub_matches: &ArgMatches, _locale: &str) -> i32 {
    run_remote_admin_request(sub_matches, reqwest::Method::GET, "/list", None)
}

pub(super) fn run_admin_clients(sub_matches: &ArgMatches, _locale: &str) -> i32 {
    run_remote_admin_request(sub_matches, reqwest::Method::GET, "/admins", None)
}

pub(super) fn run_admin_stop(sub_matches: &ArgMatches, _locale: &str) -> i32 {
    let Some(bundle_ref) = sub_matches.get_one::<String>("bundle-ref") else {
        eprintln!("missing bundle ref");
        return 2;
    };
    let bundle_dir = match resolve_local_mutable_bundle_dir(bundle_ref) {
        Ok(path) => path,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    let body = json!({ "bundle_path": bundle_dir });
    run_remote_admin_request(sub_matches, reqwest::Method::POST, "/stop", Some(body))
}

pub(super) fn run_admin_add_client(sub_matches: &ArgMatches, _locale: &str) -> i32 {
    let Some(bundle_ref) = sub_matches.get_one::<String>("bundle-ref") else {
        eprintln!("missing bundle ref");
        return 2;
    };
    let Some(client_cn) = sub_matches.get_one::<String>("cn") else {
        eprintln!("missing --cn");
        return 2;
    };
    let bundle_dir = match resolve_local_mutable_bundle_dir(bundle_ref) {
        Ok(path) => path,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    let body = json!({
        "bundle_path": bundle_dir,
        "client_cn": client_cn,
    });
    run_remote_admin_request(
        sub_matches,
        reqwest::Method::POST,
        "/admins/add",
        Some(body),
    )
}

pub(super) fn run_admin_remove_client(sub_matches: &ArgMatches, _locale: &str) -> i32 {
    let Some(bundle_ref) = sub_matches.get_one::<String>("bundle-ref") else {
        eprintln!("missing bundle ref");
        return 2;
    };
    let Some(client_cn) = sub_matches.get_one::<String>("cn") else {
        eprintln!("missing --cn");
        return 2;
    };
    let bundle_dir = match resolve_local_mutable_bundle_dir(bundle_ref) {
        Ok(path) => path,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    let body = json!({
        "bundle_path": bundle_dir,
        "client_cn": client_cn,
    });
    run_remote_admin_request(
        sub_matches,
        reqwest::Method::POST,
        "/admins/remove",
        Some(body),
    )
}

fn run_admin_deployer_command(sub_matches: &ArgMatches, admin_subcommand: &str) -> i32 {
    let Some(bundle_ref) = sub_matches.get_one::<String>("bundle-ref") else {
        eprintln!("missing bundle ref");
        return 2;
    };
    let target = sub_matches
        .get_one::<String>("target")
        .map(String::as_str)
        .unwrap_or("aws");
    let output = sub_matches
        .get_one::<String>("output")
        .map(String::as_str)
        .unwrap_or("text");

    let bundle_dir = match resolve_local_mutable_bundle_dir(bundle_ref) {
        Ok(path) => path,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };

    let deployer_bin = resolve_companion_command(DEPLOYER_BIN);
    let status = ProcessCommand::new(&deployer_bin)
        .args([
            target,
            admin_subcommand,
            "--bundle-dir",
            &bundle_dir.display().to_string(),
            "--output",
            output,
        ])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status();

    match status {
        Ok(status) if status.success() => 0,
        Ok(status) => {
            eprintln!("{admin_subcommand} exited with status {status}");
            1
        }
        Err(err) => {
            eprintln!("failed to start greentic-deployer {target} {admin_subcommand}: {err}");
            1
        }
    }
}

fn capture_admin_deployer_command(
    bundle_dir: &Path,
    target: &str,
    admin_subcommand: &str,
    output: &str,
) -> GtcResult<String> {
    let deployer_bin = resolve_companion_command(DEPLOYER_BIN);
    let output = ProcessCommand::new(&deployer_bin)
        .args([
            target,
            admin_subcommand,
            "--bundle-dir",
            &bundle_dir.display().to_string(),
            "--output",
            output,
        ])
        .output()
        .map_err(|err| {
            GtcError::io(format!("failed to execute {}", deployer_bin.display()), err)
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            return Err(GtcError::message(format!(
                "{admin_subcommand} exited with status {}",
                output.status.code().unwrap_or(1)
            )));
        }
        return Err(GtcError::message(stderr));
    }
    String::from_utf8(output.stdout)
        .map_err(|err| GtcError::message(format!("invalid UTF-8 from greentic-deployer: {err}")))
}

fn resolve_remote_admin_context(
    bundle_dir: &Path,
    target: &str,
    local_port: u16,
) -> GtcResult<RemoteAdminContext> {
    if target == "aws" {
        let certs_raw = capture_admin_deployer_command(bundle_dir, target, "admin-certs", "json")?;
        let certs: MaterializedAdminCertsSummary = serde_json::from_str(&certs_raw)
            .map_err(|err| GtcError::message(format!("failed to parse admin certs JSON: {err}")))?;
        return Ok(RemoteAdminContext {
            base_url: format!("https://127.0.0.1:{local_port}/admin/v1"),
            auth: AdminAuth::Mtls {
                ca_cert_path: certs.ca_cert_path,
                client_cert_path: certs.client_cert_path,
                client_key_path: certs.client_key_path,
            },
        });
    }

    let access_raw = capture_admin_deployer_command(bundle_dir, target, "admin-access", "json")?;
    let access: AdminAccessSummary = serde_json::from_str(&access_raw)
        .map_err(|err| GtcError::message(format!("failed to parse admin access JSON: {err}")))?;
    let token = capture_admin_deployer_command(bundle_dir, target, "admin-token", "text")?;
    let base_url = access
        .admin_public_endpoint
        .ok_or_else(|| GtcError::message("missing admin_public_endpoint".to_string()))?;
    Ok(RemoteAdminContext {
        base_url,
        auth: AdminAuth::Bearer(token.trim().to_string()),
    })
}

fn build_remote_admin_client(context: &RemoteAdminContext) -> GtcResult<Client> {
    match &context.auth {
        AdminAuth::Bearer(_) => ClientBuilder::new()
            .build()
            .map_err(|err| GtcError::message(format!("failed to build admin client: {err}"))),
        AdminAuth::Mtls {
            ca_cert_path,
            client_cert_path,
            client_key_path,
        } => {
            let ca_pem = fs::read(ca_cert_path).map_err(|err| {
                GtcError::io(format!("failed to read {}", ca_cert_path.display()), err)
            })?;
            let client_cert_pem = fs::read(client_cert_path).map_err(|err| {
                GtcError::io(
                    format!("failed to read {}", client_cert_path.display()),
                    err,
                )
            })?;
            let client_key_pem = fs::read(client_key_path).map_err(|err| {
                GtcError::io(format!("failed to read {}", client_key_path.display()), err)
            })?;
            let mut identity_pem = client_cert_pem;
            identity_pem.extend_from_slice(&client_key_pem);
            let ca = Certificate::from_pem(&ca_pem)
                .map_err(|err| GtcError::message(format!("failed to parse CA cert PEM: {err}")))?;
            let identity = Identity::from_pem(&identity_pem).map_err(|err| {
                GtcError::message(format!("failed to parse client identity PEM: {err}"))
            })?;
            ClientBuilder::new()
                .use_rustls_tls()
                .add_root_certificate(ca)
                .identity(identity)
                .build()
                .map_err(|err| {
                    GtcError::message(format!("failed to build mTLS admin client: {err}"))
                })
        }
    }
}

fn run_remote_admin_request(
    sub_matches: &ArgMatches,
    method: reqwest::Method,
    path: &str,
    body: Option<JsonValue>,
) -> i32 {
    let Some(bundle_ref) = sub_matches.get_one::<String>("bundle-ref") else {
        eprintln!("missing bundle ref");
        return 2;
    };
    let target = sub_matches
        .get_one::<String>("target")
        .map(String::as_str)
        .unwrap_or("aws");
    let output = sub_matches
        .get_one::<String>("output")
        .map(String::as_str)
        .unwrap_or("text");
    let local_port = sub_matches
        .get_one::<String>("local-port")
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(8443);

    let bundle_dir = match resolve_local_mutable_bundle_dir(bundle_ref) {
        Ok(path) => path,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };

    let context = match resolve_remote_admin_context(&bundle_dir, target, local_port) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    let client = match build_remote_admin_client(&context) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };

    let url = format!("{}{}", context.base_url.trim_end_matches('/'), path);
    let mut request = client.request(method, &url);
    match &context.auth {
        AdminAuth::Bearer(token) => {
            request = request.bearer_auth(token);
        }
        AdminAuth::Mtls { .. } => {}
    }
    if let Some(body) = body {
        request = request.json(&body);
    }

    let response = match request.send() {
        Ok(value) => value,
        Err(err) => {
            if target == "aws" {
                eprintln!(
                    "admin request failed: {err}. Ensure `gtc admin tunnel ... --target aws` is running on local port {local_port}."
                );
            } else {
                eprintln!("admin request failed: {err}");
            }
            return 1;
        }
    };
    let status = response.status();
    let body = match response.text() {
        Ok(value) => value,
        Err(err) => {
            eprintln!("failed to read admin response body: {err}");
            return 1;
        }
    };

    let rendered = render_admin_http_body(&body, output);
    match rendered {
        Ok(text) => println!("{text}"),
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    }

    if status.is_success() { 0 } else { 1 }
}

fn render_admin_http_body(body: &str, output: &str) -> GtcResult<String> {
    match output {
        "json" => {
            let value: JsonValue = serde_json::from_str(body).map_err(|err| {
                GtcError::message(format!("failed to parse admin JSON response: {err}"))
            })?;
            serde_json::to_string_pretty(&value).map_err(|err| {
                GtcError::message(format!("failed to render admin JSON response: {err}"))
            })
        }
        "yaml" => {
            let value: JsonValue = serde_json::from_str(body).map_err(|err| {
                GtcError::message(format!("failed to parse admin JSON response: {err}"))
            })?;
            serde_yaml::to_string(&value).map_err(|err| {
                GtcError::message(format!("failed to render admin YAML response: {err}"))
            })
        }
        _ => Ok(body.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AdminRegistryDocument, AdminRegistryEntry, RemoteAdminContext, admin_registry_path,
        build_remote_admin_client, ensure_admin_certs_ready, load_admin_registry,
        remove_admin_registry_entry, render_admin_http_body, resolve_admin_cert_dir,
        save_admin_registry, upsert_admin_registry_entry,
    };
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn load_admin_registry_returns_empty_when_file_is_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let registry = load_admin_registry(dir.path()).expect("registry");
        assert!(registry.admins.is_empty());
    }

    #[test]
    fn save_and_load_admin_registry_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let registry = AdminRegistryDocument {
            admins: vec![AdminRegistryEntry {
                name: Some("alice".to_string()),
                client_cn: "CN=alice".to_string(),
                public_key: "ssh-ed25519 AAAA".to_string(),
                added_at_epoch_s: 7,
            }],
        };

        save_admin_registry(dir.path(), &registry).expect("save");
        let loaded = load_admin_registry(dir.path()).expect("load");
        assert_eq!(loaded.admins, registry.admins);
        assert_eq!(
            admin_registry_path(dir.path()),
            dir.path()
                .join(".greentic")
                .join("admin")
                .join("admins.json")
        );
    }

    #[test]
    fn upsert_updates_existing_entry_and_sorts_new_entries() {
        let mut registry = AdminRegistryDocument {
            admins: vec![AdminRegistryEntry {
                name: Some("zeta".to_string()),
                client_cn: "CN=zeta".to_string(),
                public_key: "old".to_string(),
                added_at_epoch_s: 1,
            }],
        };

        upsert_admin_registry_entry(
            &mut registry,
            Some("alpha".to_string()),
            "CN=alpha".to_string(),
            "alpha-key".to_string(),
        );
        upsert_admin_registry_entry(
            &mut registry,
            Some("zeta-renamed".to_string()),
            "CN=zeta".to_string(),
            "new".to_string(),
        );

        assert_eq!(
            registry
                .admins
                .iter()
                .map(|entry| entry.client_cn.as_str())
                .collect::<Vec<_>>(),
            vec!["CN=alpha", "CN=zeta"]
        );
        let updated = registry
            .admins
            .iter()
            .find(|entry| entry.client_cn == "CN=zeta")
            .expect("updated");
        assert_eq!(updated.name.as_deref(), Some("zeta-renamed"));
        assert_eq!(updated.public_key, "new");
    }

    #[test]
    fn remove_admin_registry_entry_can_match_by_name() {
        let mut registry = AdminRegistryDocument {
            admins: vec![
                AdminRegistryEntry {
                    name: Some("alice".to_string()),
                    client_cn: "CN=alice".to_string(),
                    public_key: "a".to_string(),
                    added_at_epoch_s: 1,
                },
                AdminRegistryEntry {
                    name: Some("bob".to_string()),
                    client_cn: "CN=bob".to_string(),
                    public_key: "b".to_string(),
                    added_at_epoch_s: 2,
                },
            ],
        };

        assert!(remove_admin_registry_entry(
            &mut registry,
            None,
            Some("alice")
        ));
        assert_eq!(registry.admins.len(), 1);
        assert_eq!(registry.admins[0].client_cn, "CN=bob");
    }

    #[test]
    fn resolve_admin_cert_dir_falls_back_to_default_when_bundle_has_no_certs() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            resolve_admin_cert_dir(dir.path()),
            PathBuf::from("/etc/greentic/admin")
        );
    }

    #[test]
    fn ensure_admin_certs_ready_rejects_explicit_dir_with_missing_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let certs = dir.path().join("certs");
        fs::create_dir_all(&certs).expect("mkdir");
        fs::write(certs.join("ca.crt"), "ca").expect("write ca");

        let err = ensure_admin_certs_ready(dir.path(), Some(&certs)).expect_err("missing files");
        assert!(err.to_string().contains("required file missing"));
    }

    #[test]
    fn build_remote_admin_client_supports_bearer_auth() {
        let context = RemoteAdminContext {
            base_url: "https://example.test/admin/v1".to_string(),
            auth: super::AdminAuth::Bearer("token".to_string()),
        };
        build_remote_admin_client(&context).expect("client");
    }

    #[test]
    fn build_remote_admin_client_supports_generated_mtls_material() {
        let dir = tempfile::tempdir().expect("tempdir");
        let certs = ensure_admin_certs_ready(dir.path(), None).expect("certs");
        let context = RemoteAdminContext {
            base_url: "https://127.0.0.1:8443/admin/v1".to_string(),
            auth: super::AdminAuth::Mtls {
                ca_cert_path: certs.join("ca.crt"),
                client_cert_path: certs.join("client.crt"),
                client_key_path: certs.join("client.key"),
            },
        };
        build_remote_admin_client(&context).expect("client");
    }

    #[test]
    fn render_admin_http_body_formats_json_and_yaml() {
        let json = render_admin_http_body(r#"{"status":"ok"}"#, "json").expect("json");
        let yaml = render_admin_http_body(r#"{"status":"ok"}"#, "yaml").expect("yaml");
        let text = render_admin_http_body("plain-text", "text").expect("text");

        assert!(json.contains("\"status\": \"ok\""));
        assert!(yaml.contains("status: ok"));
        assert_eq!(text, "plain-text");
    }
}
