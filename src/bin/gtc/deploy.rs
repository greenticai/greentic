#[path = "deploy/bundle_resolution.rs"]
mod bundle_resolution;
#[path = "deploy/cloud_deploy.rs"]
mod cloud_deploy;
#[path = "deploy/start_stop.rs"]
mod start_stop;

use std::path::PathBuf;
use std::process::Command as ProcessCommand;

use tempfile::TempDir;
use zeroize::Zeroizing;

#[allow(unused_imports)]
pub(super) use bundle_resolution::{
    detect_bundle_root, fingerprint_bundle_dir, normalize_bundle_fingerprint,
    resolve_local_mutable_bundle_dir,
};
#[allow(unused_imports)]
pub(super) use cloud_deploy::{
    canonical_provider_pack_filename_for_gtc, resolve_canonical_target_provider_pack_from,
    resolve_deploy_app_pack_path, resolve_target_provider_pack, validate_cloud_deploy_inputs,
    write_single_vm_spec,
};
#[cfg(test)]
#[allow(unused_imports)]
pub(super) use cloud_deploy::{default_operator_image_for_target, default_target_variable_for_gtc};
#[allow(unused_imports)]
pub(super) use gtc::start_stop_parsing::{parse_start_request, parse_stop_request};
#[allow(unused_imports)]
pub(super) use start_stop::{
    parse_start_cli_options, parse_stop_cli_options, run_start, run_stop, select_start_target,
};

#[derive(Debug)]
pub(super) struct StartBundleResolution {
    pub(super) bundle_dir: PathBuf,
    pub(super) deployment_key: String,
    pub(super) deploy_artifact: Option<PathBuf>,
    pub(super) _hold: Option<TempDir>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StartTarget {
    Runtime,
    SingleVm,
    Aws,
    Gcp,
    Azure,
}

impl StartTarget {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            StartTarget::Runtime => "runtime",
            StartTarget::SingleVm => "single-vm",
            StartTarget::Aws => "aws",
            StartTarget::Gcp => "gcp",
            StartTarget::Azure => "azure",
        }
    }
}

#[derive(Debug)]
pub(super) struct StartCliOptions {
    pub(super) start_args: Vec<String>,
    pub(super) explicit_target: Option<StartTarget>,
    pub(super) environment: Option<String>,
    pub(super) provider_pack: Option<PathBuf>,
    pub(super) app_pack: Option<PathBuf>,
    pub(super) deploy_bundle_source: Option<String>,
}

#[derive(Debug)]
pub(super) struct StopCliOptions {
    pub(super) stop_args: Vec<String>,
    pub(super) explicit_target: Option<StartTarget>,
    pub(super) environment: Option<String>,
    pub(super) provider_pack: Option<PathBuf>,
    pub(super) app_pack: Option<PathBuf>,
    pub(super) destroy: bool,
}

pub(super) struct ChildProcessEnv {
    vars: Vec<(String, Zeroizing<String>)>,
}

impl ChildProcessEnv {
    pub(super) fn new() -> Self {
        Self { vars: Vec::new() }
    }

    pub(super) fn set<K, V>(&mut self, key: K, value: V)
    where
        K: Into<String>,
        V: Into<String>,
    {
        self.vars.push((key.into(), Zeroizing::new(value.into())));
    }

    pub(super) fn extend(&mut self, other: Self) {
        self.vars.extend(other.vars);
    }

    pub(super) fn apply(&self, process: &mut ProcessCommand) {
        for (key, value) in &self.vars {
            process.env(key, value.as_str());
        }
    }
}
