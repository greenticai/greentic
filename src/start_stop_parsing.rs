use std::path::PathBuf;

use crate::error::{GtcError, GtcResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NatsModeArg {
    Off,
    On,
    External,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudflaredModeArg {
    On,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NgrokModeArg {
    On,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartTarget {
    All,
    Cloudflared,
    Ngrok,
    Nats,
    Gateway,
    Egress,
    Subscriptions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartRequest {
    pub bundle: Option<String>,
    pub tenant: Option<String>,
    pub team: Option<String>,
    pub no_nats: bool,
    pub nats: NatsModeArg,
    pub nats_url: Option<String>,
    pub config: Option<PathBuf>,
    pub cloudflared: CloudflaredModeArg,
    pub cloudflared_binary: Option<PathBuf>,
    pub ngrok: NgrokModeArg,
    pub ngrok_binary: Option<PathBuf>,
    pub runner_binary: Option<PathBuf>,
    pub restart: Vec<RestartTarget>,
    pub log_dir: Option<PathBuf>,
    pub verbose: bool,
    pub quiet: bool,
    pub admin: bool,
    pub admin_port: u16,
    pub admin_certs_dir: Option<PathBuf>,
    pub admin_allowed_clients: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopRequest {
    pub bundle: Option<String>,
    pub state_dir: Option<PathBuf>,
    pub tenant: String,
    pub team: String,
}

pub fn parse_start_request(tail: &[String], bundle_dir: PathBuf) -> GtcResult<StartRequest> {
    let mut request = StartRequest {
        bundle: Some(bundle_dir.display().to_string()),
        tenant: None,
        team: None,
        no_nats: false,
        nats: NatsModeArg::Off,
        nats_url: None,
        config: None,
        cloudflared: CloudflaredModeArg::On,
        cloudflared_binary: None,
        ngrok: NgrokModeArg::Off,
        ngrok_binary: None,
        runner_binary: None,
        restart: Vec::new(),
        log_dir: None,
        verbose: false,
        quiet: false,
        admin: false,
        admin_port: 8443,
        admin_certs_dir: None,
        admin_allowed_clients: Vec::new(),
    };

    let mut idx = 0usize;
    while idx < tail.len() {
        let arg = &tail[idx];
        match arg.as_str() {
            "--tenant" => {
                idx += 1;
                request.tenant = Some(required_value(tail, idx, "--tenant")?);
            }
            "--team" => {
                idx += 1;
                request.team = Some(required_value(tail, idx, "--team")?);
            }
            "--no-nats" => request.no_nats = true,
            "--nats" => {
                idx += 1;
                request.nats = parse_nats_mode(&required_value(tail, idx, "--nats")?)?;
            }
            "--nats-url" => {
                idx += 1;
                request.nats_url = Some(required_value(tail, idx, "--nats-url")?);
            }
            "--config" => {
                idx += 1;
                request.config = Some(PathBuf::from(required_value(tail, idx, "--config")?));
            }
            "--cloudflared" => {
                idx += 1;
                request.cloudflared =
                    parse_cloudflared_mode(&required_value(tail, idx, "--cloudflared")?)?;
            }
            "--cloudflared-binary" => {
                idx += 1;
                request.cloudflared_binary = Some(PathBuf::from(required_value(
                    tail,
                    idx,
                    "--cloudflared-binary",
                )?));
            }
            "--ngrok" => {
                idx += 1;
                request.ngrok = parse_ngrok_mode(&required_value(tail, idx, "--ngrok")?)?;
            }
            "--ngrok-binary" => {
                idx += 1;
                request.ngrok_binary =
                    Some(PathBuf::from(required_value(tail, idx, "--ngrok-binary")?));
            }
            "--runner-binary" => {
                idx += 1;
                request.runner_binary =
                    Some(PathBuf::from(required_value(tail, idx, "--runner-binary")?));
            }
            "--restart" => {
                idx += 1;
                let value = required_value(tail, idx, "--restart")?;
                for part in value.split(',').filter(|part| !part.is_empty()) {
                    request.restart.push(parse_restart_target(part)?);
                }
            }
            "--log-dir" => {
                idx += 1;
                request.log_dir = Some(PathBuf::from(required_value(tail, idx, "--log-dir")?));
            }
            "--admin" => request.admin = true,
            "--admin-port" => {
                idx += 1;
                request.admin_port = required_value(tail, idx, "--admin-port")?
                    .parse()
                    .map_err(|_| GtcError::message("invalid --admin-port"))?;
            }
            "--admin-certs-dir" => {
                idx += 1;
                request.admin_certs_dir = Some(PathBuf::from(required_value(
                    tail,
                    idx,
                    "--admin-certs-dir",
                )?));
            }
            "--admin-allowed-clients" => {
                idx += 1;
                let value = required_value(tail, idx, "--admin-allowed-clients")?;
                request.admin_allowed_clients.extend(
                    value
                        .split(',')
                        .filter(|part| !part.is_empty())
                        .map(|part| part.to_string()),
                );
            }
            "--verbose" => request.verbose = true,
            "--quiet" => request.quiet = true,
            "--bundle" => {
                return Err(GtcError::message(
                    "--bundle is managed by gtc start; pass the bundle ref as the main argument",
                ));
            }
            other => {
                if let Some(value) = other.strip_prefix("--tenant=") {
                    request.tenant = Some(value.to_string());
                } else if let Some(value) = other.strip_prefix("--team=") {
                    request.team = Some(value.to_string());
                } else if let Some(value) = other.strip_prefix("--nats=") {
                    request.nats = parse_nats_mode(value)?;
                } else if let Some(value) = other.strip_prefix("--nats-url=") {
                    request.nats_url = Some(value.to_string());
                } else if let Some(value) = other.strip_prefix("--config=") {
                    request.config = Some(PathBuf::from(value));
                } else if let Some(value) = other.strip_prefix("--cloudflared=") {
                    request.cloudflared = parse_cloudflared_mode(value)?;
                } else if let Some(value) = other.strip_prefix("--cloudflared-binary=") {
                    request.cloudflared_binary = Some(PathBuf::from(value));
                } else if let Some(value) = other.strip_prefix("--ngrok=") {
                    request.ngrok = parse_ngrok_mode(value)?;
                } else if let Some(value) = other.strip_prefix("--ngrok-binary=") {
                    request.ngrok_binary = Some(PathBuf::from(value));
                } else if let Some(value) = other.strip_prefix("--runner-binary=") {
                    request.runner_binary = Some(PathBuf::from(value));
                } else if let Some(value) = other.strip_prefix("--restart=") {
                    for part in value.split(',').filter(|part| !part.is_empty()) {
                        request.restart.push(parse_restart_target(part)?);
                    }
                } else if let Some(value) = other.strip_prefix("--log-dir=") {
                    request.log_dir = Some(PathBuf::from(value));
                } else if other == "--admin" {
                    request.admin = true;
                } else if let Some(value) = other.strip_prefix("--admin-port=") {
                    request.admin_port = value
                        .parse()
                        .map_err(|_| GtcError::message("invalid --admin-port value"))?;
                } else if let Some(value) = other.strip_prefix("--admin-certs-dir=") {
                    request.admin_certs_dir = Some(PathBuf::from(value));
                } else if let Some(value) = other.strip_prefix("--admin-allowed-clients=") {
                    request.admin_allowed_clients.extend(
                        value
                            .split(',')
                            .filter(|part| !part.is_empty())
                            .map(|part| part.to_string()),
                    );
                } else if other.starts_with("--bundle=") {
                    return Err(GtcError::message(
                        "--bundle is managed by gtc start; pass the bundle ref as the main argument",
                    ));
                } else {
                    return Err(GtcError::message(format!(
                        "unsupported start argument: {other}"
                    )));
                }
            }
        }
        idx += 1;
    }

    Ok(request)
}

pub fn parse_stop_request(tail: &[String], bundle_dir: PathBuf) -> GtcResult<StopRequest> {
    let mut request = StopRequest {
        bundle: Some(bundle_dir.display().to_string()),
        state_dir: None,
        tenant: "demo".to_string(),
        team: "default".to_string(),
    };

    let mut idx = 0usize;
    while idx < tail.len() {
        let arg = &tail[idx];
        match arg.as_str() {
            "--tenant" => {
                idx += 1;
                request.tenant = required_value(tail, idx, "--tenant")?;
            }
            "--team" => {
                idx += 1;
                request.team = required_value(tail, idx, "--team")?;
            }
            "--state-dir" => {
                idx += 1;
                request.state_dir = Some(PathBuf::from(required_value(tail, idx, "--state-dir")?));
            }
            "--bundle" => {
                return Err(GtcError::message(
                    "--bundle is managed by gtc stop; pass the bundle ref as the main argument",
                ));
            }
            other => {
                if let Some(value) = other.strip_prefix("--tenant=") {
                    request.tenant = value.to_string();
                } else if let Some(value) = other.strip_prefix("--team=") {
                    request.team = value.to_string();
                } else if let Some(value) = other.strip_prefix("--state-dir=") {
                    request.state_dir = Some(PathBuf::from(value));
                } else if other.starts_with("--bundle=") {
                    return Err(GtcError::message(
                        "--bundle is managed by gtc stop; pass the bundle ref as the main argument",
                    ));
                } else {
                    return Err(GtcError::message(format!(
                        "unsupported stop argument: {other}"
                    )));
                }
            }
        }
        idx += 1;
    }

    Ok(request)
}

fn required_value(args: &[String], idx: usize, flag: &str) -> GtcResult<String> {
    args.get(idx)
        .cloned()
        .ok_or_else(|| GtcError::message(format!("missing value for {flag}")))
}

fn parse_nats_mode(value: &str) -> GtcResult<NatsModeArg> {
    match value.trim() {
        "off" => Ok(NatsModeArg::Off),
        "on" => Ok(NatsModeArg::On),
        "external" => Ok(NatsModeArg::External),
        other => Err(GtcError::message(format!(
            "unsupported --nats value: {other}"
        ))),
    }
}

fn parse_cloudflared_mode(value: &str) -> GtcResult<CloudflaredModeArg> {
    match value.trim() {
        "on" => Ok(CloudflaredModeArg::On),
        "off" => Ok(CloudflaredModeArg::Off),
        other => Err(GtcError::message(format!(
            "unsupported --cloudflared value: {other}"
        ))),
    }
}

fn parse_ngrok_mode(value: &str) -> GtcResult<NgrokModeArg> {
    match value.trim() {
        "on" => Ok(NgrokModeArg::On),
        "off" => Ok(NgrokModeArg::Off),
        other => Err(GtcError::message(format!(
            "unsupported --ngrok value: {other}"
        ))),
    }
}

fn parse_restart_target(value: &str) -> GtcResult<RestartTarget> {
    match value.trim() {
        "all" => Ok(RestartTarget::All),
        "cloudflared" => Ok(RestartTarget::Cloudflared),
        "ngrok" => Ok(RestartTarget::Ngrok),
        "nats" => Ok(RestartTarget::Nats),
        "gateway" => Ok(RestartTarget::Gateway),
        "egress" => Ok(RestartTarget::Egress),
        "subscriptions" => Ok(RestartTarget::Subscriptions),
        other => Err(GtcError::message(format!(
            "unsupported --restart target: {other}"
        ))),
    }
}
