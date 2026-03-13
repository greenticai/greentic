use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use greentic_distributor_client::oci_components::{
    OciComponentResolver, PulledImage, PulledLayer, RegistryClient,
};
use greentic_distributor_client::{ComponentResolveOptions, ComponentsExtension, ComponentsMode};
use oci_distribution::Reference;
use oci_distribution::client::{Client, ClientConfig, ClientProtocol, ImageData};
use oci_distribution::errors::OciDistributionError;
use oci_distribution::secrets::RegistryAuth;
use serde_json::Value;

pub trait DistAdapter: Send + Sync {
    fn pull_bytes(&self, oci_ref: &str, key: &str) -> Result<Vec<u8>, String>;
    fn pull_to_dir(&self, oci_ref: &str, key: &str, out_dir: &Path) -> Result<(), String>;
}

pub fn build_adapter() -> Result<Box<dyn DistAdapter>, String> {
    if let Ok(root) = env::var("GTC_DIST_MOCK_ROOT") {
        let mock = FileDistAdapter::new(PathBuf::from(root))?;
        return Ok(Box::new(mock));
    }
    Ok(Box::new(OciDistAdapter))
}

struct OciDistAdapter;

impl OciDistAdapter {
    fn resolve_ref_to_cached_file(&self, oci_ref: &str, key: &str) -> Result<PathBuf, String> {
        let cleaned = strip_oci_prefix(oci_ref);
        let token = key.to_string();

        let cache = tempfile::tempdir().map_err(|e| e.to_string())?;
        let opts = ComponentResolveOptions {
            allow_tags: true,
            cache_dir: cache.path().join("cache"),
            ..Default::default()
        };

        let resolver = OciComponentResolver::with_client(TokenRegistryClient::new(token), opts);
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

impl DistAdapter for OciDistAdapter {
    fn pull_bytes(&self, oci_ref: &str, key: &str) -> Result<Vec<u8>, String> {
        let path = self.resolve_ref_to_cached_file(oci_ref, key)?;
        fs::read(path).map_err(|e| e.to_string())
    }

    fn pull_to_dir(&self, oci_ref: &str, key: &str, out_dir: &Path) -> Result<(), String> {
        fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
        let path = self.resolve_ref_to_cached_file(oci_ref, key)?;

        let filename = path
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("artifact.bin");
        let target = out_dir.join(filename);
        fs::copy(path, target).map_err(|e| e.to_string())?;
        Ok(())
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
}

impl DistAdapter for FileDistAdapter {
    fn pull_bytes(&self, oci_ref: &str, _key: &str) -> Result<Vec<u8>, String> {
        let path = self.map_ref(oci_ref)?;
        fs::read(&path).map_err(|e| format!("failed to read {}: {e}", path.display()))
    }

    fn pull_to_dir(&self, oci_ref: &str, _key: &str, out_dir: &Path) -> Result<(), String> {
        fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
        let mapped = self.map_ref(oci_ref)?;
        if mapped.is_dir() {
            copy_tree(&mapped, out_dir)
        } else {
            let name = mapped
                .file_name()
                .ok_or_else(|| format!("invalid file path: {}", mapped.display()))?;
            fs::copy(&mapped, out_dir.join(name)).map_err(|e| e.to_string())?;
            Ok(())
        }
    }
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(from).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if src.is_dir() {
            copy_tree(&src, &dst)?;
        } else {
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::copy(src, dst).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn strip_oci_prefix(value: &str) -> String {
    value.strip_prefix("oci://").unwrap_or(value).to_string()
}

#[derive(Clone)]
struct TokenRegistryClient {
    token: String,
    inner: Client,
}

impl TokenRegistryClient {
    fn new(token: String) -> Self {
        let config = ClientConfig {
            protocol: ClientProtocol::Https,
            ..Default::default()
        };
        Self {
            token,
            inner: Client::new(config),
        }
    }

    fn accepted_layer_media_types<'a>(
        accepted_manifest_types: &'a [&'a str],
    ) -> Vec<&'a str> {
        let mut types = accepted_manifest_types.to_vec();
        for extra in [
            "application/json",
            "application/octet-stream",
            "application/zip",
            "application/gzip",
            "application/x-gzip",
            "application/tar",
            "application/x-tar",
            "application/vnd.oci.image.layer.v1.tar",
            "application/vnd.oci.image.layer.v1.tar+gzip",
            "application/vnd.docker.image.rootfs.diff.tar",
            "application/vnd.docker.image.rootfs.diff.tar.gzip",
        ] {
            if !types.contains(&extra) {
                types.push(extra);
            }
        }
        types
    }
}

#[async_trait]
impl RegistryClient for TokenRegistryClient {
    fn default_client() -> Self
    where
        Self: Sized,
    {
        Self::new(String::new())
    }

    async fn pull(
        &self,
        reference: &Reference,
        accepted_manifest_types: &[&str],
    ) -> Result<PulledImage, OciDistributionError> {
        let accepted_layer_types = Self::accepted_layer_media_types(accepted_manifest_types);
        let image = self
            .inner
            .pull(
                reference,
                &RegistryAuth::Basic("oauth2accesstoken".to_string(), self.token.clone()),
                accepted_layer_types,
            )
            .await?;

        Ok(convert_image(image))
    }
}

fn convert_image(image: ImageData) -> PulledImage {
    let layers = image
        .layers
        .into_iter()
        .map(|layer| {
            let digest = format!("sha256:{}", layer.sha256_digest());
            PulledLayer {
                media_type: layer.media_type,
                data: layer.data,
                digest: Some(digest),
            }
        })
        .collect();
    PulledImage {
        digest: image.digest,
        layers,
    }
}

#[cfg(test)]
mod tests {
    use super::TokenRegistryClient;

    #[test]
    fn accepted_layer_media_types_include_json_manifest_blobs() {
        let accepted = TokenRegistryClient::accepted_layer_media_types(&["application/example"]);
        assert!(accepted.contains(&"application/example"));
        assert!(accepted.contains(&"application/json"));
        assert!(accepted.contains(&"application/octet-stream"));
        assert!(accepted.contains(&"application/zip"));
        assert!(accepted.contains(&"application/vnd.oci.image.layer.v1.tar+gzip"));
    }

    #[test]
    fn accepted_layer_media_types_do_not_duplicate_json() {
        let accepted = TokenRegistryClient::accepted_layer_media_types(&["application/json"]);
        let json_count = accepted
            .iter()
            .filter(|media_type| **media_type == "application/json")
            .count();

        assert_eq!(json_count, 1);
    }
}
