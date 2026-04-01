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
use serde::{Deserialize, Serialize};

use crate::DEPLOYER_BIN;
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

    let status = ProcessCommand::new(DEPLOYER_BIN)
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
