use std::collections::{BTreeMap, HashMap};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use flate2::Compression;
use flate2::write::GzEncoder;
use greentic_distributor_client::{DistClient, DistOptions};
use sha2::{Digest, Sha256};

#[test]
fn version_flag_prints_cargo_package_version() {
    let sandbox = TestSandbox::new("version_flag_prints_cargo_package_version");
    let mut extra = HashMap::new();
    extra.insert(
        "GTC_TOOLCHAIN_STATE_DIR".to_string(),
        sandbox.path().join("toolchain-state").display().to_string(),
    );
    let output = sandbox.run_gtc_capture(["--version"], extra);
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("gtc "));
    assert!(stdout.contains("Greentic toolchain release: not installed"));
}

#[test]
fn version_flag_prints_installed_toolchain_release() {
    let sandbox = TestSandbox::new("version_flag_prints_installed_toolchain_release");
    let state_dir = sandbox.path().join("toolchain-state");
    write_installed_toolchain_state(&state_dir, "1.0.4", "stable", "sha256:testdigest");

    let mut extra = HashMap::new();
    extra.insert(
        "GTC_TOOLCHAIN_STATE_DIR".to_string(),
        state_dir.display().to_string(),
    );

    let output = sandbox.run_gtc_capture(["--version"], extra);
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("gtc "));
    assert!(stdout.contains("Greentic toolchain release: 1.0.4 (stable) [sha256:testdigest]"));
}

#[test]
fn passthrough_dev_preserves_exit_code() {
    let sandbox = TestSandbox::new("passthrough_dev_preserves_exit_code");
    sandbox.write_exit_tool("greentic-dev", 17);
    sandbox.write_exit_tool("greentic-operator", 0);

    let status = sandbox.run_gtc(["dev", "flow", "list"], HashMap::new());
    assert_eq!(status.code(), Some(17));
}

#[test]
fn wizard_missing_greentic_dev_prints_install_guidance() {
    let sandbox = TestSandbox::new("wizard_missing_greentic_dev_prints_install_guidance");
    sandbox.write_exit_tool("greentic-operator", 0);

    let mut extra = HashMap::new();
    extra.insert(
        "GREENTIC_DEV_BIN".to_string(),
        sandbox
            .path()
            .join("missing-greentic-dev")
            .display()
            .to_string(),
    );

    let output = sandbox.run_gtc_capture(["wizard"], extra);
    assert_eq!(output.status.code(), Some(1));

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("greentic-dev not found in PATH."));
    assert!(stderr.contains("gtc install"));
}

#[test]
fn passthrough_dev_uses_greentic_dev_bin_override() {
    let sandbox = TestSandbox::new("passthrough_dev_uses_greentic_dev_bin_override");
    let log_file = sandbox.path().join("override-dev.log");
    let override_bin = sandbox.path().join("bin").join("greentic-dev-local");
    fs::create_dir_all(override_bin.parent().expect("override parent")).expect("override dir");
    sandbox.compile_rust_tool_at(&override_bin, &rust_arg_logger_program(&log_file, 0));
    sandbox.write_exit_tool("greentic-dev", 91);
    sandbox.write_exit_tool("greentic-operator", 0);

    let mut extra = HashMap::new();
    extra.insert(
        "GREENTIC_DEV_BIN".to_string(),
        override_bin.display().to_string(),
    );

    let status = sandbox.run_gtc(["dev", "flow", "list"], extra);
    assert_eq!(status.code(), Some(0));

    let logged = fs::read_to_string(log_file).expect("read override dev log");
    assert!(logged.contains("flow list"));
}

// ---------------------------------------------------------------------------
// Task F3: cross-CLI smoke tests for `gtc dev <name> info` passthrough.
//
// `gtc dev <name> ...` forwards every token after `dev` to `greentic-dev`
// verbatim (see `router::route_passthrough_subcommand`). The second hop
// (`greentic-dev <name> info ...` -> `greentic-<name> info ...`) is covered by
// greentic-dev's own integration tests. Here we only need to confirm that the
// router hands the right args to `greentic-dev` for each of the 5 downstream
// CLI names (pack, bundle, flow, component, runner).
// ---------------------------------------------------------------------------

#[test]
fn gtc_dev_pack_info_forwards_args() {
    let sandbox = TestSandbox::new("gtc_dev_pack_info_forwards_args");
    let log_file = sandbox.path().join("dev.log");
    sandbox.write_arg_logger_tool("greentic-dev", &log_file, 0);
    sandbox.write_exit_tool("greentic-operator", 0);

    let status = sandbox.run_gtc(
        ["dev", "pack", "info", "/tmp/fixture.gtpack"],
        HashMap::new(),
    );
    assert_eq!(status.code(), Some(0));

    let logged = fs::read_to_string(log_file).expect("read dev log");
    assert!(
        logged.contains("pack info /tmp/fixture.gtpack"),
        "expected forwarded args, got: {logged}"
    );
}

#[test]
fn gtc_dev_bundle_info_forwards_args_with_json() {
    let sandbox = TestSandbox::new("gtc_dev_bundle_info_forwards_args_with_json");
    let log_file = sandbox.path().join("dev.log");
    sandbox.write_arg_logger_tool("greentic-dev", &log_file, 0);
    sandbox.write_exit_tool("greentic-operator", 0);

    let status = sandbox.run_gtc(
        ["dev", "bundle", "info", "/tmp/fixture.gtbundle", "--json"],
        HashMap::new(),
    );
    assert_eq!(status.code(), Some(0));

    let logged = fs::read_to_string(log_file).expect("read dev log");
    assert!(
        logged.contains("bundle info /tmp/fixture.gtbundle --json"),
        "expected forwarded args, got: {logged}"
    );
}

#[test]
fn gtc_dev_flow_info_forwards_args() {
    let sandbox = TestSandbox::new("gtc_dev_flow_info_forwards_args");
    let log_file = sandbox.path().join("dev.log");
    sandbox.write_arg_logger_tool("greentic-dev", &log_file, 0);
    sandbox.write_exit_tool("greentic-operator", 0);

    let status = sandbox.run_gtc(["dev", "flow", "info", "/tmp/fixture.ygtc"], HashMap::new());
    assert_eq!(status.code(), Some(0));

    let logged = fs::read_to_string(log_file).expect("read dev log");
    assert!(
        logged.contains("flow info /tmp/fixture.ygtc"),
        "expected forwarded args, got: {logged}"
    );
}

#[test]
fn gtc_dev_component_info_forwards_args() {
    let sandbox = TestSandbox::new("gtc_dev_component_info_forwards_args");
    let log_file = sandbox.path().join("dev.log");
    sandbox.write_arg_logger_tool("greentic-dev", &log_file, 0);
    sandbox.write_exit_tool("greentic-operator", 0);

    let status = sandbox.run_gtc(
        ["dev", "component", "info", "/tmp/fixture.wasm", "--json"],
        HashMap::new(),
    );
    assert_eq!(status.code(), Some(0));

    let logged = fs::read_to_string(log_file).expect("read dev log");
    assert!(
        logged.contains("component info /tmp/fixture.wasm --json"),
        "expected forwarded args, got: {logged}"
    );
}

#[test]
fn gtc_dev_runner_info_forwards_args() {
    let sandbox = TestSandbox::new("gtc_dev_runner_info_forwards_args");
    let log_file = sandbox.path().join("dev.log");
    sandbox.write_arg_logger_tool("greentic-dev", &log_file, 0);
    sandbox.write_exit_tool("greentic-operator", 0);

    // `runner info` takes no positional path — it reports on the running
    // runtime. `--json` is the usual companion flag.
    let status = sandbox.run_gtc(["dev", "runner", "info", "--json"], HashMap::new());
    assert_eq!(status.code(), Some(0));

    let logged = fs::read_to_string(log_file).expect("read dev log");
    assert!(
        logged.contains("runner info --json"),
        "expected forwarded args, got: {logged}"
    );
}

#[test]
fn wizard_passthrough_routes_to_greentic_dev_with_all_args() {
    let sandbox = TestSandbox::new("wizard_passthrough_routes_to_greentic_dev_with_all_args");
    let log_file = sandbox.path().join("dev.log");
    let answers_file = sandbox.path().join("answers.json");
    fs::write(&answers_file, br#"{"ok":true}"#).expect("write answers");
    sandbox.write_arg_logger_tool("greentic-dev", &log_file, 0);
    sandbox.write_exit_tool("greentic-operator", 0);

    let status = sandbox.run_gtc(
        [
            "wizard",
            "--locale",
            "fr",
            "--answers",
            answers_file.to_str().expect("answers path"),
        ],
        HashMap::new(),
    );
    assert_eq!(status.code(), Some(0));

    let logged = fs::read_to_string(log_file).expect("read dev log");
    assert!(logged.contains(&format!(
        "wizard --locale fr --answers {}",
        answers_file.display()
    )));
}

#[test]
fn wizard_passthrough_preserves_global_locale_for_greentic_dev() {
    let sandbox = TestSandbox::new("wizard_passthrough_preserves_global_locale_for_greentic_dev");
    let log_file = sandbox.path().join("dev.log");
    let answers_file = sandbox.path().join("answers.json");
    fs::write(&answers_file, br#"{"ok":true}"#).expect("write answers");
    sandbox.write_arg_logger_tool("greentic-dev", &log_file, 0);
    sandbox.write_exit_tool("greentic-operator", 0);

    let status = sandbox.run_gtc(
        [
            "--locale",
            "fr",
            "wizard",
            "--answers",
            answers_file.to_str().expect("answers path"),
        ],
        HashMap::new(),
    );
    assert_eq!(status.code(), Some(0));

    let logged = fs::read_to_string(log_file).expect("read dev log");
    assert!(logged.contains(&format!(
        "wizard --locale fr --answers {}",
        answers_file.display()
    )));
}

#[test]
fn wizard_passthrough_routes_to_greentic_dev_without_args() {
    let sandbox = TestSandbox::new("wizard_passthrough_routes_to_greentic_dev_without_args");
    let log_file = sandbox.path().join("dev.log");
    sandbox.write_arg_logger_tool("greentic-dev", &log_file, 0);
    sandbox.write_exit_tool("greentic-operator", 0);

    let status = sandbox.run_gtc(["wizard"], HashMap::new());
    assert_eq!(status.code(), Some(0));

    let logged = fs::read_to_string(log_file).expect("read dev log");
    assert!(logged.contains("wizard"));
}

#[test]
fn wizard_warns_when_release_context_is_not_latest_and_runs() {
    let sandbox = TestSandbox::new("wizard_warns_when_release_context_is_not_latest_and_runs");
    let log_file = sandbox.path().join("wizard.log");
    let manifest_path = write_release_toolchain_manifest(sandbox.path());
    let state_dir = sandbox.path().join("toolchain-state");
    write_installed_toolchain_state(&state_dir, "1.0.4", "stable", "sha256:old");
    sandbox.write_arg_logger_tool("greentic-dev", &log_file, 0);
    sandbox.write_exit_tool("greentic-operator", 0);

    let mut extra = HashMap::new();
    extra.insert(
        "GTC_TOOLCHAIN_MANIFEST_PATH".to_string(),
        manifest_path.display().to_string(),
    );
    extra.insert(
        "GTC_TOOLCHAIN_STATE_DIR".to_string(),
        state_dir.display().to_string(),
    );

    let output = sandbox.run_gtc_capture(["wizard"], extra);
    assert_eq!(output.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("warning: Greentic toolchain release context is 1.0.4 (stable)"));
    assert!(stderr.contains("latest stable release is 1.0.16"));
    assert!(stderr.contains("Run `gtc install` to upgrade."));
    let logged = fs::read_to_string(log_file).expect("read wizard log");
    assert!(logged.contains("wizard"));
}

#[test]
fn wizard_strict_release_context_errors_on_mismatch() {
    let sandbox = TestSandbox::new("wizard_strict_release_context_errors_on_mismatch");
    let log_file = sandbox.path().join("wizard.log");
    let manifest_path = write_release_toolchain_manifest(sandbox.path());
    let state_dir = sandbox.path().join("toolchain-state");
    write_installed_toolchain_state(&state_dir, "1.0.4", "stable", "sha256:old");
    sandbox.write_arg_logger_tool("greentic-dev", &log_file, 0);
    sandbox.write_exit_tool("greentic-operator", 0);

    let mut extra = HashMap::new();
    extra.insert(
        "GTC_TOOLCHAIN_MANIFEST_PATH".to_string(),
        manifest_path.display().to_string(),
    );
    extra.insert(
        "GTC_TOOLCHAIN_STATE_DIR".to_string(),
        state_dir.display().to_string(),
    );

    let output = sandbox.run_gtc_capture(["wizard", "--strict-release-context"], extra);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("latest stable release is 1.0.16"));
    assert!(!log_file.exists(), "wizard should not run in strict mode");
}

#[test]
fn setup_ignore_release_context_skips_check_and_strips_flag() {
    let sandbox = TestSandbox::new("setup_ignore_release_context_skips_check_and_strips_flag");
    let log_file = sandbox.path().join("setup.log");
    sandbox.write_arg_logger_tool("greentic-setup", &log_file, 0);
    sandbox.write_exit_tool("greentic-operator", 0);

    let output = sandbox.run_gtc_capture(
        ["setup", "--dry-run", "--ignore-release-context"],
        HashMap::new(),
    );
    assert_eq!(output.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("release context"), "{stderr}");
    let logged = fs::read_to_string(log_file).expect("read setup log");
    assert!(logged.contains("--dry-run"));
    assert!(!logged.contains("--ignore-release-context"));
}

#[test]
fn wizard_answers_local_file_is_validated_and_forwarded() {
    let sandbox = TestSandbox::new("wizard_answers_local_file_is_validated_and_forwarded");
    let log_file = sandbox.path().join("wizard.log");
    let answers_file = sandbox.path().join("answers.json");
    fs::write(&answers_file, br#"{"selected_action":"exit"}"#).expect("write answers");
    sandbox.write_arg_logger_tool("greentic-dev", &log_file, 0);

    let status = sandbox.run_gtc(
        [
            "wizard",
            "--ignore-release-context",
            "--answers",
            answers_file.to_str().expect("answers path"),
        ],
        HashMap::new(),
    );
    assert_eq!(status.code(), Some(0));

    let logged = fs::read_to_string(log_file).expect("read wizard log");
    assert!(logged.contains(&format!("--answers {}", answers_file.display())));
    assert!(!logged.contains("--ignore-release-context"));
}

#[test]
fn setup_answers_file_url_is_validated_and_forwarded() {
    let sandbox = TestSandbox::new("setup_answers_file_url_is_validated_and_forwarded");
    let log_file = sandbox.path().join("setup.log");
    let answers_file = sandbox.path().join("setup-answers.json");
    fs::write(&answers_file, br#"{"setup":true}"#).expect("write answers");
    sandbox.write_arg_logger_tool("greentic-setup", &log_file, 0);

    let answers_url = format!("file://{}", answers_file.display());
    let status = sandbox.run_gtc(
        [
            "setup",
            "--ignore-release-context",
            "--answers",
            answers_url.as_str(),
        ],
        HashMap::new(),
    );
    assert_eq!(status.code(), Some(0));

    let logged = fs::read_to_string(log_file).expect("read setup log");
    assert!(logged.contains(&format!("--answers {answers_url}")));
    assert!(!logged.contains("--ignore-release-context"));
}

#[test]
fn wizard_answers_invalid_json_errors_before_passthrough() {
    let sandbox = TestSandbox::new("wizard_answers_invalid_json_errors_before_passthrough");
    let log_file = sandbox.path().join("wizard.log");
    let answers_file = sandbox.path().join("answers.json");
    fs::write(&answers_file, b"{not-json").expect("write answers");
    sandbox.write_arg_logger_tool("greentic-dev", &log_file, 0);

    let output = sandbox.run_gtc_capture(
        [
            "wizard",
            "--ignore-release-context",
            "--answers",
            answers_file.to_str().expect("answers path"),
        ],
        HashMap::new(),
    );

    assert_eq!(output.status.code(), Some(1));
    assert!(!log_file.exists(), "wizard should not run");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("failed to parse answers JSON"));
}

#[test]
fn setup_answers_invalid_scheme_errors_before_passthrough() {
    let sandbox = TestSandbox::new("setup_answers_invalid_scheme_errors_before_passthrough");
    let log_file = sandbox.path().join("setup.log");
    sandbox.write_arg_logger_tool("greentic-setup", &log_file, 0);

    let output = sandbox.run_gtc_capture(
        [
            "setup",
            "--ignore-release-context",
            "--answers",
            "ftp://example",
        ],
        HashMap::new(),
    );

    assert_eq!(output.status.code(), Some(2));
    assert!(!log_file.exists(), "setup should not run");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unsupported scheme"));
}

#[test]
fn wizard_schema_passthrough_emits_dev_schema() {
    let sandbox = TestSandbox::new("wizard_schema_passthrough_emits_dev_schema");
    let schema = serde_json::json!({
        "title": "greentic-dev launcher wizard answers",
        "type": "object",
        "properties": {
            "schema_id": { "const": "greentic-dev.launcher.main" },
            "answers": {
                "type": "object",
                "properties": {
                    "selected_action": { "enum": ["pack", "bundle"] },
                    "delegate_answer_document": {
                        "$ref": "#/$defs/greentic_pack_wizard_answers"
                    }
                }
            }
        },
        "$defs": {
            "greentic_pack_wizard_answers": {
                "type": "object",
                "$defs": {
                    "greentic_component_wizard_any_mode": {
                        "type": "object"
                    }
                },
                "properties": {
                    "component_wizard_answers": {
                        "$ref": "#/$defs/greentic_component_wizard_any_mode"
                    }
                }
            }
        }
    });
    sandbox.write_stdout_tool(
        "greentic-dev",
        &format!("{}\n", serde_json::to_string(&schema).expect("schema json")),
        0,
    );
    sandbox.write_exit_tool("greentic-operator", 0);

    let output = sandbox.run_gtc_capture(["wizard", "--schema"], HashMap::new());
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let emitted: serde_json::Value = serde_json::from_str(&stdout).expect("valid schema JSON");
    assert_eq!(
        emitted.get("title").and_then(serde_json::Value::as_str),
        Some("greentic-dev launcher wizard answers")
    );
    assert_eq!(
        emitted
            .pointer("/properties/schema_id/const")
            .and_then(serde_json::Value::as_str),
        Some("greentic-dev.launcher.main")
    );
    assert_eq!(
        emitted
            .pointer("/properties/answers/properties/selected_action/enum")
            .and_then(serde_json::Value::as_array)
            .expect("selected_action enum")
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect::<Vec<_>>(),
        vec!["pack", "bundle"]
    );
    assert_all_local_refs_exist(&emitted);
    assert_eq!(
        emitted
            .pointer("/$defs/greentic_pack_wizard_answers/properties/component_wizard_answers/$ref")
            .and_then(serde_json::Value::as_str),
        Some("#/$defs/greentic_pack_wizard_answers/$defs/greentic_component_wizard_any_mode")
    );
}

fn assert_all_local_refs_exist(schema: &serde_json::Value) {
    let mut missing = Vec::new();
    collect_missing_local_refs(schema, schema, "", &mut missing);
    assert!(
        missing.is_empty(),
        "schema contains missing local refs: {missing:?}"
    );
}

fn collect_missing_local_refs(
    root: &serde_json::Value,
    value: &serde_json::Value,
    path: &str,
    missing: &mut Vec<String>,
) {
    match value {
        serde_json::Value::Object(object) => {
            if let Some(reference) = object.get("$ref").and_then(serde_json::Value::as_str)
                && reference.starts_with("#/")
                && !json_pointer_exists(root, reference)
            {
                missing.push(format!("{path} -> {reference}"));
            }
            for (key, child) in object {
                let child_path = format!("{path}/{}", escape_json_pointer_segment(key));
                collect_missing_local_refs(root, child, &child_path, missing);
            }
        }
        serde_json::Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                let child_path = format!("{path}/{index}");
                collect_missing_local_refs(root, child, &child_path, missing);
            }
        }
        _ => {}
    }
}

fn json_pointer_exists(root: &serde_json::Value, reference: &str) -> bool {
    let Some(pointer) = reference.strip_prefix('#') else {
        return false;
    };
    root.pointer(pointer).is_some()
}

fn escape_json_pointer_segment(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

#[test]
fn wizard_emit_answers_writes_requested_file_via_dev_wizard() {
    let sandbox = TestSandbox::new("wizard_emit_answers_writes_requested_file_via_dev_wizard");
    let answers_path = sandbox.path().join("answers.json");
    sandbox.write_emit_answers_tool("greentic-dev");
    sandbox.write_exit_tool("greentic-operator", 99);

    let output = sandbox.run_gtc_capture(
        [
            "wizard",
            "--emit-answers",
            answers_path.to_str().expect("answers path utf8"),
        ],
        HashMap::new(),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(answers_path.exists(), "answers file should exist");

    let answers = fs::read_to_string(&answers_path).expect("read answers");
    assert!(
        !answers.trim().is_empty(),
        "answers file should be non-empty"
    );

    let json: serde_json::Value = serde_json::from_str(&answers).expect("valid answer document");
    assert_eq!(
        json.get("schema_version")
            .and_then(serde_json::Value::as_str),
        Some("1.0.0")
    );
    assert!(
        json.get("answers")
            .and_then(serde_json::Value::as_object)
            .is_some(),
        "answer document should include an answers object"
    );
    assert!(
        json.get("events")
            .and_then(serde_json::Value::as_array)
            .is_some(),
        "answer document should include an events array"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.trim().is_empty(),
        "launcher should pass through to greentic-dev without leaking extra stdout"
    );
}

#[test]
fn debug_router_shows_wizard_routes_to_greentic_dev() {
    let sandbox = TestSandbox::new("debug_router_shows_wizard_routes_to_greentic_dev");
    sandbox.write_exit_tool("greentic-dev", 0);
    sandbox.write_exit_tool("greentic-operator", 99);

    let output = sandbox.run_gtc_capture(["--debug-router", "wizard", "--help"], HashMap::new());
    assert_eq!(output.status.code(), Some(0));

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Router exec: greentic-dev"));
    assert!(stderr.contains("[\"wizard\", \"--locale\", \"en\", \"--help\"]"));
}

#[test]
fn setup_help_passthrough_routes_to_greentic_setup() {
    let sandbox = TestSandbox::new("setup_help_passthrough_routes_to_greentic_setup");
    let log_file = sandbox.path().join("setup.log");
    sandbox.write_arg_logger_tool("greentic-setup", &log_file, 0);
    sandbox.write_exit_tool("greentic-dev", 0);
    sandbox.write_exit_tool("greentic-operator", 0);

    let status = sandbox.run_gtc(["setup", "--help"], HashMap::new());
    assert_eq!(status.code(), Some(0));

    let logged = fs::read_to_string(log_file).expect("read setup log");
    assert!(logged.contains("--help"));
}

#[test]
fn provider_passthrough_routes_to_greentic_setup_with_provider_token() {
    let sandbox =
        TestSandbox::new("provider_passthrough_routes_to_greentic_setup_with_provider_token");
    let log_file = sandbox.path().join("setup.log");
    sandbox.write_arg_logger_tool("greentic-setup", &log_file, 0);
    sandbox.write_exit_tool("greentic-dev", 0);
    sandbox.write_exit_tool("greentic-operator", 0);

    let status = sandbox.run_gtc(["provider", "add", "telegram"], HashMap::new());
    assert_eq!(status.code(), Some(0));

    let logged = fs::read_to_string(log_file).expect("read setup log");
    // greentic-setup must receive `provider add telegram` — the `provider`
    // token is re-prepended by the router (same pattern as `worker`).
    assert!(
        logged
            .split_whitespace()
            .eq(["provider", "add", "telegram"]),
        "expected greentic-setup to receive `provider add telegram`, got: {logged}"
    );
}

#[test]
fn provider_passthrough_preserves_exit_code() {
    let sandbox = TestSandbox::new("provider_passthrough_preserves_exit_code");
    sandbox.write_exit_tool("greentic-setup", 42);
    sandbox.write_exit_tool("greentic-dev", 0);
    sandbox.write_exit_tool("greentic-operator", 0);

    let status = sandbox.run_gtc(["provider", "list"], HashMap::new());
    assert_eq!(status.code(), Some(42));
}

#[test]
fn op_help_passthrough_routes_to_greentic_operator() {
    let sandbox = TestSandbox::new("op_help_passthrough_routes_to_greentic_operator");
    let log_file = sandbox.path().join("op.log");
    sandbox.write_arg_logger_tool("greentic-operator", &log_file, 0);
    sandbox.write_exit_tool("greentic-dev", 0);

    let status = sandbox.run_gtc(["op", "--help"], HashMap::new());
    assert_eq!(status.code(), Some(0));

    let logged = fs::read_to_string(log_file).expect("read op log");
    assert!(
        logged.split_whitespace().eq(["op", "--help"]),
        "expected greentic-operator to receive `op --help`, got: {logged}"
    );
}

// `gtc op env init` must forward as `greentic-operator op env init` — `env` is
// a noun under `OpCommand`, not a top-level operator subcommand. Regression
// guard for the routing bug where the leading `op` was stripped before
// forwarding, leaving `greentic-operator env init` and "unrecognized
// subcommand 'env'".
#[test]
fn op_deploy_spec_noun_routes_under_op() {
    let sandbox = TestSandbox::new("op_deploy_spec_noun_routes_under_op");
    let log_file = sandbox.path().join("op.log");
    sandbox.write_arg_logger_tool("greentic-operator", &log_file, 0);
    sandbox.write_exit_tool("greentic-dev", 0);

    let status = sandbox.run_gtc(["op", "env", "init"], HashMap::new());
    assert_eq!(status.code(), Some(0));

    let logged = fs::read_to_string(log_file).expect("read op log");
    assert!(
        logged.split_whitespace().eq(["op", "env", "init"]),
        "expected greentic-operator to receive `op env init`, got: {logged}"
    );
}

// Legacy `gtc op demo build` must still forward as-is (the operator's `Demo`
// top-level subcommand handles it directly).
#[test]
fn op_demo_subcommand_passes_through_unchanged() {
    let sandbox = TestSandbox::new("op_demo_subcommand_passes_through_unchanged");
    let log_file = sandbox.path().join("op.log");
    sandbox.write_arg_logger_tool("greentic-operator", &log_file, 0);
    sandbox.write_exit_tool("greentic-dev", 0);

    let status = sandbox.run_gtc(["op", "demo", "build"], HashMap::new());
    assert_eq!(status.code(), Some(0));

    let logged = fs::read_to_string(log_file).expect("read op log");
    assert!(
        logged.split_whitespace().eq(["demo", "build"]),
        "expected greentic-operator to receive `demo build`, got: {logged}"
    );
}

// Documented `gtc op op env init` double-op workaround must keep working — the
// leading `op` is the user-typed explicit selector, not a strip target.
#[test]
fn op_double_op_workaround_passes_through_unchanged() {
    let sandbox = TestSandbox::new("op_double_op_workaround_passes_through_unchanged");
    let log_file = sandbox.path().join("op.log");
    sandbox.write_arg_logger_tool("greentic-operator", &log_file, 0);
    sandbox.write_exit_tool("greentic-dev", 0);

    let status = sandbox.run_gtc(["op", "op", "env", "init"], HashMap::new());
    assert_eq!(status.code(), Some(0));

    let logged = fs::read_to_string(log_file).expect("read op log");
    assert!(
        logged.split_whitespace().eq(["op", "env", "init"]),
        "expected greentic-operator to receive `op env init`, got: {logged}"
    );
}

#[test]
fn gtc_dev_wizard_routes_to_dev_suffixed_greentic_dev() {
    let sandbox = TestSandbox::new("gtc_dev_wizard_routes_to_dev_suffixed_greentic_dev");
    let log_file = sandbox.path().join("wizard-dev.log");
    let answers_file = sandbox.path().join("answers.json");
    fs::write(&answers_file, br#"{"ok":true}"#).expect("write answers");
    sandbox.write_arg_logger_tool("greentic-dev-dev", &log_file, 0);

    let output = sandbox.run_gtc_dev_capture(
        [
            "wizard",
            "--locale",
            "fr",
            "--answers",
            answers_file.to_str().expect("answers path"),
        ],
        HashMap::new(),
    );
    assert_eq!(output.status.code(), Some(0));

    let logged = fs::read_to_string(log_file).expect("read wizard log");
    assert!(logged.contains(&format!(
        "wizard --locale fr --answers {}",
        answers_file.display()
    )));
}

#[test]
fn gtc_dev_dev_routes_to_dev_suffixed_greentic_dev() {
    let sandbox = TestSandbox::new("gtc_dev_dev_routes_to_dev_suffixed_greentic_dev");
    let log_file = sandbox.path().join("dev-dev.log");
    sandbox.write_arg_logger_tool("greentic-dev-dev", &log_file, 0);

    let output = sandbox.run_gtc_dev_capture(["dev", "flow", "list"], HashMap::new());
    assert_eq!(output.status.code(), Some(0));

    let logged = fs::read_to_string(log_file).expect("read dev log");
    assert!(logged.contains("flow list"));
}

#[test]
fn gtc_dev_op_routes_to_dev_suffixed_operator() {
    let sandbox = TestSandbox::new("gtc_dev_op_routes_to_dev_suffixed_operator");
    let log_file = sandbox.path().join("op-dev.log");
    sandbox.write_arg_logger_tool("greentic-operator-dev", &log_file, 0);

    let output = sandbox.run_gtc_dev_capture(["op", "--help"], HashMap::new());
    assert_eq!(output.status.code(), Some(0));

    let logged = fs::read_to_string(log_file).expect("read op log");
    assert!(logged.contains("--help"));
}

#[test]
fn gtc_dev_doctor_checks_dev_suffixed_companions() {
    let sandbox = TestSandbox::new("gtc_dev_doctor_checks_dev_suffixed_companions");
    sandbox.write_version_tool("greentic-dev-dev", "greentic-dev-dev 0.0.0");
    sandbox.write_version_tool("greentic-operator-dev", "greentic-operator-dev 0.0.0");
    sandbox.write_version_tool("greentic-bundle-dev", "greentic-bundle-dev 0.0.0");
    sandbox.write_version_tool("greentic-component-dev", "greentic-component-dev 0.0.0");
    sandbox.write_version_tool("greentic-flow-dev", "greentic-flow-dev 0.0.0");
    sandbox.write_version_tool("greentic-pack-dev", "greentic-pack-dev 0.0.0");
    sandbox.write_version_tool("greentic-runner-dev", "greentic-runner-dev 0.0.0");
    sandbox.write_version_tool("greentic-secrets-dev", "greentic-secrets-dev 0.0.0");
    sandbox.write_version_tool("greentic-setup-dev", "greentic-setup-dev 0.0.0");
    sandbox.write_version_tool("greentic-start-dev", "greentic-start-dev 0.0.0");
    sandbox.write_version_tool("greentic-deployer-dev", "greentic-deployer-dev 0.0.0");

    let output = sandbox.run_gtc_dev_capture(["doctor"], HashMap::new());
    assert_eq!(output.status.code(), Some(0));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("greentic-dev: OK (greentic-dev-dev 0.0.0)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("greentic-flow: OK (greentic-flow-dev 0.0.0)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("greentic-pack: OK (greentic-pack-dev 0.0.0)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("greentic-runner: OK (greentic-runner-dev 0.0.0)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("greentic-deployer: OK (greentic-deployer-dev 0.0.0)"),
        "{stdout}"
    );
}

#[test]
fn doctor_uses_greentic_dev_bin_override() {
    let sandbox = TestSandbox::new("doctor_uses_greentic_dev_bin_override");
    let override_bin = sandbox.path().join("bin").join("greentic-dev-local");
    fs::create_dir_all(override_bin.parent().expect("override parent")).expect("override dir");
    sandbox.compile_rust_tool_at(
        &override_bin,
        &rust_version_tool_program("greentic-dev local-test"),
    );
    sandbox.write_exit_tool("greentic-dev", 92);
    sandbox.write_version_tool("greentic-operator", "greentic-operator 0.0.0");
    sandbox.write_version_tool("greentic-bundle", "greentic-bundle 0.0.0");
    sandbox.write_version_tool("greentic-component", "greentic-component 0.0.0");
    sandbox.write_version_tool("greentic-flow", "greentic-flow 0.0.0");
    sandbox.write_version_tool("greentic-pack", "greentic-pack 0.0.0");
    sandbox.write_version_tool("greentic-runner", "greentic-runner 0.0.0");
    sandbox.write_version_tool("greentic-secrets", "greentic-secrets 0.0.0");
    sandbox.write_version_tool("greentic-setup", "greentic-setup 0.0.0");
    sandbox.write_version_tool("greentic-start", "greentic-start 0.0.0");
    sandbox.write_version_tool("greentic-deployer", "greentic-deployer 0.0.0");

    let mut extra = HashMap::new();
    let state_dir = sandbox.path().join("toolchain-state");
    write_installed_toolchain_state(&state_dir, "1.0.4", "stable", "sha256:testdigest");
    extra.insert(
        "GTC_TOOLCHAIN_STATE_DIR".to_string(),
        state_dir.display().to_string(),
    );
    extra.insert(
        "GREENTIC_DEV_BIN".to_string(),
        override_bin.display().to_string(),
    );

    let output = sandbox.run_gtc_capture(["doctor"], extra);
    assert_eq!(output.status.code(), Some(0));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Greentic toolchain release: 1.0.4 (stable) [sha256:testdigest]"));
    assert!(stdout.contains("greentic-dev: OK (greentic-dev local-test)"));
    assert!(stdout.contains("greentic-bundle: OK (greentic-bundle 0.0.0)"));
    assert!(stdout.contains("greentic-component: OK (greentic-component 0.0.0)"));
    assert!(stdout.contains("greentic-flow: OK (greentic-flow 0.0.0)"));
    assert!(stdout.contains("greentic-pack: OK (greentic-pack 0.0.0)"));
    assert!(stdout.contains("greentic-runner: OK (greentic-runner 0.0.0)"));
    assert!(stdout.contains("greentic-secrets: OK (greentic-secrets 0.0.0)"));
}

#[test]
fn doctor_prints_stable_packs_and_components_from_release_index() {
    let sandbox = TestSandbox::new("doctor_prints_stable_packs_and_components_from_release_index");
    sandbox.write_version_tool("greentic-dev", "greentic-dev 0.0.0");
    sandbox.write_version_tool("greentic-operator", "greentic-operator 0.0.0");
    sandbox.write_version_tool("greentic-bundle", "greentic-bundle 0.0.0");
    sandbox.write_version_tool("greentic-component", "greentic-component 0.0.0");
    sandbox.write_version_tool("greentic-flow", "greentic-flow 0.0.0");
    sandbox.write_version_tool("greentic-pack", "greentic-pack 0.0.0");
    sandbox.write_version_tool("greentic-runner", "greentic-runner 0.0.0");
    sandbox.write_version_tool("greentic-secrets", "greentic-secrets 0.0.0");
    sandbox.write_version_tool("greentic-setup", "greentic-setup 0.0.0");
    sandbox.write_version_tool("greentic-start", "greentic-start 0.0.0");
    sandbox.write_version_tool("greentic-deployer", "greentic-deployer 0.0.0");

    let state_dir = sandbox.path().join("toolchain-state");
    let cache_dir = sandbox.path().join("cache");
    write_installed_toolchain_state(&state_dir, "1.0.4", "stable", "sha256:testdigest");
    write_test_release_index(&cache_dir, "stable", "1.0.4");

    let mut extra = HashMap::new();
    extra.insert(
        "GTC_TOOLCHAIN_STATE_DIR".to_string(),
        state_dir.display().to_string(),
    );
    extra.insert(
        "GREENTIC_CACHE_DIR".to_string(),
        cache_dir.display().to_string(),
    );

    let output = sandbox.run_gtc_capture(["doctor"], extra);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Greentic release artifacts: 1.0.4 (stable)"));
    assert!(stdout.contains("stable packs:"));
    assert!(stdout.contains("ghcr.io/greenticai/packs/messaging/messaging-webchat-gui:stable"));
    assert!(stdout.contains("stable components:"));
    assert!(stdout.contains("ghcr.io/greenticai/components/templates:stable"));
}

#[test]
fn op_setup_routes_to_demo_setup_with_default_tenant_team() {
    let sandbox = TestSandbox::new("op_setup_routes_to_demo_setup_with_default_tenant_team");
    let log_file = sandbox.path().join("op.log");
    sandbox.write_exit_tool("greentic-dev", 0);
    sandbox.write_arg_logger_tool("greentic-operator", &log_file, 0);

    let status = sandbox.run_gtc(
        ["op", "setup", "--bundle", "./myfirst.gtbundle"],
        HashMap::new(),
    );
    assert_eq!(status.code(), Some(0));

    let logged = fs::read_to_string(log_file).expect("read op log");
    assert!(logged.contains("demo setup --bundle ./myfirst.gtbundle"));
    assert!(logged.contains("--tenant default"));
    assert!(logged.contains("--team default"));
}

#[test]
fn op_start_routes_to_demo_start_with_default_tenant_team_and_cloudflared_off() {
    let sandbox = TestSandbox::new(
        "op_start_routes_to_demo_start_with_default_tenant_team_and_cloudflared_off",
    );
    let log_file = sandbox.path().join("op.log");

    sandbox.write_exit_tool("greentic-dev", 0);
    sandbox.write_arg_logger_tool("greentic-operator", &log_file, 0);

    let status = sandbox.run_gtc(
        ["op", "start", "--bundle", "./myfirst.gtbundle"],
        HashMap::new(),
    );
    assert_eq!(status.code(), Some(0));

    let logged = fs::read_to_string(log_file).expect("read op log");
    assert!(logged.contains("demo start --bundle ./myfirst.gtbundle"));
    assert!(logged.contains("--tenant default"));
    assert!(logged.contains("--team default"));
    assert!(logged.contains("--cloudflared off"));
}

/// B4b: `gtc start <bundle>` no longer boots the legacy bundle-scoped ingress.
/// It deploys the bundle into the environment via `greentic-setup env-deploy`,
/// then starts greentic-start *bundle-less* so the env/revision runtime serves
/// it (static assets, DirectLine, WebSocket, CORS — none of which the legacy
/// ingress has on the revision path).
///
/// The `--bundle` assertions here are inverted on purpose: a `--bundle` on the
/// start argv means the reroute regressed and the runtime silently loses the
/// webchat GUI's socket. That is the whole point of this test.
#[test]
fn runtime_start_deploys_bundle_into_env_then_starts_bundle_less() {
    let sandbox = TestSandbox::new("runtime_start_deploys_bundle_into_env_then_starts_bundle_less");
    let bundle_dir = sandbox.path().join("bundle");
    create_minimal_bundle_dir(&bundle_dir);
    let start_log = sandbox.path().join("start.log");
    let setup_log = sandbox.path().join("setup.log");

    sandbox.write_exit_tool("greentic-dev", 0);
    sandbox.write_bundle_build_tool("greentic-bundle", &sandbox.path().join("bundle.log"));
    sandbox.write_arg_logger_tool("greentic-setup", &setup_log, 0);
    sandbox.write_arg_logger_tool("greentic-start", &start_log, 0);

    let output = sandbox.run_gtc_capture(
        [
            "start",
            bundle_dir.to_str().expect("bundle utf8"),
            "--target",
            "runtime",
            "--tenant",
            "demo",
            "--team",
            "ops",
            "--cloudflared=off",
            "--admin",
        ],
        HashMap::new(),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // The bundle is deployed into the env before anything is served.
    let deployed = fs::read_to_string(&setup_log).expect("read setup log");
    assert!(
        deployed.contains("env-deploy"),
        "greentic-setup must be invoked with env-deploy; got: {deployed}"
    );
    assert!(
        deployed.contains("--env local"),
        "env-deploy must target the resolved env; got: {deployed}"
    );

    // greentic-start is then booted bundle-less against that same env.
    let logged = fs::read_to_string(&start_log).expect("read start log");
    assert!(logged.contains("--locale en start"));
    assert!(
        !logged.contains("--bundle"),
        "start must be bundle-less so the env/revision runtime serves the \
         revision (a --bundle here silently drops the webchat WebSocket); got: {logged}"
    );
    assert!(
        logged.contains("--env local"),
        "start must be pinned to the same env the bundle was deployed into; got: {logged}"
    );
    assert!(logged.contains("--tenant demo"));
    assert!(logged.contains("--team ops"));
    assert!(logged.contains("--cloudflared off"));
    assert!(logged.contains("--admin"));
}

/// `--admin` generates a dev CA plus server and client PRIVATE keys. Under the
/// legacy path those lived in a tempdir that died on shutdown. B4b hands the
/// prepared root to `env-deploy`, which stages a PERSISTENT copy — and an
/// environment store can be remote. The keys must never enter the deployed
/// revision; greentic-start reads them from the source bundle via
/// `--admin-certs-dir` instead.
#[test]
fn admin_private_keys_are_not_staged_into_the_environment() {
    let sandbox = TestSandbox::new("admin_private_keys_are_not_staged_into_the_environment");
    let bundle_dir = sandbox.path().join("bundle");
    create_minimal_bundle_dir(&bundle_dir);
    let start_log = sandbox.path().join("start.log");
    let setup_log = sandbox.path().join("setup.log");

    sandbox.write_exit_tool("greentic-dev", 0);
    sandbox.write_bundle_build_tool("greentic-bundle", &sandbox.path().join("bundle.log"));
    sandbox.write_env_deploy_probe_tool("greentic-setup", &setup_log);
    sandbox.write_arg_logger_tool("greentic-start", &start_log, 0);

    let output = sandbox.run_gtc_capture(
        [
            "start",
            bundle_dir.to_str().expect("bundle utf8"),
            "--target",
            "runtime",
            "--admin",
        ],
        HashMap::new(),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let deployed = fs::read_to_string(&setup_log).expect("read setup log");
    assert!(
        deployed.contains("env-deploy"),
        "env-deploy must run; got: {deployed}"
    );
    assert!(
        !deployed.contains("LEAKED_PRIVATE_KEY"),
        "admin private keys must never be staged into the environment store \
         (it can be remote); probe reported: {deployed}"
    );
    assert!(
        deployed.contains("ADMIN_DIR_IN_DEPLOYED_BUNDLE=false"),
        "the deployed bundle must carry no .greentic/admin directory; got: {deployed}"
    );

    // The admin server still gets its certs — from the source bundle, which is
    // never handed to env-deploy.
    let served = fs::read_to_string(&start_log).expect("read start log");
    assert!(served.contains("--admin"), "admin mode still enabled");
    let bundle_str = bundle_dir.to_str().expect("bundle utf8");
    assert!(
        served.contains(&format!("--admin-certs-dir {bundle_str}")),
        "certs must be served from the source bundle, not the prepared root; got: {served}"
    );
}

/// The env-deploy step and the serving process must agree on one env, even if
/// `$GREENTIC_ENV` changes between the two spawns — so `gtc` resolves it once
/// and pins it explicitly on both.
#[test]
fn runtime_start_pins_explicit_env_on_both_deploy_and_serve() {
    let sandbox = TestSandbox::new("runtime_start_pins_explicit_env_on_both_deploy_and_serve");
    let bundle_dir = sandbox.path().join("bundle");
    create_minimal_bundle_dir(&bundle_dir);
    let start_log = sandbox.path().join("start.log");
    let setup_log = sandbox.path().join("setup.log");

    sandbox.write_exit_tool("greentic-dev", 0);
    sandbox.write_bundle_build_tool("greentic-bundle", &sandbox.path().join("bundle.log"));
    sandbox.write_arg_logger_tool("greentic-setup", &setup_log, 0);
    sandbox.write_arg_logger_tool("greentic-start", &start_log, 0);

    let output = sandbox.run_gtc_capture(
        [
            "start",
            bundle_dir.to_str().expect("bundle utf8"),
            "--target",
            "runtime",
            "--env",
            "staging",
        ],
        HashMap::new(),
    );
    assert_eq!(output.status.code(), Some(0));

    let deployed = fs::read_to_string(&setup_log).expect("read setup log");
    let served = fs::read_to_string(&start_log).expect("read start log");
    assert!(
        deployed.contains("--env staging"),
        "env-deploy must honor --env; got: {deployed}"
    );
    assert!(
        served.contains("--env staging"),
        "serve must honor --env; got: {served}"
    );
}

#[test]
fn install_public_mode_installs_manifest_toolchain() {
    let sandbox = TestSandbox::new("install_public_mode_installs_manifest_toolchain");
    let log_file = sandbox.path().join("dev.log");
    let cargo_log_file = sandbox.path().join("cargo.log");
    let cargo_home = sandbox.path().join("cargo-home");
    let manifest_path = write_toolchain_manifest(sandbox.path());

    sandbox.write_arg_logger_tool("greentic-dev", &log_file, 0);
    sandbox.write_exit_tool("greentic-operator", 0);
    sandbox.write_cargo_binstall_tool(&cargo_log_file, None);
    sandbox.write_install_prereq_tools();
    fs::create_dir_all(&cargo_home).expect("cargo_home");

    let mut extra = HashMap::new();
    extra.insert("CARGO_HOME".to_string(), cargo_home.display().to_string());
    extra.insert(
        "GTC_TOOLCHAIN_STATE_DIR".to_string(),
        sandbox.path().join("toolchain-state").display().to_string(),
    );

    let output = sandbox.run_gtc_capture(
        [
            "install",
            "--manifest",
            manifest_path.to_str().expect("utf8"),
        ],
        extra,
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let logged = fs::read_to_string(log_file).unwrap_or_default();
    assert!(!logged.contains("install tools"));
    let cargo_logged = fs::read_to_string(cargo_log_file).unwrap_or_default();
    assert!(cargo_logged.contains("binstall -V"));
    assert!(cargo_logged.contains("search cargo-binstall --limit 1"));
    assert!(
        cargo_logged.contains(
            "binstall -y --locked --force --maximum-resolution-timeout 60 greentic-dev --version 0.5.9 --bin greentic-dev"
        )
    );
    assert!(cargo_logged.contains(
        "binstall -y --locked --force --maximum-resolution-timeout 60 greentic-runner --version 0.5.10 --bin greentic-runner"
    ));
    assert!(cargo_logged.contains(
        "binstall -y --locked --force --maximum-resolution-timeout 60 greentic-runner --version 0.5.10 --bin greentic-runner-cli"
    ));
}

#[test]
fn install_release_prefetches_artifacts_and_writes_release_index() {
    let sandbox = TestSandbox::new("install_release_prefetches_artifacts_and_writes_release_index");
    let cargo_log_file = sandbox.path().join("cargo.log");
    let cargo_home = sandbox.path().join("cargo-home");
    let cache_dir = sandbox.path().join("dist-cache");
    let release_state_dir = sandbox.path().join("release-state");
    let manifest_path = write_release_toolchain_manifest(sandbox.path());
    let mock_root = write_release_artifact_mock_root(sandbox.path());

    sandbox.write_exit_tool("greentic-dev", 0);
    sandbox.write_exit_tool("greentic-operator", 0);
    sandbox.write_contract_deployer_tool("greentic-deployer");
    sandbox.write_cargo_binstall_tool(&cargo_log_file, None);
    sandbox.write_install_prereq_tools();
    fs::create_dir_all(&cargo_home).expect("cargo_home");

    let mut extra = HashMap::new();
    extra.insert("CARGO_HOME".to_string(), cargo_home.display().to_string());
    extra.insert(
        "GTC_TOOLCHAIN_MANIFEST_PATH".to_string(),
        manifest_path.display().to_string(),
    );
    extra.insert(
        "GTC_TOOLCHAIN_STATE_DIR".to_string(),
        sandbox.path().join("toolchain-state").display().to_string(),
    );
    extra.insert(
        "GTC_RELEASE_STATE_DIR".to_string(),
        release_state_dir.display().to_string(),
    );
    extra.insert(
        "GTC_RELEASE_ARTIFACT_MOCK_ROOT".to_string(),
        mock_root.display().to_string(),
    );
    extra.insert(
        "GREENTIC_CACHE_DIR".to_string(),
        cache_dir.display().to_string(),
    );
    extra.insert(
        "GREENTIC_DEPLOYER_BIN".to_string(),
        sandbox
            .path()
            .join("missing-greentic-deployer")
            .display()
            .to_string(),
    );

    let output = sandbox.run_gtc_capture(
        ["install", "--release", "1.0.16", "--channel", "stable"],
        extra,
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Prefetching pack packs/messaging/messaging-webchat-gui:0.5.4"));
    assert!(stdout.contains("Prefetched component components/templates:0.5.8 -> sha256:"));

    let index_path = cache_dir.join("release-index/v1/stable/1.0.16.json");
    let index: serde_json::Value =
        serde_json::from_slice(&fs::read(&index_path).expect("read release index"))
            .expect("parse release index");
    assert_eq!(index["schema"], "greentic.release-index.v1");
    assert_eq!(index["release"], "1.0.16");
    assert_eq!(index["channel"], "stable");

    let refs = index["refs"].as_object().expect("refs object");
    for key in [
        "ghcr.io/greenticai/packs/messaging/messaging-webchat-gui:stable",
        "ghcr.io/greenticai/components/templates:stable",
    ] {
        let entry = refs.get(key).unwrap_or_else(|| panic!("missing {key}"));
        let digest = entry["digest"].as_str().expect("digest");
        assert!(digest.starts_with("sha256:"));
        assert!(
            entry["canonical_ref"]
                .as_str()
                .expect("canonical ref")
                .starts_with("oci://ghcr.io/greenticai/")
        );
        let hex = digest.trim_start_matches("sha256:");
        let blob = cache_dir
            .join("artifacts/sha256")
            .join(&hex[..2])
            .join(&hex[2..])
            .join("blob");
        let cache_entry = blob.with_file_name("entry.json");
        assert!(blob.is_file(), "missing blob for {digest}");
        assert!(cache_entry.is_file(), "missing entry for {digest}");
    }

    let current: serde_json::Value = serde_json::from_slice(
        &fs::read(release_state_dir.join("current.json")).expect("read current context"),
    )
    .expect("parse current context");
    assert_eq!(current["release"], "1.0.16");
    assert_eq!(current["channel"], "stable");
}

#[test]
fn release_cache_export_then_import_restores_index_and_artifacts() {
    let sandbox = TestSandbox::new("release_cache_export_then_import_restores_index_and_artifacts");
    let cargo_log_file = sandbox.path().join("cargo.log");
    let cargo_home = sandbox.path().join("cargo-home");
    let source_cache = sandbox.path().join("source-cache");
    let import_cache = sandbox.path().join("import-cache");
    let release_state_dir = sandbox.path().join("release-state");
    let manifest_path = write_release_toolchain_manifest(sandbox.path());
    let mock_root = write_release_artifact_mock_root(sandbox.path());
    let archive_path = sandbox.path().join("release-cache.tar.gz");

    sandbox.write_exit_tool("greentic-dev", 0);
    sandbox.write_exit_tool("greentic-operator", 0);
    sandbox.write_contract_deployer_tool("greentic-deployer");
    sandbox.write_cargo_binstall_tool(&cargo_log_file, None);
    sandbox.write_install_prereq_tools();
    fs::create_dir_all(&cargo_home).expect("cargo_home");

    let mut install_env = HashMap::new();
    install_env.insert("CARGO_HOME".to_string(), cargo_home.display().to_string());
    install_env.insert(
        "GTC_TOOLCHAIN_MANIFEST_PATH".to_string(),
        manifest_path.display().to_string(),
    );
    install_env.insert(
        "GTC_TOOLCHAIN_STATE_DIR".to_string(),
        sandbox.path().join("toolchain-state").display().to_string(),
    );
    install_env.insert(
        "GTC_RELEASE_STATE_DIR".to_string(),
        release_state_dir.display().to_string(),
    );
    install_env.insert(
        "GTC_RELEASE_ARTIFACT_MOCK_ROOT".to_string(),
        mock_root.display().to_string(),
    );
    install_env.insert(
        "GREENTIC_CACHE_DIR".to_string(),
        source_cache.display().to_string(),
    );

    let install = sandbox.run_gtc_capture(
        ["install", "--release", "1.0.16", "--channel", "stable"],
        install_env,
    );
    assert_eq!(
        install.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&install.stdout),
        String::from_utf8_lossy(&install.stderr)
    );

    let mut export_env = HashMap::new();
    export_env.insert(
        "GREENTIC_CACHE_DIR".to_string(),
        source_cache.display().to_string(),
    );
    let export = sandbox.run_gtc_capture(
        [
            "release-cache",
            "export",
            "--release",
            "1.0.16",
            "--channel",
            "stable",
            "--output",
            archive_path.to_str().expect("archive utf8"),
        ],
        export_env,
    );
    assert_eq!(
        export.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&export.stdout),
        String::from_utf8_lossy(&export.stderr)
    );
    assert!(archive_path.is_file());

    let mut import_env = HashMap::new();
    import_env.insert(
        "GREENTIC_CACHE_DIR".to_string(),
        import_cache.display().to_string(),
    );
    let import = sandbox.run_gtc_capture(
        [
            "release-cache",
            "import",
            "--input",
            archive_path.to_str().expect("archive utf8"),
        ],
        import_env,
    );
    assert_eq!(
        import.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&import.stdout),
        String::from_utf8_lossy(&import.stderr)
    );

    let index_path = import_cache.join("release-index/v1/stable/1.0.16.json");
    let index: serde_json::Value =
        serde_json::from_slice(&fs::read(&index_path).expect("read imported index"))
            .expect("parse imported index");
    let refs = index["refs"].as_object().expect("refs object");
    assert_eq!(refs.len(), 2);
    for entry in refs.values() {
        let digest = entry["digest"].as_str().expect("digest");
        let hex = digest.trim_start_matches("sha256:");
        let blob = import_cache
            .join("artifacts/sha256")
            .join(&hex[..2])
            .join(&hex[2..])
            .join("blob");
        assert!(blob.is_file(), "missing imported blob for {digest}");
        assert!(
            blob.with_file_name("entry.json").is_file(),
            "missing imported entry for {digest}"
        );
    }

    assert_imported_cache_resolves_stable_offline(&import_cache, &index);
}

#[test]
fn install_release_prefetch_is_idempotent() {
    let sandbox = TestSandbox::new("install_release_prefetch_is_idempotent");
    let cargo_log_file = sandbox.path().join("cargo.log");
    let cargo_home = sandbox.path().join("cargo-home");
    let cache_dir = sandbox.path().join("dist-cache");
    let release_state_dir = sandbox.path().join("release-state");
    let manifest_path = write_release_toolchain_manifest(sandbox.path());
    let mock_root = write_release_artifact_mock_root(sandbox.path());
    let broken_mock_root = write_release_artifact_mock_root_missing_component(sandbox.path());

    sandbox.write_exit_tool("greentic-dev", 0);
    sandbox.write_exit_tool("greentic-operator", 0);
    sandbox.write_contract_deployer_tool("greentic-deployer");
    sandbox.write_cargo_binstall_tool(&cargo_log_file, None);
    sandbox.write_install_prereq_tools();
    fs::create_dir_all(&cargo_home).expect("cargo_home");

    let mut extra = release_install_env(
        sandbox.path(),
        &cargo_home,
        &manifest_path,
        &release_state_dir,
        &mock_root,
        &cache_dir,
    );

    let first = sandbox.run_gtc_capture(
        ["install", "--release", "1.0.16", "--channel", "stable"],
        extra.clone(),
    );
    assert_eq!(
        first.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );

    let index_path = cache_dir.join("release-index/v1/stable/1.0.16.json");
    let first_index = fs::read(&index_path).expect("read first index");

    extra.insert(
        "GTC_RELEASE_ARTIFACT_MOCK_ROOT".to_string(),
        broken_mock_root.display().to_string(),
    );
    let second = sandbox.run_gtc_capture(
        ["install", "--release", "1.0.16", "--channel", "stable"],
        extra,
    );
    assert_eq!(
        second.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
    let stdout = String::from_utf8_lossy(&second.stdout);
    assert!(stdout.contains("Cached pack packs/messaging/messaging-webchat-gui:0.5.4"));
    assert!(stdout.contains("Cached component components/templates:0.5.8"));
    let second_index = fs::read(&index_path).expect("read second index");
    assert_eq!(first_index, second_index);
}

#[test]
fn install_skips_binary_when_installed_version_matches() {
    let sandbox = TestSandbox::new("install_skips_binary_when_installed_version_matches");
    let cargo_log_file = sandbox.path().join("cargo.log");
    let cargo_home = sandbox.path().join("cargo-home");
    let manifest_path = write_single_binary_toolchain_manifest(sandbox.path());

    sandbox.write_version_tool("greentic-dev", "greentic-dev 0.5.9\n");
    sandbox.write_contract_deployer_tool("greentic-deployer");
    sandbox.write_cargo_binstall_tool(&cargo_log_file, None);
    sandbox.write_install_prereq_tools();
    fs::create_dir_all(&cargo_home).expect("cargo_home");

    let mut extra = HashMap::new();
    extra.insert("CARGO_HOME".to_string(), cargo_home.display().to_string());
    extra.insert(
        "GTC_TOOLCHAIN_MANIFEST_PATH".to_string(),
        manifest_path.display().to_string(),
    );
    extra.insert(
        "GTC_TOOLCHAIN_STATE_DIR".to_string(),
        sandbox.path().join("toolchain-state").display().to_string(),
    );
    extra.insert(
        "GREENTIC_CACHE_DIR".to_string(),
        sandbox.path().join("dist-cache").display().to_string(),
    );

    let output = sandbox.run_gtc_capture(
        ["install", "--release", "1.0.16", "--channel", "stable"],
        extra,
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Installed greentic-dev binary already matches 0.5.9; skipping."));
    let cargo_logged = fs::read_to_string(cargo_log_file).unwrap_or_default();
    assert!(!cargo_logged.contains("greentic-dev --version 0.5.9"));
}

#[test]
fn install_partial_prefetch_failure_preserves_previous_release_index() {
    let sandbox =
        TestSandbox::new("install_partial_prefetch_failure_preserves_previous_release_index");
    let cargo_log_file = sandbox.path().join("cargo.log");
    let cargo_home = sandbox.path().join("cargo-home");
    let cache_dir = sandbox.path().join("dist-cache");
    let release_state_dir = sandbox.path().join("release-state");
    let manifest_path = write_release_toolchain_manifest(sandbox.path());
    let mock_root = write_release_artifact_mock_root(sandbox.path());
    let broken_mock_root = write_release_artifact_mock_root_missing_component(sandbox.path());

    sandbox.write_exit_tool("greentic-dev", 0);
    sandbox.write_exit_tool("greentic-operator", 0);
    sandbox.write_contract_deployer_tool("greentic-deployer");
    sandbox.write_cargo_binstall_tool(&cargo_log_file, None);
    sandbox.write_install_prereq_tools();
    fs::create_dir_all(&cargo_home).expect("cargo_home");

    let first = sandbox.run_gtc_capture(
        ["install", "--release", "1.0.16", "--channel", "stable"],
        release_install_env(
            sandbox.path(),
            &cargo_home,
            &manifest_path,
            &release_state_dir,
            &mock_root,
            &cache_dir,
        ),
    );
    assert_eq!(
        first.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );

    let index_path = cache_dir.join("release-index/v1/stable/1.0.16.json");
    let previous_index = fs::read(&index_path).expect("read previous index");
    let changed_manifest_path =
        write_release_toolchain_manifest_with_component_version(sandbox.path(), "0.5.9");

    let second = sandbox.run_gtc_capture(
        ["install", "--release", "1.0.16", "--channel", "stable"],
        release_install_env(
            sandbox.path(),
            &cargo_home,
            &changed_manifest_path,
            &release_state_dir,
            &broken_mock_root,
            &cache_dir,
        ),
    );
    assert_eq!(second.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(
        stderr.contains("failed to prefetch release artifacts"),
        "{stderr}"
    );
    let after_failure = fs::read(&index_path).expect("read index after failure");
    assert_eq!(previous_index, after_failure);
}

#[test]
fn release_cache_import_rejects_checksum_mismatch_without_mutating_cache() {
    let sandbox =
        TestSandbox::new("release_cache_import_rejects_checksum_mismatch_without_mutating_cache");
    let archive_path = write_checksum_mismatch_release_cache_archive(sandbox.path());
    let import_cache = sandbox.path().join("import-cache");

    let mut import_env = HashMap::new();
    import_env.insert(
        "GREENTIC_CACHE_DIR".to_string(),
        import_cache.display().to_string(),
    );
    let import = sandbox.run_gtc_capture(
        [
            "release-cache",
            "import",
            "--input",
            archive_path.to_str().expect("archive utf8"),
        ],
        import_env,
    );
    assert_eq!(import.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&import.stderr);
    assert!(stderr.contains("checksum mismatch"), "{stderr}");
    assert!(
        !import_cache
            .join("release-index/v1/stable/1.0.16.json")
            .exists()
    );
    assert!(!import_cache.join("artifacts").exists());
}

#[test]
fn release_cache_export_fails_when_release_index_is_missing() {
    let sandbox = TestSandbox::new("release_cache_export_fails_when_release_index_is_missing");
    let cache_dir = sandbox.path().join("empty-cache");
    let archive_path = sandbox.path().join("release-cache.tar.gz");

    let mut export_env = HashMap::new();
    export_env.insert(
        "GREENTIC_CACHE_DIR".to_string(),
        cache_dir.display().to_string(),
    );
    let export = sandbox.run_gtc_capture(
        [
            "release-cache",
            "export",
            "--release",
            "1.0.16",
            "--channel",
            "stable",
            "--output",
            archive_path.to_str().expect("archive utf8"),
        ],
        export_env,
    );
    assert_eq!(export.status.code(), Some(1));
    assert!(!archive_path.exists());
}

#[test]
fn release_cache_export_fails_when_indexed_blob_is_missing() {
    let sandbox = TestSandbox::new("release_cache_export_fails_when_indexed_blob_is_missing");
    let cargo_log_file = sandbox.path().join("cargo.log");
    let cargo_home = sandbox.path().join("cargo-home");
    let cache_dir = sandbox.path().join("dist-cache");
    let release_state_dir = sandbox.path().join("release-state");
    let manifest_path = write_release_toolchain_manifest(sandbox.path());
    let mock_root = write_release_artifact_mock_root(sandbox.path());
    let archive_path = sandbox.path().join("release-cache.tar.gz");

    sandbox.write_exit_tool("greentic-dev", 0);
    sandbox.write_exit_tool("greentic-operator", 0);
    sandbox.write_contract_deployer_tool("greentic-deployer");
    sandbox.write_cargo_binstall_tool(&cargo_log_file, None);
    sandbox.write_install_prereq_tools();
    fs::create_dir_all(&cargo_home).expect("cargo_home");

    let install = sandbox.run_gtc_capture(
        ["install", "--release", "1.0.16", "--channel", "stable"],
        release_install_env(
            sandbox.path(),
            &cargo_home,
            &manifest_path,
            &release_state_dir,
            &mock_root,
            &cache_dir,
        ),
    );
    assert_eq!(
        install.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&install.stdout),
        String::from_utf8_lossy(&install.stderr)
    );

    let index_path = cache_dir.join("release-index/v1/stable/1.0.16.json");
    let index: serde_json::Value =
        serde_json::from_slice(&fs::read(&index_path).expect("read release index"))
            .expect("parse release index");
    let digest = index["refs"]
        .as_object()
        .expect("refs object")
        .values()
        .next()
        .expect("first ref")["digest"]
        .as_str()
        .expect("digest");
    let blob = artifact_blob_path(&cache_dir, digest);
    fs::remove_file(&blob).expect("remove indexed blob");

    let mut export_env = HashMap::new();
    export_env.insert(
        "GREENTIC_CACHE_DIR".to_string(),
        cache_dir.display().to_string(),
    );
    let export = sandbox.run_gtc_capture(
        [
            "release-cache",
            "export",
            "--release",
            "1.0.16",
            "--channel",
            "stable",
            "--output",
            archive_path.to_str().expect("archive utf8"),
        ],
        export_env,
    );
    assert_eq!(export.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&export.stderr);
    assert!(stderr.contains("failed to read"), "{stderr}");
}

#[test]
fn release_cache_import_rejects_missing_blob_without_mutating_cache() {
    let sandbox =
        TestSandbox::new("release_cache_import_rejects_missing_blob_without_mutating_cache");
    let archive_path = write_missing_blob_release_cache_archive(sandbox.path());
    let import_cache = sandbox.path().join("import-cache");

    let mut import_env = HashMap::new();
    import_env.insert(
        "GREENTIC_CACHE_DIR".to_string(),
        import_cache.display().to_string(),
    );
    let import = sandbox.run_gtc_capture(
        [
            "release-cache",
            "import",
            "--input",
            archive_path.to_str().expect("archive utf8"),
        ],
        import_env,
    );
    assert_eq!(import.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&import.stderr);
    assert!(stderr.contains("release cache missing"), "{stderr}");
    assert!(!import_cache.join("release-index").exists());
    assert!(!import_cache.join("artifacts").exists());
}

#[test]
fn release_cache_import_rejects_invalid_index_digest_without_mutating_cache() {
    let sandbox = TestSandbox::new(
        "release_cache_import_rejects_invalid_index_digest_without_mutating_cache",
    );
    let archive_path = write_invalid_digest_release_cache_archive(sandbox.path());
    let import_cache = sandbox.path().join("import-cache");

    let mut import_env = HashMap::new();
    import_env.insert(
        "GREENTIC_CACHE_DIR".to_string(),
        import_cache.display().to_string(),
    );
    let import = sandbox.run_gtc_capture(
        [
            "release-cache",
            "import",
            "--input",
            archive_path.to_str().expect("archive utf8"),
        ],
        import_env,
    );
    assert_eq!(import.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&import.stderr);
    assert!(stderr.contains("invalid artifact digest"), "{stderr}");
    assert!(!import_cache.join("release-index").exists());
    assert!(!import_cache.join("artifacts").exists());
}

#[test]
#[cfg(unix)]
fn install_tenant_only_skips_public_install_and_delegates_to_dev() {
    let sandbox = TestSandbox::new("install_tenant_only_skips_public_install_and_delegates_to_dev");
    let log_file = sandbox.path().join("dev.log");

    sandbox.write_arg_env_logger_tool("greentic-dev", &log_file, 0, "GREENTIC_ACME_KEY");

    let mut extra = HashMap::new();
    extra.insert("GREENTIC_ACME_KEY".to_string(), "secret-token".to_string());

    let output = sandbox.run_gtc_capture(
        ["install", "--install-tenant-only", "--tenant", "acme"],
        extra,
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("Installing public Greentic tools."));
    let logged = fs::read_to_string(log_file).expect("read dev log");
    let args_line = logged.lines().next().unwrap_or_default();
    assert_eq!(
        args_line,
        "install --tenant acme --token env:GREENTIC_ACME_KEY"
    );
    assert!(logged.contains("GREENTIC_ACME_KEY=secret-token"));
}

#[test]
fn install_binaries_only_skips_release_artifact_prefetch() {
    let sandbox = TestSandbox::new("install_binaries_only_skips_release_artifact_prefetch");
    let cargo_log_file = sandbox.path().join("cargo.log");
    let cargo_home = sandbox.path().join("cargo-home");
    let cache_dir = sandbox.path().join("dist-cache");
    let release_state_dir = sandbox.path().join("release-state");
    let manifest_path = write_release_toolchain_manifest(sandbox.path());
    let mock_root = write_release_artifact_mock_root(sandbox.path());

    sandbox.write_exit_tool("greentic-dev", 0);
    sandbox.write_exit_tool("greentic-operator", 0);
    sandbox.write_contract_deployer_tool("greentic-deployer");
    sandbox.write_cargo_binstall_tool(&cargo_log_file, None);
    sandbox.write_install_prereq_tools();
    fs::create_dir_all(&cargo_home).expect("cargo_home");

    let mut extra = HashMap::new();
    extra.insert("CARGO_HOME".to_string(), cargo_home.display().to_string());
    extra.insert(
        "GTC_TOOLCHAIN_MANIFEST_PATH".to_string(),
        manifest_path.display().to_string(),
    );
    extra.insert(
        "GTC_TOOLCHAIN_STATE_DIR".to_string(),
        sandbox.path().join("toolchain-state").display().to_string(),
    );
    extra.insert(
        "GTC_RELEASE_STATE_DIR".to_string(),
        release_state_dir.display().to_string(),
    );
    extra.insert(
        "GTC_RELEASE_ARTIFACT_MOCK_ROOT".to_string(),
        mock_root.display().to_string(),
    );
    extra.insert(
        "GREENTIC_CACHE_DIR".to_string(),
        cache_dir.display().to_string(),
    );

    let output = sandbox.run_gtc_capture(
        [
            "install",
            "--release",
            "1.0.16",
            "--channel",
            "stable",
            "--install-binaries-only",
        ],
        extra,
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let cargo_logged = fs::read_to_string(cargo_log_file).unwrap_or_default();
    assert!(cargo_logged.contains("greentic-dev"));
    assert!(
        !cache_dir
            .join("release-index/v1/stable/1.0.16.json")
            .exists()
    );
    assert!(!release_state_dir.join("current.json").exists());
}

#[test]
fn install_packs_only_prefetches_packs_without_binaries_or_components() {
    let sandbox =
        TestSandbox::new("install_packs_only_prefetches_packs_without_binaries_or_components");
    let cargo_log_file = sandbox.path().join("cargo.log");
    let cache_dir = sandbox.path().join("dist-cache");
    let release_state_dir = sandbox.path().join("release-state");
    let manifest_path = write_release_toolchain_manifest(sandbox.path());
    let mock_root = write_release_artifact_mock_root(sandbox.path());

    sandbox.write_cargo_binstall_tool(&cargo_log_file, None);

    let mut extra = HashMap::new();
    extra.insert(
        "GTC_TOOLCHAIN_MANIFEST_PATH".to_string(),
        manifest_path.display().to_string(),
    );
    extra.insert(
        "GTC_TOOLCHAIN_STATE_DIR".to_string(),
        sandbox.path().join("toolchain-state").display().to_string(),
    );
    extra.insert(
        "GTC_RELEASE_STATE_DIR".to_string(),
        release_state_dir.display().to_string(),
    );
    extra.insert(
        "GTC_RELEASE_ARTIFACT_MOCK_ROOT".to_string(),
        mock_root.display().to_string(),
    );
    extra.insert(
        "GREENTIC_CACHE_DIR".to_string(),
        cache_dir.display().to_string(),
    );

    let output = sandbox.run_gtc_capture(
        [
            "install",
            "--manifest",
            manifest_path.to_str().expect("utf8"),
            "--install-packs-only",
        ],
        extra,
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let cargo_logged = fs::read_to_string(cargo_log_file).unwrap_or_default();
    assert!(cargo_logged.is_empty());

    let index_path = cache_dir.join("release-index/v1/stable/1.0.16.json");
    let index: serde_json::Value =
        serde_json::from_slice(&fs::read(&index_path).expect("read release index"))
            .expect("parse release index");
    let refs = index["refs"].as_object().expect("refs object");
    assert!(refs.contains_key("ghcr.io/greenticai/packs/messaging/messaging-webchat-gui:stable"));
    assert!(!refs.contains_key("ghcr.io/greenticai/components/templates:stable"));
    assert!(release_state_dir.join("current.json").is_file());
}

#[test]
fn install_latest_manifest_resolves_prerelease_before_binstall() {
    let sandbox = TestSandbox::new("install_latest_manifest_resolves_prerelease_before_binstall");
    let cargo_log_file = sandbox.path().join("cargo.log");
    let cargo_home = sandbox.path().join("cargo-home");
    let manifest_path = write_latest_toolchain_manifest(sandbox.path());

    sandbox.write_cargo_binstall_tool(&cargo_log_file, None);
    sandbox.write_install_prereq_tools();
    sandbox.write_contract_deployer_tool("greentic-deployer");
    fs::create_dir_all(&cargo_home).expect("cargo_home");

    let mut extra = HashMap::new();
    extra.insert("CARGO_HOME".to_string(), cargo_home.display().to_string());
    extra.insert(
        "GTC_TOOLCHAIN_STATE_DIR".to_string(),
        sandbox.path().join("toolchain-state").display().to_string(),
    );

    let output = sandbox.run_gtc_capture(
        [
            "install",
            "--manifest",
            manifest_path.to_str().expect("utf8"),
        ],
        extra,
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let cargo_logged = fs::read_to_string(cargo_log_file).expect("read cargo log");
    assert!(cargo_logged.contains("search greentic-flow --limit 1"));
    assert!(cargo_logged.contains(
        "binstall -y --locked --force --maximum-resolution-timeout 60 greentic-flow --version 0.6.0-dev.25001174716 --bin greentic-flow"
    ));
}

#[test]
#[cfg(unix)]
fn install_tenant_mode_delegates_to_greentic_dev_after_toolchain_success() {
    let sandbox =
        TestSandbox::new("install_tenant_mode_delegates_to_greentic_dev_after_toolchain_success");
    let log_file = sandbox.path().join("dev.log");
    let cargo_log_file = sandbox.path().join("cargo.log");
    let manifest_path = write_toolchain_manifest(sandbox.path());

    sandbox.write_arg_env_logger_tool("greentic-dev", &log_file, 0, "GREENTIC_ACME_KEY");
    sandbox.write_exit_tool("greentic-operator", 0);
    sandbox.write_contract_deployer_tool("greentic-deployer");
    sandbox.write_cargo_binstall_tool(&cargo_log_file, None);
    sandbox.write_install_prereq_tools();

    let cargo_home = sandbox.path().join("cargo-home");
    let home = sandbox.path().join("home");
    fs::create_dir_all(&cargo_home).expect("cargo_home");
    fs::create_dir_all(&home).expect("home");

    let mut extra = HashMap::new();
    extra.insert("GREENTIC_ACME_KEY".to_string(), "secret-token".to_string());
    extra.insert("CARGO_HOME".to_string(), cargo_home.display().to_string());
    extra.insert("HOME".to_string(), home.display().to_string());
    extra.insert(
        "GTC_TOOLCHAIN_STATE_DIR".to_string(),
        sandbox.path().join("toolchain-state").display().to_string(),
    );

    let output = sandbox.run_gtc_capture(
        [
            "install",
            "--manifest",
            manifest_path.to_str().expect("utf8"),
            "--tenant",
            "acme",
        ],
        extra,
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let logged = fs::read_to_string(log_file).expect("read dev log");
    let args_line = logged
        .lines()
        .find(|line| line.starts_with("install --tenant"))
        .unwrap_or_default();
    assert_eq!(
        args_line,
        "install --tenant acme --token env:GREENTIC_ACME_KEY"
    );
    assert!(!logged.contains("--key"));
    assert!(!args_line.contains("secret-token"));
    assert!(logged.contains("GREENTIC_ACME_KEY=secret-token"));
    assert!(!logged.contains("install tools"));
    let cargo_logged = fs::read_to_string(cargo_log_file).expect("read cargo log");
    assert!(
        cargo_logged.contains(
            "binstall -y --locked --force --maximum-resolution-timeout 60 greentic-dev --version 0.5.9 --bin greentic-dev"
        )
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("secret-token"));
}

#[test]
fn install_skips_tenant_when_public_install_fails() {
    let sandbox = TestSandbox::new("install_skips_tenant_when_public_install_fails");
    let log_file = sandbox.path().join("dev.log");
    let cargo_log_file = sandbox.path().join("cargo.log");
    let manifest_path = write_toolchain_manifest(sandbox.path());

    sandbox.write_arg_logger_tool("greentic-dev", &log_file, 0);
    sandbox.write_exit_tool("greentic-operator", 0);
    sandbox.write_cargo_binstall_tool(&cargo_log_file, Some("greentic-runner-cli"));
    sandbox.write_install_prereq_tools();
    sandbox.write_contract_deployer_tool("greentic-deployer");

    let cargo_home = sandbox.path().join("cargo-home");
    fs::create_dir_all(&cargo_home).expect("cargo_home");

    let mut extra = HashMap::new();
    extra.insert("CARGO_HOME".to_string(), cargo_home.display().to_string());
    extra.insert(
        "GTC_TOOLCHAIN_STATE_DIR".to_string(),
        sandbox.path().join("toolchain-state").display().to_string(),
    );
    extra.insert("GREENTIC_ACME_KEY".to_string(), "secret-token".to_string());

    let output = sandbox.run_gtc_capture(
        [
            "install",
            "--manifest",
            manifest_path.to_str().expect("utf8"),
            "--tenant",
            "acme",
        ],
        extra,
    );
    assert_eq!(output.status.code(), Some(1));
    let cargo_logged = fs::read_to_string(cargo_log_file).expect("read cargo log");
    assert!(cargo_logged.contains("greentic-runner-cli"));
    let logged = fs::read_to_string(log_file).unwrap_or_default();
    assert!(!logged.contains("--tenant acme"));
}

#[test]
fn install_dry_run_executes_nothing() {
    let sandbox = TestSandbox::new("install_dry_run_executes_nothing");
    let log_file = sandbox.path().join("dev.log");
    let cargo_log_file = sandbox.path().join("cargo.log");
    let manifest_path = write_toolchain_manifest(sandbox.path());
    let cargo_home = sandbox.path().join("cargo-home");
    let state_dir = sandbox.path().join("toolchain-state");
    fs::create_dir_all(&cargo_home).expect("cargo_home");

    sandbox.write_arg_logger_tool("greentic-dev", &log_file, 0);
    sandbox.write_contract_deployer_tool("greentic-deployer");
    sandbox.write_cargo_binstall_tool(&cargo_log_file, None);
    sandbox.write_install_prereq_tools();

    let mut extra = HashMap::new();
    extra.insert("CARGO_HOME".to_string(), cargo_home.display().to_string());
    extra.insert("GREENTIC_ACME_KEY".to_string(), "secret-token".to_string());
    extra.insert(
        "GTC_TOOLCHAIN_STATE_DIR".to_string(),
        state_dir.display().to_string(),
    );

    let output = sandbox.run_gtc_capture(
        [
            "install",
            "--manifest",
            manifest_path.to_str().expect("utf8"),
            "--tenant",
            "acme",
            "--dry-run",
        ],
        extra,
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let cargo_logged = fs::read_to_string(cargo_log_file).expect("read cargo log");
    assert!(!cargo_logged.contains("--locked --force"));
    assert!(!state_dir.join("installed.json").exists());
    let dev_logged = fs::read_to_string(log_file).unwrap_or_default();
    assert!(!dev_logged.contains("--tenant acme"));
}

#[test]
fn install_skips_when_digest_is_unchanged() {
    let sandbox = TestSandbox::new("install_skips_when_digest_is_unchanged");
    let cargo_log_file = sandbox.path().join("cargo.log");
    let manifest_path = write_toolchain_manifest(sandbox.path());
    let cargo_home = sandbox.path().join("cargo-home");
    let state_dir = sandbox.path().join("toolchain-state");
    fs::create_dir_all(&cargo_home).expect("cargo_home");

    sandbox.write_exit_tool("greentic-dev", 0);
    sandbox.write_contract_deployer_tool("greentic-deployer");
    sandbox.write_cargo_binstall_tool(&cargo_log_file, None);
    sandbox.write_install_prereq_tools();

    let mut extra = HashMap::new();
    extra.insert("CARGO_HOME".to_string(), cargo_home.display().to_string());
    extra.insert(
        "GTC_TOOLCHAIN_STATE_DIR".to_string(),
        state_dir.display().to_string(),
    );

    let first = sandbox.run_gtc_capture(
        [
            "install",
            "--manifest",
            manifest_path.to_str().expect("utf8"),
        ],
        extra.clone(),
    );
    assert_eq!(
        first.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );

    fs::write(&cargo_log_file, "").expect("clear cargo log");
    let second = sandbox.run_gtc_capture(
        [
            "install",
            "--manifest",
            manifest_path.to_str().expect("utf8"),
        ],
        extra,
    );
    assert_eq!(
        second.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );

    let cargo_logged = fs::read_to_string(cargo_log_file).expect("read cargo log");
    assert!(!cargo_logged.contains("--locked --force"));
    let stdout = String::from_utf8_lossy(&second.stdout);
    assert!(stdout.contains("Greentic toolchain is already up to date."));
}

#[test]
fn update_installs_manifest_toolchain_with_force() {
    let sandbox = TestSandbox::new("update_installs_manifest_toolchain_with_force");
    let cargo_log_file = sandbox.path().join("cargo.log");
    let manifest_path = write_toolchain_manifest(sandbox.path());
    let cargo_home = sandbox.path().join("cargo-home");
    fs::create_dir_all(&cargo_home).expect("cargo_home");

    sandbox.write_cargo_binstall_tool(&cargo_log_file, None);
    sandbox.write_install_prereq_tools();
    sandbox.write_contract_deployer_tool("greentic-deployer");
    sandbox.write_exit_tool("greentic-operator", 0);

    let mut extra = HashMap::new();
    extra.insert("CARGO_HOME".to_string(), cargo_home.display().to_string());
    extra.insert(
        "GTC_TOOLCHAIN_MANIFEST_PATH".to_string(),
        manifest_path.display().to_string(),
    );
    extra.insert(
        "GTC_TOOLCHAIN_STATE_DIR".to_string(),
        sandbox.path().join("toolchain-state").display().to_string(),
    );

    let output = sandbox.run_gtc_capture(["update"], extra);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let cargo_logged = fs::read_to_string(&cargo_log_file).expect("read cargo log");
    assert!(
        cargo_logged.contains(
            "binstall -y --locked --force --maximum-resolution-timeout 60 greentic-dev --version 0.5.9 --bin greentic-dev"
        )
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Installing greentic-dev binary greentic-dev"));
}

#[test]
fn update_reports_manifest_install_failure() {
    let sandbox = TestSandbox::new("update_reports_manifest_install_failure");
    let cargo_log_file = sandbox.path().join("cargo.log");
    let manifest_path = write_toolchain_manifest(sandbox.path());
    let cargo_home = sandbox.path().join("cargo-home");
    fs::create_dir_all(&cargo_home).expect("cargo_home");

    sandbox.write_cargo_binstall_tool(&cargo_log_file, Some("greentic-runner-cli"));
    sandbox.write_install_prereq_tools();
    sandbox.write_contract_deployer_tool("greentic-deployer");
    sandbox.write_exit_tool("greentic-operator", 0);

    let mut extra = HashMap::new();
    extra.insert("CARGO_HOME".to_string(), cargo_home.display().to_string());
    extra.insert(
        "GTC_TOOLCHAIN_MANIFEST_PATH".to_string(),
        manifest_path.display().to_string(),
    );
    extra.insert(
        "GTC_TOOLCHAIN_STATE_DIR".to_string(),
        sandbox.path().join("toolchain-state").display().to_string(),
    );

    let output = sandbox.run_gtc_capture(["update"], extra);
    assert_eq!(
        output.status.code(),
        Some(1),
        "should exit 1 when a manifest package fails"
    );

    let cargo_logged = fs::read_to_string(&cargo_log_file).expect("read cargo log");
    assert!(cargo_logged.contains("greentic-dev"));
    assert!(cargo_logged.contains("greentic-runner-cli"));

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Installed greentic-runner binary greentic-runner-cli: FAIL"));
}

#[test]
fn add_and_remove_admin_roundtrip_updates_registry() {
    let sandbox = TestSandbox::new("add_and_remove_admin_roundtrip_updates_registry");
    let bundle_dir = sandbox.path().join("bundle");
    fs::create_dir_all(&bundle_dir).expect("bundle dir");
    let public_key = sandbox.path().join("admin.pub");
    fs::write(
        &public_key,
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITestKey admin@test\n",
    )
    .expect("write public key");

    let add_output = sandbox.run_gtc_capture(
        [
            "add-admin",
            bundle_dir.to_str().expect("bundle utf8"),
            "--cn",
            "admin-cn",
            "--name",
            "Demo Admin",
            "--public-key-file",
            public_key.to_str().expect("pubkey utf8"),
        ],
        HashMap::new(),
    );
    assert_eq!(
        add_output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&add_output.stdout),
        String::from_utf8_lossy(&add_output.stderr)
    );

    let registry_path = bundle_dir
        .join(".greentic")
        .join("admin")
        .join("admins.json");
    let registry_raw = fs::read_to_string(&registry_path).expect("read registry");
    let registry: serde_json::Value =
        serde_json::from_str(&registry_raw).expect("registry should be valid json");
    let admins = registry
        .get("admins")
        .and_then(serde_json::Value::as_array)
        .expect("admins array");
    assert_eq!(admins.len(), 1);
    assert_eq!(
        admins[0]
            .get("client_cn")
            .and_then(serde_json::Value::as_str),
        Some("admin-cn")
    );
    assert_eq!(
        admins[0].get("name").and_then(serde_json::Value::as_str),
        Some("Demo Admin")
    );

    let remove_output = sandbox.run_gtc_capture(
        [
            "remove-admin",
            bundle_dir.to_str().expect("bundle utf8"),
            "--cn",
            "admin-cn",
        ],
        HashMap::new(),
    );
    assert_eq!(
        remove_output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&remove_output.stdout),
        String::from_utf8_lossy(&remove_output.stderr)
    );

    let registry_raw = fs::read_to_string(&registry_path).expect("read registry after removal");
    let registry: serde_json::Value =
        serde_json::from_str(&registry_raw).expect("registry should be valid json");
    let admins = registry
        .get("admins")
        .and_then(serde_json::Value::as_array)
        .expect("admins array");
    assert!(
        admins.is_empty(),
        "admin registry should be empty after removal"
    );
}

fn create_minimal_bundle_dir(path: &Path) {
    fs::create_dir_all(path.join("packs")).expect("packs dir");
    fs::write(path.join("bundle.yaml"), "bundle_id: demo\n").expect("bundle.yaml");
    fs::write(path.join("packs").join("default.gtpack"), b"fixture-pack").expect("default pack");
}

struct TestSandbox {
    root: PathBuf,
}

impl TestSandbox {
    fn new(test_name: &str) -> Self {
        let mut root = env::temp_dir();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        root.push(format!(
            "gtc-test-{}-{}-{}",
            test_name,
            std::process::id(),
            nanos
        ));
        fs::create_dir_all(&root).expect("create sandbox dir");
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }

    fn binary_path(&self, name: &str) -> PathBuf {
        #[cfg(windows)]
        {
            self.root.join(format!("{name}.exe"))
        }
        #[cfg(not(windows))]
        {
            self.root.join(name)
        }
    }

    fn compile_rust_tool_at(&self, path: &Path, source: &str) {
        let src_path = path.with_extension("rs");
        fs::write(&src_path, source).expect("write rust tool source");
        let status = Command::new("rustc")
            .arg("--edition=2024")
            .arg(&src_path)
            .arg("-o")
            .arg(path)
            .status()
            .expect("run rustc");
        assert!(status.success(), "rustc should compile helper tool");
    }

    fn write_exit_tool(&self, name: &str, exit_code: i32) {
        let path = self.binary_path(name);
        self.compile_rust_tool_at(&path, &rust_exit_tool_program(exit_code));
    }

    fn write_arg_logger_tool(&self, name: &str, log_file: &Path, exit_code: i32) {
        let path = self.binary_path(name);
        self.compile_rust_tool_at(&path, &rust_arg_logger_program(log_file, exit_code));
    }

    fn write_env_deploy_probe_tool(&self, name: &str, log_file: &Path) {
        let path = self.binary_path(name);
        self.compile_rust_tool_at(&path, &rust_env_deploy_probe_program(log_file));
    }

    fn write_arg_env_logger_tool(
        &self,
        name: &str,
        log_file: &Path,
        exit_code: i32,
        env_key: &str,
    ) {
        let path = self.binary_path(name);
        self.compile_rust_tool_at(
            &path,
            &rust_arg_env_logger_program(log_file, exit_code, env_key),
        );
    }

    fn write_version_tool(&self, name: &str, version_line: &str) {
        let path = self.binary_path(name);
        self.compile_rust_tool_at(&path, &rust_version_tool_program(version_line));
    }

    fn write_emit_answers_tool(&self, name: &str) {
        let path = self.binary_path(name);
        self.compile_rust_tool_at(&path, &rust_emit_answers_tool_program());
    }

    fn write_stdout_tool(&self, name: &str, stdout: &str, exit_code: i32) {
        let path = self.binary_path(name);
        self.compile_rust_tool_at(&path, &rust_stdout_tool_program(stdout, exit_code));
    }

    fn write_cargo_binstall_tool(&self, log_file: &Path, fail_on_contains: Option<&str>) {
        let path = self.binary_path("cargo");
        self.compile_rust_tool_at(
            &path,
            &rust_cargo_binstall_tool_program(log_file, fail_on_contains),
        );
    }

    fn write_install_prereq_tools(&self) {
        self.write_stdout_tool("rustc", "rustc 1.95.0\n", 0);
        self.compile_rust_tool_at(
            &self.binary_path("rustup"),
            &rust_rustup_target_tool_program(),
        );
        self.write_exit_tool("cargo-component", 0);
    }

    #[cfg(unix)]
    #[allow(dead_code)]
    fn write_setup_bundle_tool(&self, name: &str, log_file: &Path) {
        let path = self.binary_path(name);
        self.compile_rust_tool_at(&path, &rust_setup_bundle_tool_program(log_file));
    }

    fn write_bundle_build_tool(&self, name: &str, log_file: &Path) {
        let path = self.binary_path(name);
        self.compile_rust_tool_at(&path, &rust_bundle_build_tool_program(log_file));
    }

    fn write_contract_deployer_tool(&self, name: &str) {
        let path = self.binary_path(name);
        self.compile_rust_tool_at(&path, &rust_contract_deployer_tool_program());
    }

    fn run_gtc<const N: usize>(
        &self,
        args: [&str; N],
        extra_env: HashMap<String, String>,
    ) -> std::process::ExitStatus {
        self.run_gtc_capture(args, extra_env).status
    }

    fn run_gtc_capture<const N: usize>(
        &self,
        args: [&str; N],
        extra_env: HashMap<String, String>,
    ) -> std::process::Output {
        self.run_gtc_binary_capture(Path::new(env!("CARGO_BIN_EXE_gtc")), args, extra_env)
    }

    fn run_gtc_dev_capture<const N: usize>(
        &self,
        args: [&str; N],
        extra_env: HashMap<String, String>,
    ) -> std::process::Output {
        let dev_launcher = self.binary_path("gtc-dev");
        if !dev_launcher.is_file() {
            fs::hard_link(env!("CARGO_BIN_EXE_gtc"), &dev_launcher)
                .or_else(|_| fs::copy(env!("CARGO_BIN_EXE_gtc"), &dev_launcher).map(|_| ()))
                .expect("create gtc-dev launcher");
        }
        self.run_gtc_binary_capture(&dev_launcher, args, extra_env)
    }

    fn run_gtc_binary_capture<const N: usize>(
        &self,
        binary: &Path,
        args: [&str; N],
        extra_env: HashMap<String, String>,
    ) -> std::process::Output {
        let current_path = env::var_os("PATH").unwrap_or_default();
        let mut path_entries = vec![self.root.clone()];
        path_entries.extend(env::split_paths(&current_path));
        let merged_path = env::join_paths(path_entries).expect("join PATH");

        let mut cmd = Command::new(binary);
        cmd.args(args).env("PATH", merged_path);

        // Keep gtc's system.log bookkeeping (version/checksum snapshots) inside
        // the sandbox so tests never append to the developer's real
        // ~/.greentic/logs. Explicit per-test overrides in extra_env still win.
        cmd.env(
            "GTC_SYSTEM_LOG_DIR",
            self.path().join("system-log").display().to_string(),
        );

        for (binary, env_key) in [
            ("greentic-dev", "GREENTIC_DEV_BIN"),
            ("greentic-operator", "GREENTIC_OPERATOR_BIN"),
            ("greentic-setup", "GREENTIC_SETUP_BIN"),
            ("greentic-bundle", "GREENTIC_BUNDLE_BIN"),
            ("greentic-component", "GREENTIC_COMPONENT_BIN"),
            ("greentic-flow", "GREENTIC_FLOW_BIN"),
            ("greentic-pack", "GREENTIC_PACK_BIN"),
            ("greentic-runner", "GREENTIC_RUNNER_BIN"),
            ("greentic-secrets", "GREENTIC_SECRETS_BIN"),
            ("greentic-start", "GREENTIC_START_BIN"),
            ("greentic-deployer", "GREENTIC_DEPLOYER_BIN"),
        ] {
            cmd.env_remove(env_key);
            let path = self.binary_path(binary);
            if path.is_file() {
                cmd.env(env_key, path);
            }
        }

        for (k, v) in extra_env {
            cmd.env(k, v);
        }

        // ETXTBSY guard. These tests compile helper executables while other
        // test threads are spawning processes. Between `fork()` and `execve()`
        // a child transiently inherits the writable fd of an executable another
        // thread is still creating, and exec'ing that file returns ETXTBSY.
        // The window is short, so a bounded retry is enough; without it the
        // suite fails roughly one run in four, on a different test each time.
        for attempt in 0..10 {
            match cmd.output() {
                Ok(output) => return output,
                Err(err) if err.kind() == io::ErrorKind::ExecutableFileBusy => {
                    std::thread::sleep(std::time::Duration::from_millis(20 * (attempt + 1)));
                }
                Err(err) => panic!("run gtc: {err:?}"),
            }
        }
        panic!("run gtc: still ETXTBSY after retries");
    }
}

impl Drop for TestSandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn rust_string_literal(value: &str) -> String {
    format!("{value:?}")
}

fn write_toolchain_manifest(root: &Path) -> PathBuf {
    let path = root.join("toolchain-manifest.json");
    let manifest = serde_json::json!({
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
            },
            {
                "crate": "greentic-deployer",
                "bins": ["greentic-deployer"],
                "version": "0.4.3"
            }
        ]
    });
    fs::write(
        &path,
        serde_json::to_vec_pretty(&manifest).expect("manifest json"),
    )
    .expect("write toolchain manifest");
    path
}

fn write_release_toolchain_manifest(root: &Path) -> PathBuf {
    write_release_toolchain_manifest_with_component_version(root, "0.5.8")
}

fn write_release_toolchain_manifest_with_component_version(
    root: &Path,
    component_version: &str,
) -> PathBuf {
    let path = root.join("release-toolchain-manifest.json");
    let manifest = serde_json::json!({
        "schema": "greentic.toolchain-manifest.v1",
        "toolchain": "gtc",
        "version": "1.0.16",
        "channel": "stable",
        "packages": [
            {
                "crate": "greentic-dev",
                "bins": ["greentic-dev"],
                "version": "0.5.9"
            },
            {
                "crate": "greentic-deployer",
                "bins": ["greentic-deployer"],
                "version": "0.4.3"
            }
        ],
        "extension_packs": [
            {
                "id": "packs/messaging/messaging-webchat-gui",
                "version": "0.5.4"
            }
        ],
        "components": [
            {
                "id": "components/templates",
                "version": component_version
            }
        ]
    });
    fs::write(
        &path,
        serde_json::to_vec_pretty(&manifest).expect("manifest json"),
    )
    .expect("write release toolchain manifest");
    path
}

fn write_single_binary_toolchain_manifest(root: &Path) -> PathBuf {
    let path = root.join("single-binary-toolchain-manifest.json");
    let manifest = serde_json::json!({
        "schema": "greentic.toolchain-manifest.v1",
        "toolchain": "gtc",
        "version": "1.0.16",
        "channel": "stable",
        "packages": [
            {
                "crate": "greentic-dev",
                "bins": ["greentic-dev"],
                "version": "0.5.9"
            },
            {
                "crate": "greentic-deployer",
                "bins": ["greentic-deployer"],
                "version": "0.4.3"
            }
        ]
    });
    fs::write(
        &path,
        serde_json::to_vec_pretty(&manifest).expect("manifest json"),
    )
    .expect("write single binary toolchain manifest");
    path
}

fn write_release_artifact_mock_root(root: &Path) -> PathBuf {
    let mock_root = root.join("release-artifacts");
    let pack = mock_root.join("packs/webchat.gtpack");
    let component = mock_root.join("components/templates.wasm");
    fs::create_dir_all(pack.parent().expect("pack parent")).expect("pack dirs");
    fs::create_dir_all(component.parent().expect("component parent")).expect("component dirs");
    fs::write(&pack, b"webchat-pack").expect("write pack");
    fs::write(&component, b"templates-component").expect("write component");
    let index = serde_json::json!({
        "ghcr.io/greenticai/packs/messaging/messaging-webchat-gui:0.5.4": "packs/webchat.gtpack",
        "ghcr.io/greenticai/components/templates:0.5.8": "components/templates.wasm"
    });
    fs::write(
        mock_root.join("index.json"),
        serde_json::to_vec_pretty(&index).expect("index json"),
    )
    .expect("write mock release artifact index");
    mock_root
}

fn write_release_artifact_mock_root_missing_component(root: &Path) -> PathBuf {
    let mock_root = root.join("release-artifacts-missing-component");
    let pack = mock_root.join("packs/webchat.gtpack");
    fs::create_dir_all(pack.parent().expect("pack parent")).expect("pack dirs");
    fs::write(&pack, b"webchat-pack").expect("write pack");
    let index = serde_json::json!({
        "ghcr.io/greenticai/packs/messaging/messaging-webchat-gui:0.5.4": "packs/webchat.gtpack"
    });
    fs::write(
        mock_root.join("index.json"),
        serde_json::to_vec_pretty(&index).expect("index json"),
    )
    .expect("write mock release artifact index");
    mock_root
}

fn release_install_env(
    sandbox_root: &Path,
    cargo_home: &Path,
    manifest_path: &Path,
    release_state_dir: &Path,
    mock_root: &Path,
    cache_dir: &Path,
) -> HashMap<String, String> {
    let mut extra = HashMap::new();
    extra.insert("CARGO_HOME".to_string(), cargo_home.display().to_string());
    extra.insert(
        "GTC_TOOLCHAIN_MANIFEST_PATH".to_string(),
        manifest_path.display().to_string(),
    );
    extra.insert(
        "GTC_TOOLCHAIN_STATE_DIR".to_string(),
        sandbox_root.join("toolchain-state").display().to_string(),
    );
    extra.insert(
        "GTC_RELEASE_STATE_DIR".to_string(),
        release_state_dir.display().to_string(),
    );
    extra.insert(
        "GTC_RELEASE_ARTIFACT_MOCK_ROOT".to_string(),
        mock_root.display().to_string(),
    );
    extra.insert(
        "GREENTIC_CACHE_DIR".to_string(),
        cache_dir.display().to_string(),
    );
    extra
}

fn assert_imported_cache_resolves_stable_offline(cache_dir: &Path, index: &serde_json::Value) {
    let expected_digest = index["refs"]["ghcr.io/greenticai/components/templates:stable"]["digest"]
        .as_str()
        .expect("component digest");
    let opts = DistOptions {
        cache_dir: cache_dir.to_path_buf(),
        offline: true,
        ..Default::default()
    };
    let client = DistClient::new(opts);
    let artifact = client
        .open_cached(expected_digest)
        .expect("imported digest opens from offline cache");
    assert_eq!(artifact.descriptor.digest, expected_digest);
}

fn artifact_blob_path(cache_dir: &Path, digest: &str) -> PathBuf {
    let hex = digest.strip_prefix("sha256:").expect("sha256 digest");
    cache_dir
        .join("artifacts/sha256")
        .join(&hex[..2])
        .join(&hex[2..])
        .join("blob")
}

fn write_checksum_mismatch_release_cache_archive(root: &Path) -> PathBuf {
    let archive_path = root.join("bad-release-cache.tar.gz");
    let digest = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
    let artifact_rel = PathBuf::from("artifacts")
        .join("sha256")
        .join("11")
        .join("11111111111111111111111111111111111111111111111111111111111111");
    let index_rel = PathBuf::from("release-index/v1/stable/1.0.16.json");
    let manifest = serde_json::json!({
        "schema": "greentic.release-cache.v1",
        "format": "tar.gz",
        "release": "1.0.16",
        "channel": "stable",
        "created_at": "unix:0",
        "artifact_count": 1
    });
    let index = serde_json::json!({
        "schema": "greentic.release-index.v1",
        "release": "1.0.16",
        "channel": "stable",
        "refs": {
            "ghcr.io/greenticai/packs/demo:stable": {
                "version": "0.1.0",
                "digest": digest,
                "canonical_ref": format!("oci://ghcr.io/greenticai/packs/demo@{digest}")
            }
        }
    });
    let entry = serde_json::json!({
        "format_version": 1,
        "cache_key": digest,
        "digest": digest,
        "media_type": "application/octet-stream",
        "size_bytes": 4,
        "artifact_type": "Pack",
        "source_kind": "Oci",
        "raw_ref": "oci://ghcr.io/greenticai/packs/demo:0.1.0",
        "canonical_ref": format!("oci://ghcr.io/greenticai/packs/demo@{digest}"),
        "fetched_at": 0,
        "last_accessed_at": 0,
        "last_verified_at": null,
        "state": "Ready",
        "advisory_epoch": null,
        "signature_summary": null,
        "local_path": "/unused",
        "source_snapshot": {
            "raw_ref": "oci://ghcr.io/greenticai/packs/demo:0.1.0",
            "canonical_ref": format!("oci://ghcr.io/greenticai/packs/demo@{digest}"),
            "source_kind": "Oci",
            "authoritative": false
        }
    });

    let mut payloads = BTreeMap::new();
    payloads.insert(
        PathBuf::from("manifest.json"),
        serde_json::to_vec_pretty(&manifest).expect("manifest bytes"),
    );
    payloads.insert(
        index_rel,
        serde_json::to_vec_pretty(&index).expect("index bytes"),
    );
    payloads.insert(artifact_rel.join("blob"), b"bad!".to_vec());
    payloads.insert(
        artifact_rel.join("entry.json"),
        serde_json::to_vec_pretty(&entry).expect("entry bytes"),
    );

    let mut checksums = BTreeMap::new();
    for (path, bytes) in &payloads {
        let checksum_bytes = if path.ends_with("blob") {
            b"good".as_slice()
        } else {
            bytes.as_slice()
        };
        checksums.insert(
            test_archive_path_string(path),
            test_sha256_bytes(checksum_bytes),
        );
    }
    let checksums = serde_json::to_vec_pretty(&checksums).expect("checksums bytes");

    let file = fs::File::create(&archive_path).expect("archive file");
    let encoder = GzEncoder::new(file, Compression::default());
    let mut archive = tar::Builder::new(encoder);
    for (path, bytes) in payloads {
        append_test_archive_bytes(&mut archive, &path, &bytes);
    }
    append_test_archive_bytes(&mut archive, Path::new("checksums.json"), &checksums);
    let encoder = archive.into_inner().expect("finish tar");
    encoder.finish().expect("finish gzip");
    archive_path
}

fn write_missing_blob_release_cache_archive(root: &Path) -> PathBuf {
    write_synthetic_release_cache_archive(
        root,
        "missing-blob-release-cache.tar.gz",
        "sha256:2222222222222222222222222222222222222222222222222222222222222222",
        SyntheticArchiveMode::MissingBlob,
    )
}

fn write_invalid_digest_release_cache_archive(root: &Path) -> PathBuf {
    write_synthetic_release_cache_archive(
        root,
        "invalid-digest-release-cache.tar.gz",
        "sha256:not-a-valid-digest",
        SyntheticArchiveMode::InvalidDigest,
    )
}

enum SyntheticArchiveMode {
    MissingBlob,
    InvalidDigest,
}

fn write_synthetic_release_cache_archive(
    root: &Path,
    filename: &str,
    digest: &str,
    mode: SyntheticArchiveMode,
) -> PathBuf {
    let archive_path = root.join(filename);
    let index_rel = PathBuf::from("release-index/v1/stable/1.0.16.json");
    let manifest = serde_json::json!({
        "schema": "greentic.release-cache.v1",
        "format": "tar.gz",
        "release": "1.0.16",
        "channel": "stable",
        "created_at": "unix:0",
        "artifact_count": 1
    });
    let index = serde_json::json!({
        "schema": "greentic.release-index.v1",
        "release": "1.0.16",
        "channel": "stable",
        "refs": {
            "ghcr.io/greenticai/packs/demo:stable": {
                "version": "0.1.0",
                "digest": digest,
                "canonical_ref": format!("oci://ghcr.io/greenticai/packs/demo@{digest}")
            }
        }
    });

    let mut payloads = BTreeMap::new();
    payloads.insert(
        PathBuf::from("manifest.json"),
        serde_json::to_vec_pretty(&manifest).expect("manifest bytes"),
    );
    payloads.insert(
        index_rel,
        serde_json::to_vec_pretty(&index).expect("index bytes"),
    );

    if matches!(mode, SyntheticArchiveMode::MissingBlob) {
        let hex = digest.trim_start_matches("sha256:");
        let artifact_rel = PathBuf::from("artifacts")
            .join("sha256")
            .join(&hex[..2])
            .join(&hex[2..]);
        let entry = serde_json::json!({
            "format_version": 1,
            "cache_key": digest,
            "digest": digest,
            "media_type": "application/octet-stream",
            "size_bytes": 4,
            "artifact_type": "Pack",
            "source_kind": "Oci",
            "raw_ref": "oci://ghcr.io/greenticai/packs/demo:0.1.0",
            "canonical_ref": format!("oci://ghcr.io/greenticai/packs/demo@{digest}"),
            "fetched_at": 0,
            "last_accessed_at": 0,
            "last_verified_at": null,
            "state": "Ready",
            "advisory_epoch": null,
            "signature_summary": null,
            "local_path": "/unused",
            "source_snapshot": {
                "raw_ref": "oci://ghcr.io/greenticai/packs/demo:0.1.0",
                "canonical_ref": format!("oci://ghcr.io/greenticai/packs/demo@{digest}"),
                "source_kind": "Oci",
                "authoritative": false
            }
        });
        payloads.insert(
            artifact_rel.join("entry.json"),
            serde_json::to_vec_pretty(&entry).expect("entry bytes"),
        );
    }

    let mut checksums = BTreeMap::new();
    for (path, bytes) in &payloads {
        checksums.insert(test_archive_path_string(path), test_sha256_bytes(bytes));
    }
    let checksums = serde_json::to_vec_pretty(&checksums).expect("checksums bytes");

    let file = fs::File::create(&archive_path).expect("archive file");
    let encoder = GzEncoder::new(file, Compression::default());
    let mut archive = tar::Builder::new(encoder);
    for (path, bytes) in payloads {
        append_test_archive_bytes(&mut archive, &path, &bytes);
    }
    append_test_archive_bytes(&mut archive, Path::new("checksums.json"), &checksums);
    let encoder = archive.into_inner().expect("finish tar");
    encoder.finish().expect("finish gzip");
    archive_path
}

fn append_test_archive_bytes<W: std::io::Write>(
    archive: &mut tar::Builder<W>,
    path: &Path,
    bytes: &[u8],
) {
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    archive
        .append_data(&mut header, path, bytes)
        .expect("append archive bytes");
}

fn test_archive_path_string(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn test_sha256_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity("sha256:".len() + digest.len() * 2);
    out.push_str("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

fn write_latest_toolchain_manifest(root: &Path) -> PathBuf {
    let path = root.join("latest-toolchain-manifest.json");
    let manifest = serde_json::json!({
        "schema": "greentic.toolchain-manifest.v1",
        "toolchain": "gtc",
        "version": "dev",
        "channel": "dev",
        "packages": [
            {
                "crate": "greentic-flow",
                "bins": ["greentic-flow"],
                "version": "latest"
            },
            {
                "crate": "greentic-deployer",
                "bins": ["greentic-deployer"],
                "version": "0.4.3"
            }
        ]
    });
    fs::write(
        &path,
        serde_json::to_vec_pretty(&manifest).expect("manifest json"),
    )
    .expect("write latest toolchain manifest");
    path
}

fn write_installed_toolchain_state(root: &Path, version: &str, channel: &str, digest: &str) {
    fs::create_dir_all(root).expect("create toolchain state dir");
    let state = serde_json::json!({
        "schema": "greentic.installed-toolchain.v1",
        "source_kind": "channel",
        "source": format!("ghcr.io/greenticai/greentic-versions/gtc:{channel}"),
        "resolved_digest": digest,
        "channel": channel,
        "version": version,
        "installed_at": "2026-04-28T00:00:00Z",
        "packages": [
            {
                "crate": "greentic-dev",
                "bins": ["greentic-dev"],
                "version": "0.5.9"
            }
        ]
    });
    fs::write(
        root.join("installed.json"),
        serde_json::to_vec_pretty(&state).expect("installed state json"),
    )
    .expect("write installed toolchain state");
}

fn write_test_release_index(cache_dir: &Path, channel: &str, release: &str) {
    let path = cache_dir
        .join("release-index")
        .join("v1")
        .join(channel)
        .join(format!("{release}.json"));
    fs::create_dir_all(path.parent().expect("release index parent"))
        .expect("create release index dir");
    let pack_digest = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
    let component_digest =
        "sha256:2222222222222222222222222222222222222222222222222222222222222222";
    let index = serde_json::json!({
        "schema": "greentic.release-index.v1",
        "release": release,
        "channel": channel,
        "refs": {
            format!("ghcr.io/greenticai/packs/messaging/messaging-webchat-gui:{channel}"): {
                "version": "0.5.4",
                "digest": pack_digest,
                "canonical_ref": format!("oci://ghcr.io/greenticai/packs/messaging/messaging-webchat-gui@{pack_digest}")
            },
            format!("ghcr.io/greenticai/components/templates:{channel}"): {
                "version": "0.5.8",
                "digest": component_digest,
                "canonical_ref": format!("oci://ghcr.io/greenticai/components/templates@{component_digest}")
            }
        }
    });
    fs::write(
        path,
        serde_json::to_vec_pretty(&index).expect("release index json"),
    )
    .expect("write release index");
}

fn rust_exit_tool_program(exit_code: i32) -> String {
    format!("fn main() {{ std::process::exit({exit_code}); }}",)
}

/// A `greentic-setup` stand-in that logs its argv AND inspects the bundle it was
/// handed by `env-deploy`, recording whether admin private keys are inside it.
/// The prepared root is a tempdir that gets cleaned up, so the only way to know
/// what was actually staged into the environment is to look from in here.
fn rust_env_deploy_probe_program(log_file: &Path) -> String {
    let log_literal = rust_string_literal(&log_file.display().to_string());
    format!(
        r#"
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

fn main() {{
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open({log_literal})
        .expect("open log");
    writeln!(file, "{{}}", args.join(" ")).expect("write log");

    if let Some(idx) = args.iter().position(|a| a == "env-deploy")
        && let Some(bundle) = args.get(idx + 1)
    {{
        let admin = Path::new(bundle).join(".greentic").join("admin");
        writeln!(file, "ADMIN_DIR_IN_DEPLOYED_BUNDLE={{}}", admin.exists())
            .expect("write log");
        for key in ["ca.key", "server.key", "client.key"] {{
            let path = admin.join("certs").join(key);
            if path.exists() {{
                writeln!(file, "LEAKED_PRIVATE_KEY={{}}", key).expect("write log");
            }}
        }}
    }}
    std::process::exit(0);
}}
"#
    )
}

fn rust_arg_logger_program(log_file: &Path, exit_code: i32) -> String {
    let log_literal = rust_string_literal(&log_file.display().to_string());
    format!(
        r#"
use std::fs::OpenOptions;
use std::io::Write;

fn main() {{
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open({log_literal})
        .expect("open log");
    let line = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    writeln!(file, "{{}}", line).expect("write log");
    std::process::exit({exit_code});
}}
"#
    )
}

fn rust_arg_env_logger_program(log_file: &Path, exit_code: i32, env_key: &str) -> String {
    let log_literal = rust_string_literal(&log_file.display().to_string());
    let env_key_literal = rust_string_literal(env_key);
    format!(
        r#"
use std::fs::OpenOptions;
use std::io::Write;

fn main() {{
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open({log_literal})
        .expect("open log");
    let line = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    writeln!(file, "{{}}", line).expect("write args log");
    let env_key = {env_key_literal};
    if let Ok(value) = std::env::var(env_key) {{
        writeln!(file, "{{}}={{}}", env_key, value).expect("write env log");
    }}
    std::process::exit({exit_code});
}}
"#
    )
}

fn rust_version_tool_program(version_line: &str) -> String {
    let version_literal = rust_string_literal(version_line);
    format!(
        r#"
fn main() {{
    if std::env::args().nth(1).as_deref() == Some("--version") {{
        println!({version_literal});
    }}
}}
"#
    )
}

fn rust_emit_answers_tool_program() -> String {
    r#"
fn main() {
    let mut emit = None::<String>;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--emit-answers" {
            emit = args.next();
            break;
        }
    }

    let Some(path) = emit else {
        eprintln!("missing --emit-answers target");
        std::process::exit(9);
    };

    std::fs::write(
        path,
        "{\"schema_version\":\"1.0.0\",\"answers\":{},\"events\":[]}\n",
    )
    .expect("write answers");
}
"#
    .to_string()
}

fn rust_stdout_tool_program(stdout: &str, exit_code: i32) -> String {
    let stdout_literal = rust_string_literal(stdout);
    format!(
        r#"
fn main() {{
    print!("{{}}", {stdout_literal});
    std::process::exit({exit_code});
}}
"#
    )
}

fn rust_rustup_target_tool_program() -> String {
    r#"
fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().map(String::as_str) == Some("target")
        && args.get(1).map(String::as_str) == Some("list")
    {
        println!("wasm32-wasip2 (installed)");
    }
}
"#
    .to_string()
}

fn rust_cargo_binstall_tool_program(log_file: &Path, fail_on_contains: Option<&str>) -> String {
    let log_literal = rust_string_literal(&log_file.display().to_string());
    let fail_check = if let Some(needle) = fail_on_contains {
        let needle_literal = rust_string_literal(needle);
        format!(
            r#"
    if joined.contains({needle_literal}) {{
        std::process::exit(1);
    }}
"#
        )
    } else {
        String::new()
    };
    format!(
        r#"
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

fn write_minimal_tar_pack(path: &std::path::Path) {{
    fn write_octal(field: &mut [u8], value: u64) {{
        let width = field.len();
        let digits = width.saturating_sub(1);
        let formatted = format!("{{:0width$o}}", value, width = digits);
        let bytes = formatted.as_bytes();
        let start = digits.saturating_sub(bytes.len());
        field[..digits].fill(b'0');
        field[start..start + bytes.len()].copy_from_slice(bytes);
        field[digits] = 0;
    }}

    let payload = b"fixture-manifest";
    let mut header = [0u8; 512];
    let name = b"manifest.cbor";
    header[..name.len()].copy_from_slice(name);
    write_octal(&mut header[100..108], 0o644);
    write_octal(&mut header[108..116], 0);
    write_octal(&mut header[116..124], 0);
    write_octal(&mut header[124..136], payload.len() as u64);
    write_octal(&mut header[136..148], 0);
    header[148..156].fill(b' ');
    header[156] = b'0';
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    let checksum: u32 = header.iter().map(|byte| u32::from(*byte)).sum();
    write_octal(&mut header[148..156], u64::from(checksum));

    let mut out = Vec::new();
    out.extend_from_slice(&header);
    out.extend_from_slice(payload);
    let payload_padding = (512 - (payload.len() % 512)) % 512;
    out.resize(out.len() + payload_padding, 0);
    out.resize(out.len() + 1024, 0);
    std::fs::write(path, out).expect("write fake terraform pack");
}}

fn main() {{
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let joined = args.join(" ");
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open({log_literal})
        .expect("open log");
    writeln!(file, "{{}}", joined).expect("write log");

    if args.first().map(String::as_str) == Some("search") {{
        if args.get(1).map(String::as_str) == Some("greentic-flow") {{
            println!("greentic-flow = \"0.6.0-dev.25001174716\" # mock");
            return;
        }}
        println!("cargo-binstall = \"1.0.0\" # mock");
        return;
    }}
    if args.first().map(String::as_str) == Some("binstall")
        && matches!(args.get(1).map(String::as_str), Some("-V" | "--version"))
    {{
        println!("cargo-binstall 1.0.0");
        return;
    }}
    if joined.contains("greentic-deployer") {{
        let cargo_home = std::env::var_os("CARGO_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".cargo"));
        let dist_dir = cargo_home.join("bin").join("dist");
        std::fs::create_dir_all(&dist_dir).expect("create dist dir");
        write_minimal_tar_pack(&dist_dir.join("terraform.gtpack"));
    }}
{fail_check}
}}
"#
    )
}

#[cfg(unix)]
#[allow(dead_code)]
fn rust_setup_bundle_tool_program(log_file: &Path) -> String {
    let log_literal = rust_string_literal(&log_file.display().to_string());
    format!(
        r#"
use std::fs::OpenOptions;
use std::io::Write;

fn main() {{
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let joined = args.join(" ");
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open({log_literal})
        .expect("open log");
    writeln!(file, "{{}}", joined).expect("write log");

    let mut out = None::<String>;
    let mut idx = 0usize;
    while idx < args.len() {{
        if args[idx] == "--out" {{
            out = args.get(idx + 1).cloned();
            break;
        }}
        idx += 1;
    }}
    let Some(out_path) = out else {{
        eprintln!("missing --out");
        std::process::exit(9);
    }};
    std::fs::write(out_path, b"bundle-bytes").expect("write artifact");
}}
"#
    )
}

fn rust_bundle_build_tool_program(log_file: &Path) -> String {
    let log_literal = rust_string_literal(&log_file.display().to_string());
    format!(
        r#"
use std::fs::OpenOptions;
use std::io::Write;

fn main() {{
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let joined = args.join(" ");
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open({log_literal})
        .expect("open log");
    writeln!(file, "{{}}", joined).expect("write log");

    let mut output = None::<String>;
    let mut idx = 0usize;
    while idx < args.len() {{
        if args[idx] == "--output" {{
            output = args.get(idx + 1).cloned();
            break;
        }}
        idx += 1;
    }}
    let Some(output_path) = output else {{
        eprintln!("missing --output");
        std::process::exit(9);
    }};
    std::fs::write(output_path, b"prepared bundle fixture").expect("write prepared bundle");
}}
"#
    )
}

fn rust_contract_deployer_tool_program() -> String {
    r#"
fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().map(String::as_str) == Some("target-requirements") {
        let provider = args
            .windows(2)
            .find(|pair| pair[0] == "--provider")
            .map(|pair| pair[1].as_str())
            .unwrap_or("aws");
        println!(
            "{{\"target\":\"{provider}\",\"target_label\":\"{provider}\",\"credential_requirements\":[],\"variable_requirements\":[],\"remote_bundle_source_required\":true,\"remote_bundle_source_help\":null,\"provider_pack_filename\":\"terraform.gtpack\",\"informational_notes\":[]}}"
        );
        return;
    }

    if args.first().map(String::as_str) == Some("--version") {
        println!("greentic-deployer 0.0.0");
    }
}
"#
    .to_string()
}
