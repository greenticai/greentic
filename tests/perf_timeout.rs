use std::fs;
use std::time::{Duration, Instant};

use gtc::dist::stage_resolved_artifact;
use gtc::perf_targets::{collect_bundle_entries, sha256_file};

#[test]
fn tiny_bundle_walk_should_finish_quickly() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(dir.path().join("nested/deeper")).expect("mkdir");
    fs::write(dir.path().join("nested/deeper/file.txt"), b"hello").expect("write");

    let start = Instant::now();
    let mut out = Vec::new();
    collect_bundle_entries(dir.path(), dir.path(), &mut out).expect("walk");
    let elapsed = start.elapsed();

    assert!(!out.is_empty());
    assert!(
        elapsed < Duration::from_secs(1),
        "tiny bundle walk too slow: {elapsed:?}",
    );
}

#[test]
fn sha256_small_file_should_finish_quickly() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("artifact.bin");
    fs::write(&path, vec![1u8; 256 * 1024]).expect("write");

    let start = Instant::now();
    let digest = sha256_file(&path).expect("digest");
    let elapsed = start.elapsed();

    assert!(digest.starts_with("sha256:"));
    assert!(
        elapsed < Duration::from_secs(1),
        "sha256 workload too slow: {elapsed:?}",
    );
}

#[test]
fn stage_resolved_artifact_should_finish_quickly() {
    let src_dir = tempfile::tempdir().expect("tempdir");
    let stage_dir = tempfile::tempdir().expect("tempdir");
    let source = src_dir.path().join("artifact.bin");
    fs::write(&source, vec![3u8; 128 * 1024]).expect("write");

    let start = Instant::now();
    let staged = stage_resolved_artifact(&source, stage_dir.path()).expect("stage");
    let elapsed = start.elapsed();

    assert!(staged.exists());
    assert!(
        elapsed < Duration::from_secs(1),
        "staging workload too slow: {elapsed:?}",
    );
}
