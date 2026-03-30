PR-GTC-05 — Lightweight performance and concurrency test harness

Title

perf: add lightweight performance and concurrency test harness

PR Description

This PR introduces a lightweight performance and concurrency testing harness for `gtc` to detect:

- performance regressions in critical code paths
- slowdowns when increasing thread counts
- potential hangs or deadlocks under concurrency
- early warning signals before issues surface in heavier end-to-end perf environments

Scope

This is not a full benchmark suite. It is intentionally:

- fast enough for PR CI
- minimal, with tiny in-repo fixtures only
- focused on the highest-risk hot paths in this CLI
- complementary to future cross-repo or end-to-end perf testing

For `gtc`, this harness is aimed at the code paths the audit already highlighted:

- CLI parsing and argument rewriting in [`gtc.rs`](/projects/ai/greentic-ng/greentic/src/bin/gtc.rs)
- digest computation for artifact verification in [`gtc.rs`](/projects/ai/greentic-ng/greentic/src/bin/gtc.rs#L1029)
- bundle walking and archive extraction helpers in [`gtc.rs`](/projects/ai/greentic-ng/greentic/src/bin/gtc.rs#L2437) and [`gtc.rs`](/projects/ai/greentic-ng/greentic/src/bin/gtc.rs#L4481)
- OCI resolution/staging glue in [`dist.rs`](/projects/ai/greentic-ng/greentic/src/dist.rs)

Why this PR fits this repo

`gtc` is a large single-binary CLI with a few hot helpers that are easy to regress accidentally:

- `parse_raw_passthrough`
- `rewrite_legacy_op_args`
- `detect_locale`
- `build_cli`
- `sha256_file`
- `collect_bundle_entries`
- `recurse_files`
- OCI staging logic in `dist.rs`

Those paths are small enough to exercise in CI without external OCI registries, network calls, or full CLI end-to-end flows.

What’s included

1. Criterion benchmarks for real `gtc` hot paths

- Detects regressions in parsing, hashing, and bundle helper code.
- Uses representative in-memory or tempfile-backed workloads.
- Avoids network, subprocess, and remote bundle resolution.

2. Concurrency scaling tests

- Runs selected pure or tempdir-backed operations at multiple thread counts.
- Checks that throughput does not collapse as concurrency rises.
- Focuses on helpers that should scale or at least remain stable without hanging.

3. Timeout guards

- Ensures selected operations complete within bounded time on tiny fixtures.
- Prevents accidental infinite recursion, deadlocks, or pathological slowdowns from hanging CI.

Design principles

- keep the suite under roughly 5 to 10 seconds in normal PR CI
- test representative helper workloads, not full `gtc start` or `gtc install` flows
- isolate parsing, hashing, bundle walking, and staging logic from external systems
- avoid OCI registries, network sockets, GitHub APIs, and shelling out
- fail clearly with simple diagnostics

Repo-specific implementation note

Because this crate is still a pure binary crate with no [`lib.rs`](/projects/ai/greentic-ng/greentic/src), the harness should not benchmark placeholder functions.

This PR should therefore add a very small testable surface, not a large refactor:

- add [`src/lib.rs`](/projects/ai/greentic-ng/greentic/src/lib.rs) as a thin library entry point
- move or wrap only the benchmarked helper functions into a small module such as [`src/perf_targets.rs`](/projects/ai/greentic-ng/greentic/src/perf_targets.rs)
- keep `src/bin/gtc.rs` as the CLI entrypoint calling the same underlying helpers

This keeps the perf harness realistic while avoiding a broad architecture rewrite.

Files added

- [`benches/perf.rs`](/projects/ai/greentic-ng/greentic/benches/perf.rs)
- [`tests/perf_scaling.rs`](/projects/ai/greentic-ng/greentic/tests/perf_scaling.rs)
- [`tests/perf_timeout.rs`](/projects/ai/greentic-ng/greentic/tests/perf_timeout.rs)
- [`.github/workflows/perf.yml`](/projects/ai/greentic-ng/greentic/.github/workflows/perf.yml)

Files changed

- [`Cargo.toml`](/projects/ai/greentic-ng/greentic/Cargo.toml)
- [`src/lib.rs`](/projects/ai/greentic-ng/greentic/src/lib.rs)
- [`src/perf_targets.rs`](/projects/ai/greentic-ng/greentic/src/perf_targets.rs)
- [`src/bin/gtc.rs`](/projects/ai/greentic-ng/greentic/src/bin/gtc.rs)
- [`src/dist.rs`](/projects/ai/greentic-ng/greentic/src/dist.rs)

Concrete benchmark targets for this repo

`benches/perf.rs` should benchmark these real helpers:

1. `parse_raw_passthrough`
   Workload:
   parse realistic argv arrays for `gtc wizard`, `gtc op --help`, and locale-bearing invocations.

2. `rewrite_legacy_op_args`
   Workload:
   rewrite small and medium argument lists with and without explicit `--locale`.

3. `detect_locale`
   Workload:
   resolve locale from representative raw args without mutating env during the benchmark.

4. `sha256_file`
   Workload:
   hash a tempfile of moderate size such as 256 KiB to 1 MiB.
   This is especially useful after the planned streaming fix.

5. `collect_bundle_entries`
   Workload:
   walk a tiny synthetic bundle tree with nested directories and a few files.

6. `dist` staging helper
   Workload:
   stage a resolved local artifact path through the non-network staging logic only.
   Do not benchmark actual OCI pulls in PR CI.

Suggested benchmark shape

The benchmark file should look like this conceptually, but with real `gtc` functions:

```rust
use criterion::{criterion_group, criterion_main, Criterion};
use gtc::perf_targets::{
    bench_collect_bundle_entries,
    bench_detect_locale,
    bench_parse_raw_passthrough,
    bench_rewrite_legacy_op_args,
    bench_sha256_file,
    bench_stage_resolved_artifact,
};

fn bench_cli_parsing(c: &mut Criterion) {
    c.bench_function("parse_raw_passthrough/wizard", |b| {
        b.iter(bench_parse_raw_passthrough)
    });
    c.bench_function("rewrite_legacy_op_args/defaults", |b| {
        b.iter(bench_rewrite_legacy_op_args)
    });
    c.bench_function("detect_locale/cli_args", |b| {
        b.iter(bench_detect_locale)
    });
}

fn bench_io_helpers(c: &mut Criterion) {
    c.bench_function("sha256_file/1mb", |b| b.iter(bench_sha256_file));
    c.bench_function("collect_bundle_entries/tiny_bundle", |b| {
        b.iter(bench_collect_bundle_entries)
    });
    c.bench_function("dist/stage_resolved_artifact", |b| {
        b.iter(bench_stage_resolved_artifact)
    });
}

criterion_group!(benches, bench_cli_parsing, bench_io_helpers);
criterion_main!(benches);
```

Concurrency scaling tests for this repo

`tests/perf_scaling.rs` should use thread-safe, dependency-free workloads only.

Recommended workloads:

1. parallel parsing workload
   Each thread repeatedly runs `parse_raw_passthrough` and `rewrite_legacy_op_args` against fixed argv inputs.

2. parallel bundle walk workload
   Each thread walks its own tempdir-backed tiny bundle tree using `collect_bundle_entries` or `recurse_files`.

3. parallel hashing workload
   Each thread hashes its own tempfile using `sha256_file`.

Avoid in the scaling tests:

- global env mutation
- `build_cli` if it materially leaks strings on each iteration
- OCI registry/network access
- subprocess-based CLI calls

Suggested scaling assertions

Use broad, low-flake limits rather than aggressive speedup expectations:

- compare `1`, `2`, `4`, and maybe `8` threads
- fail only on obvious collapse, not on small scheduler noise
- prefer ratios like `t4 <= t1 * 2.0` for total wall time on independent workloads

Example structure:

```rust
#[test]
fn parsing_scaling_should_not_collapse() {
    let t1 = run_parallel_workload(1, parse_workload);
    let t4 = run_parallel_workload(4, parse_workload);

    assert!(
        t4 <= t1.mul_f64(2.0),
        "4-thread parse workload regressed badly: t1={:?}, t4={:?}",
        t1,
        t4
    );
}
```

Timeout protection for this repo

`tests/perf_timeout.rs` should guard the operations most likely to regress into hangs or pathological slowdowns:

1. tiny bundle walk finishes quickly
   Protects against recursion or traversal regressions.

2. archive extraction on tiny fixture finishes quickly
   Protects against accidental algorithmic blowups in zip/tar handling.

3. local artifact staging finishes quickly
   Protects against unexpected blocking in `dist` staging helpers.

Use tiny fixtures and conservative thresholds such as:

- 250 ms to 2 s depending on the workload
- no network
- no random sleeps

Example targets:

- `tiny_bundle_walk_should_finish_quickly`
- `tiny_tar_extract_should_finish_quickly`
- `stage_resolved_artifact_should_finish_quickly`

CI workflow

Add [`.github/workflows/perf.yml`](/projects/ai/greentic-ng/greentic/.github/workflows/perf.yml) with a dedicated lightweight job instead of folding this into the shared reusable `ci.yml`.

That fits this repo because current CI delegates to `greenticai/.github/.github/workflows/host-crate-ci.yml@main`, and we want the perf checks to stay visible and controlled here.

Suggested workflow:

```yaml
name: Perf (lightweight)

on:
  pull_request:
  push:
    branches: [main]

permissions:
  contents: read

jobs:
  perf:
    runs-on: ubuntu-latest

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: 1.91.0

      - name: Run perf guard tests
        run: cargo test --test perf_scaling --test perf_timeout

      - name: Run benchmark smoke
        run: cargo bench --bench perf -- --sample-size 10
```

Cargo changes

Add dev dependency:

```toml
[dev-dependencies]
criterion = "0.5"
```

Also add a bench target because this crate currently has only `[[bin]]`:

```toml
[[bench]]
name = "perf"
harness = false
```

Suggested minimal helper surface

To keep the PR small, expose only narrowly-scoped functions from `src/lib.rs`:

- `perf_targets::parse_raw_passthrough_case()`
- `perf_targets::rewrite_legacy_op_args_case()`
- `perf_targets::detect_locale_case()`
- `perf_targets::sha256_tempfile_case()`
- `perf_targets::collect_bundle_entries_case()`
- `perf_targets::stage_resolved_artifact_case()`

These wrappers can build the tiny temp fixtures internally so the tests stay readable and deterministic.

What this PR intentionally does not do

- no full CLI end-to-end benchmarking
- no remote bundle fetch benchmarking
- no OCI registry benchmarking
- no network sockets in perf tests
- no loom or model-checking in this first pass
- no broad modularization beyond the tiny helper extraction needed to benchmark real code

Expected impact

- catches hot-path regressions early in PRs
- surfaces obvious concurrency collapse before release
- guards against accidental hangs in bundle and staging helpers
- gives this repo a local perf safety net without waiting for larger shared perf infrastructure

Future improvements

- add `loom` for truly shared mutable concurrency if future refactors add lock-heavy sections
- share a small perf-helper pattern across Greentic repos
- persist benchmark baselines for regression comparison
- integrate benchmark summaries into a broader perf reporting pipeline

Key takeaway

This PR gives `gtc`:

- fast feedback
- early regression detection
- low CI overhead

while leaving heavier end-to-end performance coverage for future dedicated perf environments.
