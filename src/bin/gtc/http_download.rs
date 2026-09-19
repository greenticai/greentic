//! HTTP(S) downloads for `gtc install`, self-update and bundle fetches.
//!
//! # Why there is no total-request timeout
//!
//! `reqwest::blocking` defaults to a 30-second timeout, and `Response::bytes()`
//! applies it to the WHOLE body. A ~33 MB toolchain archive on a slow or
//! congested link cannot finish in 30 s, so every attempt died with
//! `failed to read response body: error decoding response body` (issue #346) —
//! and a retry hit exactly the same wall.
//!
//! What we want to bound is a transfer that has STOPPED, not one that is slow.
//! In the pinned reqwest (0.13) the blocking client's `timeout` bounds
//! `send()` (connect + response headers) and each individual
//! `Response::read()` call, while `bytes()` wraps the whole body in one
//! timeout. So the client below sets `timeout` to the IDLE budget and the body
//! is streamed through `Read`: every read must make progress within
//! `idle_timeout`, and a download that keeps trickling is never cut off.
//!
//! # Retries
//!
//! Transient failures — connect errors, timeouts, a body that stalls or is
//! cut short, and HTTP 408/429/5xx — are retried with exponential backoff.
//! Every attempt restarts the download from the first byte (following the
//! redirect chain again), so a partial body is never stitched onto another.
//! Integrity is checked by the callers against the manifest's sha256 after the
//! bytes arrive, exactly as before.

use std::io::Read;
use std::time::Duration;

use gtc::error::{GtcError, GtcResult};
use reqwest::blocking::{Client, ClientBuilder};

use super::i18n_support::t;
use super::install::should_send_auth_header;

/// Timeouts and retry budget for one download.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DownloadPolicy {
    /// Upper bound on establishing the TCP/TLS connection.
    pub(super) connect_timeout: Duration,
    /// Upper bound on waiting for the response headers, and on any single gap
    /// between body chunks. Never a bound on the whole transfer.
    pub(super) idle_timeout: Duration,
    /// Total attempts, including the first one.
    pub(super) attempts: u32,
    /// Delay before the second attempt; doubles for each attempt after it.
    pub(super) base_backoff: Duration,
}

/// Production policy: 30 s to connect, 60 s of silence tolerated, and four
/// attempts spaced 2 s, 4 s, 8 s apart.
pub(super) const DOWNLOAD_POLICY: DownloadPolicy = DownloadPolicy {
    connect_timeout: Duration::from_secs(30),
    idle_timeout: Duration::from_secs(60),
    attempts: 4,
    base_backoff: Duration::from_secs(2),
};

const MAX_REDIRECTS: usize = 10;

#[cfg(test)]
thread_local! {
    static POLICY_OVERRIDE: std::cell::Cell<Option<DownloadPolicy>> =
        const { std::cell::Cell::new(None) };
}

/// Replace the policy for downloads made on THIS thread until the guard drops.
/// Lets the tests exercise the real entry point with millisecond timeouts
/// instead of sleeping through the production ones.
#[cfg(test)]
pub(super) fn override_policy_for_test(policy: DownloadPolicy) -> PolicyOverrideGuard {
    POLICY_OVERRIDE.with(|cell| cell.set(Some(policy)));
    PolicyOverrideGuard
}

#[cfg(test)]
pub(super) struct PolicyOverrideGuard;

#[cfg(test)]
impl Drop for PolicyOverrideGuard {
    fn drop(&mut self) {
        POLICY_OVERRIDE.with(|cell| cell.set(None));
    }
}

fn active_policy() -> DownloadPolicy {
    #[cfg(test)]
    if let Some(policy) = POLICY_OVERRIDE.with(std::cell::Cell::get) {
        return policy;
    }
    DOWNLOAD_POLICY
}

/// A client builder with the download timeouts applied: a connect timeout and
/// an idle timeout, and no cap on the total transfer time (see module docs).
pub(super) fn download_client_builder() -> ClientBuilder {
    client_builder_for(active_policy())
}

fn client_builder_for(policy: DownloadPolicy) -> ClientBuilder {
    Client::builder()
        .connect_timeout(policy.connect_timeout)
        .timeout(policy.idle_timeout)
}

/// Download `url`, following redirects manually so the bearer `key` is only
/// sent to the original authority. Retries transient failures; see the module
/// docs for the timeout design.
pub(crate) fn fetch_https_bytes(
    url: &str,
    key: &str,
    locale: &str,
    accept: &str,
) -> GtcResult<Vec<u8>> {
    let policy = active_policy();
    let client = client_builder_for(policy)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| GtcError::message(format!("failed to create HTTP client: {e}")))?;

    let attempts = policy.attempts.max(1);
    let mut attempt = 1;
    loop {
        match fetch_once(&client, url, key, locale, accept, policy) {
            Ok(bytes) => return Ok(bytes),
            Err(AttemptError::Transient(error)) if attempt < attempts => {
                let delay = backoff_delay(policy.base_backoff, attempt);
                eprintln!(
                    "download {url}: attempt {attempt}/{attempts} failed ({error}); \
                     retrying in {}s",
                    delay.as_secs_f32()
                );
                std::thread::sleep(delay);
                attempt += 1;
            }
            Err(AttemptError::Transient(error)) if attempts > 1 => {
                return Err(GtcError::message(format!(
                    "{error} (gave up after {attempts} attempts)"
                )));
            }
            Err(AttemptError::Transient(error)) | Err(AttemptError::Permanent(error)) => {
                return Err(error);
            }
        }
    }
}

/// `base * 2^(attempt-1)`, saturating rather than overflowing.
fn backoff_delay(base: Duration, attempt: u32) -> Duration {
    let factor = 1_u32
        .checked_shl(attempt.saturating_sub(1))
        .unwrap_or(u32::MAX);
    base.saturating_mul(factor)
}

enum AttemptError {
    /// Worth another attempt: the network, not the request, failed.
    Transient(GtcError),
    /// Retrying would get the same answer (bad redirect, 404, 401, ...).
    Permanent(GtcError),
}

fn fetch_once(
    client: &Client,
    url: &str,
    key: &str,
    locale: &str,
    accept: &str,
    policy: DownloadPolicy,
) -> Result<Vec<u8>, AttemptError> {
    let mut current = reqwest::Url::parse(url).map_err(|e| {
        AttemptError::Permanent(GtcError::invalid_data(
            "download URL",
            format!("{url}: {e}"),
        ))
    })?;
    let original = current.clone();
    for _ in 0..MAX_REDIRECTS {
        let mut request = client
            .get(current.clone())
            .header("Accept", accept)
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", format!("gtc/{}", env!("CARGO_PKG_VERSION")));
        if !key.is_empty() && should_send_auth_header(&original, &current) {
            request = request.header("Authorization", format!("Bearer {key}"));
        }
        let mut response = request.send().map_err(|e| {
            let error = GtcError::message(format!("{}: {e}", t(locale, "gtc.err.pull_failed")));
            if e.is_builder() || e.is_redirect() {
                AttemptError::Permanent(error)
            } else {
                AttemptError::Transient(error)
            }
        })?;

        if response.status().is_redirection() {
            current = redirect_target(&current, &response).map_err(AttemptError::Permanent)?;
            continue;
        }

        let status = response.status();
        if !status.is_success() {
            let error = GtcError::message(format!(
                "{}: HTTP {} for {}",
                t(locale, "gtc.err.pull_failed"),
                status,
                current
            ));
            return Err(if is_transient_status(status) {
                AttemptError::Transient(error)
            } else {
                AttemptError::Permanent(error)
            });
        }

        return read_body(&mut response, policy).map_err(AttemptError::Transient);
    }

    Err(AttemptError::Permanent(GtcError::invalid_data(
        "redirect handling",
        format!("too many redirects while fetching {url}"),
    )))
}

fn redirect_target(
    current: &reqwest::Url,
    response: &reqwest::blocking::Response,
) -> GtcResult<reqwest::Url> {
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .ok_or_else(|| {
            GtcError::invalid_data(
                "redirect response",
                format!("missing Location header for {current}"),
            )
        })?
        .to_str()
        .map_err(|e| {
            GtcError::invalid_data(
                "redirect response",
                format!("invalid Location for {current}: {e}"),
            )
        })?;
    current.join(location).map_err(|e| {
        GtcError::invalid_data(
            "redirect response",
            format!("invalid redirect target {location}: {e}"),
        )
    })
}

fn is_transient_status(status: reqwest::StatusCode) -> bool {
    status.is_server_error()
        || status == reqwest::StatusCode::REQUEST_TIMEOUT
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
}

/// Stream the body through `Read`, where each read is bounded by the idle
/// timeout and the transfer as a whole is not.
fn read_body(
    response: &mut reqwest::blocking::Response,
    policy: DownloadPolicy,
) -> GtcResult<Vec<u8>> {
    // Cap the pre-allocation: Content-Length is the server's claim, not ours.
    const MAX_PREALLOCATION: u64 = 256 * 1024 * 1024;
    let capacity = response
        .content_length()
        .map(|len| len.min(MAX_PREALLOCATION))
        .and_then(|len| usize::try_from(len).ok())
        .unwrap_or(0);
    let mut body = Vec::with_capacity(capacity);
    let mut chunk = vec![0_u8; 64 * 1024];
    loop {
        match response.read(&mut chunk) {
            Ok(0) => return Ok(body),
            Ok(read) => body.extend_from_slice(&chunk[..read]),
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) => {
                let received = body.len();
                return Err(if is_timeout(&err) {
                    GtcError::message(format!(
                        "failed to read response body: no data received for {}s \
                         after {received} bytes",
                        policy.idle_timeout.as_secs_f32()
                    ))
                } else {
                    GtcError::message(format!(
                        "failed to read response body after {received} bytes: {err}"
                    ))
                });
            }
        }
    }
}

fn is_timeout(err: &std::io::Error) -> bool {
    if err.kind() == std::io::ErrorKind::TimedOut {
        return true;
    }
    let mut source: Option<&(dyn std::error::Error + 'static)> = err.get_ref().map(|e| e as _);
    while let Some(current) = source {
        if let Some(reqwest_err) = current.downcast_ref::<reqwest::Error>()
            && reqwest_err.is_timeout()
        {
            return true;
        }
        if let Some(io_err) = current.downcast_ref::<std::io::Error>()
            && io_err.kind() == std::io::ErrorKind::TimedOut
        {
            return true;
        }
        source = current.source();
    }
    false
}

#[cfg(test)]
#[path = "http_download_test_server.rs"]
pub(super) mod test_server;

#[cfg(test)]
#[path = "http_download_tests.rs"]
mod tests;
