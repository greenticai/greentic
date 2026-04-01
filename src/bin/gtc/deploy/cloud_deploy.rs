#[path = "cloud_deploy/deployment_state.rs"]
mod deployment_state;
#[path = "cloud_deploy/provider_packs.rs"]
mod provider_packs;
#[path = "cloud_deploy/single_vm.rs"]
mod single_vm;

use std::path::Path;

use crate::DEPLOYER_BIN;
use crate::process::{resolve_binary_in_dir, run_binary_capture};
use crate::prompt::{
    can_prompt_interactively, prompt_choice, prompt_non_empty, prompt_optional,
    prompt_optional_secret, prompt_secret, prompt_value_with_default,
};
use gtc::config::GtcConfig;
use gtc::error::{GtcError, GtcResult};
use serde::Deserialize;

use super::{ChildProcessEnv, StartTarget};

pub(crate) use deployment_state::{destroy_deployment, ensure_started_or_deployed};
pub(crate) use provider_packs::{
    resolve_canonical_target_provider_pack_from, resolve_deploy_app_pack_path,
    resolve_target_provider_pack,
};
pub(crate) use single_vm::write_single_vm_spec;

pub(crate) fn default_operator_image_for_target(target: StartTarget) -> Option<String> {
    match target {
        StartTarget::Aws | StartTarget::Gcp | StartTarget::Azure => {
            default_target_variable_for_gtc(
                target,
                "en",
                "GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE",
            )
            .ok()
            .flatten()
        }
        StartTarget::SingleVm | StartTarget::Runtime => None,
    }
}

pub(crate) fn describe_cloud_target_requirements_for_gtc(
    target: StartTarget,
    locale: &str,
) -> GtcResult<CloudTargetRequirementsV1> {
    describe_cloud_target_requirements(target, locale)
}

pub(crate) fn default_target_variable_for_gtc(
    target: StartTarget,
    locale: &str,
    name: &str,
) -> GtcResult<Option<String>> {
    let requirements = describe_cloud_target_requirements(target, locale)?;
    Ok(requirements
        .variable_requirements
        .into_iter()
        .find(|requirement| requirement.name == name)
        .and_then(|requirement| requirement.default_value))
}

pub(crate) fn canonical_provider_pack_filename_for_gtc(
    target: StartTarget,
    locale: &str,
) -> GtcResult<Option<String>> {
    match target {
        StartTarget::Aws | StartTarget::Gcp | StartTarget::Azure => {
            let requirements = describe_cloud_target_requirements(target, locale)?;
            Ok(Some(requirements.provider_pack_filename))
        }
        StartTarget::Runtime | StartTarget::SingleVm => Ok(None),
    }
}

pub(crate) fn required_provider_pack_filenames_for_gtc(locale: &str) -> GtcResult<Vec<String>> {
    let mut filenames = Vec::new();
    for target in [StartTarget::Aws, StartTarget::Azure, StartTarget::Gcp] {
        if let Some(filename) = canonical_provider_pack_filename_for_gtc(target, locale)?
            && !filenames.contains(&filename)
        {
            filenames.push(filename);
        }
    }
    Ok(filenames)
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
    let _ = bundle_dir;
    match target {
        StartTarget::Aws | StartTarget::Gcp | StartTarget::Azure => {
            let requirements = describe_cloud_target_requirements(target, locale)?;
            child_env.extend(ensure_cloud_credentials(target, locale)?);
            child_env.extend(ensure_target_terraform_inputs(target, locale)?);
            if requirements.remote_bundle_source_required {
                let remote_bundle_help = requirements
                    .remote_bundle_source_help
                    .as_deref()
                    .unwrap_or("Pass --deploy-bundle-source https://.../bundle.gtbundle or set GREENTIC_DEPLOY_BUNDLE_SOURCE");
                let remote_bundle_source = remote_bundle_source.ok_or_else(|| {
                    GtcError::message(format!(
                        "{} deploy requires a remote bundle source; {}",
                        requirements.target, remote_bundle_help
                    ))
                })?;
                if !is_remote_bundle_source(remote_bundle_source) {
                    return Err(GtcError::message(format!(
                        "{} deploy requires a remote bundle source, got local path: {remote_bundle_source}",
                        requirements.target
                    )));
                }
                validate_bundle_registry_mapping_env(remote_bundle_source)?;
            }
            Ok(child_env)
        }
        StartTarget::SingleVm | StartTarget::Runtime => Ok(child_env),
    }
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

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PromptFieldKindV1 {
    Required,
    Optional,
    Secret,
    OptionalSecret,
    Static,
}

#[derive(Debug, Clone, Deserialize)]
struct PromptFieldSpecV1 {
    env_name: String,
    prompt: String,
    kind: PromptFieldKindV1,
    #[serde(default)]
    static_value: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CredentialRequirementV1 {
    label: String,
    env_vars: Vec<String>,
    #[serde(default)]
    satisfaction_env_groups: Vec<Vec<String>>,
    #[serde(default)]
    prompt_fields: Vec<PromptFieldSpecV1>,
    help: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VariableRequirementV1 {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) required: bool,
    #[serde(default)]
    pub(crate) prompt: Option<String>,
    #[serde(default)]
    pub(crate) default_value: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct CloudTargetRequirementsV1 {
    pub(crate) target: String,
    #[allow(dead_code)]
    pub(crate) target_label: String,
    pub(crate) provider_pack_filename: String,
    pub(crate) remote_bundle_source_required: bool,
    #[serde(default)]
    pub(crate) remote_bundle_source_help: Option<String>,
    #[serde(default)]
    pub(crate) informational_notes: Vec<String>,
    #[serde(default)]
    credential_requirements: Vec<CredentialRequirementV1>,
    #[serde(default)]
    pub(crate) variable_requirements: Vec<VariableRequirementV1>,
}

fn describe_cloud_target_requirements(
    target: StartTarget,
    locale: &str,
) -> GtcResult<CloudTargetRequirementsV1> {
    let provider = match target {
        StartTarget::Aws => "aws",
        StartTarget::Azure => "azure",
        StartTarget::Gcp => "gcp",
        StartTarget::SingleVm | StartTarget::Runtime => {
            return Err(GtcError::message(format!(
                "cloud target requirements are not available for {}",
                target.as_str()
            )));
        }
    };
    let args = vec![
        "target-requirements".to_string(),
        "--provider".to_string(),
        provider.to_string(),
    ];
    let output = run_binary_capture(DEPLOYER_BIN, &args, false, locale)?;
    let requirements: CloudTargetRequirementsV1 = serde_json::from_str(&output).map_err(|err| {
        GtcError::message(format!(
            "failed to parse greentic-deployer target requirements for {provider}: {err}"
        ))
    })?;
    if requirements.target != provider {
        return Err(GtcError::message(format!(
            "greentic-deployer returned target requirements for {}, expected {provider}",
            requirements.target
        )));
    }
    Ok(requirements)
}

fn ensure_cloud_credentials(target: StartTarget, locale: &str) -> GtcResult<ChildProcessEnv> {
    let requirements = describe_cloud_target_requirements(target, locale)?;
    if cloud_credentials_satisfied(&requirements) {
        return Ok(ChildProcessEnv::new());
    }
    let names: Vec<&str> = requirements
        .credential_requirements
        .iter()
        .flat_map(|req| req.env_vars.iter().map(String::as_str))
        .collect();
    let help = requirements
        .credential_requirements
        .iter()
        .map(|req| req.help.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    if !can_prompt_interactively() {
        return Err(missing_cloud_credentials_error(&names, &help));
    }
    let _ = locale;
    println!(
        "Cloud credentials for {} are missing. gtc can collect them for this run.",
        target.as_str()
    );
    let env = prompt_cloud_credentials_for_requirements(target.as_str(), &requirements)?;
    if cloud_credentials_satisfied(&requirements) || !env.vars.is_empty() {
        Ok(env)
    } else {
        Err(missing_cloud_credentials_error(&names, &help))
    }
}

fn ensure_target_terraform_inputs(target: StartTarget, locale: &str) -> GtcResult<ChildProcessEnv> {
    let requirements = describe_cloud_target_requirements(target, locale)?;
    if requirements.variable_requirements.is_empty() {
        return Ok(ChildProcessEnv::new());
    }
    let missing = collect_missing_required_variables(&requirements);
    if missing.is_empty() {
        return Ok(ChildProcessEnv::new());
    }
    if !can_prompt_interactively() {
        return Err(GtcError::message(format!(
            "missing required deployment configuration: {}",
            missing
                .iter()
                .map(|requirement| requirement.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    println!(
        "Additional {} deployment inputs are required for this run.",
        target.as_str()
    );
    let mut env = ChildProcessEnv::new();
    for requirement in missing {
        let prompt = requirement
            .prompt
            .as_deref()
            .unwrap_or(requirement.name.as_str());
        let value = prompt_value_with_default(prompt, requirement.default_value.as_deref())?;
        env.set(requirement.name, value);
    }
    Ok(env)
}

fn collect_missing_required_variables(
    requirements: &CloudTargetRequirementsV1,
) -> Vec<VariableRequirementV1> {
    requirements
        .variable_requirements
        .iter()
        .filter(|requirement| requirement.required && !env_var_present(&requirement.name))
        .cloned()
        .collect()
}

fn cloud_credentials_satisfied(requirements: &CloudTargetRequirementsV1) -> bool {
    requirements
        .credential_requirements
        .iter()
        .any(credential_requirement_satisfied)
}

fn credential_requirement_satisfied(requirement: &CredentialRequirementV1) -> bool {
    let groups = if requirement.satisfaction_env_groups.is_empty() {
        vec![requirement.env_vars.clone()]
    } else {
        requirement.satisfaction_env_groups.clone()
    };
    groups
        .iter()
        .any(|group| group.iter().all(|name| env_var_present(name)))
}

fn env_var_present(name: &str) -> bool {
    GtcConfig::from_env().non_empty_var(name).is_some()
}

fn prompt_cloud_credentials_for_requirements(
    provider_name: &str,
    requirements: &CloudTargetRequirementsV1,
) -> GtcResult<ChildProcessEnv> {
    if requirements.credential_requirements.is_empty() {
        return Ok(ChildProcessEnv::new());
    }
    prompt_cloud_credentials(
        &format!("Select {provider_name} credential input mode:"),
        provider_name,
        &requirements.credential_requirements,
    )
}

fn prompt_cloud_credentials(
    selection_prompt: &str,
    provider_name: &str,
    modes: &[CredentialRequirementV1],
) -> GtcResult<ChildProcessEnv> {
    let mut options: Vec<&str> = modes.iter().map(|mode| mode.label.as_str()).collect();
    options.push("Abort");
    let mode = prompt_choice(selection_prompt, &options)?;
    if mode >= modes.len() {
        return Err(GtcError::message(format!(
            "cloud deploy aborted before {provider_name} credentials were configured"
        )));
    }

    let mut env = ChildProcessEnv::new();
    for field in &modes[mode].prompt_fields {
        match field.kind {
            PromptFieldKindV1::Required => {
                env.set(field.env_name.clone(), prompt_non_empty(&field.prompt)?);
            }
            PromptFieldKindV1::Optional => {
                if let Some(value) = prompt_optional(&field.prompt)? {
                    env.set(field.env_name.clone(), value);
                }
            }
            PromptFieldKindV1::Secret => {
                env.vars
                    .push((field.env_name.clone(), prompt_secret(&field.prompt)?));
            }
            PromptFieldKindV1::OptionalSecret => {
                if let Some(value) = prompt_optional_secret(&field.prompt)? {
                    env.vars.push((field.env_name.clone(), value));
                }
            }
            PromptFieldKindV1::Static => env.set(
                field.env_name.clone(),
                field.static_value.clone().unwrap_or_default(),
            ),
        }
    }

    Ok(env)
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
        CloudTargetRequirementsV1, CredentialRequirementV1, VariableRequirementV1,
        append_bundle_registry_args, binary_in_path, cloud_credentials_satisfied,
        collect_missing_required_variables, default_operator_image_for_target, env_var_present,
        matches_remote_bundle_ref, validate_bundle_registry_mapping_env,
    };
    #[cfg(unix)]
    use super::{require_tool_in_path, validate_cloud_deploy_inputs};
    use crate::deploy::StartTarget;
    use crate::tests::{env_test_lock, fake_deployer_contract};
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
        assert!(!cloud_credentials_satisfied(&CloudTargetRequirementsV1 {
            target: "aws".to_string(),
            target_label: "AWS".to_string(),
            provider_pack_filename: "terraform.gtpack".to_string(),
            remote_bundle_source_required: true,
            remote_bundle_source_help: None,
            informational_notes: Vec::new(),
            credential_requirements: vec![CredentialRequirementV1 {
                label: "Access key pair".to_string(),
                env_vars: vec![
                    "AWS_ACCESS_KEY_ID".to_string(),
                    "AWS_SECRET_ACCESS_KEY".to_string(),
                ],
                satisfaction_env_groups: vec![vec![
                    "AWS_ACCESS_KEY_ID".to_string(),
                    "AWS_SECRET_ACCESS_KEY".to_string(),
                ]],
                prompt_fields: Vec::new(),
                help: "AWS access key credentials".to_string(),
            }],
            variable_requirements: Vec::new(),
        }));
        unsafe {
            env::set_var("AWS_SECRET_ACCESS_KEY", "secret");
        }
        assert!(cloud_credentials_satisfied(&CloudTargetRequirementsV1 {
            target: "aws".to_string(),
            target_label: "AWS".to_string(),
            provider_pack_filename: "terraform.gtpack".to_string(),
            remote_bundle_source_required: true,
            remote_bundle_source_help: None,
            informational_notes: Vec::new(),
            credential_requirements: vec![CredentialRequirementV1 {
                label: "Access key pair".to_string(),
                env_vars: vec![
                    "AWS_ACCESS_KEY_ID".to_string(),
                    "AWS_SECRET_ACCESS_KEY".to_string(),
                ],
                satisfaction_env_groups: vec![vec![
                    "AWS_ACCESS_KEY_ID".to_string(),
                    "AWS_SECRET_ACCESS_KEY".to_string(),
                ]],
                prompt_fields: Vec::new(),
                help: "AWS access key credentials".to_string(),
            }],
            variable_requirements: Vec::new(),
        }));
        unsafe {
            env::remove_var("AWS_ACCESS_KEY_ID");
            env::remove_var("AWS_SECRET_ACCESS_KEY");
        }
    }

    #[test]
    fn default_operator_image_for_target_returns_cloud_defaults_only() {
        let (_deployer_dir, _deployer_guard) = fake_deployer_contract(None);
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
        let missing = collect_missing_required_variables(&CloudTargetRequirementsV1 {
            target: "gcp".to_string(),
            target_label: "GCP".to_string(),
            provider_pack_filename: "terraform.gtpack".to_string(),
            remote_bundle_source_required: true,
            remote_bundle_source_help: None,
            informational_notes: Vec::new(),
            credential_requirements: Vec::new(),
            variable_requirements: vec![
                VariableRequirementV1 {
                    name: "GREENTIC_DEPLOY_TERRAFORM_VAR_GCP_PROJECT_ID".to_string(),
                    required: true,
                    prompt: None,
                    default_value: None,
                },
                VariableRequirementV1 {
                    name: "GREENTIC_DEPLOY_TERRAFORM_VAR_GCP_REGION".to_string(),
                    required: true,
                    prompt: None,
                    default_value: Some("us-central1".to_string()),
                },
            ],
        });
        assert_eq!(missing.len(), 2);
        assert_eq!(
            missing[0].name,
            "GREENTIC_DEPLOY_TERRAFORM_VAR_GCP_PROJECT_ID"
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
        assert!(cloud_credentials_satisfied(&CloudTargetRequirementsV1 {
            target: "azure".to_string(),
            target_label: "Azure".to_string(),
            provider_pack_filename: "terraform.gtpack".to_string(),
            remote_bundle_source_required: true,
            remote_bundle_source_help: None,
            informational_notes: Vec::new(),
            credential_requirements: vec![CredentialRequirementV1 {
                label: "Azure OIDC".to_string(),
                env_vars: vec![
                    "ARM_USE_OIDC".to_string(),
                    "AZURE_CLIENT_ID".to_string(),
                    "AZURE_TENANT_ID".to_string(),
                    "AZURE_SUBSCRIPTION_ID".to_string(),
                ],
                satisfaction_env_groups: vec![
                    vec![
                        "ARM_CLIENT_ID".to_string(),
                        "ARM_TENANT_ID".to_string(),
                        "ARM_SUBSCRIPTION_ID".to_string(),
                        "ARM_USE_OIDC".to_string(),
                    ],
                    vec![
                        "AZURE_CLIENT_ID".to_string(),
                        "AZURE_TENANT_ID".to_string(),
                        "AZURE_SUBSCRIPTION_ID".to_string(),
                    ],
                ],
                prompt_fields: Vec::new(),
                help: "Azure OIDC credentials".to_string(),
            }],
            variable_requirements: Vec::new(),
        }));
        assert!(cloud_credentials_satisfied(&CloudTargetRequirementsV1 {
            target: "gcp".to_string(),
            target_label: "GCP".to_string(),
            provider_pack_filename: "terraform.gtpack".to_string(),
            remote_bundle_source_required: true,
            remote_bundle_source_help: None,
            informational_notes: Vec::new(),
            credential_requirements: vec![CredentialRequirementV1 {
                label: "Access token".to_string(),
                env_vars: vec![
                    "GOOGLE_OAUTH_ACCESS_TOKEN".to_string(),
                    "CLOUDSDK_AUTH_ACCESS_TOKEN".to_string(),
                ],
                satisfaction_env_groups: vec![
                    vec!["GOOGLE_OAUTH_ACCESS_TOKEN".to_string()],
                    vec!["CLOUDSDK_AUTH_ACCESS_TOKEN".to_string()],
                ],
                prompt_fields: Vec::new(),
                help: "GCP access token credentials".to_string(),
            }],
            variable_requirements: Vec::new(),
        }));
        unsafe {
            env::remove_var("ARM_CLIENT_ID");
            env::remove_var("ARM_TENANT_ID");
            env::remove_var("ARM_SUBSCRIPTION_ID");
            env::remove_var("ARM_USE_OIDC");
            env::remove_var("CLOUDSDK_AUTH_ACCESS_TOKEN");
        }
    }

    #[test]
    #[cfg(unix)]
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
