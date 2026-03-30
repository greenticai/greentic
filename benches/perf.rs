use std::fs;

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use gtc::dist::stage_resolved_artifact;
use gtc::perf_targets::{
    collect_bundle_entries, detect_locale, parse_raw_passthrough, rewrite_legacy_op_args,
    sha256_file,
};

fn bench_cli_parsing(c: &mut Criterion) {
    let wizard_args = vec![
        "gtc".to_string(),
        "--locale".to_string(),
        "en".to_string(),
        "wizard".to_string(),
        "--help".to_string(),
    ];
    let op_args = vec![
        "start".to_string(),
        "--provider".to_string(),
        "aws".to_string(),
    ];

    c.bench_function("parse_raw_passthrough/wizard", |b| {
        b.iter(|| parse_raw_passthrough(black_box(&wizard_args)))
    });

    c.bench_function("rewrite_legacy_op_args/start", |b| {
        b.iter(|| rewrite_legacy_op_args(black_box(&op_args)))
    });

    c.bench_function("detect_locale/cli_args", |b| {
        b.iter(|| detect_locale(black_box(&wizard_args), "en", Some("nl")))
    });
}

fn bench_io_helpers(c: &mut Criterion) {
    let hash_dir = tempfile::tempdir().expect("tempdir");
    let hash_path = hash_dir.path().join("artifact.bin");
    fs::write(&hash_path, vec![7u8; 256 * 1024]).expect("write");

    let bundle_dir = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(bundle_dir.path().join("nested")).expect("mkdir");
    fs::write(bundle_dir.path().join("nested/file.txt"), b"hello").expect("write");

    let stage_src_dir = tempfile::tempdir().expect("tempdir");
    let stage_root = tempfile::tempdir().expect("tempdir");
    let stage_src = stage_src_dir.path().join("artifact.bin");
    fs::write(&stage_src, vec![9u8; 64 * 1024]).expect("write");

    c.bench_function("sha256_file/256kb", |b| {
        b.iter(|| sha256_file(black_box(&hash_path)).expect("digest"))
    });

    c.bench_function("collect_bundle_entries/tiny_bundle", |b| {
        b.iter(|| {
            let mut out = Vec::new();
            collect_bundle_entries(
                black_box(bundle_dir.path()),
                black_box(bundle_dir.path()),
                &mut out,
            )
            .expect("walk");
            out
        })
    });

    c.bench_with_input(
        BenchmarkId::new("stage_resolved_artifact", "local_file"),
        &stage_src,
        |b, path| {
            b.iter(|| stage_resolved_artifact(black_box(path), black_box(stage_root.path())));
        },
    );
}

criterion_group!(benches, bench_cli_parsing, bench_io_helpers);
criterion_main!(benches);
