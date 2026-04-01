# CI and Release Contract

This repository uses a mix of delegated CI and local workflows.

## Delegated CI

The main CI entrypoint is [`ci.yml`](../.github/workflows/ci.yml), which delegates to:

- `greenticai/.github/.github/workflows/host-crate-ci.yml@main`

That shared workflow is the default pull request and `main` branch gate for this
repository. At the time of writing, contributors should assume:

- the delegated workflow is the primary general-purpose CI check
- it is not defined in this repository
- platform coverage from the delegated workflow should not be inferred without
  inspecting the shared workflow itself

## Local workflows in this repo

This repository also defines local workflows that cover repo-specific needs:

- [`perf.yml`](../.github/workflows/perf.yml)
  Lightweight performance and concurrency guardrails for pull requests and `main`
- [`nightly-coverage.yml`](../.github/workflows/nightly-coverage.yml)
  Scheduled and on-demand coverage policy validation using `greentic-dev coverage`
- [`release.yml`](../.github/workflows/release.yml)
  Main-branch release packaging and publication

## Nightly coverage expectations

The nightly coverage workflow is intentionally separate from pull request gating:

- it runs on a nightly schedule and via manual dispatch
- it installs `cargo-binstall` via the first-party `cargo-bins/cargo-binstall`
  action
- it uses `cargo binstall` for every required coverage binary:
  - `cargo-llvm-cov`
  - `cargo-nextest`
  - `greentic-dev`
- it adds the Rust `llvm-tools-preview` component required by `cargo-llvm-cov`
- it runs `greentic-dev coverage`, which generates a coverage report and checks
  the repo-local [`coverage-policy.json`](../coverage-policy.json)
- it uploads the generated JSON coverage report as an artifact for inspection when
  the policy fails

## Release workflow expectations

The release workflow does more than package binaries:

- it reuses the delegated CI gate before release steps run
- it prepares the embedded deployer pack used by `gtc`
- it builds release binaries for Linux, macOS, and Windows targets
- it runs `cargo test --locked` on native runner/target pairs before packaging:
  - `ubuntu-latest` -> `x86_64-unknown-linux-gnu`
  - `ubuntu-24.04-arm` -> `aarch64-unknown-linux-gnu`
  - `macos-15` -> `aarch64-apple-darwin`
  - `windows-latest` -> `x86_64-pc-windows-msvc`
- it skips test execution for cross/compatibility build targets that are packaged
  but not natively executed on the runner:
  - `x86_64-apple-darwin` on `macos-15`
  - `aarch64-pc-windows-msvc` on `windows-latest`

## Known gaps

- Integration tests in [`gtc_router_integration.rs`](../tests/gtc_router_integration.rs) are
  still Unix-only, so Windows release-time testing does not yet provide equivalent
  integration coverage.
- The delegated CI workflow is still referenced by branch name rather than a pinned
  commit SHA.

Those gaps are tracked as architecture follow-up work rather than being silently
assumed away in the workflow definitions.
