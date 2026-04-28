PR-GTC-07 — Install Greentic Toolchain From GHCR Version Manifest

Summary

This PR changes `gtc install` from a split installer into the canonical installer
for the complete Greentic public toolchain.

Today, `gtc install`:

1. installs host prerequisites,
2. installs a fixed list of companion crates from crates.io with `cargo binstall`,
3. delegates the rest of the public toolchain to `greentic-dev install tools`,
4. optionally installs tenant-authorized artifacts.

The new behavior should install every Greentic public tool listed in a resolved
toolchain manifest published to GHCR. `gtc install` must no longer call the old
legacy public tool bootstrap delegation `greentic-dev install tools`.

Tenant installs remain delegated. When `--tenant` is present, `gtc install`
should call:

```text
greentic-dev install --tenant <tenant> --key <key>
```

after the public toolchain install and deployer dist pack checks succeed.

This PR is scoped to `greenticai/greentic` only. Customer-approved public
toolchain installation moves to `gtc install`. `greentic-dev install tools`
remains supported as the development/bootstrap installer and may still be used
by `greentic-dev install --tenant`.

Current Code Affected

Primary installer code:

- `src/bin/gtc/install.rs`
  - `run_install`
  - `run_update`
  - `ensure_install_prereqs`
  - `resolve_tenant_key`
  - `is_binstall_available`
  - `install_companion_package`
  - `companion_binstall_args`
  - `published_crate_versions`
  - `latest_crate_version`
  - `resolve_artifacts_root`
  - `resolve_tenant_manifest_url`
  - `install_tenant_tool_reference`
  - `install_tenant_doc_reference`
  - `install_store_asset_reference`
  - `run_cargo`
  - `run_cargo_capture`

CLI wiring:

- `src/bin/gtc/cli.rs`
  - `build_cli`
- `src/bin/gtc/commands.rs`
  - `run`
- `src/bin/gtc.rs`
  - module declarations and test imports

User-facing strings:

- `assets/i18n/en.json`
- `assets/i18n/en-GB.json`

Tests:

- `tests/gtc_router_integration.rs`
  - `install_public_mode_calls_greentic_dev_install_tools`
  - `install_tenant_mode_uses_env_key_and_installs_tools_and_docs`
  - `install_skips_tenant_when_public_install_fails`
  - `update_calls_binstall_force_for_all_companions`
- `src/bin/gtc/tests.rs`
  - prereq and install helper tests near `ensure_install_prereqs_*`
- `src/bin/gtc/install.rs`
  - module tests near `ensure_install_prereqs_installs_missing_binstall_and_required_packages`
  - `install_companion_package_falls_back_to_previous_version_without_compile`
  - `ensure_install_prereqs_skips_binstall_reinstall_when_latest_lookup_fails`

Required Architecture

Add a new module:

- `src/bin/gtc/toolchain.rs`

The new module should own:

- manifest schema types,
- install source resolution,
- local installed state,
- digest comparison,
- `cargo binstall` command generation,
- package/bin installation loop.

Add to `src/bin/gtc.rs`:

```rust
#[path = "gtc/toolchain.rs"]
mod toolchain;
```

Keep tenant orchestration in `install.rs`. Do not move tenant/customer manifest
fetching, entitlement/auth token handling, tenant-specific binaries, tenant
docs, checksums, or local tenant install state into this repo.

Manifest Types

Create these types in `src/bin/gtc/toolchain.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ToolchainManifest {
    pub schema: String,
    pub toolchain: String,
    pub version: String,
    pub channel: Option<String>,
    pub created_at: Option<String>,
    pub packages: Vec<ToolchainPackage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ToolchainPackage {
    #[serde(rename = "crate")]
    pub crate_name: String,
    pub bins: Vec<String>,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedManifest {
    pub source: String,
    pub digest: Option<String>,
    pub manifest: ToolchainManifest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct InstalledToolchain {
    pub schema: String,
    pub source_kind: String,
    pub source: String,
    pub resolved_digest: Option<String>,
    pub channel: Option<String>,
    pub version: String,
    pub installed_at: String,
    pub packages: Vec<ToolchainPackage>,
}
```

Add validation helpers:

- `validate_toolchain_manifest(manifest: &ToolchainManifest) -> GtcResult<()>`
  - require `schema == "greentic.toolchain-manifest.v1"`
  - require `toolchain == "gtc"`
  - require at least one package
  - require every package has non-empty `crate_name`, non-empty `bins`, non-empty `version`
  - reject duplicate `crate_name` + `bin` entries
- `is_latest_version(version: &str) -> bool`
  - returns true only for `latest`

Install Source Options

Update `src/bin/gtc/cli.rs` in `build_cli`, inside the `install` subcommand:

- add `--channel <CHANNEL>`
- add `--release <RELEASE>`
- add `--manifest <PATH>`
- add `--force`
- add `--dry-run`

Rules:

- default source is `--channel stable`
- `--release` conflicts with `--channel`
- `--manifest` conflicts with both `--channel` and `--release`
- `--force` can be used with any source
- `--dry-run` can be used with any source
- existing `--tenant` and `--key` behavior stays available

Suggested argument names:

```rust
Arg::new("channel")
    .long("channel")
    .value_name("CHANNEL")
    .num_args(1)

Arg::new("release")
    .long("release")
    .value_name("RELEASE")
    .num_args(1)
    .conflicts_with("channel")

Arg::new("manifest")
    .long("manifest")
    .value_name("PATH")
    .num_args(1)
    .conflicts_with_all(["channel", "release"])

Arg::new("force")
    .long("force")
    .action(ArgAction::SetTrue)

Arg::new("dry-run")
    .long("dry-run")
    .action(ArgAction::SetTrue)
```

Use `release` instead of `version` so the install subcommand does not collide
conceptually with the global `--version` flag.

Final aligned CLI examples:

```text
gtc install --channel stable
gtc install --release 1.0.5
gtc install --manifest ./gtc-1.0.5.json
```

The terminology should align with greentic-dev release commands:

```text
greentic-dev release generate --release 1.0.5
greentic-dev release publish --release 1.0.5
greentic-dev release promote --release 1.0.5 --tag stable
```

Use `release` consistently because this value identifies a toolchain release
manifest, not the `gtc` binary version and not an individual crate version.

Add i18n keys to `assets/i18n/en.json` and `assets/i18n/en-GB.json`:

- `gtc.arg.install.channel.help`
- `gtc.arg.install.release.help`
- `gtc.arg.install.manifest.help`
- `gtc.arg.install.force.help`
- `gtc.arg.install.dry_run.help`
- `gtc.install.toolchain.up_to_date`
- `gtc.install.toolchain.resolving`
- `gtc.install.toolchain.installing_package`
- `gtc.install.toolchain.dry_run`
- `gtc.install.toolchain.item_ok`
- `gtc.install.toolchain.item_fail`
- `gtc.install.toolchain.state_write_failed`
- `gtc.err.invalid_toolchain_manifest`

Install Flow Changes

Update `src/bin/gtc/install.rs::run_install`.

Target flow:

```text
gtc install
  -> ensure host prerequisites
  -> install public toolchain from GHCR release manifest
  -> ensure deployer dist pack
  -> if --tenant is present:
       call greentic-dev install --tenant ... --key ...
```

Current behavior to remove:

```rust
let public_args = vec!["install".to_string(), "tools".to_string()];
let public_status = passthrough(DEV_BIN, &public_args, debug, locale);
if public_status != 0 {
    return public_status;
}
```

New behavior:

1. print public install message,
2. call `ensure_install_prereqs(debug, locale)`,
3. build a `ToolchainInstallOptions` from `sub_matches`,
4. call `toolchain::run_toolchain_install(options, debug, locale)`,
5. if `--dry-run` was set, return successfully after printing planned actions,
6. call `ensure_deployer_dist_pack(debug, locale)`,
7. if `--tenant` is present, resolve the tenant key and delegate tenant install
   to `greentic-dev install --tenant <tenant> --key <key>`.

The tenant install should run only after public toolchain install and deployer
dist pack validation succeed. If public toolchain install fails, tenant install
must be skipped and the toolchain install status should be returned.

Remove only this public-phase delegation:

```text
greentic-dev install tools
```

Keep this tenant-phase delegation:

```text
greentic-dev install --tenant <tenant> --key <key>
```

`greentic-dev install --tenant` owns tenant/customer manifest fetching,
entitlement/auth token handling, tenant-specific binaries, tenant docs,
checksums, and local tenant install state. `gtc` should orchestrate this call,
not reimplement it.

The existing in-repo tenant artifact implementation in `install.rs` should be
removed or disconnected from `run_install`. Keep `resolve_tenant_key` and
`tenant_env_var_name` if they are still useful for resolving `--key`, env, or
prompted credentials before calling greentic-dev.

Split Prerequisites From Toolchain Packages

Update `src/bin/gtc/install.rs::ensure_install_prereqs`.

Keep these checks here:

- `mksquashfs`
- Rust version `1.95+`
- `wasm32-wasip2`
- `cargo-component`
- `cargo-binstall`

Remove this package loop from `ensure_install_prereqs`:

```rust
for package in [
    DEV_BIN,
    OP_BIN,
    BUNDLE_BIN,
    SETUP_BIN,
    START_BIN,
    DEPLOYER_BIN,
] {
    let status = install_companion_package(package, false, debug, locale);
    if status != 0 {
        return status;
    }
}
```

After this PR, `ensure_install_prereqs` should not install Greentic companion
crates. It should only prepare the host so manifest-driven install can run.

Toolchain Install Options

Create in `src/bin/gtc/toolchain.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolchainInstallOptions {
    pub source: ToolchainSource,
    pub force: bool,
    pub dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ToolchainSource {
    Channel(String),
    Release(String),
    LocalManifest(PathBuf),
}
```

Create:

- `ToolchainInstallOptions::from_matches(matches: &ArgMatches) -> GtcResult<Self>`

Default:

```rust
ToolchainSource::Channel("stable".to_string())
```

GHCR Source Mapping

Create in `src/bin/gtc/toolchain.rs`:

- `toolchain_source_ref(source: &ToolchainSource) -> Option<String>`

Mapping:

- `Channel(name)` -> `ghcr.io/greenticai/greentic-versions/gtc:<name>`
- `Release(release)` -> `ghcr.io/greenticai/greentic-versions/gtc:<release>`
- `LocalManifest(path)` -> no GHCR ref

Do not restrict channel names. `stable`, `dev`, `rc`, `demo`,
customer-specific channels, and future rollout tags should all work without a
`gtc` code change.

Do not hard-code the package list in `install.rs` anymore. The manifest is the
only source of truth for Greentic toolchain composition.

Manifest Resolution

Create in `src/bin/gtc/toolchain.rs`:

- `resolve_toolchain_manifest(source: &ToolchainSource, debug: bool, locale: &str) -> GtcResult<ResolvedManifest>`
- `resolve_local_manifest(path: &Path) -> GtcResult<ResolvedManifest>`
- `resolve_ghcr_manifest(reference: &str, debug: bool, locale: &str) -> GtcResult<ResolvedManifest>`

`resolve_local_manifest`:

- read the JSON file,
- parse `ToolchainManifest`,
- validate it,
- compute a local digest using SHA-256 over the file bytes,
- return `source` as the file path and `digest` as `sha256:<hex>`.

`resolve_ghcr_manifest`:

- pull the OCI artifact from GHCR,
- require exactly one JSON layer with media type
  `application/vnd.greentic.toolchain.manifest.v1+json`,
- parse the JSON manifest payload as `ToolchainManifest`,
- validate it,
- return the GHCR reference as `source`,
- return the resolved OCI digest as `digest`.

The repo already depends on `oci-distribution`; prefer that over shelling out to
external `oras` or `docker` commands. If the GHCR artifact format needs a thin
adapter, keep it inside `toolchain.rs`.

Local Installed State

Create in `src/bin/gtc/toolchain.rs`:

- `installed_toolchain_path() -> GtcResult<PathBuf>`
- `read_installed_toolchain() -> GtcResult<Option<InstalledToolchain>>`
- `write_installed_toolchain(state: &InstalledToolchain) -> GtcResult<()>`
- `installed_state_from_resolved(resolved: &ResolvedManifest) -> InstalledToolchain`

Path:

```text
~/.greentic/toolchain/installed.json
```

Persist these fields:

```json
{
  "schema": "greentic.installed-toolchain.v1",
  "source_kind": "channel | release | local",
  "source": "...",
  "resolved_digest": "...",
  "version": "...",
  "channel": "...",
  "installed_at": "...",
  "packages": []
}
```

Use `directories::BaseDirs` consistently with
`install.rs::resolve_artifacts_root`.

Comparison rule:

- compare remote/local `ResolvedManifest.digest` to
  `InstalledToolchain.resolved_digest`
- if both are present and equal and `--force` is false, print
  `gtc.install.toolchain.up_to_date` and return success
- do not compare only `manifest.version`

For local manifest installs, use the SHA-256 digest of the local manifest file
so repeated local installs also skip when unchanged.

Write `installed.json` only after every package/bin install succeeds. If any
install fails, return a failure status and leave the previous installed state
unchanged.

Installing Packages

Create in `src/bin/gtc/toolchain.rs`:

- `run_toolchain_install(options: ToolchainInstallOptions, debug: bool, locale: &str) -> i32`
- `install_toolchain_manifest(resolved: &ResolvedManifest, force: bool, debug: bool, locale: &str) -> i32`
- `install_toolchain_package(package: &ToolchainPackage, debug: bool, locale: &str) -> i32`
- `toolchain_binstall_args(package: &ToolchainPackage, bin: &str) -> Vec<String>`

Required command generation:

Pinned package:

```text
cargo binstall -y --locked --force <crate> --version <version> --bin <bin>
```

Latest package:

```text
cargo binstall -y --locked --force <crate> --bin <bin>
```

Multi-bin crate:

```text
cargo binstall -y --locked --force greentic-runner --version 0.5.10 --bin greentic-runner
cargo binstall -y --locked --force greentic-runner --version 0.5.10 --bin greentic-runner-cli
```

Do not use the old fallback behavior from `install_companion_package`. A pinned
manifest entry must install the exact requested version or fail. A `latest`
entry is allowed only when the manifest chooses it, normally for the `dev`
channel.

Always pass `--force` to `cargo binstall`, even when `gtc install --force` was
not set. The `gtc`-level `--force` controls digest skip behavior; the
`cargo-binstall` `--force` keeps bin replacement deterministic once
installation has been selected.

Dry Run

If `--dry-run` is set:

1. resolve and validate the manifest,
2. print the resolved source and digest,
3. print every `cargo binstall` command that would run,
4. do not execute `cargo binstall`,
5. do not call `ensure_deployer_dist_pack`,
6. do not run tenant install,
7. do not write `installed.json`.

`--dry-run` should still perform manifest resolution so GHCR/local manifest
errors are visible before a real install.

Old Helpers To Remove Or Stop Using

In `src/bin/gtc/install.rs`, remove or stop using:

- `install_companion_package`
- `companion_binstall_args`
- `published_crate_versions`
- `latest_crate_version`
- `fake_crate_versions_from_env`
- `resolve_tenant_manifest_url`
- `install_tenant_tool_reference`
- `install_tenant_doc_reference`
- `install_store_asset_reference`
- `install_store_asset_item`

Keep:

- `resolve_tenant_key`
- `tenant_env_var_name`
- `is_binstall_available`
- `detect_binstall_version`
- `latest_binstall_version`
- `run_cargo`
- `run_cargo_capture`
- `semver_compare`
- `parse_first_semver`

If `toolchain.rs` needs to call `run_cargo`, change its visibility from private
to:

```rust
pub(crate) fn run_cargo(...)
```

Alternatively, move generic command execution helpers into `process.rs`. Keep
the PR smaller by changing visibility unless there is a clear reason to move
them.

Update Behavior

Update `src/bin/gtc/install.rs::run_update`.

Current behavior:

- requires existing cargo-binstall,
- force-installs the fixed companion list,
- calls `greentic-dev install tools`.

New behavior:

- call `ensure_install_prereqs(debug, locale)`,
- call the same manifest-driven installer with `force: true`, using the default
  stable channel.

Recommended implementation:

```rust
pub(super) fn run_update(debug: bool, locale: &str) -> i32 {
    let preflight_status = ensure_install_prereqs(debug, locale);
    if preflight_status != 0 {
        return preflight_status;
    }

    run_toolchain_install(
        ToolchainInstallOptions {
            source: ToolchainSource::Channel("stable".to_string()),
            force: true,
            dry_run: false,
        },
        debug,
        locale,
    )
}
```

Do not call `greentic-dev install tools` from `run_update`.

Deployer Dist Pack Check

Keep `install.rs::ensure_deployer_dist_pack` after public toolchain install.

The manifest must include `greentic-deployer`, but `ensure_deployer_dist_pack`
still protects the contract that the deployer dist pack exists after install.
Do not remove this check in the first PR.

Tests To Add Or Update

Unit tests in `src/bin/gtc/toolchain.rs`:

- `parses_pinned_toolchain_manifest`
- `parses_latest_toolchain_manifest`
- `rejects_manifest_with_wrong_schema`
- `rejects_manifest_with_wrong_toolchain`
- `rejects_manifest_with_empty_packages`
- `rejects_manifest_with_missing_crate_bins_or_version`
- `rejects_manifest_with_duplicate_crate_bin_entries`
- `toolchain_source_defaults_to_stable`
- `toolchain_source_maps_channel_to_ghcr_ref`
- `toolchain_source_maps_unrestricted_channel_to_ghcr_ref`
- `toolchain_source_maps_release_to_ghcr_ref`
- `toolchain_binstall_args_for_pinned_package`
- `toolchain_binstall_args_for_latest_package`
- `toolchain_binstall_args_for_multi_bin_crate`
- `unchanged_digest_skips_install`
- `force_installs_despite_unchanged_digest`
- `dry_run_prints_commands_without_installing_or_writing_state`
- `local_manifest_uses_file_digest`
- `installed_toolchain_path_is_under_greentic_toolchain`

Integration tests in `tests/gtc_router_integration.rs`:

- replace `install_public_mode_calls_greentic_dev_install_tools` with
  `install_public_mode_installs_manifest_toolchain`
  - write a local manifest fixture,
  - run `gtc install --manifest <fixture>`,
  - assert cargo log contains pinned `--version` and `--bin` calls,
  - assert the `greentic-dev` logger is not called with `install tools`.
- update all install CLI tests and docs from `gtc install --version ...` to
  `gtc install --release ...`.
- update `install_tenant_mode_uses_env_key_and_installs_tools_and_docs`
  - rename to `install_tenant_mode_delegates_to_greentic_dev_after_toolchain_success`,
  - use `--manifest <fixture>`,
  - assert public manifest install completes first,
  - assert `greentic-dev install --tenant acme --key <resolved-key>` is called,
  - assert no delegated `greentic-dev install tools` call.
- replace `install_skips_tenant_when_public_install_fails`
  - fail the cargo shim for one manifest package,
  - assert `greentic-dev install --tenant ...` is skipped,
  - assert returned status is the failing toolchain install status.
- update `update_calls_binstall_force_for_all_companions`
  - rename to `update_installs_manifest_toolchain_with_force`,
  - assert `gtc update` runs manifest-driven public toolchain install with
    `force: true`.
- add `install_dry_run_executes_nothing`
  - run `gtc install --manifest <fixture> --dry-run`,
  - assert cargo log has no `binstall` install commands,
  - assert no `installed.json` is written,
  - assert `greentic-dev install --tenant ...` does not run even if `--tenant`
    is also present.

Module tests in `src/bin/gtc/install.rs`:

- update `ensure_install_prereqs_installs_missing_binstall_and_required_packages`
  - rename to `ensure_install_prereqs_installs_missing_binstall_only`
  - assert no Greentic companion package names are installed there.
- update `ensure_install_prereqs_skips_binstall_reinstall_when_latest_lookup_fails`
  - assert prereqs still succeed,
  - assert no companion crates are installed there.
- remove or rewrite `install_companion_package_falls_back_to_previous_version_without_compile`
  - fallback versions are no longer desired for manifest-pinned installs.
- remove or rewrite tenant artifact unit tests that target implementation now
  owned by greentic-dev:
  - `install_tenant_doc_reference_installs_local_doc_into_docs_tree`
  - `install_tenant_doc_reference_rejects_unsafe_file_names`
  - `install_tenant_tool_reference_installs_matching_local_artifact`
  - `install_tenant_tool_reference_rejects_missing_platform_target`
  - `install_tenant_tool_reference_rejects_digest_mismatch`

Test Fixtures

Use local manifest fixtures in tests to avoid GHCR/network access.

Example pinned test manifest:

```json
{
  "schema": "greentic.toolchain-manifest.v1",
  "toolchain": "gtc",
  "version": "1.0.4",
  "channel": "stable",
  "packages": [
    {
      "crate": "greentic-dev",
      "bins": ["greentic-dev"],
      "version": "0.5.9"
    },
    {
      "crate": "greentic-runner",
      "bins": ["greentic-runner", "greentic-runner-cli"],
      "version": "0.5.10"
    }
  ]
}
```

Expected cargo log lines:

```text
binstall -y --locked --force greentic-dev --version 0.5.9 --bin greentic-dev
binstall -y --locked --force greentic-runner --version 0.5.10 --bin greentic-runner
binstall -y --locked --force greentic-runner --version 0.5.10 --bin greentic-runner-cli
```

Example latest test manifest:

```json
{
  "schema": "greentic.toolchain-manifest.v1",
  "toolchain": "gtc",
  "version": "dev",
  "channel": "dev",
  "packages": [
    {
      "crate": "greentic-flow",
      "bins": ["greentic-flow"],
      "version": "latest"
    }
  ]
}
```

Expected cargo log line:

```text
binstall -y --locked --force greentic-flow --bin greentic-flow
```

Dependency Changes

`Cargo.toml` already has:

- `serde` with derive
- `serde_json`
- `directories`
- `sha2`
- `oci-distribution`
- `tokio` runtime support

Add only if needed:

- `time = { version = "0.3", features = ["formatting"] }`

Use it for RFC3339 `installed_at`. Do not add a larger date/time dependency just
for one timestamp.

Acceptance Criteria

- `gtc install` resolves a toolchain manifest from GHCR by default.
- `gtc install --channel dev` resolves the dev manifest.
- `gtc install --release 1.0.4` resolves the pinned manifest.
- `gtc install --manifest ./gtc-1.0.4.json` installs from a local manifest.
- `gtc install --force` reinstalls even when the local digest matches.
- `gtc install --dry-run` resolves the manifest and prints planned commands
  without executing installs, writing state, checking deployer dist packs, or
  running tenant install.
- `gtc install` installs every crate/bin listed in the manifest.
- Pinned manifest entries install the exact requested version.
- `latest` manifest entries omit `--version`.
- Multi-bin crates run one `cargo binstall` call per requested bin.
- `gtc install` writes `~/.greentic/toolchain/installed.json`.
- `gtc install` writes installed state only after full success.
- digest comparison skips unchanged installs.
- `gtc install` no longer calls `greentic-dev install tools`.
- `gtc update` no longer calls `greentic-dev install tools`.
- `gtc install --tenant acme --key xyz` installs/verifies the public Greentic
  toolchain from the GHCR manifest, ensures deployer dist packs, then calls
  `greentic-dev install --tenant acme --key xyz`.
- tenant install delegation is skipped when public toolchain install fails.

Out Of Scope

- changing `greentic-dev install tools`
- adding greentic-dev release commands
- publishing GHCR manifests
- changing tenant manifest format
- reimplementing tenant/customer manifest fetching in gtc
- reimplementing entitlement/auth token handling in gtc
- reimplementing tenant-specific binaries, docs, checksums, or tenant install
  state in gtc
- removing deployer dist pack validation
- changing `gtc dev`, `gtc wizard`, `gtc setup`, or other passthrough routing

Notes For The Follow-Up greentic-dev PR

The greentic-dev PR should keep `greentic-dev install tools` supported as the
development/bootstrap installer, backed by the canonical Greentic tool
catalogue. It should not be described as deprecated, but docs should distinguish
it from customer-approved pinned installs via `gtc install`.
