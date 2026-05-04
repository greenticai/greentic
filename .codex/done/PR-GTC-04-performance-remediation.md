PR-GTC-04 — Performance remediation plan for audit findings

Summary

This PR plan covers the remaining performance findings from the March 28, 2026 static analysis report, excluding `PERF-004`.

`PERF-004` is intentionally out of scope because this project wants multi-language assets embedded in the binary.

The intent here is the same as the bug remediation pass:

1. make each performance issue measurable or at least structurally testable,
2. fix it with the smallest reviewable change, and
3. leave behind a regression test or characterization test where it adds real value.

Scope

In scope:

- `PERF-001` streaming `sha256_file`
- `PERF-002` reuse the OCI tokio runtime
- `PERF-003` remove redundant OCI read-then-write staging
- `PERF-005` re-evaluate `reqwest` blocking feature usage
- `PERF-006` reduce avoidable CLI arg processing allocations
- `PERF-007` simplify duplicate OCI tempdir usage
- `PERF-008` document and characterize the intentional `leak_str` trade-off

Out of scope:

- `PERF-004` embedded locale size
- architecture refactors unless a tiny extraction is needed to test a performance fix cleanly

Current code status

- `PERF-001` is still present. [`sha256_file`](/projects/ai/greentic-ng/greentic/src/bin/gtc.rs#L1029) uses `fs::read(path)` and hashes the full file from memory.
- `PERF-002`, `PERF-003`, and `PERF-007` are still present in [`dist.rs`](/projects/ai/greentic-ng/greentic/src/dist.rs). The OCI adapter creates a runtime per call, reads the full artifact into memory, writes it back out, and uses separate tempdirs for cache and staging.
- `PERF-005` is still potentially valid. [`Cargo.toml`](/projects/ai/greentic-ng/greentic/Cargo.toml#L23) still enables `reqwest` with `features = ["blocking", "rustls"]`.
- `PERF-006` is still present. [`collect_tail`](/projects/ai/greentic-ng/greentic/src/bin/gtc.rs#L305), [`rewrite_legacy_op_args`](/projects/ai/greentic-ng/greentic/src/bin/gtc.rs#L371), [`ensure_flag_value`](/projects/ai/greentic-ng/greentic/src/bin/gtc.rs#L405), and [`has_flag`](/projects/ai/greentic-ng/greentic/src/bin/gtc.rs#L415) still do avoidable allocation work.
- `PERF-008` is still intentionally present. [`leak_str`](/projects/ai/greentic-ng/greentic/src/bin/gtc.rs#L4900) remains the clap localization bridge.

General test policy

- Prefer tests that protect invariants over noisy wall-clock benchmarks.
- If a performance change is hard to assert numerically in CI, use a structural test that proves the expensive pattern is gone.
- For I/O paths, favor tests that validate behavior and shape of the implementation rather than microbenchmarks.
- Keep the fixes split enough that each performance change can be reviewed independently.

Implementation order

1. `PERF-002` reuse OCI runtime
2. `PERF-003` remove redundant OCI read+write staging
3. `PERF-007` collapse duplicate OCI tempdir flow
4. `PERF-001` stream `sha256_file`
5. `PERF-006` reduce CLI arg allocation churn
6. `PERF-005` decide whether blocking `reqwest` stays or goes
7. `PERF-008` keep as documented characterization unless clap ownership changes

Tracking table

| ID | Title | Current status | Test/guard | Fix | Green condition |
| --- | --- | --- | --- | --- | --- |
| PERF-001 | `sha256_file` reads whole file | still live | behavioral hash test plus source-shape guard | switch to `BufReader` + chunked reads | hashing no longer requires `fs::read` |
| PERF-002 | OCI runtime created per call | still live | unit test proves shared runtime accessor returns same instance | introduce module-level `OnceLock<Runtime>` | OCI pulls reuse one runtime |
| PERF-003 | OCI adapter does double I/O | still live | focused test around staging helper | replace read+write with `fs::copy` or rename/link | no full artifact load into memory |
| PERF-005 | `reqwest` blocking feature may be unused | still open question | code search plus build check | remove feature if unused, otherwise document why it stays | dependency features match actual usage |
| PERF-006 | avoidable arg allocations | still live | unit tests on flag helpers and rewriting behavior | use borrowed matching where practical | helper behavior unchanged with fewer temp strings |
| PERF-007 | duplicate tempdir creation | still live | helper-level test around OCI staging path | stage into one owned tempdir | OCI path uses one temp root |
| PERF-008 | intentional `leak_str` allocation | informational | characterization test and doc comment | likely keep as-is | behavior is documented and stable |

PERF-001 — `sha256_file` reads entire file into memory

Current code

- [`sha256_file`](/projects/ai/greentic-ng/greentic/src/bin/gtc.rs#L1029) calls `fs::read(path)` and feeds the resulting `Vec<u8>` into `Sha256`.
- This is now more important than before because digest verification is part of tool installation and can run on downloaded artifacts, not just tiny local fixtures.

Test plan

- Keep a normal behavioral test that hashes a known file and matches the expected digest.
- Add a lightweight source-shape guard that asserts the production implementation no longer contains `fs::read(path)` inside `sha256_file`.
- Avoid a wall-clock or memory-usage benchmark in CI.

Fix

- Open the file directly.
- Wrap it in `BufReader`.
- Read in fixed-size chunks such as 8 KiB.
- Feed each chunk into the hasher and return the same `sha256:<hex>` output format.

Green condition

- Existing digest verification behavior remains unchanged.
- `sha256_file` no longer slurps the whole file into memory.

Suggested tests

- `sha256_file_matches_known_digest`
- `sha256_file_is_streaming_not_fs_read_based`

PERF-002 — OCI adapter creates a tokio runtime per call

Current code

- [`OciDistAdapter::resolve_ref_to_cached_file`](/projects/ai/greentic-ng/greentic/src/dist.rs#L18) builds a fresh `tokio::runtime::Builder::new_current_thread().enable_all().build()` on every invocation.
- That adds overhead and leaves the code vulnerable to a future nested-runtime panic if this path is ever reached from async-aware callers.

Test plan

- Extract a tiny runtime accessor such as `oci_runtime() -> &'static tokio::runtime::Runtime`.
- Add a unit test that calls it twice and proves the same runtime instance is returned.
- If desired, also add a structural test that the resolve path goes through the shared accessor instead of constructing a runtime inline.

Fix

- Add a module-level `OnceLock<tokio::runtime::Runtime>`.
- Initialize it with the current-thread runtime once.
- Reuse it for every OCI resolve call.

Green condition

- Repeated OCI pulls share one runtime instance.
- No per-call runtime build remains in the hot path.

Suggested tests

- `oci_runtime_is_reused_across_calls`

PERF-003 — OCI adapter performs redundant double I/O

Current code

- After OCI resolution, [`dist.rs`](/projects/ai/greentic-ng/greentic/src/dist.rs#L46) reads the resolved file into memory with `fs::read(&path)` and then writes it back out with `fs::write(&out_path, bytes)`.
- That duplicates both I/O and peak memory.

Test plan

- Extract a small staging helper that takes an on-disk artifact path and returns the kept output path.
- Test that the helper preserves file contents and filename.
- Use a source-shape guard or direct inspection in the unit under test to ensure it uses filesystem copy/move semantics rather than `fs::read` plus `fs::write`.

Fix

- Prefer `fs::copy(&path, &out_path)`.
- Optionally consider `hard_link` first with `copy` fallback if preserving performance across same-filesystem cases is worthwhile.

Green condition

- OCI staging no longer loads the whole artifact into a `Vec<u8>`.
- Output file contents are unchanged.

Suggested tests

- `stage_resolved_artifact_copies_contents_and_name`

PERF-005 — `reqwest` blocking feature may be unused

Current code

- [`Cargo.toml`](/projects/ai/greentic-ng/greentic/Cargo.toml#L23) still declares `reqwest = { ..., features = ["blocking", "rustls"] }`.
- The current CLI code does use `reqwest::blocking::Client`, so this item is not an automatic removal.

Assessment

- As the code stands today, this finding should be treated as a validation task, not a guaranteed code change.
- If the blocking client remains the chosen transport, the best outcome may be to close this item as "accepted and documented" rather than forcing a swap to `ureq`.

Test plan

- No runtime test is needed first.
- Confirm actual usage stays on the blocking client path.
- If a later refactor removes blocking calls, then remove the feature and let `cargo check` be the guard.

Fix options

- Option A: keep `reqwest::blocking::Client`, keep the feature, and explicitly document why.
- Option B: if the HTTP path is refactored later, drop the `blocking` feature and rely on `cargo check` plus existing HTTP tests.

Green condition

- Dependency features match real code usage.
- We do not carry speculative dependency cleanup that breaks the current implementation.

Suggested output

- A short note in the PR body or code comments if we intentionally keep the feature.

PERF-006 — Redundant allocations in argument rewriting

Current code

- [`collect_tail`](/projects/ai/greentic-ng/greentic/src/bin/gtc.rs#L305) clones all passthrough args into a new `Vec<String>`.
- [`has_flag`](/projects/ai/greentic-ng/greentic/src/bin/gtc.rs#L415) allocates `format!("--{flag}")` and `format!("{long}=")` on every call.
- [`ensure_flag_value`](/projects/ai/greentic-ng/greentic/src/bin/gtc.rs#L405) and [`rewrite_legacy_op_args`](/projects/ai/greentic-ng/greentic/src/bin/gtc.rs#L371) also copy more than they strictly need to.

Test plan

- Preserve the current unit coverage around passthrough parsing and arg rewriting.
- Add focused tests for `has_flag` behavior with:
  - `--flag`
  - `--flag=value`
  - near-miss values like `--flag-extra`
- Since this is a micro-optimization, behavioral equivalence matters more than measurable timing in CI.

Fix

- Rework `has_flag` to use `strip_prefix("--")` and borrowed comparisons.
- Keep `collect_tail` as-is unless a larger clap-facing refactor makes borrowed storage practical.
- Only optimize the helpers where it is clearly low-risk and locally understandable.

Green condition

- Existing passthrough behavior is unchanged.
- Helper functions stop allocating temporary formatted strings on the hottest checks.

Suggested tests

- `has_flag_matches_plain_and_equals_forms`
- `has_flag_rejects_prefix_collisions`

PERF-007 — Duplicate tempdir creation in OCI adapter

Current code

- [`dist.rs`](/projects/ai/greentic-ng/greentic/src/dist.rs#L21) creates a tempdir for OCI cache and a second tempdir for staging the kept artifact.
- The second tempdir only exists because the first tempdir is dropped after resolution.

Test plan

- Fold this into the same small staging extraction used for `PERF-003`.
- Add a unit test proving the helper returns a stable kept path whose parent exists after the tempdir is converted to owned storage.

Fix

- Use one tempdir root for OCI resolve plus final kept artifact.
- Store cache inside a subdirectory like `<temp>/cache`.
- Copy or move the resolved artifact into another subpath such as `<temp>/artifact/<filename>`.
- Call `keep()` once on the single tempdir and return the final artifact path inside it.

Green condition

- The OCI adapter owns one temp root per resolve call instead of two.
- The returned artifact path remains valid after the tempdir is kept.

Suggested tests

- `oci_resolve_kept_artifact_survives_single_tempdir_keep`

PERF-008 — `build_cli` leaks strings for clap i18n

Current code

- [`build_cli`](/projects/ai/greentic-ng/greentic/src/bin/gtc.rs#L131) still relies on [`leak_str`](/projects/ai/greentic-ng/greentic/src/bin/gtc.rs#L4900).
- This is an intentional trade-off to satisfy clap's effectively static builder data model.

Assessment

- This should stay informational unless a clap API change or a local ownership wrapper makes it easy to remove.
- The current behavior is already documented in code and covered by a characterization-style test from the bug pass.

Plan

- Keep the implementation.
- Keep or lightly refine the doc comment.
- Do not churn this area just to eliminate a tiny one-shot CLI allocation leak.

Green condition

- Behavior stays documented and unsurprising.

Suggested tests

- Reuse the existing characterization test for repeated `build_cli()` construction.

Suggested PR slicing

1. `dist.rs` runtime reuse plus OCI staging cleanup
2. streaming `sha256_file`
3. low-risk CLI arg helper allocation cleanup
4. dependency-feature decision for `reqwest`
5. no-op documentation-only closeout for `PERF-008`

Open questions

- Whether `PERF-005` should resolve as a code diff or as a documented "kept intentionally" finding depends on whether the team wants to stay on blocking `reqwest` for now.
- If we later extract a `lib.rs`, `PERF-006` helpers become easier to unit test without adding more test weight to the binary file.
