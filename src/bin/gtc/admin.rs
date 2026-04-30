use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as ProcessCommand, Stdio};
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::ArgMatches;
use gtc::error::{GtcError, GtcResult};
use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose, SanType,
};
use reqwest::blocking::{Client, ClientBuilder};
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
struct AdminTokenPathSummary {
    token_path: PathBuf,
}

#[derive(Debug, Deserialize)]
struct TerraformOutputValue {
    value: JsonValue,
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

struct RemoteAdminSession {
    context: RemoteAdminContext,
    tunnel_child: Option<Child>,
}

fn resolve_materialized_admin_certs_from_bundle_dir(
    bundle_dir: &Path,
) -> GtcResult<MaterializedAdminCertsSummary> {
    let tunnels_root = bundle_dir.join(".greentic").join("admin").join("tunnels");
    let mut newest: Option<(SystemTime, MaterializedAdminCertsSummary)> = None;
    let entries = fs::read_dir(&tunnels_root).map_err(|err| {
        GtcError::io(
            format!(
                "failed to read AWS admin tunnels directory {}",
                tunnels_root.display()
            ),
            err,
        )
    })?;
    for entry in entries {
        let entry =
            entry.map_err(|err| GtcError::message(format!("failed to read tunnel entry: {err}")))?;
        let path = entry.path();
        if !entry
            .file_type()
            .map_err(|err| GtcError::message(format!("failed to stat tunnel entry: {err}")))?
            .is_dir()
        {
            continue;
        }
        let ca_cert_path = path.join("ca.crt");
        let client_cert_path = path.join("client.crt");
        let client_key_path = path.join("client.key");
        if !(ca_cert_path.exists() && client_cert_path.exists() && client_key_path.exists()) {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .unwrap_or(UNIX_EPOCH);
        let certs = MaterializedAdminCertsSummary {
            ca_cert_path,
            client_cert_path,
            client_key_path,
        };
        match &newest {
            Some((current_modified, _)) if modified <= *current_modified => {}
            _ => newest = Some((modified, certs)),
        }
    }
    newest.map(|(_, certs)| certs).ok_or_else(|| {
        GtcError::message(format!(
            "no materialized AWS admin client certs found under {}",
            tunnels_root.display()
        ))
    })
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
        let certs = resolve_materialized_admin_certs_from_bundle_dir(bundle_dir)?;
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
    let token = load_remote_admin_bearer_token(bundle_dir, target)?;
    let base_url = access
        .admin_public_endpoint
        .or_else(|| load_admin_public_endpoint_from_local_deploy_outputs(bundle_dir, target).ok())
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
        AdminAuth::Mtls { .. } => ClientBuilder::new()
            .build()
            .map_err(|err| GtcError::message(format!("failed to build admin client: {err}"))),
    }
}

fn resolve_remote_admin_session(bundle_dir: &Path, target: &str) -> GtcResult<RemoteAdminSession> {
    if target != "aws" {
        let context = resolve_remote_admin_context(bundle_dir, target, 8443)?;
        return Ok(RemoteAdminSession {
            context,
            tunnel_child: None,
        });
    }

    let access_raw = capture_admin_deployer_command(bundle_dir, target, "admin-access", "json")?;
    let access: AdminAccessSummary = serde_json::from_str(&access_raw)
        .map_err(|err| GtcError::message(format!("failed to parse admin access JSON: {err}")))?;
    if let Some(base_url) = access
        .admin_public_endpoint
        .or_else(|| load_admin_public_endpoint_from_local_deploy_outputs(bundle_dir, target).ok())
    {
        let token = load_remote_admin_bearer_token(bundle_dir, target)?;
        return Ok(RemoteAdminSession {
            context: RemoteAdminContext {
                base_url,
                auth: AdminAuth::Bearer(token.trim().to_string()),
            },
            tunnel_child: None,
        });
    }

    let deployer_bin = resolve_companion_command(DEPLOYER_BIN);
    let child = ProcessCommand::new(&deployer_bin)
        .args([
            "aws",
            "admin-tunnel",
            "--bundle-dir",
            &bundle_dir.display().to_string(),
            "--local-port",
            "8443",
            "--container",
            "app",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| GtcError::io("failed to start aws admin tunnel".to_string(), err))?;

    thread::sleep(Duration::from_secs(3));
    let certs = resolve_materialized_admin_certs_from_bundle_dir(bundle_dir)?;

    Ok(RemoteAdminSession {
        context: RemoteAdminContext {
            base_url: "https://127.0.0.1:8443/admin/v1".to_string(),
            auth: AdminAuth::Mtls {
                ca_cert_path: certs.ca_cert_path,
                client_cert_path: certs.client_cert_path,
                client_key_path: certs.client_key_path,
            },
        },
        tunnel_child: Some(child),
    })
}

fn load_remote_admin_bearer_token(bundle_dir: &Path, target: &str) -> GtcResult<String> {
    let token_raw = capture_admin_deployer_command(bundle_dir, target, "admin-token", "json")?;
    let token_summary: AdminTokenPathSummary = serde_json::from_str(&token_raw)
        .map_err(|err| GtcError::message(format!("failed to parse admin token JSON: {err}")))?;
    fs::read_to_string(&token_summary.token_path).map_err(|err| {
        GtcError::io(
            format!("failed to read {}", token_summary.token_path.display()),
            err,
        )
    })
}

fn parse_admin_response(body: &str) -> GtcResult<JsonValue> {
    serde_json::from_str(body).map_err(|err| {
        let snippet = body.trim();
        let snippet = if snippet.len() > 400 {
            format!("{}...", &snippet[..400])
        } else {
            snippet.to_string()
        };
        GtcError::message(format!(
            "failed to parse admin JSON response: {err}; body={snippet:?}"
        ))
    })
}

fn remote_admin_json_request(
    client: &Client,
    context: &RemoteAdminContext,
    method: reqwest::Method,
    path: &str,
    body: Option<&JsonValue>,
) -> GtcResult<JsonValue> {
    let url = format!("{}{}", context.base_url.trim_end_matches('/'), path);
    let method_name = method.as_str().to_string();
    if let AdminAuth::Mtls {
        ca_cert_path,
        client_cert_path,
        client_key_path,
    } = &context.auth
    {
        let mut args = vec![
            "--silent".to_string(),
            "--show-error".to_string(),
            "--cacert".to_string(),
            ca_cert_path.display().to_string(),
            "--cert".to_string(),
            client_cert_path.display().to_string(),
            "--key".to_string(),
            client_key_path.display().to_string(),
            "-X".to_string(),
            method_name.clone(),
            url.clone(),
        ];
        if let Some(body) = body {
            args.push("-H".to_string());
            args.push("content-type: application/json".to_string());
            args.push("--data-binary".to_string());
            args.push(body.to_string());
        }
        let output = ProcessCommand::new("curl")
            .args(&args)
            .output()
            .map_err(|err| GtcError::io("failed to execute curl".to_string(), err))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(GtcError::message(format!(
                "admin request {} {} failed via curl: {}",
                &method_name,
                path,
                if stderr.is_empty() {
                    format!("exit {}", output.status.code().unwrap_or(1))
                } else {
                    stderr
                }
            )));
        }
        let text = String::from_utf8(output.stdout).map_err(|err| {
            GtcError::message(format!("invalid UTF-8 from curl admin response: {err}"))
        })?;
        return parse_admin_response(&text);
    }

    let mut request = client.request(method, &url);
    match &context.auth {
        AdminAuth::Bearer(token) => {
            request = request.bearer_auth(token);
        }
        AdminAuth::Mtls { .. } => {}
    }
    if let Some(body) = body {
        request = request.json(body);
    }
    let response = request
        .send()
        .map_err(|err| GtcError::message(format!("admin request failed: {err}")))?;
    let status = response.status();
    let text = response
        .text()
        .map_err(|err| GtcError::message(format!("failed to read admin response body: {err}")))?;
    if !status.is_success() {
        let snippet = text.trim();
        let snippet = if snippet.len() > 400 {
            format!("{}...", &snippet[..400])
        } else {
            snippet.to_string()
        };
        return Err(GtcError::message(format!(
            "admin request {} {} failed with status {}: {}",
            &method_name,
            path,
            status.as_u16(),
            snippet
        )));
    }
    let json = parse_admin_response(&text)?;
    Ok(json)
}

fn load_local_setup_answers(bundle_dir: &Path) -> GtcResult<serde_json::Map<String, JsonValue>> {
    let config_root = bundle_dir.join("state").join("config");
    if !config_root.exists() {
        return Ok(serde_json::Map::new());
    }

    let mut entries = Vec::new();
    for dir_entry in fs::read_dir(&config_root)
        .map_err(|err| GtcError::io(format!("failed to read {}", config_root.display()), err))?
    {
        let dir_entry =
            dir_entry.map_err(|err| GtcError::message(format!("failed to read dir entry: {err}")))?;
        if !dir_entry
            .file_type()
            .map_err(|err| GtcError::message(format!("failed to stat dir entry: {err}")))?
            .is_dir()
        {
            continue;
        }
        let provider_id = dir_entry.file_name().to_string_lossy().to_string();
        let path = dir_entry.path().join("setup-answers.json");
        if !path.exists() {
            continue;
        }
        let raw = fs::read_to_string(&path)
            .map_err(|err| GtcError::io(format!("failed to read {}", path.display()), err))?;
        let answers: JsonValue = serde_json::from_str(&raw).map_err(|err| {
            GtcError::message(format!("failed to parse {}: {err}", path.display()))
        })?;
        if answers.as_object().is_some_and(|m| !m.is_empty()) {
            entries.push((provider_id, answers));
        }
    }

    entries.sort_by(|a, b| {
        let a_key = if a.0 == "messaging-webchat-gui" { 0 } else { 1 };
        let b_key = if b.0 == "messaging-webchat-gui" { 0 } else { 1 };
        a_key.cmp(&b_key).then_with(|| a.0.cmp(&b.0))
    });
    Ok(entries.into_iter().collect())
}

pub(crate) fn replay_remote_setup_answers(
    bundle_dir: &Path,
    target: &str,
    requested_tenant: &str,
    team: Option<&str>,
) -> GtcResult<()> {
    let setup_answers = load_local_setup_answers(bundle_dir)?;
    if setup_answers.is_empty() {
        return Ok(());
    }

    let mut session = resolve_remote_admin_session(bundle_dir, target)?;
    let client = build_remote_admin_client(&session.context)?;

    let replay_result = (|| -> GtcResult<()> {
        let setup_body = json!({
            "tenant": requested_tenant,
            "team": team,
            "answers": setup_answers,
        });
        let mut last_error: Option<GtcError> = None;
        for attempt in 1..=12 {
            match remote_admin_json_request(
                &client,
                &session.context,
                reqwest::Method::POST,
                "/setup",
                Some(&setup_body),
            ) {
                Ok(_) => return Ok(()),
                Err(err) => {
                    let message = err.to_string();
                    let retryable = message.contains("status 401")
                        || message.contains("status 403")
                        || message.contains("status 404")
                        || message.contains("status 409")
                        || message.contains("status 429")
                        || message.contains("status 500")
                        || message.contains("status 502")
                        || message.contains("status 503")
                        || message.contains("status 504")
                        || message.contains("curl: (7)")
                        || message.contains("Couldn't connect to server")
                        || message.contains("Connection refused");
                    last_error = Some(err);
                    if retryable && attempt < 12 {
                        thread::sleep(Duration::from_secs(5));
                        continue;
                    }
                    break;
                }
            }
        }
        Err(last_error.unwrap_or_else(|| {
            GtcError::message("remote setup replay failed with no error".to_string())
        }))?;
        Ok(())
    })();

    if let Some(mut child) = session.tunnel_child.take() {
        let _ = child.kill();
        let _ = child.wait();
    }

    replay_result
}

fn load_admin_public_endpoint_from_local_deploy_outputs(
    bundle_dir: &Path,
    target: &str,
) -> GtcResult<String> {
    let mut roots = vec![bundle_dir.to_path_buf()];
    let mut current = bundle_dir.parent();
    while let Some(parent) = current {
        roots.push(parent.to_path_buf());
        current = parent.parent();
    }

    let mut stack = roots
        .into_iter()
        .map(|root| root.join(".greentic").join("deploy").join(target))
        .collect::<Vec<_>>();
    let mut newest: Option<(SystemTime, PathBuf)> = None;
    while let Some(path) = stack.pop() {
        let entries = match fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let entry_path = entry.path();
            if entry_path.is_dir() {
                stack.push(entry_path);
                continue;
            }
            if entry_path.file_name().and_then(|n| n.to_str()) != Some("terraform-outputs.json") {
                continue;
            }
            let modified = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .unwrap_or(UNIX_EPOCH);
            match &newest {
                Some((current_modified, _)) if modified <= *current_modified => {}
                _ => newest = Some((modified, entry_path)),
            }
        }
    }

    let Some((_, path)) = newest else {
        return Err(GtcError::message(format!(
            "no terraform-outputs.json found under bundle or ancestor .greentic/deploy roots for target {target}"
        )));
    };
    let raw = fs::read_to_string(&path).map_err(|err| {
        GtcError::io(
            format!("failed to read terraform outputs {}", path.display()),
            err,
        )
    })?;
    let outputs: std::collections::BTreeMap<String, TerraformOutputValue> =
        serde_json::from_str(&raw).map_err(|err| {
            GtcError::json(
                format!("failed to parse terraform outputs {}", path.display()),
                err,
            )
        })?;
    outputs
        .get("admin_public_endpoint")
        .and_then(|output| output.value.as_str())
        .map(ToString::to_string)
        .ok_or_else(|| {
            GtcError::message(format!(
                "terraform outputs {} did not contain admin_public_endpoint.value",
                path.display()
            ))
        })
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
