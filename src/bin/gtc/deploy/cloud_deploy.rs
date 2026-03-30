#[path = "cloud_deploy/deployment_state.rs"]
mod deployment_state;
#[path = "cloud_deploy/provider_packs.rs"]
mod provider_packs;
#[path = "cloud_deploy/single_vm.rs"]
mod single_vm;

use std::fs;
use std::io::Read;
use std::path::Path;

use crate::process::resolve_binary_in_dir;
use crate::prompt::{
    can_prompt_interactively, prompt_choice, prompt_non_empty, prompt_optional,
    prompt_optional_secret, prompt_secret, prompt_value_with_default,
};
use crate::{DEFAULT_GCP_OPERATOR_IMAGE, DEFAULT_GHCR_OPERATOR_IMAGE};
use greentic_types::decode_pack_manifest;
use gtc::config::{GtcConfig, OperatorImageSource};
use gtc::error::{GtcError, GtcResult};
use zip::ZipArchive;

use super::{ChildProcessEnv, StartTarget};

pub(crate) use deployment_state::{destroy_deployment, ensure_started_or_deployed};
pub(crate) use provider_packs::{
    resolve_canonical_target_provider_pack_from, resolve_deploy_app_pack_path,
    resolve_target_provider_pack,
};
pub(crate) use single_vm::write_single_vm_spec;

pub(crate) fn default_operator_image_for_target(target: StartTarget) -> Option<&'static str> {
    let config = GtcConfig::from_env();
    match target {
        StartTarget::Aws => Some(default_operator_image_for_name("aws", &config)),
        StartTarget::Gcp => Some(default_operator_image_for_name("gcp", &config)),
        StartTarget::Azure => Some(default_operator_image_for_name("azure", &config)),
        StartTarget::SingleVm | StartTarget::Runtime => None,
    }
}

fn default_operator_image_for_name(target: &str, config: &GtcConfig) -> &'static str {
    match config.operator_image_source(target) {
        OperatorImageSource::Ghcr => DEFAULT_GHCR_OPERATOR_IMAGE,
        OperatorImageSource::GcpArtifactRegistry => DEFAULT_GCP_OPERATOR_IMAGE,
    }
}

pub(crate) fn validate_cloud_deploy_inputs(
    target: StartTarget,
    remote_bundle_source: Option<&str>,
    bundle_dir: &Path,
    locale: &str,
) -> GtcResult<ChildProcessEnv> {
    let mut child_env = ChildProcessEnv::new();
    require_tool_in_path(
        "terraform",
        "install terraform and make sure it is available in PATH",
    )?;
    validate_public_base_url_for_static_routes(bundle_dir)?;
    match target {
        StartTarget::Aws => {
            child_env.extend(ensure_cloud_credentials(target, locale)?);
            child_env.extend(ensure_target_terraform_inputs(target)?);
            let remote_bundle_source = remote_bundle_source.ok_or_else(|| {
                GtcError::message("aws deploy requires a remote bundle source; pass --deploy-bundle-source https://.../bundle.gtbundle or set GREENTIC_DEPLOY_BUNDLE_SOURCE")
            })?;
            if !is_remote_bundle_source(remote_bundle_source) {
                return Err(GtcError::message(format!(
                    "aws deploy requires a remote bundle source, got local path: {remote_bundle_source}"
                )));
            }
            validate_bundle_registry_mapping_env(remote_bundle_source)?;
            Ok(child_env)
        }
        StartTarget::Gcp | StartTarget::Azure => {
            child_env.extend(ensure_cloud_credentials(target, locale)?);
            child_env.extend(ensure_target_terraform_inputs(target)?);
            let remote_bundle_source = remote_bundle_source.ok_or_else(|| {
                GtcError::message(format!(
                    "{} deploy requires a remote bundle source; pass --deploy-bundle-source https://.../bundle.gtbundle or set GREENTIC_DEPLOY_BUNDLE_SOURCE",
                    target.as_str()
                ))
            })?;
            if !is_remote_bundle_source(remote_bundle_source) {
                return Err(GtcError::message(format!(
                    "{} deploy requires a remote bundle source, got local path: {remote_bundle_source}",
                    target.as_str()
                )));
            }
            validate_bundle_registry_mapping_env(remote_bundle_source)?;
            Ok(child_env)
        }
        StartTarget::SingleVm | StartTarget::Runtime => Ok(child_env),
    }
}

fn validate_public_base_url_for_static_routes(bundle_dir: &Path) -> GtcResult<()> {
    if !bundle_declares_static_routes(bundle_dir)? {
        return Ok(());
    }
    Ok(())
}

fn bundle_declares_static_routes(bundle_dir: &Path) -> GtcResult<bool> {
    for root in [bundle_dir.join("providers"), bundle_dir.join("packs")] {
        if !root.exists() {
            continue;
        }
        if dir_declares_static_routes(&root)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn dir_declares_static_routes(root: &Path) -> GtcResult<bool> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir)
            .map_err(|err| GtcError::io(format!("failed to read {}", dir.display()), err))?;
        for entry in entries {
            let entry = entry
                .map_err(|err| GtcError::message(format!("failed to read dir entry: {err}")))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|err| GtcError::io(format!("failed to stat {}", path.display()), err))?;
            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("gtpack") {
                continue;
            }
            if pack_declares_static_routes(&path)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn pack_declares_static_routes(path: &Path) -> GtcResult<bool> {
    const EXT_STATIC_ROUTES_V1: &str = "greentic.static-routes.v1";
    let file = fs::File::open(path)
        .map_err(|err| GtcError::io(format!("failed to open {}", path.display()), err))?;
    let mut archive = ZipArchive::new(file).map_err(|err| {
        GtcError::message(format!(
            "failed to open zip archive {}: {err}",
            path.display()
        ))
    })?;
    let mut manifest_entry = archive.by_name("manifest.cbor").map_err(|err| {
        GtcError::message(format!(
            "failed to open manifest.cbor in {}: {err}",
            path.display()
        ))
    })?;
    let mut bytes = Vec::new();
    manifest_entry.read_to_end(&mut bytes).map_err(|err| {
        GtcError::message(format!(
            "failed to read manifest.cbor in {}: {err}",
            path.display()
        ))
    })?;
    let manifest = decode_pack_manifest(&bytes).map_err(|err| {
        GtcError::message(format!(
            "failed to decode pack manifest in {}: {err}",
            path.display()
        ))
    })?;
    Ok(manifest
        .extensions
        .as_ref()
        .is_some_and(|extensions| extensions.contains_key(EXT_STATIC_ROUTES_V1)))
}

fn validate_bundle_registry_mapping_env(bundle_source: &str) -> GtcResult<()> {
    let config = GtcConfig::from_env();
    if bundle_source.starts_with("repo://") {
        config.require_non_empty_var("GREENTIC_REPO_REGISTRY_BASE")?;
    }
    if bundle_source.starts_with("store://") {
        config.require_non_empty_var("GREENTIC_STORE_REGISTRY_BASE")?;
    }
    Ok(())
}

pub(super) fn append_bundle_registry_args(
    args: &mut Vec<String>,
    bundle_source: &str,
) -> GtcResult<()> {
    let config = GtcConfig::from_env();
    if bundle_source.starts_with("repo://") {
        let value = config.require_non_empty_var("GREENTIC_REPO_REGISTRY_BASE")?;
        args.push("--repo-registry-base".to_string());
        args.push(value);
    }
    if bundle_source.starts_with("store://") {
        let value = config.require_non_empty_var("GREENTIC_STORE_REGISTRY_BASE")?;
        args.push("--store-registry-base".to_string());
        args.push(value);
    }
    Ok(())
}

fn is_remote_bundle_source(value: &str) -> bool {
    matches_remote_bundle_ref(value)
}

fn matches_remote_bundle_ref(value: &str) -> bool {
    value.starts_with("http://")
        || value.starts_with("https://")
        || value.starts_with("oci://")
        || value.starts_with("repo://")
        || value.starts_with("store://")
}

fn missing_cloud_credentials_error(names: &[&str], help: &str) -> GtcError {
    GtcError::message(format!(
        "missing cloud credentials; {}. Expected one of: {}",
        help,
        names.join(", ")
    ))
}

fn ensure_cloud_credentials(target: StartTarget, locale: &str) -> GtcResult<ChildProcessEnv> {
    let (names, help) = match target {
        StartTarget::Aws => (
            &[
                "AWS_ACCESS_KEY_ID",
                "AWS_PROFILE",
                "AWS_DEFAULT_PROFILE",
                "AWS_WEB_IDENTITY_TOKEN_FILE",
            ][..],
            "set AWS credentials (for example AWS_ACCESS_KEY_ID/AWS_SECRET_ACCESS_KEY or AWS_PROFILE)",
        ),
        StartTarget::Azure => (
            &[
                "ARM_CLIENT_ID",
                "ARM_USE_OIDC",
                "AZURE_CLIENT_ID",
                "AZURE_TENANT_ID",
                "AZURE_SUBSCRIPTION_ID",
            ][..],
            "set Azure credentials (for example ARM_CLIENT_ID/ARM_TENANT_ID/ARM_SUBSCRIPTION_ID or the corresponding AZURE_* variables)",
        ),
        StartTarget::Gcp => (
            &[
                "GOOGLE_APPLICATION_CREDENTIALS",
                "GOOGLE_OAUTH_ACCESS_TOKEN",
                "CLOUDSDK_AUTH_ACCESS_TOKEN",
            ][..],
            "set GCP credentials (for example GOOGLE_APPLICATION_CREDENTIALS or CLOUDSDK_AUTH_ACCESS_TOKEN)",
        ),
        StartTarget::SingleVm | StartTarget::Runtime => return Ok(ChildProcessEnv::new()),
    };
    if cloud_credentials_satisfied(target) {
        return Ok(ChildProcessEnv::new());
    }
    if !can_prompt_interactively() {
        return Err(missing_cloud_credentials_error(names, help));
    }
    let _ = locale;
    println!(
        "Cloud credentials for {} are missing. gtc can collect them for this run.",
        target.as_str()
    );
    let env = match target {
        StartTarget::Aws => prompt_aws_credentials()?,
        StartTarget::Azure => prompt_azure_credentials()?,
        StartTarget::Gcp => prompt_gcp_credentials()?,
        StartTarget::SingleVm | StartTarget::Runtime => ChildProcessEnv::new(),
    };
    if cloud_credentials_satisfied(target) || !env.vars.is_empty() {
        Ok(env)
    } else {
        Err(missing_cloud_credentials_error(names, help))
    }
}

fn ensure_target_terraform_inputs(target: StartTarget) -> GtcResult<ChildProcessEnv> {
    let requirements: &[(&str, bool, Option<&str>)] = match target {
        StartTarget::Aws => &[(
            "GREENTIC_DEPLOY_TERRAFORM_VAR_REMOTE_STATE_BACKEND",
            true,
            None,
        )],
        StartTarget::Gcp => &[
            ("GREENTIC_DEPLOY_TERRAFORM_VAR_GCP_PROJECT_ID", true, None),
            (
                "GREENTIC_DEPLOY_TERRAFORM_VAR_GCP_REGION",
                true,
                Some("us-central1"),
            ),
        ],
        StartTarget::Azure => &[
            (
                "GREENTIC_DEPLOY_TERRAFORM_VAR_AZURE_KEY_VAULT_ID",
                true,
                None,
            ),
            (
                "GREENTIC_DEPLOY_TERRAFORM_VAR_AZURE_LOCATION",
                true,
                Some("westeurope"),
            ),
        ],
        StartTarget::SingleVm | StartTarget::Runtime => return Ok(ChildProcessEnv::new()),
    };
    if requirements.is_empty() {
        return Ok(ChildProcessEnv::new());
    }
    let missing: Vec<_> = requirements
        .iter()
        .filter(|(name, required, _)| *required && !env_var_present(name))
        .copied()
        .collect();
    if missing.is_empty() {
        return Ok(ChildProcessEnv::new());
    }
    if !can_prompt_interactively() {
        return Err(GtcError::message(format!(
            "missing required deployment configuration: {}",
            missing
                .iter()
                .map(|(name, _, _)| *name)
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    println!(
        "Additional {} deployment inputs are required for this run.",
        target.as_str()
    );
    let mut env = ChildProcessEnv::new();
    for (name, _, default) in missing {
        let prompt = match name {
            "GREENTIC_DEPLOY_TERRAFORM_VAR_REMOTE_STATE_BACKEND" => {
                "Terraform remote state backend:"
            }
            "GREENTIC_DEPLOY_TERRAFORM_VAR_GCP_PROJECT_ID" => "GCP project ID:",
            "GREENTIC_DEPLOY_TERRAFORM_VAR_GCP_REGION" => "GCP region:",
            "GREENTIC_DEPLOY_TERRAFORM_VAR_AZURE_KEY_VAULT_ID" => "Azure Key Vault resource ID:",
            "GREENTIC_DEPLOY_TERRAFORM_VAR_AZURE_LOCATION" => "Azure location:",
            _ => name,
        };
        let value = prompt_value_with_default(prompt, default)?;
        env.set(name, value);
    }
    Ok(env)
}

fn cloud_credentials_satisfied(target: StartTarget) -> bool {
    match target {
        StartTarget::Aws => {
            env_var_present("AWS_PROFILE")
                || env_var_present("AWS_DEFAULT_PROFILE")
                || env_var_present("AWS_WEB_IDENTITY_TOKEN_FILE")
                || (env_var_present("AWS_ACCESS_KEY_ID")
                    && env_var_present("AWS_SECRET_ACCESS_KEY"))
        }
        StartTarget::Azure => {
            (env_var_present("ARM_CLIENT_ID")
                && env_var_present("ARM_TENANT_ID")
                && env_var_present("ARM_SUBSCRIPTION_ID")
                && (env_var_present("ARM_CLIENT_SECRET") || env_var_present("ARM_USE_OIDC")))
                || (env_var_present("AZURE_CLIENT_ID")
                    && env_var_present("AZURE_TENANT_ID")
                    && env_var_present("AZURE_SUBSCRIPTION_ID"))
        }
        StartTarget::Gcp => {
            env_var_present("GOOGLE_APPLICATION_CREDENTIALS")
                || env_var_present("GOOGLE_OAUTH_ACCESS_TOKEN")
                || env_var_present("CLOUDSDK_AUTH_ACCESS_TOKEN")
        }
        StartTarget::SingleVm | StartTarget::Runtime => true,
    }
}

fn env_var_present(name: &str) -> bool {
    GtcConfig::from_env().non_empty_var(name).is_some()
}

enum PromptFieldKind {
    Required,
    Optional,
    Secret,
    OptionalSecret,
    Static(&'static str),
}

struct PromptField {
    env_name: &'static str,
    prompt: &'static str,
    kind: PromptFieldKind,
}

struct CredentialModeSpec {
    label: &'static str,
    fields: &'static [PromptField],
}

fn prompt_cloud_credentials(
    selection_prompt: &str,
    provider_name: &str,
    modes: &[CredentialModeSpec],
) -> GtcResult<ChildProcessEnv> {
    let mut options: Vec<&str> = modes.iter().map(|mode| mode.label).collect();
    options.push("Abort");
    let mode = prompt_choice(selection_prompt, &options)?;
    if mode >= modes.len() {
        return Err(GtcError::message(format!(
            "cloud deploy aborted before {provider_name} credentials were configured"
        )));
    }

    let mut env = ChildProcessEnv::new();
    for field in modes[mode].fields {
        match field.kind {
            PromptFieldKind::Required => {
                env.set(field.env_name, prompt_non_empty(field.prompt)?);
            }
            PromptFieldKind::Optional => {
                if let Some(value) = prompt_optional(field.prompt)? {
                    env.set(field.env_name, value);
                }
            }
            PromptFieldKind::Secret => {
                env.vars
                    .push((field.env_name.to_string(), prompt_secret(field.prompt)?));
            }
            PromptFieldKind::OptionalSecret => {
                if let Some(value) = prompt_optional_secret(field.prompt)? {
                    env.vars.push((field.env_name.to_string(), value));
                }
            }
            PromptFieldKind::Static(value) => env.set(field.env_name, value),
        }
    }

    Ok(env)
}

const AWS_ACCESS_KEY_PAIR_FIELDS: &[PromptField] = &[
    PromptField {
        env_name: "AWS_ACCESS_KEY_ID",
        prompt: "AWS access key ID:",
        kind: PromptFieldKind::Required,
    },
    PromptField {
        env_name: "AWS_SECRET_ACCESS_KEY",
        prompt: "AWS secret access key:",
        kind: PromptFieldKind::Secret,
    },
    PromptField {
        env_name: "AWS_SESSION_TOKEN",
        prompt: "AWS session token (optional):",
        kind: PromptFieldKind::OptionalSecret,
    },
    PromptField {
        env_name: "AWS_DEFAULT_REGION",
        prompt: "AWS default region (optional):",
        kind: PromptFieldKind::Optional,
    },
];

const AWS_PROFILE_FIELDS: &[PromptField] = &[
    PromptField {
        env_name: "AWS_PROFILE",
        prompt: "AWS profile:",
        kind: PromptFieldKind::Required,
    },
    PromptField {
        env_name: "AWS_DEFAULT_REGION",
        prompt: "AWS default region (optional):",
        kind: PromptFieldKind::Optional,
    },
];

const AWS_WEB_IDENTITY_FIELDS: &[PromptField] = &[
    PromptField {
        env_name: "AWS_WEB_IDENTITY_TOKEN_FILE",
        prompt: "AWS web identity token file:",
        kind: PromptFieldKind::Required,
    },
    PromptField {
        env_name: "AWS_ROLE_ARN",
        prompt: "AWS role ARN (optional):",
        kind: PromptFieldKind::Optional,
    },
];

const AWS_CREDENTIAL_MODES: &[CredentialModeSpec] = &[
    CredentialModeSpec {
        label: "Access key pair",
        fields: AWS_ACCESS_KEY_PAIR_FIELDS,
    },
    CredentialModeSpec {
        label: "AWS profile",
        fields: AWS_PROFILE_FIELDS,
    },
    CredentialModeSpec {
        label: "Web identity token file",
        fields: AWS_WEB_IDENTITY_FIELDS,
    },
];

const AZURE_SERVICE_PRINCIPAL_FIELDS: &[PromptField] = &[
    PromptField {
        env_name: "ARM_SUBSCRIPTION_ID",
        prompt: "Azure subscription ID:",
        kind: PromptFieldKind::Required,
    },
    PromptField {
        env_name: "ARM_TENANT_ID",
        prompt: "Azure tenant ID:",
        kind: PromptFieldKind::Required,
    },
    PromptField {
        env_name: "ARM_CLIENT_ID",
        prompt: "Azure client ID:",
        kind: PromptFieldKind::Required,
    },
    PromptField {
        env_name: "ARM_CLIENT_SECRET",
        prompt: "Azure client secret:",
        kind: PromptFieldKind::Secret,
    },
];

const AZURE_OIDC_FIELDS: &[PromptField] = &[
    PromptField {
        env_name: "ARM_SUBSCRIPTION_ID",
        prompt: "Azure subscription ID:",
        kind: PromptFieldKind::Required,
    },
    PromptField {
        env_name: "ARM_TENANT_ID",
        prompt: "Azure tenant ID:",
        kind: PromptFieldKind::Required,
    },
    PromptField {
        env_name: "ARM_CLIENT_ID",
        prompt: "Azure client ID:",
        kind: PromptFieldKind::Required,
    },
    PromptField {
        env_name: "ARM_USE_OIDC",
        prompt: "",
        kind: PromptFieldKind::Static("true"),
    },
];

const AZURE_CREDENTIAL_MODES: &[CredentialModeSpec] = &[
    CredentialModeSpec {
        label: "ARM service principal",
        fields: AZURE_SERVICE_PRINCIPAL_FIELDS,
    },
    CredentialModeSpec {
        label: "Azure OIDC",
        fields: AZURE_OIDC_FIELDS,
    },
];

const GCP_SERVICE_ACCOUNT_FIELDS: &[PromptField] = &[PromptField {
    env_name: "GOOGLE_APPLICATION_CREDENTIALS",
    prompt: "GOOGLE_APPLICATION_CREDENTIALS path:",
    kind: PromptFieldKind::Required,
}];

const GCP_ACCESS_TOKEN_FIELDS: &[PromptField] = &[PromptField {
    env_name: "CLOUDSDK_AUTH_ACCESS_TOKEN",
    prompt: "GCP access token:",
    kind: PromptFieldKind::Secret,
}];

const GCP_CREDENTIAL_MODES: &[CredentialModeSpec] = &[
    CredentialModeSpec {
        label: "Service account credentials file",
        fields: GCP_SERVICE_ACCOUNT_FIELDS,
    },
    CredentialModeSpec {
        label: "Access token",
        fields: GCP_ACCESS_TOKEN_FIELDS,
    },
];

fn prompt_aws_credentials() -> GtcResult<ChildProcessEnv> {
    prompt_cloud_credentials(
        "Select AWS credential input mode:",
        "AWS",
        AWS_CREDENTIAL_MODES,
    )
}

fn prompt_azure_credentials() -> GtcResult<ChildProcessEnv> {
    prompt_cloud_credentials(
        "Select Azure credential input mode:",
        "Azure",
        AZURE_CREDENTIAL_MODES,
    )
}

fn prompt_gcp_credentials() -> GtcResult<ChildProcessEnv> {
    prompt_cloud_credentials(
        "Select GCP credential input mode:",
        "GCP",
        GCP_CREDENTIAL_MODES,
    )
}

fn require_tool_in_path(binary: &str, help: &str) -> GtcResult<()> {
    if binary_in_path(binary) {
        Ok(())
    } else {
        Err(GtcError::message(format!(
            "required tool `{binary}` not found in PATH; {help}"
        )))
    }
}

fn binary_in_path(binary: &str) -> bool {
    std::env::var_os("PATH")
        .map(|path| {
            std::env::split_paths(&path).any(|dir| resolve_binary_in_dir(&dir, binary).is_some())
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::{
        append_bundle_registry_args, binary_in_path, cloud_credentials_satisfied,
        default_operator_image_for_target, dir_declares_static_routes,
        ensure_target_terraform_inputs, env_var_present, matches_remote_bundle_ref,
        require_tool_in_path, validate_bundle_registry_mapping_env, validate_cloud_deploy_inputs,
        validate_public_base_url_for_static_routes,
    };
    use crate::deploy::StartTarget;
    use crate::tests::env_test_lock;
    use gtc::config::GtcConfig;
    use std::env;
    use std::fs;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn matches_remote_bundle_ref_recognizes_supported_schemes() {
        assert!(matches_remote_bundle_ref(
            "https://example.com/demo.gtbundle"
        ));
        assert!(matches_remote_bundle_ref("oci://ghcr.io/demo:latest"));
        assert!(matches_remote_bundle_ref("repo://providers/demo:latest"));
        assert!(!matches_remote_bundle_ref("./bundle.gtbundle"));
    }

    #[test]
    fn require_env_var_errors_on_missing_or_blank_values() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            env::remove_var("TEST_REQUIRED_ENV");
        }
        assert!(
            GtcConfig::from_env()
                .require_non_empty_var("TEST_REQUIRED_ENV")
                .is_err()
        );
        unsafe {
            env::set_var("TEST_REQUIRED_ENV", " ");
        }
        assert!(
            GtcConfig::from_env()
                .require_non_empty_var("TEST_REQUIRED_ENV")
                .is_err()
        );
        unsafe {
            env::set_var("TEST_REQUIRED_ENV", "demo");
        }
        assert!(
            GtcConfig::from_env()
                .require_non_empty_var("TEST_REQUIRED_ENV")
                .is_ok()
        );
        unsafe {
            env::remove_var("TEST_REQUIRED_ENV");
        }
    }

    #[test]
    fn validate_bundle_registry_mapping_env_checks_required_bases() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            env::remove_var("GREENTIC_REPO_REGISTRY_BASE");
        }
        let err = validate_bundle_registry_mapping_env("repo://providers/demo:latest").unwrap_err();
        assert!(err.contains("GREENTIC_REPO_REGISTRY_BASE"));
    }

    #[test]
    fn append_bundle_registry_args_adds_registry_flags() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            env::set_var("GREENTIC_REPO_REGISTRY_BASE", "https://repo.example");
        }
        let mut args = Vec::new();
        append_bundle_registry_args(&mut args, "repo://providers/demo:latest").expect("args");
        unsafe {
            env::remove_var("GREENTIC_REPO_REGISTRY_BASE");
        }
        assert_eq!(
            args,
            vec![
                "--repo-registry-base".to_string(),
                "https://repo.example".to_string()
            ]
        );
    }

    #[test]
    fn append_bundle_registry_args_adds_store_flags_and_rejects_blank_values() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            env::set_var("GREENTIC_STORE_REGISTRY_BASE", "https://store.example");
        }
        let mut args = Vec::new();
        append_bundle_registry_args(&mut args, "store://packs/demo:latest").expect("args");
        assert_eq!(
            args,
            vec![
                "--store-registry-base".to_string(),
                "https://store.example".to_string()
            ]
        );

        unsafe {
            env::set_var("GREENTIC_STORE_REGISTRY_BASE", " ");
        }
        let err =
            append_bundle_registry_args(&mut Vec::new(), "store://packs/demo:latest").unwrap_err();
        unsafe {
            env::remove_var("GREENTIC_STORE_REGISTRY_BASE");
        }
        assert!(
            err.to_string()
                .contains("missing required environment variable: GREENTIC_STORE_REGISTRY_BASE")
        );
    }

    #[test]
    fn env_var_present_and_cloud_credentials_satisfied_track_aws_requirements() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            env::set_var("AWS_ACCESS_KEY_ID", "demo");
            env::remove_var("AWS_SECRET_ACCESS_KEY");
            env::remove_var("AWS_PROFILE");
            env::remove_var("AWS_DEFAULT_PROFILE");
            env::remove_var("AWS_WEB_IDENTITY_TOKEN_FILE");
        }
        assert!(env_var_present("AWS_ACCESS_KEY_ID"));
        assert!(!cloud_credentials_satisfied(StartTarget::Aws));
        unsafe {
            env::set_var("AWS_SECRET_ACCESS_KEY", "secret");
        }
        assert!(cloud_credentials_satisfied(StartTarget::Aws));
        unsafe {
            env::remove_var("AWS_ACCESS_KEY_ID");
            env::remove_var("AWS_SECRET_ACCESS_KEY");
        }
    }

    #[test]
    fn default_operator_image_for_target_returns_cloud_defaults_only() {
        assert!(default_operator_image_for_target(StartTarget::Aws).is_some());
        assert!(default_operator_image_for_target(StartTarget::Gcp).is_some());
        assert!(default_operator_image_for_target(StartTarget::Azure).is_some());
        assert_eq!(
            default_operator_image_for_target(StartTarget::Runtime),
            None
        );
    }

    #[test]
    fn ensure_target_terraform_inputs_errors_when_missing_noninteractive() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            env::remove_var("GREENTIC_DEPLOY_TERRAFORM_VAR_GCP_PROJECT_ID");
            env::remove_var("GREENTIC_DEPLOY_TERRAFORM_VAR_GCP_REGION");
        }
        let err = match ensure_target_terraform_inputs(StartTarget::Gcp) {
            Ok(_) => panic!("expected missing terraform inputs to fail"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("GREENTIC_DEPLOY_TERRAFORM_VAR_GCP_PROJECT_ID")
        );
    }

    #[test]
    fn cloud_credentials_satisfied_covers_azure_and_gcp_modes() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            env::set_var("ARM_CLIENT_ID", "client");
            env::set_var("ARM_TENANT_ID", "tenant");
            env::set_var("ARM_SUBSCRIPTION_ID", "sub");
            env::set_var("ARM_USE_OIDC", "true");
            env::remove_var("GOOGLE_APPLICATION_CREDENTIALS");
            env::set_var("CLOUDSDK_AUTH_ACCESS_TOKEN", "token");
        }
        assert!(cloud_credentials_satisfied(StartTarget::Azure));
        assert!(cloud_credentials_satisfied(StartTarget::Gcp));
        unsafe {
            env::remove_var("ARM_CLIENT_ID");
            env::remove_var("ARM_TENANT_ID");
            env::remove_var("ARM_SUBSCRIPTION_ID");
            env::remove_var("ARM_USE_OIDC");
            env::remove_var("CLOUDSDK_AUTH_ACCESS_TOKEN");
        }
    }

    #[test]
    fn validate_cloud_deploy_inputs_runtime_returns_empty_env() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let tool_dir = dir.path().join("bin");
        fs::create_dir_all(&tool_dir).expect("mkdir");
        let terraform = tool_dir.join("terraform");
        fs::write(&terraform, "#!/bin/sh\nexit 0\n").expect("write");
        fs::set_permissions(&terraform, fs::Permissions::from_mode(0o755)).expect("chmod");
        let original_path = env::var_os("PATH");
        unsafe {
            env::set_var("PATH", &tool_dir);
        }
        let env = validate_cloud_deploy_inputs(StartTarget::Runtime, None, dir.path(), "en")
            .expect("env");
        let mut cmd = std::process::Command::new("env");
        env.apply(&mut cmd);
        assert_eq!(cmd.get_envs().count(), 0);
        unsafe {
            match original_path {
                Some(path) => env::set_var("PATH", path),
                None => env::remove_var("PATH"),
            }
        }
    }

    #[test]
    fn binary_in_path_returns_false_for_missing_tools() {
        assert!(!binary_in_path("definitely-not-a-real-terraform-binary"));
    }

    #[test]
    fn validate_public_base_url_for_static_routes_accepts_bundles_without_pack_dirs() {
        let dir = tempfile::tempdir().expect("tempdir");
        validate_public_base_url_for_static_routes(dir.path()).expect("validate");
    }

    #[test]
    fn dir_declares_static_routes_ignores_non_pack_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("notes.txt"), "hello").expect("write");
        assert!(!dir_declares_static_routes(dir.path()).expect("scan"));
    }

    #[cfg(unix)]
    #[test]
    fn require_tool_in_path_uses_current_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tool = dir.path().join("terraform");
        fs::write(&tool, "#!/bin/sh\nexit 0\n").expect("write");
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).expect("chmod");
        let original = env::var_os("PATH");
        unsafe {
            env::set_var("PATH", dir.path());
        }

        assert!(binary_in_path("terraform"));
        assert!(require_tool_in_path("terraform", "install it").is_ok());

        unsafe {
            match original {
                Some(path) => env::set_var("PATH", path),
                None => env::remove_var("PATH"),
            }
        }
    }
}
