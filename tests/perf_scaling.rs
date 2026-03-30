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

    assert!(
        t4 <= t1.mul_f64(3.0),
        "4-thread parsing workload regressed badly: t1={t1:?}, t4={t4:?}",
    );
}

#[test]
fn hashing_scaling_should_not_collapse() {
    let files_dir = tempfile::tempdir().expect("tempdir");
    let paths: Vec<_> = (0..4)
        .map(|idx| {
            let path = files_dir.path().join(format!("artifact-{idx}.bin"));
            fs::write(&path, vec![idx as u8; 64 * 1024]).expect("write");
            path
        })
        .collect();

    let t1 = run_parallel_workload(1, {
        let paths = paths.clone();
        move || {
            for path in &paths {
                let digest = sha256_file(path).expect("digest");
                assert!(digest.starts_with("sha256:"));
            }
        }
    });

    let t4 = run_parallel_workload(4, {
        let paths = paths.clone();
        move || {
            for path in &paths {
                let digest = sha256_file(path).expect("digest");
                assert!(digest.starts_with("sha256:"));
            }
        }
    });

    assert!(
        t4 <= t1.mul_f64(3.0),
        "4-thread hashing workload regressed badly: t1={t1:?}, t4={t4:?}",
    );
}
