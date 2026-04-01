use std::env;
use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Debug, Default, Clone, Copy)]
pub struct GtcConfig;

impl GtcConfig {
    pub fn from_env() -> Self {
        Self
    }

    pub fn non_empty_var(&self, name: &str) -> Option<String> {
        env::var(name)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    pub fn non_empty_var_os(&self, name: &str) -> Option<OsString> {
        let value = env::var_os(name)?;
        if value.is_empty() { None } else { Some(value) }
    }

    pub fn cargo_home(&self) -> Option<PathBuf> {
        self.non_empty_var("CARGO_HOME").map(PathBuf::from)
    }

    pub fn dist_mock_root(&self) -> Option<PathBuf> {
        self.non_empty_var("GTC_DIST_MOCK_ROOT").map(PathBuf::from)
    }

    pub fn tenant_manifest_url_template(&self) -> Option<String> {
        self.non_empty_var("GTC_TENANT_MANIFEST_URL_TEMPLATE")
    }

    pub fn locale_override(&self) -> Option<String> {
        self.non_empty_var("GTC_LOCALE")
    }

    pub fn deploy_bundle_source_override(&self) -> Option<String> {
        self.non_empty_var("GREENTIC_DEPLOY_BUNDLE_SOURCE")
    }

    pub fn repo_registry_base(&self) -> Option<String> {
        self.non_empty_var("GREENTIC_REPO_REGISTRY_BASE")
    }

    pub fn store_registry_base(&self) -> Option<String> {
        self.non_empty_var("GREENTIC_STORE_REGISTRY_BASE")
    }

    pub fn require_non_empty_var(&self, name: &str) -> crate::error::GtcResult<String> {
        self.non_empty_var(name).ok_or_else(|| {
            crate::error::GtcError::message(format!(
                "missing required environment variable: {name}"
            ))
        })
    }

    pub fn terraform_operator_image(&self) -> Option<String> {
        self.non_empty_var("GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE")
    }

    pub fn terraform_operator_image_digest(&self) -> Option<String> {
        self.non_empty_var("GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE_DIGEST")
    }

    pub fn dev_bin_override(&self) -> Option<OsString> {
        self.non_empty_var_os("GREENTIC_DEV_BIN")
    }

    pub fn operator_bin_override(&self) -> Option<OsString> {
        self.non_empty_var_os("GREENTIC_OPERATOR_BIN")
    }

    pub fn bundle_bin_override(&self) -> Option<OsString> {
        self.non_empty_var_os("GREENTIC_BUNDLE_BIN")
    }

    pub fn deployer_bin_override(&self) -> Option<OsString> {
        self.non_empty_var_os("GREENTIC_DEPLOYER_BIN")
    }

    pub fn setup_bin_override(&self) -> Option<OsString> {
        self.non_empty_var_os("GREENTIC_SETUP_BIN")
    }

    pub fn start_bin_override(&self) -> Option<OsString> {
        self.non_empty_var_os("GREENTIC_START_BIN")
    }

    pub fn tenant_key(&self, tenant: &str) -> Option<String> {
        self.non_empty_var(&format!("GREENTIC_{}_KEY", tenant_key_segment(tenant)))
    }
}

fn tenant_key_segment(tenant: &str) -> String {
    tenant
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::GtcConfig;
    use std::env;

    #[test]
    fn non_empty_var_trims_and_filters_blank_values() {
        unsafe {
            env::set_var("GTC_CONFIG_TEST", "  demo  ");
        }
        let cfg = GtcConfig::from_env();
        assert_eq!(
            cfg.non_empty_var("GTC_CONFIG_TEST").as_deref(),
            Some("demo")
        );
        unsafe {
            env::set_var("GTC_CONFIG_TEST", "   ");
        }
        assert_eq!(cfg.non_empty_var("GTC_CONFIG_TEST"), None);
        unsafe {
            env::remove_var("GTC_CONFIG_TEST");
        }
    }

    #[test]
    fn tenant_key_uses_normalized_env_name() {
        unsafe {
            env::set_var("GREENTIC_ACME_DEV_KEY", "  secret  ");
        }
        let cfg = GtcConfig::from_env();
        assert_eq!(cfg.tenant_key("acme-dev").as_deref(), Some("secret"));
        unsafe {
            env::remove_var("GREENTIC_ACME_DEV_KEY");
        }
    }
}
