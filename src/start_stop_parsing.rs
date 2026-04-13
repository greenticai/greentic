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
    /// Whether the user explicitly set `--cloudflared` or `--ngrok` on the CLI.
    pub tunnel_explicit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopRequest {
    pub bundle: Option<String>,
    pub state_dir: Option<PathBuf>,
    pub tenant: String,
    pub team: String,
}

impl StartRequest {
    pub fn to_runtime_start_args(&self, locale: &str) -> Vec<String> {
        let mut args = vec![
            "--locale".to_string(),
            locale.to_string(),
            "start".to_string(),
        ];
        if let Some(bundle) = self.bundle.as_deref() {
            args.push("--bundle".to_string());
            args.push(bundle.to_string());
        }
        if let Some(tenant) = self.tenant.as_deref() {
            args.push("--tenant".to_string());
            args.push(tenant.to_string());
        }
        if let Some(team) = self.team.as_deref() {
            args.push("--team".to_string());
            args.push(team.to_string());
        }
        if self.no_nats {
            args.push("--no-nats".to_string());
        }
        args.push("--nats".to_string());
        args.push(self.nats.as_cli_value().to_string());
        if let Some(nats_url) = self.nats_url.as_deref() {
            args.push("--nats-url".to_string());
            args.push(nats_url.to_string());
        }
        if let Some(config) = self.config.as_deref() {
            args.push("--config".to_string());
            args.push(config.display().to_string());
        }
        // Only pass tunnel flags when explicitly set, so greentic-start
        // can apply its own defaults (tunnel.json / deployer auto-detect).
        if self.tunnel_explicit {
            args.push("--cloudflared".to_string());
            args.push(self.cloudflared.as_cli_value().to_string());
            args.push("--ngrok".to_string());
            args.push(self.ngrok.as_cli_value().to_string());
        }
        if let Some(binary) = self.cloudflared_binary.as_deref() {
            args.push("--cloudflared-binary".to_string());
            args.push(binary.display().to_string());
        }
        if let Some(binary) = self.ngrok_binary.as_deref() {
            args.push("--ngrok-binary".to_string());
            args.push(binary.display().to_string());
        }
        if let Some(binary) = self.runner_binary.as_deref() {
            args.push("--runner-binary".to_string());
            args.push(binary.display().to_string());
        }
        if !self.restart.is_empty() {
            let value = self
                .restart
                .iter()
                .map(RestartTarget::as_cli_value)
                .collect::<Vec<_>>()
                .join(",");
            args.push("--restart".to_string());
            args.push(value);
        }
        if let Some(log_dir) = self.log_dir.as_deref() {
            args.push("--log-dir".to_string());
            args.push(log_dir.display().to_string());
        }
        if self.verbose {
            args.push("--verbose".to_string());
        }
        if self.quiet {
            args.push("--quiet".to_string());
        }
        if self.admin {
            args.push("--admin".to_string());
        }
        args.push("--admin-port".to_string());
        args.push(self.admin_port.to_string());
        if let Some(certs_dir) = self.admin_certs_dir.as_deref() {
            args.push("--admin-certs-dir".to_string());
            args.push(certs_dir.display().to_string());
        }
        if !self.admin_allowed_clients.is_empty() {
            args.push("--admin-allowed-clients".to_string());
            args.push(self.admin_allowed_clients.join(","));
        }
        args
    }
}

impl StopRequest {
    pub fn to_runtime_stop_args(&self, locale: &str) -> Vec<String> {
        let mut args = vec![
            "--locale".to_string(),
            locale.to_string(),
            "stop".to_string(),
        ];
        if let Some(bundle) = self.bundle.as_deref() {
            args.push("--bundle".to_string());
            args.push(bundle.to_string());
        }
        if let Some(state_dir) = self.state_dir.as_deref() {
            args.push("--state-dir".to_string());
            args.push(state_dir.display().to_string());
        }
        args.push("--tenant".to_string());
        args.push(self.tenant.clone());
        args.push("--team".to_string());
        args.push(self.team.clone());
        args
    }
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
        cloudflared: CloudflaredModeArg::Off,
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
        tunnel_explicit: false,
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
                request.tunnel_explicit = true;
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
                request.tunnel_explicit = true;
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
                    request.tunnel_explicit = true;
                } else if let Some(value) = other.strip_prefix("--cloudflared-binary=") {
                    request.cloudflared_binary = Some(PathBuf::from(value));
                } else if let Some(value) = other.strip_prefix("--ngrok=") {
                    request.ngrok = parse_ngrok_mode(value)?;
                    request.tunnel_explicit = true;
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

impl NatsModeArg {
    fn as_cli_value(self) -> &'static str {
        match self {
            NatsModeArg::Off => "off",
            NatsModeArg::On => "on",
            NatsModeArg::External => "external",
        }
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

impl CloudflaredModeArg {
    fn as_cli_value(self) -> &'static str {
        match self {
            CloudflaredModeArg::On => "on",
            CloudflaredModeArg::Off => "off",
        }
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

impl NgrokModeArg {
    fn as_cli_value(self) -> &'static str {
        match self {
            NgrokModeArg::On => "on",
            NgrokModeArg::Off => "off",
        }
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

impl RestartTarget {
    fn as_cli_value(&self) -> &'static str {
        match self {
            RestartTarget::All => "all",
            RestartTarget::Cloudflared => "cloudflared",
            RestartTarget::Ngrok => "ngrok",
            RestartTarget::Nats => "nats",
            RestartTarget::Gateway => "gateway",
            RestartTarget::Egress => "egress",
            RestartTarget::Subscriptions => "subscriptions",
        }
    }
}
