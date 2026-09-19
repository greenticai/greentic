use std::time::{Duration, Instant};

use super::test_server::{Reply, serve};
use super::{DOWNLOAD_POLICY, DownloadPolicy, backoff_delay, fetch_https_bytes};

/// Millisecond-scale stand-in for the production policy: the same shape,
/// scaled down so a "stall" costs a fraction of a second.
fn fast_policy(attempts: u32) -> DownloadPolicy {
    DownloadPolicy {
        connect_timeout: Duration::from_secs(2),
        idle_timeout: Duration::from_millis(300),
        attempts,
        base_backoff: Duration::from_millis(10),
    }
}

fn fetch(url: &str) -> gtc::error::GtcResult<Vec<u8>> {
    fetch_https_bytes(url, "", "en", "application/octet-stream")
}

fn payload(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

#[test]
fn production_policy_bounds_silence_not_total_transfer_time() {
    // The whole fix: a connect bound and an idle bound, nothing else. If a
    // total cap is ever reintroduced it has to come through this struct.
    assert_eq!(DOWNLOAD_POLICY.connect_timeout, Duration::from_secs(30));
    assert_eq!(DOWNLOAD_POLICY.idle_timeout, Duration::from_secs(60));
    const { assert!(DOWNLOAD_POLICY.attempts >= 3) };
}

#[test]
fn backoff_doubles_per_attempt_and_saturates() {
    let base = Duration::from_secs(2);
    assert_eq!(backoff_delay(base, 1), Duration::from_secs(2));
    assert_eq!(backoff_delay(base, 2), Duration::from_secs(4));
    assert_eq!(backoff_delay(base, 3), Duration::from_secs(8));
    assert!(backoff_delay(base, 200) >= Duration::from_secs(2));
}

#[test]
fn a_body_trickling_far_longer_than_the_idle_timeout_still_completes() {
    let _policy = super::override_policy_for_test(fast_policy(1));
    let body = payload(12 * 1024);
    // 12 gaps of 150 ms: ~1.8 s in total, six times the 300 ms idle timeout,
    // with no single gap reaching it. Under a total-request cap of the idle
    // length this is exactly the download #346 lost.
    let server = serve(
        "tool.tgz",
        vec![Reply::Trickle {
            body: body.clone(),
            chunks: 12,
            gap: Duration::from_millis(150),
        }],
    );

    let started = Instant::now();
    let got = fetch(&server.url).expect("slow but live download completes");
    let elapsed = started.elapsed();

    assert_eq!(got, body);
    assert!(
        elapsed > Duration::from_millis(300) * 4,
        "transfer should have outlasted the idle timeout several times over, took {elapsed:?}"
    );
    assert_eq!(server.accepted(), 1, "no retry was needed");
    server.join();
}

#[test]
fn a_body_that_stalls_past_the_idle_timeout_is_restarted_from_the_first_byte() {
    let _policy = super::override_policy_for_test(fast_policy(3));
    let body = payload(4096);
    let server = serve(
        "tool.tgz",
        vec![
            Reply::Stall {
                body: body.clone(),
                sent: 1000,
                stall: Duration::from_millis(900),
            },
            Reply::Trickle {
                body: body.clone(),
                chunks: 1,
                gap: Duration::ZERO,
            },
        ],
    );

    let got = fetch(&server.url).expect("second attempt succeeds");

    // Exactly the served body: the 1000 bytes of the stalled attempt were
    // discarded, not prepended.
    assert_eq!(got, body);
    assert_eq!(server.accepted(), 2);
    server.join();
}

#[test]
fn a_download_that_keeps_stalling_gives_up_after_the_attempt_budget() {
    let _policy = super::override_policy_for_test(fast_policy(2));
    let body = payload(4096);
    let stall = || Reply::Stall {
        body: body.clone(),
        sent: 10,
        stall: Duration::from_millis(900),
    };
    let server = serve("tool.tgz", vec![stall(), stall()]);

    let err = fetch(&server.url).expect_err("every attempt stalls");
    let message = err.to_string();

    assert!(message.contains("no data received"), "{message}");
    assert!(message.contains("gave up after 2 attempts"), "{message}");
    assert_eq!(server.accepted(), 2);
    server.join();
}

#[test]
fn a_body_cut_short_is_retried() {
    let _policy = super::override_policy_for_test(fast_policy(3));
    let body = payload(4096);
    let server = serve(
        "tool.tgz",
        vec![
            Reply::Truncate {
                body: body.clone(),
                sent: 100,
            },
            Reply::Trickle {
                body: body.clone(),
                chunks: 1,
                gap: Duration::ZERO,
            },
        ],
    );

    assert_eq!(fetch(&server.url).expect("retry succeeds"), body);
    assert_eq!(server.accepted(), 2);
    server.join();
}

#[test]
fn a_server_error_is_retried() {
    let _policy = super::override_policy_for_test(fast_policy(3));
    let body = payload(64);
    let server = serve(
        "tool.tgz",
        vec![
            Reply::Status(503),
            Reply::Trickle {
                body: body.clone(),
                chunks: 1,
                gap: Duration::ZERO,
            },
        ],
    );

    assert_eq!(fetch(&server.url).expect("retry succeeds"), body);
    assert_eq!(server.accepted(), 2);
    server.join();
}

#[test]
fn a_not_found_is_not_retried() {
    let _policy = super::override_policy_for_test(fast_policy(3));
    let server = serve("tool.tgz", vec![Reply::Status(404)]);

    let message = fetch(&server.url).expect_err("404 fails").to_string();

    assert!(message.contains("404"), "{message}");
    assert!(!message.contains("gave up"), "{message}");
    assert_eq!(server.accepted(), 1);
    server.join();
}
