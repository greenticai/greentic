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
    sub_matches: &ArgMatches,
    tail: &[String],
    debug: bool,
    locale: &str,
) -> i32 {
    let Some(path) = sub_matches.get_one::<String>("extension-start-handoff") else {
        eprintln!("missing --extension-start-handoff");
        return 2;
    };
    let handoff = match load_extension_start_handoff(Path::new(path)) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    let merged_tail = build_start_tail_from_handoff(&handoff, tail);
    run_start_with_bundle_ref_and_tail(&handoff.bundle_ref, &merged_tail, debug, locale)
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
        build_extension_wizard_args, collect_extension_ids, has_extension_flags, load_descriptor,
        load_extension_setup_handoff, load_extension_start_handoff, load_registry,
        resolve_descriptor_path, resolve_descriptor_working_directory, write_launcher_handoff,
    };
    use clap::{Arg, Command};
    use std::fs;

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
            .arg(Arg::new("extensions").long("extensions").num_args(1..))
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
    }

    fn load_descriptor_from_str(raw: &str) -> super::ExtensionDescriptor {
        serde_json::from_str(raw).expect("descriptor")
    }
}
