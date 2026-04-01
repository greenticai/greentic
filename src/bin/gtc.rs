#[path = "gtc/admin.rs"]
mod admin;
#[path = "gtc/archive.rs"]
mod archive;
#[path = "gtc/cli.rs"]
mod cli;
#[path = "gtc/commands.rs"]
mod commands;
#[path = "gtc/deploy.rs"]
mod deploy;
#[path = "gtc/i18n.rs"]
mod i18n_support;
#[path = "gtc/install.rs"]
mod install;
#[path = "gtc/process.rs"]
mod process;
#[path = "gtc/prompt.rs"]
mod prompt;
#[path = "gtc/router.rs"]
mod router;

use std::path::Path;

#[cfg(test)]
#[allow(unused_imports)]
use admin::{
    AdminRegistryDocument, admin_registry_path, ensure_admin_certs_ready,
    remove_admin_registry_entry, resolve_admin_cert_dir, save_admin_registry,
    upsert_admin_registry_entry,
};
#[cfg(test)]
#[allow(unused_imports)]
use archive::{extract_tar_archive, extract_zip_bytes};
#[cfg(test)]
#[allow(unused_imports)]
use cli::build_cli;
use commands::run;
#[cfg(test)]
#[allow(unused_imports)]
use deploy::resolve_local_mutable_bundle_dir;
#[cfg(test)]
#[allow(unused_imports)]
use deploy::{ChildProcessEnv, StartTarget, default_operator_image_for_target};
#[cfg(test)]
#[allow(unused_imports)]
use deploy::{
    StartBundleResolution, detect_bundle_root, fingerprint_bundle_dir,
    normalize_bundle_fingerprint, parse_start_cli_options, parse_start_request,
    parse_stop_cli_options, parse_stop_request, resolve_canonical_target_provider_pack_from,
    resolve_deploy_app_pack_path, resolve_target_provider_pack, select_start_target,
    validate_cloud_deploy_inputs, write_single_vm_spec,
};
use gtc::perf_targets::sha256_file as perf_sha256_file;
#[cfg(test)]
#[allow(unused_imports)]
use install::fetch_https_bytes;
#[cfg(test)]
#[allow(unused_imports)]
use install::{
    list_files_recursive, normalize_expected_sha256, normalize_install_arch, resolve_tenant_key,
    rewrite_store_tenant_placeholder, should_send_auth_header, tenant_env_var_name,
    verify_sha256_digest,
};
#[cfg(test)]
#[allow(unused_imports)]
use process::{apply_default_deploy_env_for_target, resolve_companion_binary_from};
#[cfg(test)]
#[allow(unused_imports)]
use prompt::parse_prompt_choice;
#[cfg(test)]
#[allow(unused_imports)]
use prompt::{prompt_optional_secret, prompt_secret};
#[cfg(test)]
#[allow(unused_imports)]
use router::{
    build_wizard_args, collect_tail, detect_locale, locale_from_args, route_passthrough_subcommand,
};

const DEV_BIN: &str = "greentic-dev";

const OP_BIN: &str = "greentic-operator";
const BUNDLE_BIN: &str = "greentic-bundle";
const DEPLOYER_BIN: &str = "greentic-deployer";
const SETUP_BIN: &str = "greentic-setup";
const START_BIN: &str = "greentic-start";
#[cfg(test)]
const DEFAULT_GHCR_OPERATOR_IMAGE: &str = "ghcr.io/greenticai/greentic-start-distroless@sha256:a7f4741a1206900b73a77c5e40860c2695206274374546dd3bb9cab8e752f79b";
#[cfg(test)]
const DEFAULT_GCP_OPERATOR_IMAGE: &str = "europe-west1-docker.pkg.dev/x-plateau-483512-p6/greentic-images/greentic-start-distroless@sha256:555fb6ebdac836c16c5c11fce0f4080a0d7ccda03abd9e89bb9d561280ca67db";
#[cfg(test)]
const DEFAULT_OPERATOR_IMAGE_DIGEST: &str =
    "sha256:a7f4741a1206900b73a77c5e40860c2695206274374546dd3bb9cab8e752f79b";
fn main() {
    let raw_args: Vec<String> = std::env::args().collect();
    let exit_code = run(raw_args);
    std::process::exit(exit_code);
}

fn sha256_file(path: &Path) -> Result<String, String> {
    perf_sha256_file(path)
}

#[cfg(test)]
#[path = "gtc/tests.rs"]
mod tests;
