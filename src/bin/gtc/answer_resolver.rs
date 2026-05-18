use std::fs;
use std::path::PathBuf;

use greentic_distributor_client::{
    DistClient, DistOptions, PackFetchOptions, default_pack_layer_media_types,
    fetch_pack_to_cache_with_options,
};
use gtc::error::{GtcError, GtcResult};
use oci_distribution::Reference;
use reqwest::blocking::Client;
use serde_json::{Map, Value};

pub(super) trait AnswerSourceLoader {
    fn load_http(&self, source: &str) -> GtcResult<Vec<u8>>;
    fn load_distributor(&self, source: &str) -> GtcResult<Vec<u8>>;
}

pub(super) struct DefaultAnswerSourceLoader;

impl AnswerSourceLoader for DefaultAnswerSourceLoader {
    fn load_http(&self, source: &str) -> GtcResult<Vec<u8>> {
        let client = Client::builder()
            .build()
            .map_err(|err| GtcError::message(format!("failed to create HTTP client: {err}")))?;
        let response = client
            .get(source)
            .header("Accept", "application/json")
            .header("User-Agent", format!("gtc/{}", env!("CARGO_PKG_VERSION")))
            .send()
            .map_err(|err| GtcError::message(format!("failed to fetch answers {source}: {err}")))?;
        let status = response.status();
        if !status.is_success() {
            return Err(GtcError::message(format!(
                "failed to fetch answers {source}: HTTP {status}"
            )));
        }
        response
            .bytes()
            .map(|bytes| bytes.to_vec())
            .map_err(|err| GtcError::message(format!("failed to read answers {source}: {err}")))
    }

    fn load_distributor(&self, source: &str) -> GtcResult<Vec<u8>> {
        let options = DistOptions::default();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| GtcError::message(format!("failed to create runtime: {err}")))?;
        let resolved = match classify_answers_source(source)? {
            AnswerSourceKind::Distributor => {
                if source.starts_with("store://") {
                    let client = DistClient::new(options);
                    let artifact = runtime
                        .block_on(client.download_store_artifact(source))
                        .map_err(|err| {
                            GtcError::message(format!(
                                "failed to fetch answers source {source}: {err}"
                            ))
                        })?;
                    DistributedAnswerBytes {
                        bytes: artifact.bytes,
                        media_type: artifact.media_type,
                    }
                } else {
                    let mapped = map_oci_answers_reference(source, &options)?;
                    let artifact = runtime
                        .block_on(fetch_pack_to_cache_with_options(
                            &mapped,
                            answer_pack_fetch_options(&options),
                        ))
                        .map_err(|err| {
                            GtcError::message(format!(
                                "failed to fetch answers source {source}: {err}"
                            ))
                        })?;
                    let bytes = fs::read(&artifact.path).map_err(|err| {
                        GtcError::io(
                            format!(
                                "failed to read resolved answers {}",
                                artifact.path.display()
                            ),
                            err,
                        )
                    })?;
                    DistributedAnswerBytes {
                        bytes,
                        media_type: artifact.media_type,
                    }
                }
            }
            _ => unreachable!("load_distributor is only called for distributor answer sources"),
        };
        if !is_json_media_type(&resolved.media_type) {
            return Err(GtcError::invalid_data(
                "answers OCI artifact",
                format!(
                    "{source} resolved to media type {}; expected application/json or a +json media type",
                    resolved.media_type
                ),
            ));
        }
        Ok(resolved.bytes)
    }
}

struct DistributedAnswerBytes {
    bytes: Vec<u8>,
    media_type: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AnswerSourceKind {
    LocalPath,
    FileUrl,
    Http,
    Distributor,
}

pub(super) fn classify_answers_source(source: &str) -> GtcResult<AnswerSourceKind> {
    match source.split_once("://").map(|(scheme, _)| scheme) {
        None => Ok(AnswerSourceKind::LocalPath),
        Some("file") => Ok(AnswerSourceKind::FileUrl),
        Some("http") | Some("https") => Ok(AnswerSourceKind::Http),
        Some("oci") | Some("store") | Some("repo") => Ok(AnswerSourceKind::Distributor),
        Some(scheme) => Err(GtcError::invalid_data(
            "answers source",
            format!(
                "unsupported scheme '{scheme}' for {source}; expected local path, file://, http://, https://, oci://, store://, or repo://"
            ),
        )),
    }
}

pub(super) fn load_answers(source: &str) -> GtcResult<Map<String, Value>> {
    load_answers_with(source, &DefaultAnswerSourceLoader)
}

pub(super) fn load_answers_with(
    source: &str,
    loader: &dyn AnswerSourceLoader,
) -> GtcResult<Map<String, Value>> {
    let bytes = load_answer_bytes(source, loader)?;
    parse_answers_bytes(source, &bytes)
}

pub(super) fn load_answer_bytes(
    source: &str,
    loader: &dyn AnswerSourceLoader,
) -> GtcResult<Vec<u8>> {
    match classify_answers_source(source)? {
        AnswerSourceKind::LocalPath => fs::read(source)
            .map_err(|err| GtcError::io(format!("failed to read answers file {source}"), err)),
        AnswerSourceKind::FileUrl => {
            let path = file_url_path(source).ok_or_else(|| {
                GtcError::invalid_data("answers source", format!("invalid file URL: {source}"))
            })?;
            fs::read(&path).map_err(|err| {
                GtcError::io(
                    format!("failed to read answers file {}", path.display()),
                    err,
                )
            })
        }
        AnswerSourceKind::Http => loader.load_http(source),
        AnswerSourceKind::Distributor => loader.load_distributor(source),
    }
}

pub(super) fn parse_answers_bytes(source: &str, bytes: &[u8]) -> GtcResult<Map<String, Value>> {
    match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(object)) => Ok(object),
        Ok(_) => Err(GtcError::invalid_data(
            "answers JSON",
            format!("{source} must contain a JSON object"),
        )),
        Err(err) => Err(GtcError::json(
            format!("failed to parse answers JSON from {source}"),
            err,
        )),
    }
}

fn file_url_path(url: &str) -> Option<PathBuf> {
    let path = url.strip_prefix("file://")?;
    if path.is_empty() {
        return None;
    }
    Some(PathBuf::from(path))
}

fn map_oci_answers_reference(source: &str, options: &DistOptions) -> GtcResult<String> {
    if let Some(reference) = source.strip_prefix("oci://") {
        validate_oci_reference(source, reference)?;
        return Ok(reference.to_string());
    }

    if let Some(target) = source.strip_prefix("repo://") {
        return map_registry_target(source, target, options.repo_registry_base.as_deref());
    }

    Err(GtcError::invalid_data(
        "answers source",
        format!("unsupported distributor answers source {source}"),
    ))
}

fn map_registry_target(source: &str, target: &str, base: Option<&str>) -> GtcResult<String> {
    if Reference::try_from(target).is_ok() {
        return Ok(target.to_string());
    }
    let Some(base) = base else {
        return Err(GtcError::message(format!(
            "{source} requires GREENTIC_REPO_REGISTRY_BASE to map to OCI"
        )));
    };
    let mapped = format!(
        "{}/{}",
        base.trim_end_matches('/'),
        target.trim_start_matches('/')
    );
    validate_oci_reference(source, &mapped)?;
    Ok(mapped)
}

fn validate_oci_reference(source: &str, reference: &str) -> GtcResult<()> {
    Reference::try_from(reference).map(|_| ()).map_err(|err| {
        GtcError::invalid_data(
            "answers source",
            format!("{} is not a valid OCI reference: {}", source, err),
        )
    })
}

fn answer_pack_fetch_options(options: &DistOptions) -> PackFetchOptions {
    let mut accepted = vec![
        "application/json".to_string(),
        "application/vnd.greentic.answers.v1+json".to_string(),
        "application/vnd.greentic.answers.create.v1+json".to_string(),
        "application/vnd.greentic.answers.setup.v1+json".to_string(),
    ];
    extend_unique_media_types(&mut accepted, default_pack_layer_media_types());
    PackFetchOptions {
        allow_tags: options.allow_tags,
        offline: options.offline,
        cache_dir: options.cache_dir.join("answers"),
        accepted_layer_media_types: accepted,
        preferred_layer_media_types: vec![
            "application/vnd.greentic.answers.setup.v1+json".to_string(),
            "application/vnd.greentic.answers.create.v1+json".to_string(),
            "application/vnd.greentic.answers.v1+json".to_string(),
            "application/json".to_string(),
        ],
        ..PackFetchOptions::default()
    }
}

fn extend_unique_media_types<I>(accepted: &mut Vec<String>, media_types: I)
where
    I: IntoIterator<Item = String>,
{
    for media_type in media_types {
        if !accepted.iter().any(|candidate| candidate == &media_type) {
            accepted.push(media_type);
        }
    }
}

fn is_json_media_type(media_type: &str) -> bool {
    media_type == "application/json" || media_type.ends_with("+json")
}

#[cfg(test)]
mod tests {
    use super::{
        AnswerSourceKind, AnswerSourceLoader, answer_pack_fetch_options, classify_answers_source,
        is_json_media_type, load_answer_bytes, load_answers_with, map_oci_answers_reference,
    };
    use greentic_distributor_client::DistOptions;
    use gtc::error::{GtcError, GtcResult};
    use std::cell::RefCell;
    use std::fs;
    use std::path::PathBuf;

    #[derive(Default)]
    struct MockLoader {
        http: RefCell<Vec<String>>,
        dist: RefCell<Vec<String>>,
    }

    impl AnswerSourceLoader for MockLoader {
        fn load_http(&self, source: &str) -> GtcResult<Vec<u8>> {
            self.http.borrow_mut().push(source.to_string());
            Ok(br#"{"from":"http"}"#.to_vec())
        }

        fn load_distributor(&self, source: &str) -> GtcResult<Vec<u8>> {
            self.dist.borrow_mut().push(source.to_string());
            Ok(format!(r#"{{"from":"{source}"}}"#).into_bytes())
        }
    }

    #[test]
    fn classifies_supported_answer_sources() {
        assert_eq!(
            classify_answers_source("answers.json").unwrap(),
            AnswerSourceKind::LocalPath
        );
        assert_eq!(
            classify_answers_source("file:///tmp/answers.json").unwrap(),
            AnswerSourceKind::FileUrl
        );
        assert_eq!(
            classify_answers_source("https://example.com/answers.json").unwrap(),
            AnswerSourceKind::Http
        );
        assert_eq!(
            classify_answers_source("oci://ghcr.io/acme/answers:stable").unwrap(),
            AnswerSourceKind::Distributor
        );
        assert_eq!(
            classify_answers_source("store://acme/answers:stable").unwrap(),
            AnswerSourceKind::Distributor
        );
        assert_eq!(
            classify_answers_source("repo://acme/answers:stable").unwrap(),
            AnswerSourceKind::Distributor
        );
    }

    #[test]
    fn rejects_invalid_scheme() {
        let err = classify_answers_source("ftp://example.com/answers.json").unwrap_err();
        assert!(matches!(err, GtcError::InvalidData { .. }));
        assert!(err.to_string().contains("unsupported scheme"));
    }

    #[test]
    fn loads_local_file_answers() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("answers.json");
        fs::write(&path, br#"{"ok":true}"#).expect("write");

        let answers = load_answers_with(path.to_str().expect("utf8"), &MockLoader::default())
            .expect("answers");

        assert_eq!(
            answers.get("ok").and_then(serde_json::Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn loads_file_url_answers() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("answers.json");
        fs::write(&path, br#"{"ok":"file"}"#).expect("write");

        let answers = load_answers_with(
            &format!("file://{}", path.display()),
            &MockLoader::default(),
        )
        .expect("answers");

        assert_eq!(
            answers.get("ok").and_then(serde_json::Value::as_str),
            Some("file")
        );
    }

    #[test]
    fn loads_https_answers_through_injected_loader() {
        let loader = MockLoader::default();
        let answers =
            load_answers_with("https://example.com/answers.json", &loader).expect("answers");

        assert_eq!(
            answers.get("from").and_then(serde_json::Value::as_str),
            Some("http")
        );
        assert_eq!(
            loader.http.borrow().as_slice(),
            ["https://example.com/answers.json"]
        );
    }

    #[test]
    fn loads_distributor_answer_schemes_through_injected_loader() {
        let loader = MockLoader::default();
        for source in [
            "oci://ghcr.io/acme/answers:stable",
            "store://acme/answers:stable",
            "repo://acme/answers:stable",
        ] {
            let answers = load_answers_with(source, &loader).expect("answers");
            assert_eq!(
                answers.get("from").and_then(serde_json::Value::as_str),
                Some(source)
            );
        }
        assert_eq!(loader.dist.borrow().len(), 3);
    }

    #[test]
    fn rejects_invalid_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("answers.json");
        fs::write(&path, b"{not-json").expect("write");

        let err =
            load_answers_with(path.to_str().expect("utf8"), &MockLoader::default()).unwrap_err();

        assert!(matches!(err, GtcError::Json { .. }));
        assert!(err.to_string().contains("failed to parse answers JSON"));
    }

    #[test]
    fn rejects_non_object_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("answers.json");
        fs::write(&path, b"[]").expect("write");

        let err =
            load_answers_with(path.to_str().expect("utf8"), &MockLoader::default()).unwrap_err();

        assert!(matches!(err, GtcError::InvalidData { .. }));
        assert!(err.to_string().contains("must contain a JSON object"));
    }

    #[test]
    fn raw_byte_loader_uses_distributor_for_oci() {
        let loader = MockLoader::default();

        let bytes = load_answer_bytes("oci://ghcr.io/acme/answers:stable", &loader).expect("bytes");

        assert!(String::from_utf8(bytes).unwrap().contains("oci://"));
        assert_eq!(loader.dist.borrow().len(), 1);
    }

    #[test]
    fn json_media_type_accepts_standard_and_vendor_json() {
        assert!(is_json_media_type("application/json"));
        assert!(is_json_media_type(
            "application/vnd.greentic.answers.v1+json"
        ));
        assert!(is_json_media_type(
            "application/vnd.greentic.answers.create.v1+json"
        ));
        assert!(is_json_media_type(
            "application/vnd.greentic.answers.setup.v1+json"
        ));
        assert!(!is_json_media_type("application/wasm"));
        assert!(!is_json_media_type("application/octet-stream"));
    }

    #[test]
    fn answer_pack_fetch_options_prefers_answers_json_layers() {
        let options = DistOptions {
            cache_dir: PathBuf::from("/tmp/gtc-answer-test-cache"),
            allow_tags: true,
            offline: false,
            allow_insecure_local_http: false,
            cache_max_bytes: 42,
            repo_registry_base: None,
            store_registry_base: None,
            store_auth_path: PathBuf::from("/tmp/store-auth.json"),
            store_state_path: PathBuf::from("/tmp/store-state.json"),
        };

        let fetch_options = answer_pack_fetch_options(&options);

        assert_eq!(
            fetch_options
                .preferred_layer_media_types
                .first()
                .map(String::as_str),
            Some("application/vnd.greentic.answers.setup.v1+json")
        );
        for media_type in [
            "application/json",
            "application/vnd.greentic.answers.v1+json",
            "application/vnd.greentic.answers.create.v1+json",
            "application/vnd.greentic.answers.setup.v1+json",
        ] {
            assert!(
                fetch_options
                    .accepted_layer_media_types
                    .iter()
                    .any(|accepted| accepted == media_type),
                "missing accepted media type {media_type}"
            );
            assert!(
                fetch_options
                    .preferred_layer_media_types
                    .iter()
                    .any(|preferred| preferred == media_type),
                "missing preferred media type {media_type}"
            );
        }
    }

    #[test]
    fn maps_oci_and_repo_answers_to_oci_references() {
        let options = DistOptions {
            cache_dir: PathBuf::from("/tmp/gtc-answer-test-cache"),
            allow_tags: true,
            offline: false,
            allow_insecure_local_http: false,
            cache_max_bytes: 42,
            repo_registry_base: Some("ghcr.io/acme".to_string()),
            store_registry_base: None,
            store_auth_path: PathBuf::from("/tmp/store-auth.json"),
            store_state_path: PathBuf::from("/tmp/store-state.json"),
        };

        assert_eq!(
            map_oci_answers_reference("oci://ghcr.io/acme/answers:stable", &options).unwrap(),
            "ghcr.io/acme/answers:stable"
        );
        assert_eq!(
            map_oci_answers_reference("repo:///answers:stable", &options).unwrap(),
            "ghcr.io/acme/answers:stable"
        );
    }

    #[test]
    fn repo_answers_require_registry_base_when_target_is_not_oci() {
        let options = DistOptions {
            cache_dir: PathBuf::from("/tmp/gtc-answer-test-cache"),
            allow_tags: true,
            offline: false,
            allow_insecure_local_http: false,
            cache_max_bytes: 42,
            repo_registry_base: None,
            store_registry_base: None,
            store_auth_path: PathBuf::from("/tmp/store-auth.json"),
            store_state_path: PathBuf::from("/tmp/store-state.json"),
        };

        let err = map_oci_answers_reference("repo:///answers:stable", &options).unwrap_err();

        assert!(err.to_string().contains("GREENTIC_REPO_REGISTRY_BASE"));
    }
}
