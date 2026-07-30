//! `gtc start <BUNDLE> --target gcp`: deploy through the **env-packs** Cloud Run
//! deployer instead of the legacy Terraform layer.
//!
//! `--target local` already deploys into an environment whose deployer slot is
//! `greentic.deployer.local-process`. This module makes `--target gcp` the same
//! shape: synthesize a `greentic.env-manifest.v1` document that binds the
//! deployer slot to `greentic.deployer.gcp-cloudrun`, then hand it to
//! `op env up` — the sibling of the `gtc start k8s` / `gtc start cloudrun`
//! passthroughs, except the manifest is built for you instead of demanded from
//! you.
//!
//! No Terraform, no provider pack, no deployment-target metadata. The legacy
//! path (`cloud_deploy::ensure_started_or_deployed`) still owns `--target aws`
//! and `--target azure`; there is no azure env-pack to route to.
//!
//! # Why the bundle must already be an `oci://` artifact
//!
//! Cloud Run pulls the bundle over the network *at container boot*, so a path
//! on the caller's disk is meaningless to it. The env-apply engine enforces
//! this narrowly: `bundle_source_uri` accepts `oci://` and nothing else. The
//! `--upload-bundle` machinery (`s3://`, `gs://`) belongs to the Terraform
//! path — and `gs://` is not implemented at all — so it cannot serve this one.
//! Rather than pretend, this path requires the ref and says exactly how to make
//! one.

use std::path::Path;
use std::process::Command;

use gtc::error::{GtcError, GtcResult};
use serde_json::{Value, json};

use super::StartCliOptions;
use crate::OP_BIN;
use crate::i18n_support::t_or;
use crate::process::run_binary_checked;

/// Env-pack descriptor for the Cloud Run deployer, pinned to the major the
/// handler implements. The registry rejects a version it does not support
/// rather than silently certifying it, so this is a real contract.
const CLOUDRUN_DEPLOYER_KIND: &str = "greentic.deployer.gcp-cloudrun@1.0.0";

/// The only `bundle_source_uri` scheme the env-apply engine will fetch.
const OCI_SCHEME: &str = "oci://";

/// Bundle-manifest filename inside a built `.gtbundle` (the build artifact).
const BUNDLE_MANIFEST_JSON: &str = "bundle-manifest.json";

/// Workspace marker carrying `bundle_id` when no build manifest is present.
const BUNDLE_WORKSPACE_MARKER: &str = "bundle.yaml";

/// Deploy `bundle_ref` to Cloud Run via `op env up`.
///
/// Every precondition is resolved and reported *before* the operator is
/// spawned, so a missing input never lands halfway through a cloud mutation.
pub(super) fn start_via_cloudrun_env_pack(
    bundle_ref: &str,
    bundle_dir: &Path,
    env_id: &str,
    cli_options: &StartCliOptions,
    debug: bool,
    locale: &str,
) -> GtcResult<()> {
    let bundle_source_uri = resolve_bundle_source_uri(bundle_ref, cli_options, locale)?;
    let project = resolve_gcp_project(locale)?;
    let region = resolve_gcp_region(locale)?;
    let bundle_id = read_bundle_id(bundle_dir)?;

    let manifest =
        build_cloudrun_env_manifest(env_id, &bundle_id, &bundle_source_uri, &project, &region);

    println!("Deployment mode: Cloud Run (env-packs, no terraform)");
    println!("  project:      {project}");
    println!("  region:       {region}");
    println!("  bundle:       {bundle_id} <- {bundle_source_uri}");
    // `public` grants `allUsers` the invoker role: the service answers the
    // internet with no authentication. That is what a demo wants and what the
    // local path already does, but it is not something to do silently to
    // someone's cloud project.
    println!("  access:       public — reachable without authentication");

    // A NamedTempFile deletes on drop, so it must outlive the child process.
    let manifest_file = tempfile::NamedTempFile::new()
        .map_err(|e| GtcError::message(format!("create temporary env-manifest file: {e}")))?;
    std::fs::write(
        manifest_file.path(),
        serde_json::to_string_pretty(&manifest)
            .map_err(|e| GtcError::message(format!("serialize env-manifest: {e}")))?,
    )
    .map_err(|e| GtcError::message(format!("write temporary env-manifest file: {e}")))?;

    let args = env_up_args(manifest_file.path());
    run_binary_checked(OP_BIN, &args, debug, locale, "env up (cloud-run)")
}

/// Tear down a Cloud Run environment created by
/// [`start_via_cloudrun_env_pack`].
///
/// The legacy `--destroy` path renders a Terraform destroy and hard-requires a
/// provider pack plus state that an env-packs deploy never produced, so it
/// cannot tear this down: it fails and leaves a **billing** Cloud Run service
/// with no CLI way to name it. `op env destroy` is the matching teardown — it
/// deletes the cloud resources first and only then removes local state, so a
/// failed teardown leaves the environment intact to retry instead of orphaning
/// resources.
///
/// This destroys the whole environment, not one bundle: Cloud Run has no
/// per-bundle teardown verb. The environment id is printed so that is never a
/// surprise.
pub(super) fn destroy_cloudrun_env_pack(env_id: &str, debug: bool, locale: &str) -> GtcResult<()> {
    println!("Destroying Cloud Run environment `{env_id}` (env-packs)");
    println!("  this removes every bundle deployed into `{env_id}`, not just one");
    let args = env_destroy_args(env_id);
    run_binary_checked(OP_BIN, &args, debug, locale, "env destroy (cloud-run)")
}

/// Build the `op env destroy` argv.
///
/// Split out for the same reason as [`env_up_args`]. `--force-local` is
/// deliberately absent: it purges local state while *skipping* cloud teardown,
/// which is precisely the orphaned-billing-resource outcome this path exists to
/// prevent. Without it, a teardown this build cannot perform refuses instead.
fn env_destroy_args(env_id: &str) -> Vec<String> {
    vec![
        "op".to_string(),
        "env".to_string(),
        "destroy".to_string(),
        env_id.to_string(),
        "--confirm".to_string(),
    ]
}

/// Build the `op env up` argv.
///
/// Split out so the test exercises the REAL construction instead of
/// re-deriving it: `--answers` is a **global** flag on `op`, so it must precede
/// the `env up` noun/verb. A test that rebuilds the expected argv itself would
/// pass even if production emitted `env up --answers …`, which clap accepts
/// only because the flag is declared `global = true` — a fragile thing to rely
/// on by accident.
fn env_up_args(manifest_path: &Path) -> Vec<String> {
    vec![
        "op".to_string(),
        "--answers".to_string(),
        manifest_path.display().to_string(),
        "env".to_string(),
        "up".to_string(),
        // Skip the plan confirmation. `--non-interactive` is deliberately NOT
        // passed: it also refuses to prompt for a genuinely missing secret,
        // turning a one-line prompt into a hard failure.
        "--yes".to_string(),
    ]
}

/// Synthesize the single-bundle Cloud Run env-manifest.
///
/// Binds **only** the deployer slot. Secrets / Telemetry / Sessions / State are
/// already bound when the environment bootstraps
/// (`greentic-deployer`'s `LOCAL_DEFAULT_BINDINGS`), and re-declaring one is
/// worse than redundant: the built-in handlers advertise `^0.1.0`, so an
/// explicit `greentic.secrets.dev-store@1.0.0` is reported as version skew by
/// `op env doctor` — found by running this against a real store, not by reading.
///
/// Deliberately omits `bundle_digest`. The engine hashes whatever it resolves
/// and only fails when a *declared* digest disagrees, so pinning the digest of
/// a locally-built artifact against a `bundle_source_uri` that CI published
/// would fail every time the two builds are not byte-identical — which is the
/// normal case.
fn build_cloudrun_env_manifest(
    env_id: &str,
    bundle_id: &str,
    bundle_source_uri: &str,
    project: &str,
    region: &str,
) -> Value {
    json!({
        "schema": "greentic.env-manifest.v1",
        "environment": { "id": env_id },
        "trust_root": "bootstrap",
        "packs": [
            {
                "slot": "deployer",
                "kind": CLOUDRUN_DEPLOYER_KIND,
                "pack_ref": "builtin",
                "answers": {
                    "project": project,
                    "region": region,
                    "access_mode": "public",
                },
            },
        ],
        "bundles": [
            {
                "bundle_id": bundle_id,
                "bundle_source_uri": bundle_source_uri,
                "route_binding": { "path_prefixes": ["/"] },
            },
        ],
    })
}

/// Resolve the remote artifact Cloud Run will pull.
///
/// Prefers the bundle ref itself when it is already an `oci://` reference —
/// then `gtc start oci://… --target gcp` needs no extra flag and deploys
/// exactly the artifact it resolved. Otherwise honours an explicit
/// `--deploy-bundle-source` / `GREENTIC_DEPLOY_BUNDLE_SOURCE`, which is the
/// escape hatch for "built locally, published elsewhere".
fn resolve_bundle_source_uri(
    bundle_ref: &str,
    cli_options: &StartCliOptions,
    locale: &str,
) -> GtcResult<String> {
    if let Some(uri) = oci_ref(bundle_ref) {
        return Ok(uri);
    }
    let explicit = cli_options
        .deploy_bundle_source
        .clone()
        .or_else(|| std::env::var("GREENTIC_DEPLOY_BUNDLE_SOURCE").ok())
        .filter(|value| !value.trim().is_empty());
    match explicit {
        Some(value) => oci_ref(&value).ok_or_else(|| {
            GtcError::message(format!(
                "{}: `{}` is not an {OCI_SCHEME} reference. Cloud Run can only pull \
                 {OCI_SCHEME} bundles.",
                t_or(
                    locale,
                    "gtc.start.err.gcp_bundle_source_scheme",
                    "unusable bundle source for --target gcp"
                ),
                value.trim(),
            ))
        }),
        None => Err(GtcError::message(format!(
            "{}\n\n\
             Cloud Run pulls the bundle over the network at boot, so `{bundle_ref}` \
             is unreachable from it.\n\
             Publish the bundle, then deploy the published ref:\n\n\
            \x20   oras push ghcr.io/YOU/YOUR-BUNDLE:v1 ./YOUR-BUNDLE.gtbundle\n\
            \x20   gtc start oci://ghcr.io/YOU/YOUR-BUNDLE:v1 --target gcp\n\n\
             Or keep the local ref and name the published copy explicitly:\n\n\
            \x20   gtc start {bundle_ref} --target gcp \\\n\
            \x20     --deploy-bundle-source oci://ghcr.io/YOU/YOUR-BUNDLE:v1",
            t_or(
                locale,
                "gtc.start.err.gcp_needs_remote_bundle",
                "--target gcp requires a remote bundle source"
            ),
        ))),
    }
}

/// Normalize a value to an `oci://` reference, or `None` when it is not one.
fn oci_ref(value: &str) -> Option<String> {
    let trimmed = value.trim();
    let rest = trimmed.strip_prefix(OCI_SCHEME)?;
    if rest.trim().is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

/// Resolve the GCP project: environment first, then the ambient `gcloud`
/// config the deployer's ADC chain already assumes.
fn resolve_gcp_project(locale: &str) -> GtcResult<String> {
    const VARS: [&str; 2] = ["GOOGLE_CLOUD_PROJECT", "CLOUDSDK_CORE_PROJECT"];
    if let Some(value) = first_env_value(&VARS) {
        return Ok(value);
    }
    if let Some(value) = gcloud_config_value("project") {
        return Ok(value);
    }
    Err(GtcError::message(format!(
        "{}\n\n\
        \x20   gcloud config set project YOUR-PROJECT\n\n\
         or set one of: {}",
        t_or(
            locale,
            "gtc.start.err.gcp_project_unresolved",
            "cannot determine the GCP project for --target gcp"
        ),
        VARS.join(", "),
    )))
}

/// Resolve the Cloud Run region.
///
/// Deliberately has **no default**. A region determines latency, egress cost
/// and data residency; guessing one on the caller's behalf is the kind of
/// silently-surprising default this porcelain must not have.
fn resolve_gcp_region(locale: &str) -> GtcResult<String> {
    const VARS: [&str; 3] = [
        "GOOGLE_CLOUD_REGION",
        "CLOUDSDK_RUN_REGION",
        "CLOUDSDK_COMPUTE_REGION",
    ];
    if let Some(value) = first_env_value(&VARS) {
        return Ok(value);
    }
    for key in ["run/region", "compute/region"] {
        if let Some(value) = gcloud_config_value(key) {
            return Ok(value);
        }
    }
    Err(GtcError::message(format!(
        "{}\n\n\
        \x20   gcloud config set run/region europe-west1\n\n\
         or set one of: {}",
        t_or(
            locale,
            "gtc.start.err.gcp_region_unresolved",
            "cannot determine the Cloud Run region for --target gcp"
        ),
        VARS.join(", "),
    )))
}

/// First non-empty value among `keys`.
fn first_env_value(keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        std::env::var(key)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

/// Read a `gcloud config` value, treating gcloud's `(unset)` sentinel and any
/// failure to run it as "not configured".
fn gcloud_config_value(key: &str) -> Option<String> {
    let output = Command::new("gcloud")
        .args(["config", "get-value", key])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() || value == "(unset)" {
        return None;
    }
    Some(value)
}

/// Read the bundle's declared `bundle_id` from its metadata.
///
/// The declared id — never the filename stem. Two different `.gtbundle` files
/// can declare the same `bundle_id`, and it is the declared value that decides
/// whether a deploy converges on an existing deployment or creates a second
/// one. `greentic-setup`'s `env-deploy` reads it the same way for the local
/// path; consolidating the two readers means moving this whole synthesis
/// behind that binary, which is a larger change than routing one target.
fn read_bundle_id(dir: &Path) -> GtcResult<String> {
    let manifest_path = dir.join(BUNDLE_MANIFEST_JSON);
    if manifest_path.is_file() {
        let raw = std::fs::read_to_string(&manifest_path)
            .map_err(|e| GtcError::message(format!("read {}: {e}", manifest_path.display())))?;
        let doc: Value = serde_json::from_str(&raw)
            .map_err(|e| GtcError::message(format!("parse {}: {e}", manifest_path.display())))?;
        if let Some(id) = doc.get("bundle_id").and_then(Value::as_str)
            && !id.trim().is_empty()
        {
            return Ok(id.trim().to_string());
        }
    }

    let yaml_path = dir.join(BUNDLE_WORKSPACE_MARKER);
    if yaml_path.is_file() {
        let raw = std::fs::read_to_string(&yaml_path)
            .map_err(|e| GtcError::message(format!("read {}: {e}", yaml_path.display())))?;
        let doc: serde_yaml_bw::Value = serde_yaml_bw::from_str(&raw)
            .map_err(|e| GtcError::message(format!("parse {}: {e}", yaml_path.display())))?;
        if let Some(serde_yaml_bw::Value::String(id, _)) = doc
            .as_mapping()
            .and_then(|m| m.get(serde_yaml_bw::Value::String("bundle_id".into(), None)))
            && !id.trim().is_empty()
        {
            return Ok(id.trim().to_string());
        }
    }

    Err(GtcError::message(format!(
        "cannot determine bundle_id: neither {BUNDLE_MANIFEST_JSON} nor \
         {BUNDLE_WORKSPACE_MARKER} in {} declares one",
        dir.display(),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn cli_options(deploy_bundle_source: Option<&str>) -> StartCliOptions {
        StartCliOptions {
            bundle_ref: None,
            start_args: Vec::new(),
            explicit_target: None,
            environment: None,
            provider_pack: None,
            app_pack: None,
            deploy_bundle_source: deploy_bundle_source.map(str::to_string),
            upload_bundle: None,
            upload_bundle_presign_expires: None,
        }
    }

    #[test]
    fn env_up_args_put_the_global_answers_flag_before_the_noun() {
        let args = env_up_args(Path::new("/tmp/env.json"));
        assert_eq!(
            args,
            vec![
                "op".to_string(),
                "--answers".to_string(),
                "/tmp/env.json".to_string(),
                "env".to_string(),
                "up".to_string(),
                "--yes".to_string(),
            ]
        );
        let answers = args
            .iter()
            .position(|a| a == "--answers")
            .expect("--answers");
        let noun = args.iter().position(|a| a == "env").expect("env");
        assert!(
            answers < noun,
            "--answers is a global flag on `op`; it must precede the noun"
        );
    }

    /// `--force-local` skips cloud teardown and orphans the Cloud Run service.
    /// It must never be what gtc reaches for on the user's behalf.
    #[test]
    fn env_destroy_args_never_skip_the_cloud_teardown() {
        let args = env_destroy_args("local");
        assert_eq!(
            args,
            vec![
                "op".to_string(),
                "env".to_string(),
                "destroy".to_string(),
                "local".to_string(),
                "--confirm".to_string(),
            ]
        );
        assert!(
            !args.iter().any(|a| a == "--force-local"),
            "--force-local would leave a billing Cloud Run service behind"
        );
    }

    #[test]
    fn manifest_binds_the_cloudrun_deployer_and_a_remote_bundle() {
        let manifest = build_cloudrun_env_manifest(
            "local",
            "quickstart-bundle",
            "oci://ghcr.io/acme/quickstart:v1",
            "acme-proj",
            "europe-west1",
        );
        assert_eq!(manifest["schema"], "greentic.env-manifest.v1");
        assert_eq!(manifest["environment"]["id"], "local");
        assert_eq!(manifest["trust_root"], "bootstrap");

        let deployer = &manifest["packs"][0];
        assert_eq!(deployer["slot"], "deployer");
        assert_eq!(deployer["kind"], CLOUDRUN_DEPLOYER_KIND);
        assert_eq!(deployer["answers"]["project"], "acme-proj");
        assert_eq!(deployer["answers"]["region"], "europe-west1");
        assert_eq!(deployer["answers"]["access_mode"], "public");

        let bundle = &manifest["bundles"][0];
        assert_eq!(bundle["bundle_id"], "quickstart-bundle");
        assert_eq!(
            bundle["bundle_source_uri"],
            "oci://ghcr.io/acme/quickstart:v1"
        );
        assert_eq!(bundle["route_binding"]["path_prefixes"][0], "/");
    }

    /// The env binds Secrets / Telemetry / Sessions / State itself when it
    /// bootstraps, and its built-in handlers advertise `^0.1.0`. Re-declaring
    /// one here reintroduces the `op env doctor` version skew this manifest
    /// used to carry (`dev-store@1.0.0` requested, `^0.1.0` supported).
    #[test]
    fn manifest_binds_only_the_deployer_slot() {
        let manifest =
            build_cloudrun_env_manifest("local", "demo", "oci://ghcr.io/acme/demo:v1", "p", "r");
        let packs = manifest["packs"].as_array().expect("packs is an array");
        assert_eq!(
            packs.len(),
            1,
            "only the deployer slot may be declared; the rest are env defaults: {packs:?}"
        );
        assert_eq!(packs[0]["slot"], "deployer");
    }

    /// A local `bundle_path` would make Cloud Run boot with `bundles_active: 0`
    /// next to a green `/readyz` — the exact silent failure this path exists to
    /// avoid.
    #[test]
    fn manifest_never_carries_a_local_bundle_path() {
        let manifest =
            build_cloudrun_env_manifest("local", "demo", "oci://ghcr.io/acme/demo:v1", "p", "r");
        assert!(manifest["bundles"][0].get("bundle_path").is_none());
    }

    /// The engine hashes what it resolves and fails closed on a mismatch, so a
    /// digest pinned from a local build would reject a CI-published artifact.
    #[test]
    fn manifest_omits_bundle_digest_so_the_engine_records_the_resolved_one() {
        let manifest =
            build_cloudrun_env_manifest("local", "demo", "oci://ghcr.io/acme/demo:v1", "p", "r");
        assert!(manifest["bundles"][0].get("bundle_digest").is_none());
    }

    #[test]
    fn an_oci_bundle_ref_is_its_own_source_uri() {
        let uri = resolve_bundle_source_uri("oci://ghcr.io/acme/demo:v1", &cli_options(None), "en")
            .expect("oci ref is usable as-is");
        assert_eq!(uri, "oci://ghcr.io/acme/demo:v1");
    }

    #[test]
    fn a_local_ref_falls_back_to_the_explicit_deploy_bundle_source() {
        let uri = resolve_bundle_source_uri(
            "./demo.gtbundle",
            &cli_options(Some("oci://ghcr.io/acme/demo:v1")),
            "en",
        )
        .expect("explicit source is honoured");
        assert_eq!(uri, "oci://ghcr.io/acme/demo:v1");
    }

    #[test]
    fn a_local_ref_with_no_remote_source_names_the_precondition() {
        let err = resolve_bundle_source_uri("./demo.gtbundle", &cli_options(None), "en")
            .expect_err("a local path is unreachable from Cloud Run");
        let message = err.to_string();
        assert!(
            message.contains("oras push"),
            "must show how to publish: {message}"
        );
        assert!(
            message.contains("--deploy-bundle-source"),
            "must show the escape hatch: {message}"
        );
    }

    /// `gs://` is the trap: it looks like a cloud upload target, the
    /// `--upload-bundle` flag advertises it, and the uploader is an
    /// unimplemented stub behind a non-default feature.
    #[test]
    fn a_non_oci_remote_source_is_rejected() {
        let err = resolve_bundle_source_uri(
            "./demo.gtbundle",
            &cli_options(Some("gs://acme-bundles/demo.gtbundle")),
            "en",
        )
        .expect_err("only oci:// is fetchable by the env-apply engine");
        assert!(err.to_string().contains("oci://"));
    }

    #[test]
    fn an_empty_oci_ref_is_not_a_reference() {
        assert!(oci_ref("oci://").is_none());
        assert!(oci_ref("oci://   ").is_none());
        assert!(oci_ref("ghcr.io/acme/demo:v1").is_none());
        assert_eq!(
            oci_ref("  oci://ghcr.io/acme/demo:v1  ").as_deref(),
            Some("oci://ghcr.io/acme/demo:v1")
        );
    }

    #[test]
    fn bundle_id_comes_from_the_build_manifest_first() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(
            dir.path().join(BUNDLE_MANIFEST_JSON),
            r#"{"bundle_id":"declared-id"}"#,
        )
        .expect("write manifest");
        fs::write(
            dir.path().join(BUNDLE_WORKSPACE_MARKER),
            "bundle_id: marker-id\n",
        )
        .expect("write marker");
        assert_eq!(read_bundle_id(dir.path()).expect("id"), "declared-id");
    }

    #[test]
    fn bundle_id_falls_back_to_the_workspace_marker() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(
            dir.path().join(BUNDLE_WORKSPACE_MARKER),
            "bundle_id: marker-id\n",
        )
        .expect("write marker");
        assert_eq!(read_bundle_id(dir.path()).expect("id"), "marker-id");
    }

    #[test]
    fn bundle_id_is_an_error_when_undeclared() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join(BUNDLE_MANIFEST_JSON), "{}").expect("write manifest");
        let err = read_bundle_id(dir.path()).expect_err("no bundle_id anywhere");
        assert!(err.to_string().contains("bundle_id"));
    }
}
