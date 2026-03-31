use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use clap::ArgMatches;
use directories::BaseDirs;
use gtc::error::{GtcError, GtcResult};
use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose, SanType,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::deploy::resolve_local_mutable_bundle_dir;

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

    let deploy_dir = match resolve_latest_aws_deploy_dir(&bundle_dir) {
        Ok(path) => path,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    let outputs_path = deploy_dir.join("terraform-outputs.json");
    let outputs = match load_terraform_outputs(&outputs_path) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    let Some(admin_ca_secret_ref) = terraform_output_string(&outputs, "admin_ca_secret_ref") else {
        eprintln!(
            "missing admin_ca_secret_ref in {}; deploy the bundle first",
            outputs_path.display()
        );
        return 1;
    };

    let Some(region) = aws_region_from_secret_arn(&admin_ca_secret_ref) else {
        eprintln!("failed to derive AWS region from admin secret ref");
        return 1;
    };
    let Some(name_prefix) = deploy_name_prefix_from_secret_arn(&admin_ca_secret_ref) else {
        eprintln!("failed to derive deploy name prefix from admin secret ref");
        return 1;
    };

    let cluster = format!("{name_prefix}-cluster");
    let service = format!("{name_prefix}-service");

    let task_arn = match aws_cli_capture(
        &[
            "ecs",
            "list-tasks",
            "--region",
            &region,
            "--cluster",
            &cluster,
            "--service-name",
            &service,
            "--query",
            "taskArns[0]",
            "--output",
            "text",
        ],
        "aws ecs list-tasks",
    ) {
        Ok(value) if !value.is_empty() && value != "None" => value,
        Ok(_) => {
            eprintln!("no running ECS task found for service {service}");
            return 1;
        }
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };

    let runtime_query = format!("tasks[0].containers[?name=='{container}'].runtimeId | [0]");
    let runtime_id = match aws_cli_capture(
        &[
            "ecs",
            "describe-tasks",
            "--region",
            &region,
            "--cluster",
            &cluster,
            "--tasks",
            &task_arn,
            "--query",
            &runtime_query,
            "--output",
            "text",
        ],
        "aws ecs describe-tasks",
    ) {
        Ok(value) if !value.is_empty() && value != "None" => value,
        Ok(_) => {
            eprintln!("no runtimeId found for container {container}");
            return 1;
        }
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };

    let Some(task_id) = task_id_from_arn(&task_arn) else {
        eprintln!("failed to derive task id from task ARN");
        return 1;
    };

    if let Err(err) = maybe_write_tunnel_admin_certs(&bundle_dir, &outputs, &region, &name_prefix) {
        eprintln!("{err}");
        return 1;
    }

    let target = format!("ecs:{cluster}_{task_id}_{runtime_id}");
    let parameters = format!(
        "{{\"host\":[\"127.0.0.1\"],\"portNumber\":[\"8433\"],\"localPortNumber\":[\"{local_port}\"]}}"
    );

    println!("Opening admin tunnel on https://127.0.0.1:{local_port}");
    let cert_dir = tunnel_admin_cert_dir(&bundle_dir, &name_prefix);
    if cert_dir.is_dir() {
        println!("admin certs: {}", cert_dir.display());
        println!(
            "example: curl --cacert {0}/ca.crt --cert {0}/client.crt --key {0}/client.key https://127.0.0.1:{1}/admin/v1/health",
            cert_dir.display(),
            local_port
        );
    }
    if let Some(value) = terraform_output_string(&outputs, "admin_client_cert_secret_ref") {
        println!("admin_client_cert_secret_ref: {value}");
    } else {
        println!("note: this deployment does not publish admin client cert refs yet");
    }
    if let Some(value) = terraform_output_string(&outputs, "admin_client_key_secret_ref") {
        println!("admin_client_key_secret_ref: {value}");
    }
    println!("Press Ctrl+C to stop.");

    let status = ProcessCommand::new("aws")
        .args([
            "ssm",
            "start-session",
            "--region",
            &region,
            "--target",
            &target,
            "--document-name",
            "AWS-StartPortForwardingSessionToRemoteHost",
            "--parameters",
            &parameters,
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
            eprintln!("failed to start aws ssm session: {err}");
            1
        }
    }
}

fn resolve_latest_aws_deploy_dir(bundle_dir: &Path) -> Result<PathBuf, String> {
    let base_dirs = BaseDirs::new()
        .ok_or_else(|| "failed to resolve base directories for aws deploy state".to_string())?;
    let candidates = [
        bundle_dir.join(".greentic").join("deploy").join("aws"),
        bundle_dir
            .parent()
            .map(|parent| parent.join(".greentic").join("deploy").join("aws"))
            .unwrap_or_default(),
        base_dirs
            .home_dir()
            .join(".greentic")
            .join("deploy")
            .join("aws"),
    ];
    let mut latest: Option<(SystemTime, PathBuf)> = None;
    for root in candidates {
        if root.as_os_str().is_empty() || !root.exists() {
            continue;
        }
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            let entries = fs::read_dir(&dir)
                .map_err(|err| format!("failed to read deploy dir {}: {err}", dir.display()))?;
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let outputs = path.join("terraform-outputs.json");
                    if outputs.is_file() {
                        let modified = fs::metadata(&outputs)
                            .and_then(|meta| meta.modified())
                            .unwrap_or(UNIX_EPOCH);
                        match latest.as_ref() {
                            Some((current, _)) if modified <= *current => {}
                            _ => latest = Some((modified, path.clone())),
                        }
                    }
                    stack.push(path);
                }
            }
        }
    }

    latest
        .map(|(_, path)| path)
        .ok_or_else(|| {
            format!(
                "aws deploy state not found under {}, its parent workspace, or ~/.greentic/deploy/aws; deploy the bundle first",
                bundle_dir.join(".greentic").join("deploy").join("aws").display()
            )
        })
}

fn load_terraform_outputs(path: &Path) -> Result<Value, String> {
    let raw = fs::read_to_string(path)
        .map_err(|err| format!("failed to read terraform outputs {}: {err}", path.display()))?;
    serde_json::from_str(&raw).map_err(|err| {
        format!(
            "failed to parse terraform outputs {}: {err}",
            path.display()
        )
    })
}

fn terraform_output_string(outputs: &Value, key: &str) -> Option<String> {
    outputs
        .get(key)
        .and_then(|value| value.get("value"))
        .and_then(Value::as_str)
        .map(|value| value.to_string())
}

fn aws_region_from_secret_arn(secret_arn: &str) -> Option<String> {
    secret_arn.split(':').nth(3).map(|value| value.to_string())
}

fn tunnel_admin_cert_dir(bundle_dir: &Path, deploy_name_prefix: &str) -> PathBuf {
    bundle_dir
        .join(".greentic")
        .join("admin")
        .join("tunnels")
        .join(deploy_name_prefix)
}

fn maybe_write_tunnel_admin_certs(
    bundle_dir: &Path,
    outputs: &Value,
    region: &str,
    deploy_name_prefix: &str,
) -> Result<(), String> {
    let Some(client_cert_ref) = terraform_output_string(outputs, "admin_client_cert_secret_ref")
    else {
        return Ok(());
    };
    let Some(client_key_ref) = terraform_output_string(outputs, "admin_client_key_secret_ref")
    else {
        return Ok(());
    };
    let Some(ca_ref) = terraform_output_string(outputs, "admin_ca_secret_ref") else {
        return Ok(());
    };

    let cert_dir = tunnel_admin_cert_dir(bundle_dir, deploy_name_prefix);
    fs::create_dir_all(&cert_dir).map_err(|err| {
        format!(
            "failed to create tunnel cert dir {}: {err}",
            cert_dir.display()
        )
    })?;

    let ca_pem = aws_cli_capture(
        &[
            "secretsmanager",
            "get-secret-value",
            "--region",
            region,
            "--secret-id",
            &ca_ref,
            "--query",
            "SecretString",
            "--output",
            "text",
        ],
        "aws secretsmanager get-secret-value (admin ca)",
    )?;
    let client_cert_pem = aws_cli_capture(
        &[
            "secretsmanager",
            "get-secret-value",
            "--region",
            region,
            "--secret-id",
            &client_cert_ref,
            "--query",
            "SecretString",
            "--output",
            "text",
        ],
        "aws secretsmanager get-secret-value (admin client cert)",
    )?;
    let client_key_pem = aws_cli_capture(
        &[
            "secretsmanager",
            "get-secret-value",
            "--region",
            region,
            "--secret-id",
            &client_key_ref,
            "--query",
            "SecretString",
            "--output",
            "text",
        ],
        "aws secretsmanager get-secret-value (admin client key)",
    )?;

    fs::write(cert_dir.join("ca.crt"), ca_pem).map_err(|err| {
        format!(
            "failed to write {}: {err}",
            cert_dir.join("ca.crt").display()
        )
    })?;
    fs::write(cert_dir.join("client.crt"), client_cert_pem).map_err(|err| {
        format!(
            "failed to write {}: {err}",
            cert_dir.join("client.crt").display()
        )
    })?;
    fs::write(cert_dir.join("client.key"), client_key_pem).map_err(|err| {
        format!(
            "failed to write {}: {err}",
            cert_dir.join("client.key").display()
        )
    })?;

    Ok(())
}

fn deploy_name_prefix_from_secret_arn(secret_arn: &str) -> Option<String> {
    let marker = ":secret:greentic/admin/";
    let start = secret_arn.find(marker)? + marker.len();
    let rest = &secret_arn[start..];
    let prefix = rest.split('/').next()?;
    if prefix.is_empty() {
        None
    } else {
        Some(prefix.to_string())
    }
}

fn task_id_from_arn(task_arn: &str) -> Option<String> {
    task_arn.rsplit('/').next().map(|value| value.to_string())
}

fn aws_cli_capture(args: &[&str], label: &str) -> Result<String, String> {
    let output = ProcessCommand::new("aws")
        .args(args)
        .output()
        .map_err(|err| format!("failed to launch {label}: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            return Err(format!("{label} failed with status {}", output.status));
        }
        return Err(format!("{label} failed: {stderr}"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
