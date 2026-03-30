use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::config::GtcConfig;
use crate::error::{GtcError, GtcResult};
use greentic_distributor_client::oci_components::{DefaultRegistryClient, OciComponentResolver};
use greentic_distributor_client::{ComponentResolveOptions, ComponentsExtension, ComponentsMode};
use serde_json::Value;

pub fn pull_oci_reference_to_tempfile(oci_ref: &str, key: Option<&str>) -> GtcResult<PathBuf> {
    if let Some(root) = GtcConfig::from_env().dist_mock_root() {
        let mock = FileDistAdapter::new(root)?;
        return mock.resolve_ref_to_cached_file(oci_ref);
    }
    OciDistAdapter.resolve_ref_to_cached_file(oci_ref, key.unwrap_or_default())
}

struct OciDistAdapter;

impl OciDistAdapter {
    fn resolve_ref_to_cached_file(&self, oci_ref: &str, key: &str) -> GtcResult<PathBuf> {
        let cleaned = strip_oci_prefix(oci_ref);
        let staging = tempfile::tempdir()
            .map_err(|e| GtcError::io("failed to create staging directory", e))?;
        let opts = ComponentResolveOptions {
            allow_tags: true,
            cache_dir: staging.path().join("cache"),
            ..Default::default()
        };

        let client = if key.is_empty() {
            DefaultRegistryClient::default()
        } else {
            DefaultRegistryClient::with_basic_auth("oauth2accesstoken", key)
        };
        let resolver = OciComponentResolver::with_client(client, opts);

        let resolved = oci_runtime()
            .block_on(resolver.resolve_refs(&ComponentsExtension {
                refs: vec![cleaned.clone()],
                mode: ComponentsMode::Eager,
            }))
            .map_err(|e| GtcError::message(format!("failed to pull '{cleaned}': {e}")))?;

        let artifact = resolved.into_iter().next().ok_or_else(|| {
            GtcError::invalid_data(format!("oci ref '{cleaned}'"), "no layers found")
        })?;

        let path = artifact.path;
        let out_path = stage_resolved_artifact(&path, staging.path())?;
        let relative = out_path
            .strip_prefix(staging.path())
            .map_err(|e| GtcError::path("failed to compute kept staging path", e))?
            .to_path_buf();

        let keep_dir = staging.keep();
        Ok(keep_dir.join(relative))
    }
}

fn oci_runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

pub fn stage_resolved_artifact(source: &Path, staging_root: &Path) -> GtcResult<PathBuf> {
    let artifact_dir = staging_root.join("artifact");
    fs::create_dir_all(&artifact_dir)
        .map_err(|e| GtcError::io(format!("failed to create {}", artifact_dir.display()), e))?;

    let filename = source
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("artifact.bin");
    let out_path = artifact_dir.join(filename);
    fs::copy(source, &out_path).map_err(|e| {
        GtcError::io(
            format!(
                "failed to copy resolved artifact {} to {}",
                source.display(),
                out_path.display()
            ),
            e,
        )
    })?;
    Ok(out_path)
}

struct FileDistAdapter {
    root: PathBuf,
    index: HashMap<String, String>,
}

impl FileDistAdapter {
    fn new(root: PathBuf) -> GtcResult<Self> {
        let index_path = root.join("index.json");
        let raw = fs::read_to_string(&index_path)
            .map_err(|e| GtcError::io(format!("failed to read {}", index_path.display()), e))?;
        let value: Value = serde_json::from_str(&raw)
            .map_err(|e| GtcError::json(format!("invalid {}", index_path.display()), e))?;
        let obj = value
            .as_object()
            .ok_or_else(|| GtcError::invalid_data("index.json", "root must be an object"))?;

        let mut index = HashMap::new();
        for (k, v) in obj {
            let s = v.as_str().ok_or_else(|| {
                GtcError::invalid_data("index.json", format!("entry '{k}' must be a string"))
            })?;
            index.insert(k.clone(), s.to_string());
        }

        Ok(Self { root, index })
    }

    fn map_ref(&self, oci_ref: &str) -> GtcResult<PathBuf> {
        let rel = self.index.get(oci_ref).ok_or_else(|| {
            GtcError::invalid_data("mock OCI index", format!("missing mapping for '{oci_ref}'"))
        })?;
        Ok(self.root.join(rel))
    }

    fn resolve_ref_to_cached_file(&self, oci_ref: &str) -> GtcResult<PathBuf> {
        self.map_ref(oci_ref)
    }
}

fn strip_oci_prefix(value: &str) -> String {
    value.strip_prefix("oci://").unwrap_or(value).to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        FileDistAdapter, oci_runtime, pull_oci_reference_to_tempfile, stage_resolved_artifact,
        strip_oci_prefix,
    };
    use crate::error::GtcError;
    use std::env;
    use std::fs;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

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

    #[test]
    fn oci_runtime_is_reused_across_calls() {
        assert!(std::ptr::eq(oci_runtime(), oci_runtime()));
    }

    #[test]
    fn stage_resolved_artifact_copies_contents_and_name() {
        let src_dir = tempfile::tempdir().expect("tempdir");
        let stage_dir = tempfile::tempdir().expect("tempdir");
        let source = src_dir.path().join("demo.bin");
        fs::write(&source, b"artifact-bytes").expect("write");

        let staged = stage_resolved_artifact(&source, stage_dir.path()).expect("stage");

        assert_eq!(
            staged.file_name().and_then(|v| v.to_str()),
            Some("demo.bin")
        );
        assert_eq!(fs::read(&staged).expect("read"), b"artifact-bytes");
    }

    #[test]
    fn stage_resolved_artifact_path_survives_kept_tempdir() {
        let src_dir = tempfile::tempdir().expect("tempdir");
        let staging = tempfile::tempdir().expect("tempdir");
        let source = src_dir.path().join("demo.bin");
        fs::write(&source, b"artifact-bytes").expect("write");

        let staged = stage_resolved_artifact(&source, staging.path()).expect("stage");
        let relative = staged
            .strip_prefix(staging.path())
            .expect("relative")
            .to_path_buf();
        let keep_dir = staging.keep();
        let persisted = keep_dir.join(relative);

        assert!(persisted.exists());
        assert_eq!(fs::read(&persisted).expect("read"), b"artifact-bytes");
    }

    #[test]
    fn dist_source_uses_shared_runtime_and_copy_based_staging() {
        let source = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/dist.rs"));
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("production source");

        assert!(production.contains("OnceLock<tokio::runtime::Runtime>"));
        assert!(production.contains("oci_runtime()"));
        assert!(production.contains("fs::copy(source, &out_path)"));
        assert!(!production.contains("let bytes = fs::read(&path)"));
        assert!(!production.contains("fs::write(&out_path, bytes)"));
    }

    #[test]
    fn file_dist_adapter_reads_index_and_maps_refs() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(
            dir.path().join("index.json"),
            r#"{"oci://ghcr.io/demo/app:latest":"fixtures/demo.gtpack"}"#,
        )
        .expect("write");
        let fixtures = dir.path().join("fixtures");
        fs::create_dir_all(&fixtures).expect("mkdir");

        let adapter = FileDistAdapter::new(dir.path().to_path_buf()).expect("adapter");
        let mapped = adapter
            .map_ref("oci://ghcr.io/demo/app:latest")
            .expect("mapped");
        assert_eq!(mapped, dir.path().join("fixtures/demo.gtpack"));
    }

    #[test]
    fn pull_oci_reference_to_tempfile_uses_mock_root_override() {
        let _guard = env_lock().lock().expect("lock");
        let dir = tempfile::tempdir().expect("tempdir");
        let artifact = dir.path().join("fixtures").join("demo.gtpack");
        fs::create_dir_all(artifact.parent().expect("parent")).expect("mkdir");
        fs::write(&artifact, b"fixture").expect("write");
        fs::write(
            dir.path().join("index.json"),
            r#"{"oci://ghcr.io/demo/app:latest":"fixtures/demo.gtpack"}"#,
        )
        .expect("write");

        unsafe {
            env::set_var("GTC_DIST_MOCK_ROOT", dir.path());
        }
        let resolved = pull_oci_reference_to_tempfile("oci://ghcr.io/demo/app:latest", None)
            .expect("resolved");
        unsafe {
            env::remove_var("GTC_DIST_MOCK_ROOT");
        }

        assert_eq!(resolved, artifact);
    }

    #[test]
    fn file_dist_adapter_rejects_non_object_index() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("index.json"), "[]").expect("write");

        let err = match FileDistAdapter::new(dir.path().to_path_buf()) {
            Ok(_) => panic!("expected invalid index to fail"),
            Err(err) => err,
        };
        assert!(matches!(err, GtcError::InvalidData { .. }));
        assert!(err.to_string().contains("root must be an object"));
    }

    #[test]
    fn pull_oci_reference_to_tempfile_errors_when_mock_mapping_missing() {
        let _guard = env_lock().lock().expect("lock");
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("index.json"), "{}").expect("write");
        unsafe {
            env::set_var("GTC_DIST_MOCK_ROOT", dir.path());
        }
        let err =
            pull_oci_reference_to_tempfile("oci://ghcr.io/demo/missing:latest", None).unwrap_err();
        unsafe {
            env::remove_var("GTC_DIST_MOCK_ROOT");
        }
        assert!(matches!(err, GtcError::InvalidData { .. }));
        assert!(err.to_string().contains("missing mapping"));
    }

    #[test]
    fn file_dist_adapter_rejects_non_string_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("index.json"), r#"{"demo":42}"#).expect("write");

        let err = match FileDistAdapter::new(dir.path().to_path_buf()) {
            Ok(_) => panic!("expected invalid entry to fail"),
            Err(err) => err,
        };
        assert!(matches!(err, GtcError::InvalidData { .. }));
        assert!(err.to_string().contains("must be a string"));
    }

    #[test]
    fn file_dist_adapter_rejects_invalid_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("index.json"), "{not-json").expect("write");

        let err = match FileDistAdapter::new(dir.path().to_path_buf()) {
            Ok(_) => panic!("expected invalid json to fail"),
            Err(err) => err,
        };
        assert!(matches!(err, GtcError::Json { .. }));
        assert!(err.to_string().contains("invalid"));
    }
}
