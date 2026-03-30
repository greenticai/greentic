# PR-GTC-06

## Title

`arch: close remaining architecture gaps after gtc modularization`

## Summary

This PR closes the remaining architecture findings from the March 28, 2026 audit, updated for the current repo state.

Important current-state notes:

- `ARCH-001` is already materially improved and is not part of this PR. `src/bin/gtc.rs` is now a thin entrypoint and command wiring has been split into focused modules under `src/bin/gtc/`.
- `ARCH-003` and `ARCH-012` are no longer accurate as originally written:
  - the repo now has [`src/lib.rs`](../../src/lib.rs)
  - the repo now has unit tests in the library and binary
  - the remaining issue is breadth and organization of tests, not total absence
- `ARCH-015` is still open, but the repo now has lightweight perf guards and some unit coverage, so the follow-up should build on that rather than start from zero.

This PR therefore focuses on the still-open architecture work:

- structured errors
- cross-platform and broader integration testing
- release/CI hardening
- configuration centralization
- credential prompting deduplication
- passthrough parser robustness
- dependency and release-process cleanup

## Findings Review

### ARCH-002: No structured error hierarchy

Status: `Open`

Current state:

- Most fallible paths still return `Result<T, String>`
- Errors are still flattened at the call site
- Callers still cannot distinguish validation, IO, network, process, and user-abort failures

PR changes:

1. Add [`src/error.rs`](../../src/error.rs) with a `GtcError` enum using `thiserror`
2. Introduce error variants for:
   - validation
   - io
   - process execution
   - network/download
   - archive/extraction
   - config/state
   - user abort
3. Convert the lowest-risk leaf modules first:
   - `src/dist.rs`
   - `src/perf_targets.rs`
   - `src/bin/gtc/archive.rs`
   - `src/bin/gtc/install.rs` helpers
4. Add `type GtcResult<T> = Result<T, GtcError>`
5. Keep CLI-facing messaging stable by rendering top-level errors in `main()`

Tests:

- add unit tests for variant construction and display text
- add command-level tests that assert exit code and top-level user-facing message on representative failures

### ARCH-003: Zero unit tests in main binary

Status: `Partially addressed`

Current state:

- This is no longer true literally
- The repo now has:
  - library unit tests
  - binary unit tests extracted into [`src/bin/gtc/tests.rs`](../../src/bin/gtc/tests.rs)
- The real remaining problem is coverage shape, especially for newly extracted modules and pure logic that should live in `lib.rs`

PR changes:

1. Move pure logic out of the binary crate and into library modules under `src/`
2. Add focused unit tests for:
   - passthrough parsing
   - bundle fingerprint normalization
   - provider/app-pack resolution
   - cloud config validation
   - YAML quoting/spec rendering helpers
3. Keep only command wiring / binary-only launch tests in the binary crate

Done condition:

- the majority of pure logic tests run from `src/lib.rs` modules rather than only through the binary target

### ARCH-004: Integration tests are Unix-only

Status: `Partially addressed`

Current state:

- The integration suite is no longer blanket-gated behind `#![cfg(unix)]`
- A meaningful subset can now run cross-platform:
  - version smoke test
  - admin registry CRUD flow
- The richer fake-binary orchestration tests still rely on Unix script helpers

PR changes:

1. Replace the Unix-only test sandbox with a cross-platform helper:
   - `tests/support/mod.rs`
   - `tests/support/sandbox.rs`
2. Add platform-specific script writers:
   - `.sh` on Unix
   - `.cmd` or `.ps1` on Windows
3. Replace direct Unix permission handling with helper methods guarded by `cfg`
4. Remove `#![cfg(unix)]`
5. Keep tests behaviorally identical where possible

Tests:

- `cargo test` on Linux
- `cargo test --test gtc_router_integration` on Windows in CI

### ARCH-005: Integration tests only cover routing

Status: `Open`

Current state:

- Routing and install/update paths are now covered reasonably well
- Critical end-to-end paths are still thin or absent:
  - `start`
  - `stop`
  - admin CRUD flow
  - deployment state round-trip
  - provider/app-pack selection behavior

PR changes:

Add new integration tests for:

1. `start` with a local mock bundle and mocked companion binaries
2. `stop --destroy` with mocked deployer behavior
3. admin add/remove against a real temp bundle
4. provider/app-pack selection from `.greentic/deployment-targets.json` and `bundle.yaml`
5. deployment state creation/removal for single-vm and cloud destroy flows

Done condition:

- integration coverage includes at least one happy path and one failure path for `start`, `stop`, and admin mutation commands

### ARCH-006: Release pipeline builds but never tests targets

Status: `Open`

Current state:

- [`release.yml`](../../.github/workflows/release.yml) still builds all targets but does not run tests in `build_binaries`
- reusable CI is still the only general test gate before release

PR changes:

1. Add native-platform test steps in `build_binaries`
2. At minimum:
   - `ubuntu-latest`: `cargo test`
   - `macos-15`: `cargo test`
   - `windows-latest`: `cargo test`
3. For non-native cross targets, keep build-only if runner support is unavailable
4. Fail release before packaging if native tests fail

Suggested workflow change:

```yaml
- name: Test
  if: matrix.target == 'x86_64-unknown-linux-gnu' || matrix.target == 'x86_64-apple-darwin' || matrix.target == 'x86_64-pc-windows-msvc'
  run: cargo test
```

### ARCH-007: No code signing, SBOM, or SLSA provenance

Status: `In progress`

Current state:

- release artifacts now ship with a checksum manifest
- GitHub build provenance attestations are now generated for release artifacts
- an SPDX JSON SBOM is now generated for the crate release
- no signing/notarization steps exist

PR changes

Phase 1 in this PR:

1. Generate SHA-256 checksums for every packaged artifact
2. Upload `checksums.txt` alongside release artifacts
3. Document verification for downloaded release archives
4. Publish build provenance attestations for release artifacts
5. Publish an SPDX JSON SBOM for the release

Follow-up phases:

1. Add cross-hosted provenance if GitHub artifact attestations prove insufficient

Phase 2, tracked separately if secrets/accounts are needed:

1. macOS signing/notarization
2. Windows signing

Done condition:

- every release includes checksums
- verification instructions live in-repo
- provenance attestations are published for release artifacts
- an SPDX JSON SBOM ships with the release
- signing remains explicitly tracked follow-up work

### ARCH-008: DRY violation in credential prompting

Status: `Open`

Current state:

- the env-mutation bug is fixed, but prompting logic is still duplicated across cloud providers
- provider flows are structurally similar but implemented separately

PR changes:

1. Add a small prompt schema:
   - provider
   - mode
   - fields
   - env mapping
   - secret/non-secret handling
2. Replace:
   - `prompt_aws_credentials`
   - `prompt_azure_credentials`
   - `prompt_gcp_credentials`
   with a single generic driver plus small provider tables
3. Keep exact prompts/user behavior as stable as possible

Tests:

- unit tests for provider field mapping
- regression tests for produced `ChildProcessEnv` values

### ARCH-009: Opaque CI delegation

Status: `Open`

Current state:

- [`ci.yml`](../../.github/workflows/ci.yml) still points at `greenticai/.github/.github/workflows/host-crate-ci.yml@main`
- contributors still cannot see exact CI behavior from this repo alone

PR changes:

1. Pin the reusable workflow to a commit SHA
2. Add [`docs/ci.md`](../../docs/ci.md) describing:
   - which workflows run
   - what checks are expected
   - where local equivalents exist
3. Add short comments to `ci.yml` and `release.yml` explaining the delegation

Done condition:

- CI source is reproducible
- repo-local documentation explains the effective CI contract

### ARCH-010: Hardcoded container image digests

Status: `Open`

Current state:

- image defaults are still constants in the binary
- AWS and Azure still share the same image ref

PR changes:

1. Introduce a small deploy config layer in `src/config.rs`
2. Move operator image defaults behind configuration accessors
3. Collapse the shared GHCR image into one constant
4. Keep GCP-specific default separate
5. Allow environment override from one documented location

Done condition:

- image defaults are centralized
- duplicate hardcoded GHCR refs are removed

### ARCH-011: `serde_yaml` is deprecated

Status: `Closed`

Current state:

- [`Cargo.toml`](../../Cargo.toml) now uses `serde_yaml_bw = "2.5"`
- YAML parsing in [`provider_packs.rs`](../../src/bin/gtc/deploy/cloud_deploy/provider_packs.rs) now goes through `serde_yaml_bw`

PR changes:

1. Switch to `serde_yaml_bw 2.5`
2. Update imports and parsing calls
3. Run the full test suite

Risk:

- low, but watch for small behavioral differences in YAML edge cases

### ARCH-012: No `lib.rs`

Status: `Partially addressed`

Current state:

- [`src/lib.rs`](../../src/lib.rs) now exists
- but it still exports only:
  - `dist`
  - `perf_targets`
- most reusable logic still lives under `src/bin/gtc/`

PR changes:

1. Move reusable modules into library space:
   - errors
   - config
   - bundle/deploy helpers
   - parsing helpers
2. Keep `src/bin/gtc.rs` as a thin launcher and CLI wiring file
3. Update tests to target library modules first

Done condition:

- `lib.rs` is the default home for reusable and testable logic

### ARCH-013: Fragile dual-parsing passthrough

Status: `Open`

Current state:

- parsing is cleaner and better-tested than at audit time
- but raw passthrough parsing and clap parsing are still separate

PR changes:

1. Add a guard test that fails when a new global CLI flag is not reflected in raw passthrough parsing
2. Document supported global passthrough flags in one place
3. Evaluate moving to `allow_external_subcommands` in a follow-up if behavior stays compatible

Tests:

- snapshot or assertion-based parity checks between CLI globals and raw parser support

### ARCH-014: No centralized configuration struct

Status: `Open`

Current state:

- env/config access still happens ad hoc across modules

PR changes:

1. Add [`src/config.rs`](../../src/config.rs) with a `Config` struct
2. Centralize:
   - deploy image defaults
   - bundle registry base env vars
   - deploy bundle source override
   - cloud/deployer env reads that are config, not runtime secrets
3. Thread `&Config` through library-level logic where practical

Done condition:

- recognized environment variables are loaded and documented centrally

### ARCH-015: Missing property, fuzz, and snapshot tests

Status: `Open`

Current state:

- there is now more test coverage than the audit described
- but there are still no:
  - property tests
  - fuzz targets
  - snapshot tests

PR changes:

1. Add `insta` snapshot tests for single-vm spec rendering
2. Add `proptest` coverage for:
   - passthrough parsing
   - identifier sanitization
   - bundle fingerprint normalization invariants
3. Add `cargo-fuzz` scaffolding for start/stop request parsing in a follow-up-friendly layout

Done condition:

- at least one snapshot suite and one property suite land in this PR

### ARCH-016: Internal crate version pinning

Status: `Accepted by design / Removed from scope`

Current state:

- internal crates are intentionally pinned to `0.4`
- this is a deliberate compatibility and release-management choice for the current repo

Decision:

- no remediation work in this PR
- treat this as an explicit design decision unless the release strategy changes later

### ARCH-017: `oci-distribution` version dated

Status: `Closed after verification`

Current state:

- verified against the local crates.io index on 2026-03-30
- `0.11.0` is still the newest published `oci-distribution` release
- no newer stable crates.io version exists for this repo to adopt today

PR changes:

1. Document the verification result next to the dependency in `Cargo.toml`
2. Close the finding as stale rather than carrying an impossible upgrade task

Follow-up:

- revisit only when crates.io publishes a newer release or an upstream compatibility/security reason appears

### ARCH-018: Silent skip of already-published versions

Status: `Closed`

Current state:

- the release workflow now treats duplicate publish attempts as an explicit release failure
- the job emits a GitHub Actions error annotation and writes a step summary explaining that `Cargo.toml` must be bumped

PR changes:

1. Replace the warning-only path with a hard failure
2. Add a step-summary explanation so the reason is visible in the Actions UI

Example:

```bash
echo "::error::Version already published to crates.io — bump Cargo.toml before releasing again"
exit 1
```

## Proposed PR Slices

### Slice 1: Error + Config Foundations

- `src/error.rs`
- `src/config.rs`
- `thiserror`
- central config loading

Closes or advances:

- `ARCH-002`
- `ARCH-014`
- `ARCH-010`

### Slice 2: Library Extraction

- move reusable parsing/deploy helpers from `src/bin/gtc/` into `src/`
- expand `src/lib.rs`

Closes or advances:

- `ARCH-003`
- `ARCH-012`

### Slice 3: Test Architecture

- cross-platform sandbox
- remove Unix-only integration gate
- broaden integration coverage
- add snapshot/property tests

Closes or advances:

- `ARCH-004`
- `ARCH-005`
- `ARCH-015`
- `ARCH-013`

### Slice 4: CI / Release Hardening

- test native targets in release
- checksum asset generation
- SBOM/provenance
- pinned reusable workflow
- publish warning cleanup

Closes or advances:

- `ARCH-006`
- `ARCH-007`
- `ARCH-009`
- `ARCH-018`

### Slice 5: Dependency / Duplication Cleanup

- `serde_yml`
- credential prompting dedup
- docs for dependency review decisions

Closes or advances:

- `ARCH-008`
- `ARCH-011`
- `ARCH-017`

## Acceptance Criteria

- `gtc` uses a structured error type in core reusable code paths
- `src/lib.rs` exposes real reusable logic, not just perf/dist helpers
- integration tests run on Windows and Linux
- release workflow tests at least native targets before packaging
- release artifacts include checksums and at least one provenance/SBOM artifact
- config/env handling is centralized enough to document in one place
- snapshot/property tests exist for at least one representative domain each

## Verification Plan

Local:

- `cargo test`
- `cargo bench --bench perf -- --sample-size 10`
- `cargo test --test gtc_router_integration`

CI:

- Linux `cargo test`
- Windows `cargo test`
- macOS `cargo test`
- release dry-run path validates packaging plus checksum generation

## Notes For Implementation

- Keep user-facing CLI behavior stable unless a finding requires behavior change
- Prefer moving pure logic into `src/` rather than creating more binary-only modules
- Avoid trying to close every architecture finding in one giant patch; land as a stacked series
- Preserve the current performance/concurrency harness and extend it only where it helps validate refactors
