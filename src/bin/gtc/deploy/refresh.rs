// greentic/src/bin/gtc/deploy/refresh.rs
//! `gtc deploy refresh-bundle-url <bundle-ref>` implementation.
//!
//! Spawns `greentic-deployer bundle-upload refresh-url` to re-issue a presigned
//! URL, rewrites `dev.tfvars` with the new URL, and runs the deploy state's
//! `terraform-apply.sh` to roll the operator task definition.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, ExitStatus};

use directories::BaseDirs;

use crate::deploy::bundle_upload_orchestrator::{self, UploadedBundle};
use gtc::error::{GtcError, GtcResult};

pub struct RefreshArgs {
    pub bundle_ref: String,
    pub cloud: Option<String>,
    pub environment: String,
    pub presign_expires: u64,
}

pub fn run_refresh(args: RefreshArgs) -> i32 {
    let base = match BaseDirs::new() {
        Some(b) => b,
        None => {
            eprintln!("error: home dir not found");
            return 1;
        }
    };
    let deploy_root = base.home_dir().join(".greentic").join("deploy");

    match run_refresh_with_root(
        &args,
        &deploy_root,
        &mut bundle_upload_orchestrator::refresh_bundle_url,
        &mut spawn_terraform_apply,
    ) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("error: {err}");
            1
        }
    }
}

fn spawn_terraform_apply(script: &Path) -> std::io::Result<ExitStatus> {
    ProcessCommand::new(script).status()
}

/// Orchestrate the refresh against a caller-provided deploy root, presign refresher,
/// and terraform-apply spawner. Splitting these out keeps the body unit-testable
/// without touching the user's home directory or spawning real subprocesses.
fn run_refresh_with_root(
    args: &RefreshArgs,
    deploy_root: &Path,
    refresh_url: &mut dyn FnMut(&str, u64) -> GtcResult<UploadedBundle>,
    spawn_apply: &mut dyn FnMut(&Path) -> std::io::Result<ExitStatus>,
) -> GtcResult<()> {
    let deploy_state = resolve_deploy_state_in(
        deploy_root,
        &args.bundle_ref,
        args.cloud.as_deref(),
        &args.environment,
    )?;
    let tfvars_path = deploy_state.join("terraform").join("dev.tfvars");
    let tfvars_text = fs::read_to_string(&tfvars_path)
        .map_err(|e| GtcError::message(format!("read tfvars {}: {e}", tfvars_path.display())))?;

    let bundle_source = extract_tfvars_value(&tfvars_text, "bundle_source").ok_or_else(|| {
        GtcError::message(format!(
            "bundle_source not found in {}",
            tfvars_path.display()
        ))
    })?;
    let object_ref = derive_object_ref_from_url(&bundle_source).ok_or_else(|| {
        GtcError::message(format!(
            "could not derive object_ref from bundle_source URL: {bundle_source}"
        ))
    })?;

    let refreshed = refresh_url(&object_ref, args.presign_expires)?;

    let updated = replace_tfvars_value(&tfvars_text, "bundle_source", &refreshed.url);
    fs::write(&tfvars_path, updated)
        .map_err(|e| GtcError::message(format!("write tfvars {}: {e}", tfvars_path.display())))?;

    eprintln!("Refreshed bundle URL:");
    eprintln!("  url:     {}", refreshed.url);
    if let Some(exp) = refreshed.expires_at.as_ref() {
        eprintln!("  expires: {exp}");
    }

    let apply_script = deploy_state.join("terraform-apply.sh");
    eprintln!(
        "Running {} (ECS task replacement; ~5 min)...",
        apply_script.display()
    );
    let status = spawn_apply(&apply_script)
        .map_err(|e| GtcError::message(format!("spawn terraform-apply.sh: {e}")))?;
    if !status.success() {
        return Err(GtcError::message(format!(
            "terraform-apply.sh exited with {:?}",
            status.code()
        )));
    }
    Ok(())
}

/// Resolve the deploy state directory under `<deploy_root>/<cloud>/<env>/<bundle-fingerprint>/`.
fn resolve_deploy_state_in(
    deploy_root: &Path,
    bundle_ref: &str,
    cloud: Option<&str>,
    environment: &str,
) -> GtcResult<PathBuf> {
    let fingerprint = mangle_bundle_ref_to_fingerprint(bundle_ref);

    let candidates: Vec<PathBuf> = if let Some(c) = cloud {
        vec![deploy_root.join(c).join(environment).join(&fingerprint)]
    } else {
        let mut out = Vec::new();
        for cloud_name in ["aws", "azure", "gcp"] {
            let path = deploy_root
                .join(cloud_name)
                .join(environment)
                .join(&fingerprint);
            if path.exists() {
                out.push(path);
            }
        }
        out
    };

    match candidates.len() {
        0 => Err(GtcError::message(format!(
            "no deploy state found for bundle {bundle_ref} (env={environment}); looked under {}",
            deploy_root.display()
        ))),
        1 => Ok(candidates.into_iter().next().unwrap()),
        n => {
            let list = candidates
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join("\n  ");
            Err(GtcError::message(format!(
                "{n} deploy states match bundle {bundle_ref}; pass --cloud to disambiguate:\n  {list}"
            )))
        }
    }
}

/// Mangle a bundle ref into the directory name the deployer uses under
/// `~/.greentic/deploy/<cloud>/<env>/`. The deployer replaces `/`, `-`, and
/// non-alphanumerics with `-` and prepends the path. We mirror that exactly.
fn mangle_bundle_ref_to_fingerprint(bundle_ref: &str) -> String {
    let cleaned = bundle_ref
        .replace(['/', '\\'], "-")
        .replace("..", "-")
        .replace(' ', "-");
    cleaned.trim_start_matches('-').to_string()
}

fn extract_tfvars_value(tfvars: &str, key: &str) -> Option<String> {
    for line in tfvars.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix(key) {
            let rest = rest.trim_start();
            if let Some(rest) = rest.strip_prefix('=') {
                let rest = rest.trim();
                if rest.starts_with('"') && rest.ends_with('"') && rest.len() >= 2 {
                    return Some(rest[1..rest.len() - 1].to_string());
                }
            }
        }
    }
    None
}

fn replace_tfvars_value(tfvars: &str, key: &str, new_value: &str) -> String {
    let mut out = String::with_capacity(tfvars.len());
    let mut replaced = false;
    for line in tfvars.lines() {
        let trimmed = line.trim_start();
        if !replaced && trimmed.starts_with(key) {
            out.push_str(&format!("{key} = \"{new_value}\""));
            out.push('\n');
            replaced = true;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Convert a presigned S3 URL `https://<bucket>.s3.<region>.amazonaws.com/<key>?...`
/// (or virtual-host-style with port) back into `s3://<bucket>/<key>`.
///
/// Supports both path-style and virtual-hosted-style S3 URLs; returns `None` for
/// non-S3 URLs without pulling in the `url` crate.
fn derive_object_ref_from_url(url: &str) -> Option<String> {
    // Strip scheme: we only handle https://
    let after_scheme = url.strip_prefix("https://")?;
    // Strip query string and fragment: split on '?'
    let without_query = after_scheme
        .split_once('?')
        .map(|(h, _)| h)
        .unwrap_or(after_scheme);
    // Split host from path on first '/'
    let (host, path) = without_query.split_once('/')?;
    let path = path.trim_start_matches('/');

    // Virtual-hosted-style: <bucket>.s3.amazonaws.com/<key>
    if let Some(bucket) = host.strip_suffix(".s3.amazonaws.com") {
        return Some(format!("s3://{bucket}/{path}"));
    }
    // Virtual-hosted-style with region: <bucket>.s3.<region>.amazonaws.com/<key>
    if let Some((bucket, suffix)) = host.split_once(".s3.")
        && suffix.ends_with(".amazonaws.com")
    {
        return Some(format!("s3://{bucket}/{path}"));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_existing_tfvars_value() {
        let text = r#"
cloud = "aws"
bundle_source = "https://example.com/old"
bundle_digest = "sha256:abc"
"#;
        assert_eq!(
            extract_tfvars_value(text, "bundle_source").as_deref(),
            Some("https://example.com/old")
        );
    }

    #[test]
    fn replace_tfvars_value_replaces_first_occurrence() {
        let text = r#"cloud = "aws"
bundle_source = "https://old/url"
bundle_digest = "sha256:abc"
"#;
        let updated = replace_tfvars_value(text, "bundle_source", "https://new/url");
        assert!(updated.contains(r#"bundle_source = "https://new/url""#));
        assert!(!updated.contains(r#""https://old/url""#));
    }

    #[test]
    fn derive_object_ref_from_virtual_host_url() {
        let url = "https://my-bucket.s3.eu-north-1.amazonaws.com/path/to/key.gtbundle?X-Amz-Signature=abc";
        assert_eq!(
            derive_object_ref_from_url(url).as_deref(),
            Some("s3://my-bucket/path/to/key.gtbundle")
        );
    }

    #[test]
    fn derive_object_ref_returns_none_for_non_s3() {
        assert!(derive_object_ref_from_url("https://example.com/x").is_none());
    }

    #[test]
    fn mangle_bundle_ref_replaces_slashes_and_dots() {
        let r = mangle_bundle_ref_to_fingerprint("/home/user/foo/bar.gtbundle");
        assert!(!r.contains('/'));
    }

    #[test]
    fn extract_tfvars_value_returns_none_for_missing_key() {
        let text = "cloud = \"aws\"\n";
        assert!(extract_tfvars_value(text, "bundle_source").is_none());
    }

    #[test]
    fn extract_tfvars_value_returns_none_for_unquoted_value() {
        let text = "bundle_source = oops\n";
        assert!(extract_tfvars_value(text, "bundle_source").is_none());
    }

    #[test]
    fn extract_tfvars_value_returns_none_when_no_equals() {
        // The value lookup hinges on `<key> = "<value>"`; without `=` it
        // must not return a stray match. Regression guard for prefix-only matches.
        let text = "bundle_source\n";
        assert!(extract_tfvars_value(text, "bundle_source").is_none());
    }

    #[test]
    fn replace_tfvars_value_appends_when_key_absent() {
        let text = "cloud = \"aws\"\n";
        let updated = replace_tfvars_value(text, "bundle_source", "https://new");
        // Key is missing — function leaves the input unchanged save for a
        // trailing newline, since it only replaces the first match.
        assert_eq!(updated, "cloud = \"aws\"\n");
    }

    #[test]
    fn derive_object_ref_from_global_endpoint_url() {
        // Virtual-hosted-style without an explicit region: `<bucket>.s3.amazonaws.com`.
        let url = "https://my-bucket.s3.amazonaws.com/path/to/key.gtbundle";
        assert_eq!(
            derive_object_ref_from_url(url).as_deref(),
            Some("s3://my-bucket/path/to/key.gtbundle")
        );
    }

    #[test]
    fn derive_object_ref_returns_none_for_non_https_scheme() {
        assert!(derive_object_ref_from_url("http://my-bucket.s3.amazonaws.com/x").is_none());
    }

    #[test]
    fn mangle_bundle_ref_strips_leading_separators() {
        let r = mangle_bundle_ref_to_fingerprint("/foo");
        assert!(!r.starts_with('-'));
        assert!(!r.starts_with('/'));
    }

    #[test]
    fn mangle_bundle_ref_normalises_parent_traversal() {
        let r = mangle_bundle_ref_to_fingerprint("a/../b");
        assert!(!r.contains("/"));
        assert!(!r.contains(".."));
    }

    fn make_state(
        deploy_root: &Path,
        cloud: &str,
        env: &str,
        bundle_ref: &str,
    ) -> std::path::PathBuf {
        let fp = mangle_bundle_ref_to_fingerprint(bundle_ref);
        let dir = deploy_root.join(cloud).join(env).join(fp);
        std::fs::create_dir_all(&dir).expect("create state dir");
        dir
    }

    #[test]
    fn resolve_deploy_state_in_returns_single_match_without_cloud() {
        let tmp = tempfile::tempdir().unwrap();
        let target = make_state(tmp.path(), "aws", "dev", "/home/u/demo.gtbundle");
        let resolved = resolve_deploy_state_in(tmp.path(), "/home/u/demo.gtbundle", None, "dev")
            .expect("expected single match");
        assert_eq!(resolved, target);
    }

    #[test]
    fn resolve_deploy_state_in_honours_explicit_cloud() {
        let tmp = tempfile::tempdir().unwrap();
        let aws = make_state(tmp.path(), "aws", "dev", "/home/u/demo.gtbundle");
        let _gcp = make_state(tmp.path(), "gcp", "dev", "/home/u/demo.gtbundle");
        let resolved =
            resolve_deploy_state_in(tmp.path(), "/home/u/demo.gtbundle", Some("aws"), "dev")
                .expect("expected aws match");
        assert_eq!(resolved, aws);
    }

    #[test]
    fn resolve_deploy_state_in_errors_when_no_match() {
        let tmp = tempfile::tempdir().unwrap();
        let err = resolve_deploy_state_in(tmp.path(), "/home/u/missing.gtbundle", None, "dev")
            .expect_err("expected no-match error");
        let msg = format!("{err:?}");
        assert!(msg.contains("no deploy state found"), "{msg}");
    }

    #[test]
    fn resolve_deploy_state_in_errors_when_ambiguous() {
        let tmp = tempfile::tempdir().unwrap();
        let _aws = make_state(tmp.path(), "aws", "dev", "/home/u/demo.gtbundle");
        let _azure = make_state(tmp.path(), "azure", "dev", "/home/u/demo.gtbundle");
        let err = resolve_deploy_state_in(tmp.path(), "/home/u/demo.gtbundle", None, "dev")
            .expect_err("expected ambiguous error");
        let msg = format!("{err:?}");
        assert!(msg.contains("deploy states match"), "{msg}");
        assert!(msg.contains("--cloud to disambiguate"), "{msg}");
    }

    fn fake_status(success: bool) -> ExitStatus {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;

            ExitStatus::from_raw(if success { 0 } else { 1 << 8 })
        }
        #[cfg(not(unix))]
        {
            ProcessCommand::new("cmd")
                .args(["/c", if success { "exit 0" } else { "exit 1" }])
                .status()
                .expect("spawn noop command")
        }
    }

    fn write_deploy_state_with_tfvars(deploy_root: &Path, bundle_ref: &str, bundle_source: &str) {
        let state = make_state(deploy_root, "aws", "dev", bundle_ref);
        let tf_dir = state.join("terraform");
        std::fs::create_dir_all(&tf_dir).unwrap();
        let body = format!(
            "cloud = \"aws\"\nbundle_source = \"{bundle_source}\"\nbundle_digest = \"sha256:abc\"\n"
        );
        std::fs::write(tf_dir.join("dev.tfvars"), body).unwrap();
    }

    #[test]
    fn run_refresh_with_root_rewrites_tfvars_and_runs_apply() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle_ref = "/home/u/demo.gtbundle";
        let initial_url = "https://my-bucket.s3.eu-north-1.amazonaws.com/key?old-sig";
        write_deploy_state_with_tfvars(tmp.path(), bundle_ref, initial_url);

        let mut refresh_calls: Vec<(String, u64)> = Vec::new();
        let mut apply_calls: Vec<std::path::PathBuf> = Vec::new();
        let mut refresh_url = |obj: &str, exp: u64| -> GtcResult<UploadedBundle> {
            refresh_calls.push((obj.to_string(), exp));
            Ok(UploadedBundle {
                url: "https://my-bucket.s3.eu-north-1.amazonaws.com/key?new-sig".to_string(),
                digest: "sha256:abc".to_string(),
                expires_at: Some("2026-12-31T00:00:00Z".to_string()),
                object_ref: "s3://my-bucket/key".to_string(),
            })
        };
        let mut spawn_apply = |script: &Path| -> std::io::Result<ExitStatus> {
            apply_calls.push(script.to_path_buf());
            Ok(fake_status(true))
        };

        let args = RefreshArgs {
            bundle_ref: bundle_ref.to_string(),
            cloud: Some("aws".to_string()),
            environment: "dev".to_string(),
            presign_expires: 900,
        };
        run_refresh_with_root(&args, tmp.path(), &mut refresh_url, &mut spawn_apply)
            .expect("refresh should succeed");

        assert_eq!(refresh_calls.len(), 1);
        assert_eq!(refresh_calls[0].0, "s3://my-bucket/key");
        assert_eq!(refresh_calls[0].1, 900);

        assert_eq!(apply_calls.len(), 1);
        assert!(
            apply_calls[0].ends_with("terraform-apply.sh"),
            "{:?}",
            apply_calls[0]
        );

        let fp = mangle_bundle_ref_to_fingerprint(bundle_ref);
        let written = std::fs::read_to_string(
            tmp.path()
                .join("aws/dev")
                .join(fp)
                .join("terraform/dev.tfvars"),
        )
        .unwrap();
        assert!(
            written.contains(
                r#"bundle_source = "https://my-bucket.s3.eu-north-1.amazonaws.com/key?new-sig""#
            ),
            "tfvars not rewritten: {written}"
        );
    }

    #[test]
    fn run_refresh_with_root_propagates_apply_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle_ref = "/home/u/demo.gtbundle";
        write_deploy_state_with_tfvars(
            tmp.path(),
            bundle_ref,
            "https://my-bucket.s3.eu-north-1.amazonaws.com/key?old-sig",
        );

        let mut refresh_url = |_obj: &str, _exp: u64| -> GtcResult<UploadedBundle> {
            Ok(UploadedBundle {
                url: "https://my-bucket.s3.eu-north-1.amazonaws.com/key?new-sig".to_string(),
                digest: "sha256:abc".to_string(),
                expires_at: None,
                object_ref: "s3://my-bucket/key".to_string(),
            })
        };
        let mut spawn_apply =
            |_script: &Path| -> std::io::Result<ExitStatus> { Ok(fake_status(false)) };

        let args = RefreshArgs {
            bundle_ref: bundle_ref.to_string(),
            cloud: Some("aws".to_string()),
            environment: "dev".to_string(),
            presign_expires: 60,
        };
        let err = run_refresh_with_root(&args, tmp.path(), &mut refresh_url, &mut spawn_apply)
            .expect_err("expected apply failure");
        let msg = format!("{err:?}");
        assert!(msg.contains("terraform-apply.sh exited"), "{msg}");
    }

    #[test]
    fn run_refresh_with_root_errors_when_tfvars_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle_ref = "/home/u/demo.gtbundle";
        // Create the deploy state directory but no tfvars file.
        let _ = make_state(tmp.path(), "aws", "dev", bundle_ref);

        let mut refresh_url = |_: &str, _: u64| -> GtcResult<UploadedBundle> {
            unreachable!("refresh should not be called when tfvars is missing");
        };
        let mut spawn_apply = |_: &Path| -> std::io::Result<ExitStatus> {
            unreachable!("apply should not be called when tfvars is missing");
        };

        let args = RefreshArgs {
            bundle_ref: bundle_ref.to_string(),
            cloud: Some("aws".to_string()),
            environment: "dev".to_string(),
            presign_expires: 60,
        };
        let err = run_refresh_with_root(&args, tmp.path(), &mut refresh_url, &mut spawn_apply)
            .expect_err("expected read error");
        let msg = format!("{err:?}");
        assert!(msg.contains("read tfvars"), "{msg}");
    }

    #[test]
    fn run_refresh_with_root_errors_when_bundle_source_unparseable() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle_ref = "/home/u/demo.gtbundle";
        write_deploy_state_with_tfvars(tmp.path(), bundle_ref, "https://example.com/not-s3");

        let mut refresh_url = |_: &str, _: u64| -> GtcResult<UploadedBundle> {
            unreachable!("refresh should not be called when URL parse fails");
        };
        let mut spawn_apply = |_: &Path| -> std::io::Result<ExitStatus> {
            unreachable!("apply should not be called when URL parse fails");
        };

        let args = RefreshArgs {
            bundle_ref: bundle_ref.to_string(),
            cloud: Some("aws".to_string()),
            environment: "dev".to_string(),
            presign_expires: 60,
        };
        let err = run_refresh_with_root(&args, tmp.path(), &mut refresh_url, &mut spawn_apply)
            .expect_err("expected url parse error");
        let msg = format!("{err:?}");
        assert!(msg.contains("could not derive object_ref"), "{msg}");
    }
}
