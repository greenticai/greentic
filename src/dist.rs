use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;

use greentic_distributor_client::oci_components::{DefaultRegistryClient, OciComponentResolver};
use greentic_distributor_client::{ComponentResolveOptions, ComponentsExtension, ComponentsMode};
use serde_json::Value;

pub fn pull_oci_reference_to_tempfile(oci_ref: &str, key: Option<&str>) -> Result<PathBuf, String> {
    if let Ok(root) = env::var("GTC_DIST_MOCK_ROOT") {
        let mock = FileDistAdapter::new(PathBuf::from(root))?;
        return mock.resolve_ref_to_cached_file(oci_ref);
    }
    OciDistAdapter.resolve_ref_to_cached_file(oci_ref, key.unwrap_or_default())
}

struct OciDistAdapter;

impl OciDistAdapter {
    fn resolve_ref_to_cached_file(&self, oci_ref: &str, key: &str) -> Result<PathBuf, String> {
        let cleaned = strip_oci_prefix(oci_ref);
        let cache = tempfile::tempdir().map_err(|e| e.to_string())?;
        let opts = ComponentResolveOptions {
            allow_tags: true,
            cache_dir: cache.path().join("cache"),
            ..Default::default()
        };

        let client = if key.is_empty() {
            DefaultRegistryClient::default()
        } else {
            DefaultRegistryClient::with_basic_auth("oauth2accesstoken", key)
        };
        let resolver = OciComponentResolver::with_client(client, opts);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;

        let resolved = rt
            .block_on(resolver.resolve_refs(&ComponentsExtension {
                refs: vec![cleaned.clone()],
                mode: ComponentsMode::Eager,
            }))
            .map_err(|e| format!("failed to pull '{cleaned}': {e}"))?;

        let artifact = resolved
            .into_iter()
            .next()
            .ok_or_else(|| format!("no layers found for '{cleaned}'"))?;

        let path = artifact.path;
        let bytes = fs::read(&path).map_err(|e| e.to_string())?;

        let staging = tempfile::tempdir().map_err(|e| e.to_string())?;
        let filename = path
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("artifact.bin");
        let out_path = staging.path().join(filename);
        fs::write(&out_path, bytes).map_err(|e| e.to_string())?;

        let keep_dir = staging.keep();
        drop(cache);

        Ok(keep_dir.join(filename))
    }
}

struct FileDistAdapter {
    root: PathBuf,
    index: HashMap<String, String>,
}

impl FileDistAdapter {
    fn new(root: PathBuf) -> Result<Self, String> {
        let index_path = root.join("index.json");
        let raw = fs::read_to_string(&index_path)
            .map_err(|e| format!("failed to read {}: {e}", index_path.display()))?;
        let value: Value = serde_json::from_str(&raw)
            .map_err(|e| format!("invalid {}: {e}", index_path.display()))?;
        let obj = value
            .as_object()
            .ok_or_else(|| "index.json root must be an object".to_string())?;

        let mut index = HashMap::new();
        for (k, v) in obj {
            let s = v
                .as_str()
                .ok_or_else(|| format!("index entry '{k}' must be a string"))?;
            index.insert(k.clone(), s.to_string());
        }

        Ok(Self { root, index })
    }

    fn map_ref(&self, oci_ref: &str) -> Result<PathBuf, String> {
        let rel = self
            .index
            .get(oci_ref)
            .ok_or_else(|| format!("missing mock mapping for '{oci_ref}'"))?;
        Ok(self.root.join(rel))
    }

    fn resolve_ref_to_cached_file(&self, oci_ref: &str) -> Result<PathBuf, String> {
        self.map_ref(oci_ref)
    }
}

fn strip_oci_prefix(value: &str) -> String {
    value.strip_prefix("oci://").unwrap_or(value).to_string()
}

#[cfg(test)]
mod tests {
    use super::strip_oci_prefix;

    #[test]
    fn strip_oci_prefix_removes_scheme() {
        assert_eq!(
            strip_oci_prefix("oci://ghcr.io/greentic-biz/customers-tools/demo:latest"),
            "ghcr.io/greentic-biz/customers-tools/demo:latest"
        );
    }

    #[test]
    fn strip_oci_prefix_leaves_plain_reference_unchanged() {
        assert_eq!(
            strip_oci_prefix("ghcr.io/greentic-biz/customers-tools/demo:latest"),
            "ghcr.io/greentic-biz/customers-tools/demo:latest"
        );
    }
}
