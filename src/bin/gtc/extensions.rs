use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use clap::ArgMatches;
use directories::BaseDirs;
use serde::{Deserialize, Serialize};

use crate::process::passthrough_in_dir;
use crate::router::build_wizard_args;
use crate::{SETUP_BIN, deploy::run_start_with_bundle_ref_and_tail};

#[derive(Debug, Deserialize)]
struct ExtensionRegistry {
    schema_version: String,
    extensions: Vec<ExtensionRegistryEntry>,
}

#[derive(Debug, Deserialize)]
struct ExtensionRegistryEntry {
    id: String,
    descriptor: String,
}

#[derive(Debug, Deserialize)]
struct ExtensionDescriptor {
    schema_version: String,
    extension_id: String,
    family: String,
    #[allow(dead_code)]
    summary: Option<String>,
    wizard: ExtensionWizardDescriptor,
}

#[derive(Debug, Deserialize)]
struct ExtensionWizardDescriptor {
    binary: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    working_directory: Option<String>,
}

#[derive(Debug, Serialize)]
struct MultiExtensionLauncherHandoff {
    schema_id: &'static str,
    schema_version: &'static str,
    mode: &'static str,
    registry_path: String,
    extensions: Vec<MultiExtensionLaunchRecord>,
}

#[derive(Debug, Clone, Serialize)]
struct MultiExtensionLaunchRecord {
    extension_id: String,
    family: String,
    descriptor_path: String,
    binary: String,
    args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    working_directory: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExtensionSetupHandoff {
    schema_id: String,
    schema_version: String,
    bundle_ref: String,
    #[serde(default)]
    answers_path: Option<String>,
    #[serde(default)]
    tenant: Option<String>,
    #[serde(default)]
    team: Option<String>,
    #[serde(default)]
    env: Option<String>,
    #[serde(default)]
    setup_args: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ExtensionStartHandoff {
    schema_id: String,
    schema_version: String,
    bundle_ref: String,
    #[serde(default)]
    start_args: Vec<String>,
}

pub(super) fn has_extension_flags(args: &[String]) -> bool {
    args.iter().any(|arg| {
        arg == "--extensions"
            || arg.starts_with("--extensions=")
            || arg == "--extension-registry"
            || arg.starts_with("--extension-registry=")
    })
}

pub(super) fn run_extension_wizard(
    sub_matches: &ArgMatches,
    tail: &[String],
    debug: bool,
    locale: &str,
) -> i32 {
    let extension_ids = collect_extension_ids(sub_matches);
    if extension_ids.is_empty() {
        eprintln!("no extensions were provided; use --extensions <id>[,<id>...]");
        return 2;
    }

    let registry_path = match resolve_registry_path(sub_matches) {
        Ok(Some(path)) => path,
        Ok(None) => {
            eprintln!(
                "no extension registry found; pass --extension-registry <path> or install one into ~/.greentic/artifacts/store_assets/extensions/registry.json"
            );
            return 1;
        }
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };

    let registry = match load_registry(&registry_path) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    let handoff_path = match resolve_handoff_output_path(sub_matches) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    let mut handoff_records = Vec::new();

    for extension_id in extension_ids {
        let descriptor_path =
            match resolve_descriptor_path(&registry, &registry_path, &extension_id) {
                Ok(path) => path,
                Err(err) => {
                    eprintln!("{err}");
                    return 1;
                }
            };
        let descriptor = match load_descriptor(&descriptor_path) {
            Ok(value) => value,
            Err(err) => {
                eprintln!("{err}");
                return 1;
            }
        };
        let args = build_extension_wizard_args(&descriptor, tail, locale);
        let cwd = resolve_descriptor_working_directory(&descriptor, &descriptor_path);
        handoff_records.push(MultiExtensionLaunchRecord {
            extension_id: descriptor.extension_id.clone(),
            family: descriptor.family.clone(),
            descriptor_path: descriptor_path.display().to_string(),
            binary: descriptor.wizard.binary.clone(),
            args: args.clone(),
            working_directory: cwd.as_ref().map(|path| path.display().to_string()),
        });
        eprintln!(
            "gtc: launching extension wizard {} ({}) via {}",
            descriptor.extension_id, descriptor.family, descriptor.wizard.binary
        );
        let code = passthrough_in_dir(
            &descriptor.wizard.binary,
            &args,
            debug,
            locale,
            cwd.as_deref(),
        );
        if code != 0 {
            return code;
        }
    }

    if let Err(err) = write_launcher_handoff(&registry_path, &handoff_records, &handoff_path) {
        eprintln!("{err}");
        return 1;
    }
    eprintln!(
        "gtc: wrote extension launcher handoff {}",
        handoff_path.display()
    );

    0
}

pub(super) fn run_extension_setup(
    sub_matches: &ArgMatches,
    tail: &[String],
    debug: bool,
    locale: &str,
) -> i32 {
    let Some(path) = sub_matches.get_one::<String>("extension-setup-handoff") else {
        eprintln!("missing --extension-setup-handoff");
        return 2;
    };
    let handoff = match load_extension_setup_handoff(Path::new(path)) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    let args = build_setup_args_from_handoff(&handoff, tail);
    passthrough_in_dir(SETUP_BIN, &args, debug, locale, None)
}

pub(super) fn run_extension_start(
    handoff_path: &str,
    tail: &[String],
    debug: bool,
    locale: &str,
) -> i32 {
    let handoff = match load_extension_start_handoff(Path::new(handoff_path)) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    let merged_tail = build_start_tail_from_handoff(&handoff, tail);
    run_start_with_bundle_ref_and_tail(&handoff.bundle_ref, &merged_tail, debug, locale)
}

/// Pull `--extension-start-handoff <path>` (or `=path`) out of a raw start
/// tail. Returns the path plus the tail with the flag removed, so the
/// downstream start parsing never sees a gtc-internal flag. The clap layer
/// no longer declares it — `start` is a pure catch-all.
pub(super) fn take_extension_start_handoff(
    tail: &[String],
) -> Result<Option<(String, Vec<String>)>, String> {
    let mut path = None;
    let mut rest = Vec::with_capacity(tail.len());
    let mut idx = 0usize;
    while idx < tail.len() {
        let arg = &tail[idx];
        if arg == "--extension-start-handoff" {
            idx += 1;
            let value = tail
                .get(idx)
                .ok_or_else(|| "missing value for --extension-start-handoff".to_string())?;
            path = Some(value.clone());
        } else if let Some(value) = arg.strip_prefix("--extension-start-handoff=") {
            path = Some(value.to_string());
        } else {
            rest.push(arg.clone());
        }
        idx += 1;
    }
    Ok(path.map(|path| (path, rest)))
}

fn collect_extension_ids(sub_matches: &ArgMatches) -> Vec<String> {
    let mut ids = Vec::new();
    if let Some(values) = sub_matches.get_many::<String>("extensions") {
        for value in values {
            for item in value.split(',') {
                let trimmed = item.trim();
                if !trimmed.is_empty() && !ids.iter().any(|existing| existing == trimmed) {
                    ids.push(trimmed.to_string());
                }
            }
        }
    }
    ids
}

fn resolve_registry_path(sub_matches: &ArgMatches) -> Result<Option<PathBuf>, String> {
    if let Some(path) = sub_matches.get_one::<String>("extension-registry") {
        return Ok(Some(PathBuf::from(path)));
    }

    if let Ok(path) = env::var("GTC_EXTENSION_REGISTRY")
        && !path.trim().is_empty()
    {
        return Ok(Some(PathBuf::from(path)));
    }

    let cwd_registry = env::current_dir()
        .map_err(|err| format!("failed to resolve current directory: {err}"))?
        .join("extension-registry.json");
    if cwd_registry.is_file() {
        return Ok(Some(cwd_registry));
    }

    let base = BaseDirs::new().ok_or_else(|| "failed to resolve home directory".to_string())?;
    let installed_registry = base
        .home_dir()
        .join(".greentic")
        .join("artifacts")
        .join("store_assets")
        .join("extensions")
        .join("registry.json");
    if installed_registry.is_file() {
        return Ok(Some(installed_registry));
    }

    Ok(None)
}

fn resolve_handoff_output_path(sub_matches: &ArgMatches) -> Result<PathBuf, String> {
    if let Some(path) = sub_matches.get_one::<String>("emit-extension-handoff") {
        return Ok(PathBuf::from(path));
    }

    let cwd =
        env::current_dir().map_err(|err| format!("failed to resolve current directory: {err}"))?;
    Ok(cwd
        .join(".greentic")
        .join("wizard")
        .join("extensions")
        .join("launcher-handoff.json"))
}

fn load_registry(path: &Path) -> Result<ExtensionRegistry, String> {
    let raw = fs::read_to_string(path).map_err(|err| {
        format!(
            "failed to read extension registry {}: {err}",
            path.display()
        )
    })?;
    let registry: ExtensionRegistry = serde_json::from_str(&raw).map_err(|err| {
        format!(
            "failed to parse extension registry {}: {err}",
            path.display()
        )
    })?;
    if registry.schema_version != "1" {
        return Err(format!(
            "unsupported extension registry schema_version '{}' in {}",
            registry.schema_version,
            path.display()
        ));
    }
    Ok(registry)
}

fn load_extension_setup_handoff(path: &Path) -> Result<ExtensionSetupHandoff, String> {
    let raw = fs::read_to_string(path).map_err(|err| {
        format!(
            "failed to read extension setup handoff {}: {err}",
            path.display()
        )
    })?;
    let handoff: ExtensionSetupHandoff = serde_json::from_str(&raw).map_err(|err| {
        format!(
            "failed to parse extension setup handoff {}: {err}",
            path.display()
        )
    })?;
    if handoff.schema_id != "gtc.extension.setup.handoff" {
        return Err(format!(
            "unsupported extension setup handoff schema_id '{}' in {}",
            handoff.schema_id,
            path.display()
        ));
    }
    if handoff.schema_version != "1.0.0" {
        return Err(format!(
            "unsupported extension setup handoff schema_version '{}' in {}",
            handoff.schema_version,
            path.display()
        ));
    }
    Ok(handoff)
}

fn load_extension_start_handoff(path: &Path) -> Result<ExtensionStartHandoff, String> {
    let raw = fs::read_to_string(path).map_err(|err| {
        format!(
            "failed to read extension start handoff {}: {err}",
            path.display()
        )
    })?;
    let handoff: ExtensionStartHandoff = serde_json::from_str(&raw).map_err(|err| {
        format!(
            "failed to parse extension start handoff {}: {err}",
            path.display()
        )
    })?;
    if handoff.schema_id != "gtc.extension.start.handoff" {
        return Err(format!(
            "unsupported extension start handoff schema_id '{}' in {}",
            handoff.schema_id,
            path.display()
        ));
    }
    if handoff.schema_version != "1.0.0" {
        return Err(format!(
            "unsupported extension start handoff schema_version '{}' in {}",
            handoff.schema_version,
            path.display()
        ));
    }
    Ok(handoff)
}

fn resolve_descriptor_path(
    registry: &ExtensionRegistry,
    registry_path: &Path,
    extension_id: &str,
) -> Result<PathBuf, String> {
    let Some(entry) = registry
        .extensions
        .iter()
        .find(|entry| entry.id == extension_id)
    else {
        let available = registry
            .extensions
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "extension '{}' was not found in {}; available extensions: {}",
            extension_id,
            registry_path.display(),
            available
        ));
    };

    let candidate = PathBuf::from(&entry.descriptor);
    if candidate.is_absolute() {
        return Ok(candidate);
    }
    let base = registry_path
        .parent()
        .ok_or_else(|| format!("registry path {} has no parent", registry_path.display()))?;
    Ok(base.join(candidate))
}

fn load_descriptor(path: &Path) -> Result<ExtensionDescriptor, String> {
    let raw = fs::read_to_string(path).map_err(|err| {
        format!(
            "failed to read extension descriptor {}: {err}",
            path.display()
        )
    })?;
    let descriptor: ExtensionDescriptor = serde_json::from_str(&raw).map_err(|err| {
        format!(
            "failed to parse extension descriptor {}: {err}",
            path.display()
        )
    })?;
    if descriptor.schema_version != "1" {
        return Err(format!(
            "unsupported extension descriptor schema_version '{}' in {}",
            descriptor.schema_version,
            path.display()
        ));
    }
    Ok(descriptor)
}

fn build_extension_wizard_args(
    descriptor: &ExtensionDescriptor,
    tail: &[String],
    locale: &str,
) -> Vec<String> {
    let mut args = descriptor.wizard.args.clone();
    if args.is_empty() {
        args.push("wizard".to_string());
    }
    if args.first().map(String::as_str) == Some("wizard") {
        let mut forwarded = args[1..].to_vec();
        forwarded.extend_from_slice(tail);
        return build_wizard_args(&forwarded, locale);
    }
    if !args
        .iter()
        .any(|arg| arg == "--locale" || arg.starts_with("--locale="))
        && !tail
            .iter()
            .any(|arg| arg == "--locale" || arg.starts_with("--locale="))
    {
        args.push("--locale".to_string());
        args.push(locale.to_string());
    }
    args.extend_from_slice(tail);
    args
}

fn build_setup_args_from_handoff(handoff: &ExtensionSetupHandoff, tail: &[String]) -> Vec<String> {
    let mut args = handoff.setup_args.clone();
    if let Some(answers_path) = &handoff.answers_path {
        args.push("--answers".to_string());
        args.push(answers_path.clone());
    }
    if let Some(tenant) = &handoff.tenant {
        args.push("--tenant".to_string());
        args.push(tenant.clone());
    }
    if let Some(team) = &handoff.team {
        args.push("--team".to_string());
        args.push(team.clone());
    }
    if let Some(env) = &handoff.env {
        args.push("--env".to_string());
        args.push(env.clone());
    }
    args.extend_from_slice(tail);
    args.push(handoff.bundle_ref.clone());
    args
}

fn build_start_tail_from_handoff(handoff: &ExtensionStartHandoff, tail: &[String]) -> Vec<String> {
    let mut args = handoff.start_args.clone();
    args.extend_from_slice(tail);
    args
}

fn resolve_descriptor_working_directory(
    descriptor: &ExtensionDescriptor,
    descriptor_path: &Path,
) -> Option<PathBuf> {
    let cwd = descriptor.wizard.working_directory.as_ref()?;
    let cwd = PathBuf::from(cwd);
    if cwd.is_absolute() {
        return Some(cwd);
    }
    descriptor_path.parent().map(|base| base.join(cwd))
}

fn write_launcher_handoff(
    registry_path: &Path,
    records: &[MultiExtensionLaunchRecord],
    output_path: &Path,
) -> Result<(), String> {
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            format!(
                "failed to create handoff directory {}: {err}",
                parent.display()
            )
        })?;
    }
    let handoff = MultiExtensionLauncherHandoff {
        schema_id: "gtc.extension.launcher.handoff",
        schema_version: "1.0.0",
        mode: "multi_extension_wizard",
        registry_path: registry_path.display().to_string(),
        extensions: records.to_vec(),
    };
    let json = serde_json::to_string_pretty(&handoff)
        .map_err(|err| format!("failed to serialize extension launcher handoff: {err}"))?;
    fs::write(output_path, json).map_err(|err| {
        format!(
            "failed to write extension launcher handoff {}: {err}",
            output_path.display()
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        build_extension_wizard_args, build_setup_args_from_handoff, build_start_tail_from_handoff,
        collect_extension_ids, has_extension_flags, load_descriptor, load_extension_setup_handoff,
        load_extension_start_handoff, load_registry, resolve_descriptor_path,
        resolve_descriptor_working_directory, resolve_handoff_output_path, resolve_registry_path,
        run_extension_setup, run_extension_start, run_extension_wizard,
        take_extension_start_handoff, write_launcher_handoff,
    };
    use crate::tests::env_test_lock;
    use clap::{Arg, ArgAction, Command};
    use std::env;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    #[test]
    fn extension_flags_are_detected() {
        assert!(has_extension_flags(&[
            "--extensions".to_string(),
            "telco-x".to_string()
        ]));
        assert!(has_extension_flags(&[
            "--extension-registry=registry.json".to_string()
        ]));
        assert!(!has_extension_flags(&[
            "--answers".to_string(),
            "a.json".to_string()
        ]));
    }

    #[test]
    fn collect_extension_ids_splits_commas_and_deduplicates() {
        let matches = Command::new("wizard")
            .arg(
                Arg::new("extensions")
                    .long("extensions")
                    .action(ArgAction::Append)
                    .num_args(1..),
            )
            .try_get_matches_from([
                "wizard",
                "--extensions",
                "telco-x,greentic-dw",
                "--extensions",
                "telco-x",
            ])
            .expect("matches");

        assert_eq!(
            collect_extension_ids(&matches),
            vec!["telco-x".to_string(), "greentic-dw".to_string()]
        );
    }

    #[test]
    fn registry_and_descriptor_paths_resolve_relative_to_registry() {
        let root = tempfile::tempdir().expect("tempdir");
        let registry_path = root.path().join("extension-registry.json");
        let descriptor_path = root.path().join("descriptors").join("telco-x.json");
        fs::create_dir_all(descriptor_path.parent().expect("parent")).expect("mkdir");
        fs::write(
            &registry_path,
            r#"{
  "schema_version": "1",
  "extensions": [
    { "id": "telco-x", "descriptor": "descriptors/telco-x.json" }
  ]
}"#,
        )
        .expect("write registry");
        fs::write(
            &descriptor_path,
            r#"{
  "schema_version": "1",
  "extension_id": "telco-x",
  "family": "solution-x",
  "wizard": {
    "binary": "greentic-x",
    "args": ["wizard", "--catalog", "catalog.json"],
    "working_directory": "."
  }
}"#,
        )
        .expect("write descriptor");

        let registry = load_registry(&registry_path).expect("registry");
        let resolved = resolve_descriptor_path(&registry, &registry_path, "telco-x").expect("path");
        assert_eq!(resolved, descriptor_path);

        let descriptor = load_descriptor(&resolved).expect("descriptor");
        let cwd = resolve_descriptor_working_directory(&descriptor, &resolved).expect("cwd");
        assert_eq!(cwd, descriptor_path.parent().expect("parent"));
    }

    #[test]
    fn registry_loader_rejects_unsupported_schema_and_missing_extension() {
        let root = tempfile::tempdir().expect("tempdir");
        let registry_path = root.path().join("extension-registry.json");
        fs::write(&registry_path, r#"{"schema_version":"2","extensions":[]}"#)
            .expect("write registry");
        let err = load_registry(&registry_path).unwrap_err();
        assert!(err.contains("unsupported extension registry schema_version"));

        fs::write(
            &registry_path,
            r#"{"schema_version":"1","extensions":[{"id":"alpha","descriptor":"alpha.json"}]}"#,
        )
        .expect("write registry");
        let registry = load_registry(&registry_path).expect("registry");
        let err = resolve_descriptor_path(&registry, &registry_path, "beta").unwrap_err();
        assert!(err.contains("extension 'beta' was not found"));
        assert!(err.contains("alpha"));
    }

    #[test]
    fn descriptor_loader_rejects_bad_schema() {
        let root = tempfile::tempdir().expect("tempdir");
        let descriptor_path = root.path().join("descriptor.json");
        fs::write(
            &descriptor_path,
            r#"{
  "schema_version": "2",
  "extension_id": "telco-x",
  "family": "solution-x",
  "wizard": { "binary": "greentic-x" }
}"#,
        )
        .expect("write descriptor");
        let err = load_descriptor(&descriptor_path).unwrap_err();
        assert!(err.contains("unsupported extension descriptor schema_version"));
    }

    #[test]
    fn wizard_args_preserve_descriptor_prefix_and_locale() {
        let descriptor = load_descriptor_from_str(
            r#"{
  "schema_version": "1",
  "extension_id": "telco-x",
  "family": "solution-x",
  "wizard": {
    "binary": "greentic-x",
    "args": ["wizard", "--catalog", "catalog.json"]
  }
}"#,
        );

        let args = build_extension_wizard_args(
            &descriptor,
            &["--dry-run".to_string(), "--emit-answers".to_string()],
            "en",
        );
        assert_eq!(
            args,
            vec![
                "wizard".to_string(),
                "--locale".to_string(),
                "en".to_string(),
                "--catalog".to_string(),
                "catalog.json".to_string(),
                "--dry-run".to_string(),
                "--emit-answers".to_string()
            ]
        );
    }

    #[test]
    fn wizard_args_for_non_wizard_binary_add_locale_once() {
        let descriptor = load_descriptor_from_str(
            r#"{
  "schema_version": "1",
  "extension_id": "telco-x",
  "family": "solution-x",
  "wizard": {
    "binary": "greentic-x",
    "args": ["launch", "--mode", "guided"]
  }
}"#,
        );

        let args = build_extension_wizard_args(
            &descriptor,
            &["--locale=fr".to_string(), "--dry-run".to_string()],
            "en",
        );
        assert_eq!(
            args,
            vec![
                "launch".to_string(),
                "--mode".to_string(),
                "guided".to_string(),
                "--locale=fr".to_string(),
                "--dry-run".to_string()
            ]
        );
    }

    #[test]
    fn launcher_handoff_is_written_with_extension_records() {
        let root = tempfile::tempdir().expect("tempdir");
        let output = root.path().join("handoff.json");
        write_launcher_handoff(
            &root.path().join("registry.json"),
            &[super::MultiExtensionLaunchRecord {
                extension_id: "telco-x".to_string(),
                family: "solution-x".to_string(),
                descriptor_path: "/tmp/telco-x/gtc-extension.json".to_string(),
                binary: "greentic-x".to_string(),
                args: vec!["wizard".to_string(), "--dry-run".to_string()],
                working_directory: Some("/tmp/telco-x".to_string()),
            }],
            &output,
        )
        .expect("handoff");

        let raw = std::fs::read_to_string(&output).expect("read");
        assert!(raw.contains("\"schema_id\": \"gtc.extension.launcher.handoff\""));
        assert!(raw.contains("\"extension_id\": \"telco-x\""));
        assert!(raw.contains("\"binary\": \"greentic-x\""));
    }

    #[test]
    fn setup_handoff_loader_accepts_generic_contract() {
        let root = tempfile::tempdir().expect("tempdir");
        let path = root.path().join("setup.json");
        std::fs::write(
            &path,
            r#"{
  "schema_id": "gtc.extension.setup.handoff",
  "schema_version": "1.0.0",
  "bundle_ref": "/tmp/demo-bundle",
  "answers_path": "/tmp/answers.json",
  "tenant": "demo",
  "team": "default",
  "env": "dev",
  "setup_args": ["--dry-run"]
}"#,
        )
        .expect("write");
        let handoff = load_extension_setup_handoff(&path).expect("handoff");
        assert_eq!(handoff.bundle_ref, "/tmp/demo-bundle");
        assert_eq!(handoff.answers_path.as_deref(), Some("/tmp/answers.json"));
        let args = build_setup_args_from_handoff(&handoff, &["--no-ui".to_string()]);
        assert_eq!(
            args,
            vec![
                "--dry-run".to_string(),
                "--answers".to_string(),
                "/tmp/answers.json".to_string(),
                "--tenant".to_string(),
                "demo".to_string(),
                "--team".to_string(),
                "default".to_string(),
                "--env".to_string(),
                "dev".to_string(),
                "--no-ui".to_string(),
                "/tmp/demo-bundle".to_string()
            ]
        );
    }

    #[test]
    fn setup_args_include_optional_fields_and_tail() {
        let handoff = load_setup_handoff_from_str(
            r#"{
  "schema_id": "gtc.extension.setup.handoff",
  "schema_version": "1.0.0",
  "bundle_ref": "/tmp/demo-bundle",
  "answers_path": "/tmp/answers.json",
  "tenant": "demo",
  "team": "default",
  "env": "dev",
  "setup_args": ["--dry-run"]
}"#,
        );
        let args = build_setup_args_from_handoff(&handoff, &["--verbose".to_string()]);
        assert_eq!(
            args,
            vec![
                "--dry-run".to_string(),
                "--answers".to_string(),
                "/tmp/answers.json".to_string(),
                "--tenant".to_string(),
                "demo".to_string(),
                "--team".to_string(),
                "default".to_string(),
                "--env".to_string(),
                "dev".to_string(),
                "--verbose".to_string(),
                "/tmp/demo-bundle".to_string()
            ]
        );
    }

    #[test]
    fn start_handoff_loader_accepts_generic_contract() {
        let root = tempfile::tempdir().expect("tempdir");
        let path = root.path().join("start.json");
        std::fs::write(
            &path,
            r#"{
  "schema_id": "gtc.extension.start.handoff",
  "schema_version": "1.0.0",
  "bundle_ref": "/tmp/demo-bundle",
  "start_args": ["--tenant", "demo"]
}"#,
        )
        .expect("write");
        let handoff = load_extension_start_handoff(&path).expect("handoff");
        assert_eq!(handoff.bundle_ref, "/tmp/demo-bundle");
        assert_eq!(handoff.start_args, vec!["--tenant", "demo"]);
        let tail = build_start_tail_from_handoff(&handoff, &["--verbose".to_string()]);
        assert_eq!(
            tail,
            vec![
                "--tenant".to_string(),
                "demo".to_string(),
                "--verbose".to_string()
            ]
        );
    }

    #[test]
    fn handoff_loaders_reject_wrong_schema() {
        let root = tempfile::tempdir().expect("tempdir");
        let setup_path = root.path().join("setup.json");
        fs::write(
            &setup_path,
            r#"{"schema_id":"wrong","schema_version":"1.0.0","bundle_ref":"demo"}"#,
        )
        .expect("write setup");
        assert!(
            load_extension_setup_handoff(&setup_path)
                .unwrap_err()
                .contains("unsupported extension setup handoff schema_id")
        );

        let start_path = root.path().join("start.json");
        fs::write(
            &start_path,
            r#"{"schema_id":"wrong","schema_version":"1.0.0","bundle_ref":"demo"}"#,
        )
        .expect("write start");
        assert!(
            load_extension_start_handoff(&start_path)
                .unwrap_err()
                .contains("unsupported extension start handoff schema_id")
        );
    }

    #[test]
    fn start_tail_appends_cli_tail() {
        let handoff = load_start_handoff_from_str(
            r#"{
  "schema_id": "gtc.extension.start.handoff",
  "schema_version": "1.0.0",
  "bundle_ref": "/tmp/demo-bundle",
  "start_args": ["--tenant", "demo"]
}"#,
        );
        let args = build_start_tail_from_handoff(&handoff, &["--tail".to_string()]);
        assert_eq!(
            args,
            vec![
                "--tenant".to_string(),
                "demo".to_string(),
                "--tail".to_string()
            ]
        );
    }

    #[test]
    fn descriptor_working_directory_returns_absolute_path_as_is() {
        let descriptor = load_descriptor_from_str(
            r#"{
  "schema_version": "1",
  "extension_id": "telco-x",
  "family": "solution-x",
  "wizard": {
    "binary": "greentic-x",
    "working_directory": "/tmp/telco-x"
  }
}"#,
        );
        let path = PathBuf::from("/tmp/registry/telco-x.json");
        assert_eq!(
            resolve_descriptor_working_directory(&descriptor, &path),
            Some(PathBuf::from("/tmp/telco-x"))
        );
    }

    fn load_descriptor_from_str(raw: &str) -> super::ExtensionDescriptor {
        serde_json::from_str(raw).expect("descriptor")
    }

    fn load_setup_handoff_from_str(raw: &str) -> super::ExtensionSetupHandoff {
        serde_json::from_str(raw).expect("setup handoff")
    }

    fn load_start_handoff_from_str(raw: &str) -> super::ExtensionStartHandoff {
        serde_json::from_str(raw).expect("start handoff")
    }

    fn build_wizard_matches(args: &[&str]) -> clap::ArgMatches {
        Command::new("wizard")
            .arg(
                Arg::new("extensions")
                    .long("extensions")
                    .action(ArgAction::Append)
                    .num_args(1..),
            )
            .arg(
                Arg::new("extension-registry")
                    .long("extension-registry")
                    .num_args(1),
            )
            .arg(
                Arg::new("emit-extension-handoff")
                    .long("emit-extension-handoff")
                    .num_args(1),
            )
            .try_get_matches_from(args)
            .expect("matches")
    }

    #[test]
    fn run_extension_wizard_errors_when_no_extension_ids_supplied() {
        let matches = build_wizard_matches(&["wizard"]);
        let exit = run_extension_wizard(&matches, &[], false, "en");
        assert_eq!(exit, 2);
    }

    #[test]
    fn run_extension_wizard_errors_when_registry_path_invalid() {
        let matches = build_wizard_matches(&[
            "wizard",
            "--extensions",
            "telco-x",
            "--extension-registry",
            "/definitely/missing/registry.json",
        ]);
        let exit = run_extension_wizard(&matches, &[], false, "en");
        assert_eq!(exit, 1);
    }

    #[test]
    fn run_extension_wizard_errors_when_no_registry_found() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let old_cwd = env::current_dir().expect("cwd");
        let old_home = env::var_os("HOME");
        let old_registry = env::var_os("GTC_EXTENSION_REGISTRY");
        env::set_current_dir(dir.path()).expect("set cwd");
        let fake_home = dir.path().join("home");
        fs::create_dir_all(&fake_home).expect("home");
        unsafe {
            env::set_var("HOME", &fake_home);
            env::remove_var("GTC_EXTENSION_REGISTRY");
        }
        let matches = build_wizard_matches(&["wizard", "--extensions", "telco-x"]);
        let exit = run_extension_wizard(&matches, &[], false, "en");
        env::set_current_dir(&old_cwd).expect("restore cwd");
        unsafe {
            match old_home {
                Some(value) => env::set_var("HOME", value),
                None => env::remove_var("HOME"),
            }
            match old_registry {
                Some(value) => env::set_var("GTC_EXTENSION_REGISTRY", value),
                None => env::remove_var("GTC_EXTENSION_REGISTRY"),
            }
        }
        assert_eq!(exit, 1);
    }

    #[test]
    fn run_extension_setup_errors_when_handoff_flag_missing() {
        let matches = Command::new("extension-setup")
            .arg(
                Arg::new("extension-setup-handoff")
                    .long("extension-setup-handoff")
                    .num_args(1),
            )
            .try_get_matches_from(["extension-setup"])
            .expect("matches");
        assert_eq!(run_extension_setup(&matches, &[], false, "en"), 2);
    }

    #[test]
    fn run_extension_setup_errors_when_handoff_path_invalid() {
        let matches = Command::new("extension-setup")
            .arg(
                Arg::new("extension-setup-handoff")
                    .long("extension-setup-handoff")
                    .num_args(1),
            )
            .try_get_matches_from([
                "extension-setup",
                "--extension-setup-handoff",
                "/definitely/missing/handoff.json",
            ])
            .expect("matches");
        assert_eq!(run_extension_setup(&matches, &[], false, "en"), 1);
    }

    #[test]
    fn take_extension_start_handoff_extracts_both_forms_and_strips_flag() {
        let tail = vec![
            "bundle.gtbundle".to_string(),
            "--extension-start-handoff".to_string(),
            "/tmp/handoff.json".to_string(),
            "--tenant".to_string(),
            "demo".to_string(),
        ];
        let (path, rest) = take_extension_start_handoff(&tail)
            .expect("extraction succeeds")
            .expect("flag present");
        assert_eq!(path, "/tmp/handoff.json");
        assert_eq!(rest, vec!["bundle.gtbundle", "--tenant", "demo"]);

        let tail = vec!["--extension-start-handoff=/tmp/h.json".to_string()];
        let (path, rest) = take_extension_start_handoff(&tail)
            .expect("extraction succeeds")
            .expect("flag present");
        assert_eq!(path, "/tmp/h.json");
        assert!(rest.is_empty());

        assert!(
            take_extension_start_handoff(&["--tenant".to_string(), "demo".to_string()])
                .expect("extraction succeeds")
                .is_none()
        );
        take_extension_start_handoff(&["--extension-start-handoff".to_string()])
            .expect_err("missing value must error");
    }

    #[test]
    fn run_extension_start_errors_when_handoff_path_invalid() {
        assert_eq!(
            run_extension_start("/definitely/missing/start.json", &[], false, "en"),
            1
        );
    }

    #[test]
    fn resolve_registry_path_prefers_env_var_then_cwd_then_installed() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let registry = dir.path().join("custom-registry.json");
        fs::write(&registry, "{}").expect("write registry");
        let matches = Command::new("wizard")
            .arg(
                Arg::new("extension-registry")
                    .long("extension-registry")
                    .num_args(1),
            )
            .try_get_matches_from(["wizard"])
            .expect("matches");

        let old_env = env::var_os("GTC_EXTENSION_REGISTRY");
        unsafe {
            env::set_var("GTC_EXTENSION_REGISTRY", &registry);
        }
        let resolved = resolve_registry_path(&matches).expect("registry path");
        unsafe {
            match old_env {
                Some(value) => env::set_var("GTC_EXTENSION_REGISTRY", value),
                None => env::remove_var("GTC_EXTENSION_REGISTRY"),
            }
        }
        assert_eq!(resolved.as_deref(), Some(registry.as_path()));
    }

    #[test]
    fn resolve_handoff_output_path_defaults_to_greentic_wizard_dir() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let old_cwd = env::current_dir().expect("cwd");
        env::set_current_dir(dir.path()).expect("set cwd");
        let matches = Command::new("wizard")
            .arg(
                Arg::new("emit-extension-handoff")
                    .long("emit-extension-handoff")
                    .num_args(1),
            )
            .try_get_matches_from(["wizard"])
            .expect("matches");
        let resolved = resolve_handoff_output_path(&matches).expect("handoff path");
        env::set_current_dir(&old_cwd).expect("restore cwd");
        assert!(resolved.ends_with(".greentic/wizard/extensions/launcher-handoff.json"));
    }

    #[test]
    fn resolve_handoff_output_path_honors_explicit_override() {
        let matches = Command::new("wizard")
            .arg(
                Arg::new("emit-extension-handoff")
                    .long("emit-extension-handoff")
                    .num_args(1),
            )
            .try_get_matches_from(["wizard", "--emit-extension-handoff", "/tmp/handoff.json"])
            .expect("matches");
        let resolved = resolve_handoff_output_path(&matches).expect("handoff path");
        assert_eq!(resolved, std::path::PathBuf::from("/tmp/handoff.json"));
    }

    #[test]
    fn build_setup_args_omits_optional_identity_when_absent() {
        let handoff = super::ExtensionSetupHandoff {
            schema_id: "gtc.extension.setup.handoff".to_string(),
            schema_version: "1.0.0".to_string(),
            bundle_ref: "/tmp/bundle".to_string(),
            answers_path: None,
            tenant: None,
            team: None,
            env: None,
            setup_args: Vec::new(),
        };
        let args = build_setup_args_from_handoff(&handoff, &[]);
        assert_eq!(args, vec!["/tmp/bundle".to_string()]);
    }

    #[test]
    fn resolve_descriptor_working_directory_returns_none_when_missing() {
        let descriptor = load_descriptor_from_str(
            r#"{
  "schema_version": "1",
  "extension_id": "telco-x",
  "family": "solution-x",
  "wizard": { "binary": "greentic-x", "args": ["wizard"] }
}"#,
        );
        let resolved = resolve_descriptor_working_directory(
            &descriptor,
            std::path::Path::new("/tmp/anywhere/descriptor.json"),
        );
        assert!(resolved.is_none());
    }

    #[test]
    fn resolve_descriptor_path_returns_absolute_unchanged() {
        let root = tempfile::tempdir().expect("tempdir");
        let registry_path = root.path().join("registry.json");
        let absolute = if cfg!(windows) {
            "C:/abs/path.json"
        } else {
            "/abs/path.json"
        };
        fs::write(
            &registry_path,
            format!(
                r#"{{"schema_version":"1","extensions":[{{"id":"alpha","descriptor":"{absolute}"}}]}}"#
            ),
        )
        .expect("write");
        let registry = load_registry(&registry_path).expect("registry");
        let resolved = resolve_descriptor_path(&registry, &registry_path, "alpha").expect("path");
        assert_eq!(resolved, std::path::PathBuf::from(absolute));
    }

    #[cfg(unix)]
    #[test]
    fn run_extension_setup_invokes_setup_binary_with_handoff_args() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let setup_bin = dir.path().join("greentic-setup");
        let log = dir.path().join("setup.log");
        fs::write(
            &setup_bin,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" > '{}'\nexit 0\n",
                log.display()
            ),
        )
        .expect("write setup");
        let mut perms = fs::metadata(&setup_bin).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&setup_bin, perms).expect("chmod");

        let handoff_path = dir.path().join("setup-handoff.json");
        fs::write(
            &handoff_path,
            r#"{
  "schema_id": "gtc.extension.setup.handoff",
  "schema_version": "1.0.0",
  "bundle_ref": "/tmp/bundle",
  "tenant": "demo",
  "setup_args": ["--prepare"]
}"#,
        )
        .expect("write handoff");

        let old_setup = env::var_os("GREENTIC_SETUP_BIN");
        unsafe {
            env::set_var("GREENTIC_SETUP_BIN", &setup_bin);
        }

        let matches = Command::new("extension-setup")
            .arg(
                Arg::new("extension-setup-handoff")
                    .long("extension-setup-handoff")
                    .num_args(1),
            )
            .try_get_matches_from([
                "extension-setup",
                "--extension-setup-handoff",
                handoff_path.to_str().expect("utf8"),
            ])
            .expect("matches");
        let exit = run_extension_setup(&matches, &["--no-ui".to_string()], false, "en");

        unsafe {
            match old_setup {
                Some(value) => env::set_var("GREENTIC_SETUP_BIN", value),
                None => env::remove_var("GREENTIC_SETUP_BIN"),
            }
        }
        assert_eq!(exit, 0);
        let logged = fs::read_to_string(&log).expect("read log");
        assert!(logged.contains("--prepare"));
        assert!(logged.contains("--tenant"));
        assert!(logged.contains("demo"));
        assert!(logged.contains("--no-ui"));
        assert!(logged.contains("/tmp/bundle"));
    }
}
