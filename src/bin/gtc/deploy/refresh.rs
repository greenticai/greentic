// greentic/src/bin/gtc/deploy/refresh.rs
//! `gtc deploy refresh-bundle-url <bundle-ref>` implementation.
//!
//! Spawns `greentic-deployer bundle-upload refresh-url` to re-issue a presigned
//! URL, rewrites `dev.tfvars` with the new URL, and runs the deploy state's
//! `terraform-apply.sh` to roll the operator task definition.

use std::fs;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;

use directories::BaseDirs;

use crate::deploy::bundle_upload_orchestrator;
use gtc::error::{GtcError, GtcResult};

pub struct RefreshArgs {
    pub bundle_ref: String,
    pub cloud: Option<String>,
    pub environment: String,
    pub presign_expires: u64,
}

pub fn run_refresh(args: RefreshArgs) -> i32 {
    match run_refresh_inner(args) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("error: {err}");
            1
        }
    }
}

fn run_refresh_inner(args: RefreshArgs) -> GtcResult<()> {
    let deploy_state =
        resolve_deploy_state(&args.bundle_ref, args.cloud.as_deref(), &args.environment)?;
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

    let refreshed =
        bundle_upload_orchestrator::refresh_bundle_url(&object_ref, args.presign_expires)?;

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
    let status = ProcessCommand::new(&apply_script)
        .status()
        .map_err(|e| GtcError::message(format!("spawn terraform-apply.sh: {e}")))?;
    if !status.success() {
        return Err(GtcError::message(format!(
            "terraform-apply.sh exited with {:?}",
            status.code()
        )));
    }
    Ok(())
}

/// Resolve the deploy state directory under `~/.greentic/deploy/<cloud>/<env>/<bundle-fingerprint>/`.
fn resolve_deploy_state(
    bundle_ref: &str,
    cloud: Option<&str>,
    environment: &str,
) -> GtcResult<PathBuf> {
    let base =
        BaseDirs::new().ok_or_else(|| GtcError::message("home dir not found".to_string()))?;
    let deploy_root = base.home_dir().join(".greentic").join("deploy");
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
}
