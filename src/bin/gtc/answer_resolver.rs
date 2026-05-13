use std::fs;
use std::path::PathBuf;

use greentic_distributor_client::{CachePolicy, DistClient, DistOptions, ResolvePolicy};
use gtc::error::{GtcError, GtcResult};
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
        let client = DistClient::new(DistOptions::default());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| GtcError::message(format!("failed to create runtime: {err}")))?;
        let artifact_source = client.parse_source(source).map_err(|err| {
            GtcError::message(format!("failed to parse answers source {source}: {err}"))
        })?;
        let descriptor = runtime
            .block_on(client.resolve(artifact_source, ResolvePolicy))
            .map_err(|err| {
                GtcError::message(format!("failed to resolve answers source {source}: {err}"))
            })?;
        let artifact = runtime
            .block_on(client.fetch(&descriptor, CachePolicy))
            .map_err(|err| {
                GtcError::message(format!("failed to fetch answers source {source}: {err}"))
            })?;
        artifact
            .wasm_bytes()
            .map(|bytes| bytes.to_vec())
            .map_err(|err| {
                GtcError::message(format!("failed to read resolved answers {source}: {err}"))
            })
    }
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

#[cfg(test)]
mod tests {
    use super::{
        AnswerSourceKind, AnswerSourceLoader, classify_answers_source, load_answer_bytes,
        load_answers_with,
    };
    use gtc::error::{GtcError, GtcResult};
    use std::cell::RefCell;
    use std::fs;

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
}
