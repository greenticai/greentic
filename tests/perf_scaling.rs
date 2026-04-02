use std::fs;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gtc::perf_targets::{parse_raw_passthrough, rewrite_legacy_op_args, sha256_file};

fn run_parallel_workload<F>(threads: usize, workload: F) -> Duration
where
    F: Fn() + Send + Sync + 'static,
{
    let start = Instant::now();
    let workload = Arc::new(workload);
    let handles: Vec<_> = (0..threads)
        .map(|_| {
            let workload = Arc::clone(&workload);
            std::thread::spawn(move || workload())
        })
        .collect();

    for handle in handles {
        handle.join().expect("thread");
    }

    start.elapsed()
}

fn nanos_per_thread(duration: Duration, threads: usize) -> f64 {
    duration.as_nanos() as f64 / threads as f64
}

#[test]
fn parsing_scaling_should_not_collapse() {
    let raw = vec![
        "gtc".to_string(),
        "--locale".to_string(),
        "en".to_string(),
        "wizard".to_string(),
        "--help".to_string(),
    ];
    let op = vec![
        "start".to_string(),
        "--provider".to_string(),
        "aws".to_string(),
    ];

    let t1 = run_parallel_workload(1, {
        let raw = raw.clone();
        let op = op.clone();
        move || {
            for _ in 0..5_000 {
                let parsed = parse_raw_passthrough(&raw).expect("passthrough");
                let _ = rewrite_legacy_op_args(&op);
                assert_eq!(parsed.subcommand, "wizard");
            }
        }
    });

    let t4 = run_parallel_workload(4, {
        let raw = raw.clone();
        let op = op.clone();
        move || {
            for _ in 0..5_000 {
                let parsed = parse_raw_passthrough(&raw).expect("passthrough");
                let _ = rewrite_legacy_op_args(&op);
                assert_eq!(parsed.subcommand, "wizard");
            }
        }
    });

    let t1_per_thread = nanos_per_thread(t1, 1);
    let t4_per_thread = nanos_per_thread(t4, 4);

    assert!(
        t4_per_thread <= t1_per_thread * 1.5,
        "4-thread parsing workload regressed badly: t1={t1:?}, t4={t4:?}, per_thread_t1={t1_per_thread:.0}ns, per_thread_t4={t4_per_thread:.0}ns",
    );
}

#[test]
#[ignore = "local perf/concurrency guard; too noisy for shared CI runners"]
fn hashing_scaling_should_not_collapse() {
    let files_dir = tempfile::tempdir().expect("tempdir");
    let paths: Vec<_> = (0..4)
        .map(|idx| {
            let path = files_dir.path().join(format!("artifact-{idx}.bin"));
            fs::write(&path, vec![idx as u8; 256 * 1024]).expect("write");
            path
        })
        .collect();
    let rounds = 8;

    // Warm the page cache so the scaling check measures the hashing workload
    // rather than first-read filesystem variance on tiny inputs.
    for path in &paths {
        let digest = sha256_file(path).expect("digest");
        assert!(digest.starts_with("sha256:"));
    }

    let t1 = run_parallel_workload(1, {
        let paths = paths.clone();
        move || {
            for _ in 0..rounds {
                for path in &paths {
                    let digest = sha256_file(path).expect("digest");
                    assert!(digest.starts_with("sha256:"));
                }
            }
        }
    });

    let t4 = run_parallel_workload(4, {
        let paths = paths.clone();
        move || {
            for _ in 0..rounds {
                for path in &paths {
                    let digest = sha256_file(path).expect("digest");
                    assert!(digest.starts_with("sha256:"));
                }
            }
        }
    });

    let t1_per_thread = nanos_per_thread(t1, 1);
    let t4_per_thread = nanos_per_thread(t4, 4);

    assert!(
        t4_per_thread <= t1_per_thread * 1.5,
        "4-thread hashing workload regressed badly: t1={t1:?}, t4={t4:?}, per_thread_t1={t1_per_thread:.0}ns, per_thread_t4={t4_per_thread:.0}ns",
    );
}
