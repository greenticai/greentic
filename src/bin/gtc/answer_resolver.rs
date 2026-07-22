use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use greentic_distributor_client::{
    DistClient, DistOptions, PackFetchOptions, default_pack_layer_media_types,
    fetch_pack_to_cache_with_options,
};
use gtc::error::{GtcError, GtcResult};
use oci_distribution::Reference;
use reqwest::blocking::Client;
use serde_json::{Map, Value};

/// Maximum size of an answers document fetched over HTTP (10 MiB).
const HTTP_MAX_BYTES: usize = 10 * 1024 * 1024;

/// Maximum number of HTTP redirects to follow.
const HTTP_MAX_REDIRECTS: usize = 10;

/// HTTP connect timeout for answers fetches.
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Total HTTP timeout for answers fetches.
const HTTP_TIMEOUT: Duration = Duration::from_secs(60);

pub(super) trait AnswerSourceLoader {
    fn load_http(&self, source: &str) -> GtcResult<Vec<u8>>;
    fn load_distributor(&self, source: &str) -> GtcResult<Vec<u8>>;
}

pub(super) struct DefaultAnswerSourceLoader;

impl AnswerSourceLoader for DefaultAnswerSourceLoader {
    fn load_http(&self, source: &str) -> GtcResult<Vec<u8>> {
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(HTTP_CONNECT_TIMEOUT)
            .timeout(HTTP_TIMEOUT)
            .build()
            .map_err(|err| GtcError::message(format!("failed to create HTTP client: {err}")))?;

        let mut current = reqwest::Url::parse(source).map_err(|err| {
            GtcError::invalid_data("answers source", format!("invalid URL {source}: {err}"))
        })?;
        let original_scheme = current.scheme().to_string();

        for _ in 0..HTTP_MAX_REDIRECTS {
            let response = client
                .get(current.clone())
                .header("Accept", "application/json")
                .header("User-Agent", format!("gtc/{}", env!("CARGO_PKG_VERSION")))
                .send()
                .map_err(|err| {
                    GtcError::message(format!("failed to fetch answers {source}: {err}"))
                })?;

            if response.status().is_redirection() {
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .ok_or_else(|| {
                        GtcError::invalid_data(
                            "answers fetch",
                            format!("redirect from {current} has no Location header"),
                        )
                    })?
                    .to_str()
                    .map_err(|err| {
                        GtcError::invalid_data(
                            "answers fetch",
                            format!("invalid Location header from {current}: {err}"),
                        )
                    })?;
                let next = current.join(location).map_err(|err| {
                    GtcError::invalid_data(
                        "answers fetch",
                        format!("invalid redirect target {location}: {err}"),
                    )
                })?;
                validate_redirect_scheme(&original_scheme, next.scheme(), &next)?;
                current = next;
                continue;
            }

            let status = response.status();
            if !status.is_success() {
                return Err(GtcError::message(format!(
                    "failed to fetch answers {source}: HTTP {status}"
                )));
            }
            validate_content_type(
                response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok()),
                source,
            )?;
            // Fast-path: reject immediately when Content-Length declares a
            // body larger than the cap, avoiding any body read.
            if let Some(declared) = response
                .content_length()
                .filter(|&len| len > HTTP_MAX_BYTES as u64)
            {
                return Err(GtcError::invalid_data(
                    "answers fetch",
                    format!(
                        "{source} Content-Length is {declared} bytes, \
                         exceeding the {HTTP_MAX_BYTES} byte limit",
                    ),
                ));
            }
            // Stream at most cap+1 bytes so a hostile or misconfigured
            // endpoint cannot exhaust memory regardless of what
            // Content-Length claims (or whether it is present at all).
            let mut buf = Vec::new();
            response
                .take(HTTP_MAX_BYTES as u64 + 1)
                .read_to_end(&mut buf)
                .map_err(|err| {
                    GtcError::message(format!("failed to read answers {source}: {err}"))
                })?;
            validate_response_size(buf.len(), source)?;
            return Ok(buf);
        }

        Err(GtcError::invalid_data(
            "answers fetch",
            format!("too many redirects while fetching {source}"),
        ))
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
            format!("{source} is not a valid OCI reference: {err}"),
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

/// Reject HTTPS-to-HTTP scheme downgrades during redirect following.
fn validate_redirect_scheme(
    original_scheme: &str,
    target_scheme: &str,
    target: &reqwest::Url,
) -> GtcResult<()> {
    if original_scheme == "https" && target_scheme != "https" {
        return Err(GtcError::invalid_data(
            "answers fetch",
            format!("refusing redirect to {target}: HTTPS to HTTP downgrade"),
        ));
    }
    Ok(())
}

/// Reject responses whose Content-Type is clearly not JSON (e.g. HTML error
/// pages). Missing Content-Type is allowed because some CDNs and GitHub raw
/// URLs omit it.
fn validate_content_type(content_type: Option<&str>, source: &str) -> GtcResult<()> {
    let Some(ct) = content_type else {
        return Ok(());
    };
    let media_type = ct.split(';').next().unwrap_or("").trim();
    if media_type.is_empty()
        || media_type == "application/json"
        || media_type.ends_with("+json")
        || media_type == "application/octet-stream"
        || media_type == "text/plain"
    {
        return Ok(());
    }
    Err(GtcError::invalid_data(
        "answers fetch",
        format!(
            "{source} returned Content-Type '{ct}'; expected application/json or a +json variant"
        ),
    ))
}

/// Reject responses that exceed the size cap.
fn validate_response_size(len: usize, source: &str) -> GtcResult<()> {
    if len > HTTP_MAX_BYTES {
        return Err(GtcError::invalid_data(
            "answers fetch",
            format!(
                "{source} response is {len} bytes, exceeding the {} byte limit",
                HTTP_MAX_BYTES
            ),
        ));
    }
    Ok(())
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

    // -- redirect scheme validation --

    #[test]
    fn rejects_https_to_http_redirect() {
        let target = reqwest::Url::parse("http://evil.test/answers.json").unwrap();
        let err = super::validate_redirect_scheme("https", "http", &target).unwrap_err();
        assert!(matches!(err, GtcError::InvalidData { .. }));
        assert!(err.to_string().contains("HTTPS to HTTP downgrade"));
    }

    #[test]
    fn accepts_https_to_https_redirect() {
        let target = reqwest::Url::parse("https://cdn.example.com/answers.json").unwrap();
        super::validate_redirect_scheme("https", "https", &target).unwrap();
    }

    #[test]
    fn accepts_http_to_https_upgrade_redirect() {
        let target = reqwest::Url::parse("https://secure.example.com/answers.json").unwrap();
        super::validate_redirect_scheme("http", "https", &target).unwrap();
    }

    #[test]
    fn accepts_http_to_http_redirect() {
        let target = reqwest::Url::parse("http://other.example.com/answers.json").unwrap();
        super::validate_redirect_scheme("http", "http", &target).unwrap();
    }

    // -- content-type validation --

    #[test]
    fn rejects_html_content_type() {
        let err =
            super::validate_content_type(Some("text/html"), "https://x.test/a.json").unwrap_err();
        assert!(matches!(err, GtcError::InvalidData { .. }));
        assert!(err.to_string().contains("Content-Type"));
    }

    #[test]
    fn rejects_html_content_type_with_charset() {
        let err =
            super::validate_content_type(Some("text/html; charset=utf-8"), "https://x.test/a.json")
                .unwrap_err();
        assert!(err.to_string().contains("Content-Type"));
    }

    #[test]
    fn accepts_application_json_content_type() {
        super::validate_content_type(Some("application/json"), "src").unwrap();
    }

    #[test]
    fn accepts_vendor_json_content_type() {
        super::validate_content_type(Some("application/vnd.greentic.answers.v1+json"), "src")
            .unwrap();
    }

    #[test]
    fn accepts_octet_stream_content_type() {
        super::validate_content_type(Some("application/octet-stream"), "src").unwrap();
    }

    #[test]
    fn accepts_text_plain_content_type() {
        super::validate_content_type(Some("text/plain"), "src").unwrap();
    }

    #[test]
    fn accepts_missing_content_type() {
        super::validate_content_type(None, "src").unwrap();
    }

    #[test]
    fn accepts_json_content_type_with_charset() {
        super::validate_content_type(Some("application/json; charset=utf-8"), "src").unwrap();
    }

    // -- response size validation --

    #[test]
    fn rejects_oversized_response() {
        let err =
            super::validate_response_size(super::HTTP_MAX_BYTES + 1, "https://x.test/big.json")
                .unwrap_err();
        assert!(matches!(err, GtcError::InvalidData { .. }));
        assert!(err.to_string().contains("exceeding"));
    }

    #[test]
    fn accepts_response_at_size_limit() {
        super::validate_response_size(super::HTTP_MAX_BYTES, "src").unwrap();
    }

    #[test]
    fn accepts_small_response() {
        super::validate_response_size(42, "src").unwrap();
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

    // -- call-site integration tests (drive load_http against a local server) --
    //
    // These prove the guards are wired into DefaultAnswerSourceLoader::load_http,
    // not just that the validation functions work in isolation.

    use std::io::Write;
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;

    fn drain_http_request(stream: &mut std::net::TcpStream) {
        let mut buf = [0u8; 4096];
        let mut request = Vec::new();
        loop {
            let n = std::io::Read::read(stream, &mut buf).expect("read request");
            request.extend_from_slice(&buf[..n]);
            if request.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
    }

    #[test]
    #[ignore = "requires local socket binding"]
    fn load_http_rejects_html_content_type() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            drain_http_request(&mut stream);
            let body = r#"<html><body>Not Found</body></html>"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\n\
                 Content-Type: text/html\r\n\
                 Content-Length: {}\r\n\r\n\
                 {body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();
        });

        let loader = super::DefaultAnswerSourceLoader;
        let err = loader
            .load_http(&format!("http://{addr}/answers.json"))
            .unwrap_err();

        assert!(err.to_string().contains("Content-Type"));
        handle.join().expect("join");
    }

    #[test]
    #[ignore = "requires local socket binding"]
    fn load_http_rejects_oversized_content_length() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            drain_http_request(&mut stream);
            // Declare a Content-Length far above the cap. The client should
            // reject before reading any body, so we never send body bytes.
            let response = format!(
                "HTTP/1.1 200 OK\r\n\
                 Content-Type: application/json\r\n\
                 Content-Length: {}\r\n\r\n",
                super::HTTP_MAX_BYTES + 1_000_000,
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();
        });

        let loader = super::DefaultAnswerSourceLoader;
        let err = loader
            .load_http(&format!("http://{addr}/answers.json"))
            .unwrap_err();

        assert!(
            err.to_string().contains("Content-Length"),
            "expected Content-Length rejection, got: {err}"
        );
        handle.join().expect("join");
    }

    #[test]
    #[ignore = "requires local socket binding"]
    fn load_http_caps_body_without_content_length() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let (tx, rx) = mpsc::channel::<usize>();
        let cap = super::HTTP_MAX_BYTES;
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            drain_http_request(&mut stream);
            // No Content-Length header. The client must enforce the cap via
            // the streaming .take() guard.
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\n\
                      Content-Type: application/json\r\n\
                      Transfer-Encoding: identity\r\n\r\n",
                )
                .unwrap();
            let chunk = vec![b'x'; 8192];
            let mut written = 0usize;
            // Send cap + 8192 bytes — more than the cap but not so much that
            // the test is slow.
            let target = cap + 8192;
            while written < target {
                let n = std::cmp::min(8192, target - written);
                if stream.write_all(&chunk[..n]).is_err() {
                    break; // client closed the connection
                }
                written += n;
            }
            tx.send(written).ok();
        });

        let loader = super::DefaultAnswerSourceLoader;
        let err = loader
            .load_http(&format!("http://{addr}/answers.json"))
            .unwrap_err();

        assert!(
            err.to_string().contains("exceeding"),
            "expected size-cap rejection, got: {err}"
        );
        // The server may or may not have finished sending before the client
        // closed. Either way, the client read at most cap+1 bytes.
        let _ = rx.recv();
        handle.join().expect("join");
    }

    #[test]
    #[ignore = "requires local socket binding"]
    fn load_http_follows_redirect_to_json() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let handle = thread::spawn(move || {
            // First request: redirect.
            let (mut stream, _) = listener.accept().expect("accept first");
            drain_http_request(&mut stream);
            let response = "HTTP/1.1 302 Found\r\n\
                            Location: /final\r\n\
                            Connection: close\r\n\
                            Content-Length: 0\r\n\r\n";
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();

            // Second request: serve JSON.
            let (mut stream, _) = listener.accept().expect("accept second");
            drain_http_request(&mut stream);
            let body = br#"{"ok":true}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\n\
                 Content-Type: application/json\r\n\
                 Content-Length: {}\r\n\r\n",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            stream.write_all(body).unwrap();
            stream.flush().unwrap();
        });

        let loader = super::DefaultAnswerSourceLoader;
        let bytes = loader
            .load_http(&format!("http://{addr}/start"))
            .expect("should follow redirect and return JSON");

        assert_eq!(bytes, br#"{"ok":true}"#);
        handle.join().expect("join");
    }
}
