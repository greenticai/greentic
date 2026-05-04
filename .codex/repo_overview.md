# Repository Overview

## 1. High-Level Purpose

This repository provides the Rust CLI package for `gtc`, the Greentic command-line entrypoint for local install flows, wizard/setup/start routing, deploy/start orchestration, admin operations, and related documentation and release packaging.

The repo is no longer just a minimal pass-through router. It still delegates substantial work to companion binaries such as `greentic-dev`, `greentic-operator`, `greentic-setup`, `greentic-start`, `greentic-bundle`, and `greentic-deployer`, but it also owns local command parsing, install/update flows, extension handoff routing, deploy/start/stop orchestration, admin subcommands, config handling, release packaging, and repo-local docs.

## 2. Main Components and Functionality

- **Path:** `Cargo.toml`
- **Role:** Crate manifest and package metadata.
- **Key functionality:**
- Declares package `gtc` version `1.0.16`.
- Pins `rust-version = "1.95"`.
- Declares the `gtc` binary and `cargo-binstall` metadata for packaged release artifacts.
- Pulls in runtime dependencies such as `clap`, `reqwest`, `greentic-distributor-client`, `greentic-i18n`, and `greentic-types`.
- During release install work, uses the local sibling `../greentic-distributor-client` with OCI component support so `gtc install --release ...` can prefetch release artifacts through the shared distributor cache layer.

- **Path:** `src/bin/gtc.rs`
- **Role:** Binary entrypoint and module wiring.
- **Key functionality:**
- Wires the `gtc` submodules for CLI, commands, deploy, install, release-cache export/import, extensions, routing, prompts, archive handling, and admin operations.
- Defines companion binary constants such as `greentic-dev`, `greentic-operator`, `greentic-setup`, `greentic-start`, `greentic-bundle`, and `greentic-deployer`.

- **Path:** `src/bin/gtc/cli.rs`
- **Role:** CLI command tree and help surface.
- **Key functionality:**
- Defines repo-owned commands including `install`, `release-cache`, `update`, `add-admin`, `remove-admin`, `admin`, `start`, `stop`, `dev`, `op`, `wizard`, `setup`, and `help`.
- Documents extension-handoff flags for `wizard`, `setup`, and `start`.
- Allows `gtc install --release <version> --channel <stable|dev|rnd>` so a release can be installed with an explicit channel context.
- `gtc install` supports combinable phase selectors: `--install-binaries-only`, `--install-packs-only`, `--install-components-only`, and `--install-tenant-only`.
- Plain `gtc install` defaults to the stable channel and now represents the full stable install path: binaries plus release-manifest packs and components cached through the distributor cache.
- `gtc wizard` and `gtc setup` check the installed release context before handoff, using the launcher's channel (`gtc` -> stable, `gtc-dev` -> dev, `gtc-rnd` -> rnd), with `--strict-release-context` to fail on mismatch and `--ignore-release-context` to skip the check.

- **Path:** `src/bin/gtc/commands.rs`
- **Role:** Main command dispatcher.
- **Key functionality:**
- Parses localized CLI args and dispatches to doctor, install, release-cache, update, admin, start/stop, extension-handoff flows, or passthrough routes.
- Owns the local behavior split between pure passthrough and repo-owned orchestration.

- **Path:** `src/bin/gtc/deploy/`
- **Role:** Start/stop and deploy orchestration helpers.
- **Key functionality:**
- Resolves bundle references and bundle directories.
- Owns local start/stop request parsing and target selection.
- Bridges into local runtime or deployer/cloud paths depending on current command inputs.

- **Path:** `src/bin/gtc/install.rs`
- **Role:** Tool install and update flows.
- **Key functionality:**
- Manages tenant-aware companion binary install/update behavior and remote manifest/download flows.
- `src/bin/gtc/toolchain.rs` owns the toolchain-manifest install path, including optional `extension_packs` and `components` manifest sections.
- For release installs, it prefetches those artifacts through `greentic-distributor-client`, verifies blob and cache-entry presence, writes a release index under `GREENTIC_CACHE_DIR/release-index/v1/<channel>/<release>.json`, and records the current release context under `~/.greentic/releases/current.json` or `GTC_RELEASE_STATE_DIR/current.json`.
- The install path can run binaries, release packs, release components, and tenant artifact install phases independently, while no selector keeps the full default behavior.

- **Path:** `src/bin/gtc/release_cache.rs`
- **Role:** Air-gapped release cache archive export/import.
- **Key functionality:**
- Implements `gtc release-cache export --release <release> --channel <channel> --output <archive.tar.gz>` and `gtc release-cache import --input <archive.tar.gz>`.
- Uses the same distributor cache root policy as `greentic-distributor-client::DistOptions::default()`.
- Exports the release index and referenced `artifacts/sha256/.../{blob,entry.json}` files into a gzip tar archive with `manifest.json` and `checksums.json`.
- Imports through a temporary directory, validates archive paths, schema, checksums, release index metadata, artifact count, and referenced blob/entry files before restoring into the configured cache root.

- **Path:** `src/bin/gtc/admin.rs`
- **Role:** Admin certificate, token, access, tunnel, and status operations.
- **Key functionality:**
- Manages local admin registry state and remote admin/runtime control helpers for deployed bundles.

- **Path:** `src/perf_targets.rs`
- **Role:** Perf-support utilities shared by the crate.
- **Key functionality:**
- Includes locale selection helpers and SHA-256 hashing helpers used by the binary and perf-related code paths.

- **Path:** `README.md`
- **Role:** Main onboarding and product overview document.
- **Key functionality:**
- Markets the repo as the Digital Workers OS.
- Shows demo-driven `gtc wizard`, `gtc setup`, and `gtc start` onboarding flows.
- Already contains substantial prose about components, flows, extension packs, install/setup/start, and related repos.

- **Path:** `docs/`
- **Role:** Repo-local documentation set.
- **Key functionality:**
- Includes canonical entrypoints such as `docs/00-start-here.md`, `docs/00-start-here/current-terms-and-deprecations.md`, `docs/99-agent-rules/coding-agents.md`, and `docs/04-schemas/README.md`.
- Includes generated-schema baseline docs such as `docs/04-schemas/wizard-schema.md`, `docs/04-schemas/setup-schema.md`, `docs/04-schemas/component-schemas/README.md`, and `docs/04-schemas/drift-report.md`, plus checked-in raw JSON artifacts captured from current tooling.
- Includes a new schema-refresh path via `ci/sync_schema_docs.sh` and the `gtc docs sync-schemas` wrapper, which refresh repo-owned schema docs and optional companion coverage with best-effort defaults.
- Includes a warning-first drift layer: schema sync now emits `docs/04-schemas/drift-report.md`, and `ci/local_check.sh` warns when generated schema docs changed and prose/example review is likely needed.
- Includes a new validated-example layer via `docs/examples/` and `ci/validate_doc_examples.sh`, which currently validate launcher-style wizard answer examples against the generated wizard schema.
- Includes a troubleshooting and maintenance layer via `docs/06-troubleshooting/common-authoring-and-runtime-issues.md`, plus an explicit documentation-maintenance policy in `docs/00-start-here.md` and stronger agent verification rules in `docs/99-agent-rules/coding-agents.md`.
- Includes core-model and trust-boundary docs such as `docs/01-core-model/components-flows-packs-bundles.md`, `docs/01-core-model/extensions-overview.md`, `docs/05-examples/demo-map.md`, and `docs/05-examples/repo-compatibility-map.md`.
- Includes CLI docs such as `docs/02-cli/gtc-install.md`, `docs/02-cli/gtc-setup.md`, and `docs/02-cli/gtc-start.md`, which explain install channels/cache behavior, setup ownership boundaries, and local-versus-deployer behavior.
- Agent-facing docs now call out that stable pack/component references should use `oci://ghcr.io/...:stable` rather than `oci://ghcr.io/...:latest` unless an example is explicitly testing an unverified moving target.
- Includes a new canonical wizard doc at `docs/02-cli/gtc-wizard.md`, plus authoring docs such as `docs/03-authoring/happy-path-build-an-app.md` and `docs/03-authoring/answers-json-patterns.md`.
- Includes new flow-oriented guidance such as `docs/02-cli/greentic-flow.md` and `docs/03-authoring/flow-step-schema-mapping.md`, which frame schema-first flow authoring conservatively as operational guidance.
- Includes new platform-capability guidance such as `docs/03-authoring/i18n-qa-distributor-client.md`, `docs/03-authoring/mcp-wasm-and-adapters.md`, and `docs/03-authoring/mcp-config-secrets-oauth.md`, which keep ownership boundaries explicit for i18n, QA/schema usage, distributor-client, MCP composition, config, secrets, and OAuth.
- Includes new extension-pattern docs such as `docs/03-authoring/extension-pack-patterns.md` and `docs/03-authoring/adaptive-card-orchestration.md`, which add practical guidance for extension-pack selection, extension handoff usage, and adaptive-card flow/component boundaries.
- Keeps `docs/gtc-wizard.md` only as a redirect stub to the new canonical wizard path.
- Also includes existing docs such as `docs/config.md`, `docs/ci.md`, `docs/release-verification.md`, `docs/architecture.mmd`, and repo catalog documents.
- The new canonical hierarchy now includes the full planned rollout: foundation, core-model/boundary, setup/start CLI, wizard/authoring, flow-guidance, platform-capability, extension-pattern, generated-schema baseline, schema-sync, drift-report/enforcement-warning, validated examples, and troubleshooting/maintenance hardening.

- **Path:** `.codex/global_rules.md`
- **Role:** Repo-specific Codex workflow rules.
- **Key functionality:**
- Requires pre/post refresh of `.codex/repo_overview.md`.
- Requires running `ci/local_check.sh` for PR-style work.
- Requires reuse-first behavior across Greentic repos.
- References `.codex/repo_overview_task.md`, but that file is currently missing.

- **Path:** `.codex/PR-DOCS-*.md`
- **Role:** Documentation-program planning set.
- **Key functionality:**
- Defines the planned 12-step canonical docs rollout, starting with foundation/source-of-truth work and ending with troubleshooting and maintenance hardening.

- **Path:** `ci/local_check.sh`
- **Role:** Repo-local CI wrapper.
- **Key functionality:**
- Runs `cargo fmt --all -- --check`
- Runs `cargo clippy --all-targets --all-features -- -D warnings`
- Runs `cargo test --all-targets --all-features`
- Runs ignored perf test coverage
- Runs `cargo publish --dry-run --allow-dirty`

## 3. Work In Progress, TODOs, and Stubs

- **Location:** `docs/gtc-wizard.md:94`
- **Status:** TODO
- **Short description:** Historical TODOs from the old full wizard doc were removed when the file became a redirect stub.

- **Location:** `.codex/global_rules.md:73`
- **Status:** Missing referenced file
- **Short description:** `.codex/global_rules.md` instructs Codex to follow `.codex/repo_overview_task.md`, but that file is not present in the repo.

- **Location:** `.codex/PR-DOCS-01-foundation-and-source-of-truth.md` through `.codex/PR-DOCS-12-troubleshooting-and-maintenance-hardening.md`
- **Status:** Planning set retained after implementation
- **Short description:** The planning briefs remain in `.codex/` as execution history and scope boundaries for the canonical docs rollout that has now been implemented end-to-end.

## 4. Broken, Failing, or Conflicting Areas

- **Location:** `README.md` versus the broader docs program
- **Evidence:** README now points to canonical doc entrypoints, but it still contains broad operational prose and demo-hosted answer examples.
- **Likely cause / nature of issue:** The wizard and flow-guidance layers are now in place, but the generated-schema flow and deeper validation/tooling work are not completed yet.

- **Location:** `.github/workflows/release.yml`
- **Evidence:** Release workflow still publishes on the main branch flow and may fail on already-published versions unless guarded elsewhere.
- **Likely cause / nature of issue:** Release publication policy remains sensitive to versioning and workflow assumptions.

- **Location:** `gtc` runtime dependency on companion binaries
- **Evidence:** Multiple command paths still depend on `greentic-dev`, `greentic-operator`, `greentic-setup`, `greentic-start`, `greentic-bundle`, and `greentic-deployer`.
- **Likely cause / nature of issue:** `gtc` owns orchestration and routing, but end-to-end behavior still depends on separately installed or separately produced companions.

## 5. Notes for Future Work

- Expand validated example coverage beyond the current launcher-style wizard answers once more repo-owned structured artifacts are locally provable.
- Decide whether `.codex/repo_overview_task.md` should be restored or whether the new inline maintenance instructions in `.codex/global_rules.md` are sufficient.
- Resolve the current `greentic_i18n` compile failure before treating `ci/local_check.sh` as green again.
