# PR-07: Prepared Bundle Contract for `gtc start`

Repo: `gtc`

## Goal

Make `gtc start <bundle>` and `gtc start <bundle> --target <target>` run the
same warmed, prepared, post-setup bundle content. Target selection must change
only where the bundle runs, not which bundle content is run.

Do not introduce central runtime config schema, pack-specific config plumbing,
or provider-specific knowledge of app files. The bundle artifact is the
contract.

## Current-Code Audit

### Start dispatch

- `src/bin/gtc/deploy/start_stop.rs`
  - `run_start_with_bundle_ref_and_tail` parses deploy-specific flags with
    `parse_start_cli_options`.
  - It resolves the user bundle once with `resolve_bundle_reference`.
  - It then parses runtime args with `parse_start_request`, passing
    `resolved.bundle_dir`.
  - Local runtime mode invokes `greentic-start` with `request.to_runtime_start_args`.
  - Target-backed mode calls `ensure_started_or_deployed` with the original
    `bundle_ref`, `resolved`, and CLI options.

Current issue: local runtime receives `resolved.bundle_dir`, while target-backed
branches later choose their own artifact/source behavior. There is no canonical
prepared bundle object shared by both branches.

### Bundle resolution and fingerprinting

- `src/bin/gtc/deploy/bundle_resolution.rs`
  - `resolve_bundle_reference` supports local paths, `file://`, `http(s)://`,
    `oci://`, `repo://`, and `store://`.
  - Local directories produce `StartBundleResolution { bundle_dir, deploy_artifact: None }`.
  - Archives are extracted to a temp dir and also retain the original archive in
    `deploy_artifact`.
  - `fingerprint_bundle_dir` delegates to `perf_targets::collect_bundle_entries`
    and normalizes entries with `normalize_bundle_fingerprint`.

- `src/perf_targets.rs`
  - `collect_bundle_entries` records directories and files as path, size, and
    mtime.
  - `normalize_bundle_fingerprint` strips mtime and ignores selected runtime
    noise paths.

Current issue: same-size content-only config changes can keep the same
fingerprint. Acceptance requires config-only changes to affect the prepared
bundle digest/fingerprint.

### Deployable artifact creation

- `src/bin/gtc/deploy/cloud_deploy/deployment_state.rs`
  - `prepare_deployable_bundle_artifact` returns `resolved.deploy_artifact` if
    present.
  - Otherwise it runs `greentic-setup bundle build --bundle <resolved.bundle_dir>
    --out <artifact>`.

Current issue: for archive inputs, target-backed start can reuse the original
input artifact instead of rebuilding from the extracted/prepared root. That is
not a safe prepared-bundle contract if any post-resolution or setup-mutated
workspace content differs from the original artifact.

### Upload path

- `src/bin/gtc/deploy/cloud_deploy.rs`
  - `resolve_upload_bundle` calls
    `bundle_upload_orchestrator::prepare_warmed_bundle`, which runs
    `greentic-bundle build --root <bundle_dir> --output <file> --warmup`, then
    uploads that file with `greentic-deployer bundle-upload upload`.

- `src/bin/gtc/deploy/cloud_deploy/deployment_state.rs`
  - `run_multi_target_deployer_apply` uses this warmed upload path only when
    `--upload-bundle` is supplied.
  - If upload is not supplied, it uses `prepare_deployable_bundle_artifact`.
  - `--deploy-bundle-source` and `GREENTIC_DEPLOY_BUNDLE_SOURCE` override the
    deployer bundle source.

Current issue: warmup is upload-only behavior today. There are also two artifact
builders with different commands and semantics. A user-supplied remote deploy
source can point deployers at a different artifact than the local prepared
bundle. Backward compatibility may keep the flag, but the new debug output and
docs should make this explicit and the default path should prefer the prepared
artifact/upload.

### Deployer invocation

- `run_multi_target_deployer_apply` passes:
  - `--bundle-root <resolved.bundle_dir>`
  - `--bundle-source <deploy_bundle_source>`
  - `--bundle-digest <bundle_digest>`

Current issue: these should come from one prepared bundle result, not from
separate `resolved`, upload, override, and artifact decisions.

### Deployment state

- `StartDeploymentState` stores `target`, `bundle_fingerprint`, `bundle_ref`,
  `deployed_at_epoch_s`, and optional `artifact_path`.
- Deploy-needed checks compare `normalize_bundle_fingerprint(previous)` against
  the current directory fingerprint.

Current issue: state should store and compare the prepared bundle digest. The
directory fingerprint can remain as extra diagnostics, but it cannot be the
source of truth for redeploy detection.

## Proposed Internal Contract

Add one canonical prepared bundle result under `src/bin/gtc/deploy/`, for
example in a new `prepared_bundle.rs` module:

```rust
pub(super) struct PreparedBundle {
    pub(super) input_ref: String,
    pub(super) prepared_root: PathBuf,
    pub(super) artifact_path: PathBuf,
    pub(super) digest: String,
    pub(super) source_kind: PreparedBundleSourceKind,
    pub(super) was_rebuilt: bool,
    pub(super) included_asset_config_count: usize,
    pub(super) _hold: Option<tempfile::TempDir>,
}
```

The exact shape can differ, but both local and target-backed start paths must
receive the same object.

## Implementation Plan

1. Add `PreparedBundle` and `prepare_bundle_for_start`.
   - Inputs: original `bundle_ref`, `StartBundleResolution`, `debug`, `locale`.
   - Output: prepared root, deployable `.gtbundle`, content digest, source kind,
     and rebuild/debug metadata.
   - Build from the post-setup workspace root, not from provider-specific config.
   - Run the warmup step for every start path, including local runtime. Warmup
     must not remain exclusive to `--upload-bundle`.

2. Call `prepare_bundle_for_start` once in `run_start_with_bundle_ref_and_tail`
   after bundle resolution/admin cert preparation and before target branching.
   - Update the runtime `StartRequest.bundle` to the prepared root.
   - Use prepared metadata for prints/debug output.

3. Change local runtime start to pass the warmed prepared root/artifact contract
   to `greentic-start`.
   - Preserve existing runtime flags and `--config` compatibility.
   - If `greentic-start` consumes a root today, ensure that root reflects the
     warmed prepared bundle content. If it needs an artifact or cache hint, add
     generic bundle-level plumbing rather than provider/app-specific config.

4. Change `ensure_started_or_deployed` and `ensure_bundle_deployed` to accept
   `&PreparedBundle`.
   - Use `prepared.digest` for deployment state comparison.
   - Use `prepared.artifact_path` for local-artifact deployer source.
   - Use `prepared.prepared_root` for deployer `--bundle-root`.

5. Replace or narrow `prepare_deployable_bundle_artifact`.
   - Do not let target-specific code rebuild or reuse `resolved.deploy_artifact`
     independently.
   - If kept temporarily, make it an implementation detail of
     `prepare_bundle_for_start`.

6. Rework `--upload-bundle`.
   - Upload `PreparedBundle.artifact_path`.
   - Stop rebuilding a second warmed artifact in `resolve_upload_bundle`, or
     move warmup/build into `prepare_bundle_for_start` so the uploaded artifact
     is the canonical prepared artifact.
   - The upload result may provide the remote source and digest, but the digest
     must match the uploaded prepared artifact.

7. Preserve `--deploy-bundle-source` compatibility carefully.
   - Treat it as an explicit advanced override for where an already prepared
     artifact is reachable.
   - Still pass `PreparedBundle.digest`.
   - Debug output must show both the prepared artifact and the overridden
     deployer source so mismatches are visible.
   - Do not silently prefer the original user input artifact when it would lose
     setup-mutated files.

8. Update deployment state.
   - Add `prepared_bundle_digest`.
   - Compare digest for redeploy decisions.
   - Keep old `bundle_fingerprint` only for migration/diagnostics.
   - Same-size file edits under included pack-owned config/assets must cause a
     new digest and redeploy.

9. Strengthen exclusion rules.
   - Reuse the bundle build tool's exclusion behavior where possible.
   - Add repo-local tests for `.dev.secrets.env`, `.greentic/cache`,
     `.greentic/dev`, logs, temp files, local credential/cache files, and
     machine-local state.
   - Do not print secret file contents in debug/doctor output.

10. Add debug/doctor output.
    - Input bundle ref/path.
    - Resolved bundle dir.
    - Prepared bundle root.
    - Prepared artifact path.
    - Prepared bundle digest.
    - Selected target.
    - Deployer bundle source and digest.
    - Whether the prepared bundle was rebuilt.
    - Optional count of included asset/config files.

11. Update docs.
    - `docs/02-cli/gtc-start.md` should state that `gtc start` always runs a
      prepared bundle, and `--target` changes only where that bundle runs.
    - Pack-owned setup config belongs inside bundle files/assets.
    - Deployer targets deploy the prepared bundle unchanged.

## Test Plan

Add focused unit tests around the prepared bundle module and existing start/deploy
orchestration tests. Prefer fake companion binaries rather than real cloud calls.

### Local and target-backed paths use the same prepared bundle

- Fixture bundle workspace includes
  `assets/webchat-gui/config/tenants/demo.json` as an opaque file.
- Invoke the local preparation path.
- Invoke AWS, GCP, and Azure target paths with fake deployers.
- Assert every path uses the same prepared digest and the prepared artifact/root
  includes the file.
- Assert deployer-backed paths receive the prepared artifact/source, not the
  original pre-setup source.

### Generic pack-owned config is preserved

- Fixture includes `assets/example-pack/config/runtime.json`.
- Assert local, AWS, GCP, and Azure paths preserve it without parsing
  or understanding the path.

### Config-only changes change digest

- Prepare once with `assets/example-pack/config/runtime.json`.
- Change file contents while keeping file size unchanged.
- Prepare again.
- Assert digest changes and deployment state would redeploy.

### Secret and local files are excluded

- Fixture includes `.dev.secrets.env`, `.greentic/dev/.dev.secrets.env`,
  `.greentic/cache/tmp`, logs, temp files, and local credential/cache files.
- Assert prepared artifact/root excludes them.
- Assert debug output contains paths/counts only, not secret values.

### No provider/app-specific hardcoding

- Tests may use `assets/webchat-gui/...` only as an opaque fixture path.
- Assert new code does not branch on config keys or app/provider names such as
  `webchat-gui`, `skin`, `nav_links`, `slack`, `teams`, or `webex`.

### Existing local behavior

- `gtc start <bundle>` still invokes `greentic-start`.
- The runtime receives the warmed prepared root/artifact contract.
- Local start benefits from the same warmup as uploaded and target-backed start.
- Existing `--config` handling remains compatible.

### Existing target behavior

- `--target aws`, `--target gcp`, and `--target azure` still use the existing
  deployer paths.
- Only bundle root/source/artifact/digest are corrected to come from
  `PreparedBundle`.

## Files To Change

- `src/bin/gtc/deploy.rs`
  - Wire the new prepared bundle module and struct exports.
- `src/bin/gtc/deploy/start_stop.rs`
  - Prepare once and pass the prepared bundle to local and target branches.
- `src/bin/gtc/deploy/cloud_deploy/deployment_state.rs`
  - Accept/use `PreparedBundle`, compare digest in state, remove independent
    artifact decisions from target paths.
- `src/bin/gtc/deploy/cloud_deploy.rs`
  - Upload prepared artifact; keep remote-source validation/registry args.
- `src/bin/gtc/deploy/bundle_upload_orchestrator.rs`
  - Stop owning a separate prepared artifact build, or make warmup part of the
    canonical preparation path.
- `src/bin/gtc/deploy/bundle_resolution.rs`
  - Keep resolution as input discovery; do not treat original archive as a
    deployable artifact after preparation.
- `src/perf_targets.rs`
  - Do not rely on path/size-only directory fingerprints for redeploy detection.
- `src/bin/gtc/tests.rs` and `tests/gtc_router_integration.rs`
  - Add fake-binary tests for local, cloud, and upload behavior.
- `docs/02-cli/gtc-start.md`
  - Document the warmed prepared bundle contract.

## Non-Goals

- No central runtime config schema.
- No provider-specific parsing of pack config.
- No app-specific branches for webchat/gui/skin/nav/Slack/Teams/Webex.
- No new target-specific environment variable channel for pack setup config.
- No broad redesign of setup, deployer, or pack formats.

## Review Notes

The current code is already close in shape: bundle resolution happens once and
deployer invocation already accepts root/source/digest. The missing piece is a
single warmed prepared bundle object used before the local-vs-target branch,
plus a content digest that becomes the deployment state source of truth.

This PR removes the old bespoke `single-vm` target path from `gtc`; future VM
deployment shapes should come through generic deployer-pack dispatch instead of
target-specific orchestration in this crate.
