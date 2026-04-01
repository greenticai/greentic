use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

#[test]
fn version_flag_prints_cargo_package_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_gtc"))
        .arg("--version")
        .output()
        .expect("run gtc --version");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.trim().starts_with("gtc "));
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

#[test]
fn wizard_passthrough_routes_to_greentic_dev_with_all_args() {
    let sandbox = TestSandbox::new("wizard_passthrough_routes_to_greentic_dev_with_all_args");
    let log_file = sandbox.path().join("dev.log");
    sandbox.write_arg_logger_tool("greentic-dev", &log_file, 0);
    sandbox.write_exit_tool("greentic-operator", 0);

    let status = sandbox.run_gtc(
        ["wizard", "--locale", "fr", "--answers", "oci://example"],
        HashMap::new(),
    );
    assert_eq!(status.code(), Some(0));

    let logged = fs::read_to_string(log_file).expect("read dev log");
    assert!(logged.contains("wizard --locale fr --answers oci://example"));
}

#[test]
fn wizard_passthrough_preserves_global_locale_for_greentic_dev() {
    let sandbox = TestSandbox::new("wizard_passthrough_preserves_global_locale_for_greentic_dev");
    let log_file = sandbox.path().join("dev.log");
    sandbox.write_arg_logger_tool("greentic-dev", &log_file, 0);
    sandbox.write_exit_tool("greentic-operator", 0);

    let status = sandbox.run_gtc(
        ["--locale", "fr", "wizard", "--answers", "oci://example"],
        HashMap::new(),
    );
    assert_eq!(status.code(), Some(0));

    let logged = fs::read_to_string(log_file).expect("read dev log");
    assert!(logged.contains("wizard --locale fr --answers oci://example"));
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
                    "selected_action": { "enum": ["pack", "bundle"] }
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
fn op_help_passthrough_routes_to_greentic_operator() {
    let sandbox = TestSandbox::new("op_help_passthrough_routes_to_greentic_operator");
    let log_file = sandbox.path().join("op.log");
    sandbox.write_arg_logger_tool("greentic-operator", &log_file, 0);
    sandbox.write_exit_tool("greentic-dev", 0);

    let status = sandbox.run_gtc(["op", "--help"], HashMap::new());
    assert_eq!(status.code(), Some(0));

    let logged = fs::read_to_string(log_file).expect("read op log");
    assert!(logged.contains("--help"));
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
    sandbox.write_version_tool("greentic-setup", "greentic-setup 0.0.0");
    sandbox.write_version_tool("greentic-deployer", "greentic-deployer 0.0.0");

    let mut extra = HashMap::new();
    extra.insert(
        "GREENTIC_DEV_BIN".to_string(),
        override_bin.display().to_string(),
    );

    let output = sandbox.run_gtc_capture(["doctor"], extra);
    assert_eq!(output.status.code(), Some(0));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("greentic-dev: OK (greentic-dev local-test)"));
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

#[test]
fn install_public_mode_calls_greentic_dev_install_tools() {
    let sandbox = TestSandbox::new("install_public_mode_calls_greentic_dev_install_tools");
    let log_file = sandbox.path().join("dev.log");
    let cargo_log_file = sandbox.path().join("cargo.log");

    sandbox.write_arg_logger_tool("greentic-dev", &log_file, 0);
    sandbox.write_exit_tool("greentic-operator", 0);
    sandbox.write_cargo_binstall_tool(&cargo_log_file, None);

    let output = sandbox.run_gtc_capture(["install"], HashMap::new());
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let logged = fs::read_to_string(log_file).expect("read dev log");
    assert!(logged.contains("install tools"));
    let cargo_logged = fs::read_to_string(cargo_log_file).expect("read cargo log");
    assert!(cargo_logged.contains("binstall --version"));
    assert!(cargo_logged.contains("search cargo-binstall --limit 1"));
    assert!(cargo_logged.contains("binstall -y --version 0.4 greentic-dev"));
    assert!(cargo_logged.contains("binstall -y --version 0.4 greentic-operator"));
}

#[test]
#[cfg(unix)]
fn install_tenant_mode_uses_env_key_and_installs_tools_and_docs() {
    let sandbox = TestSandbox::new("install_tenant_mode_uses_env_key_and_installs_tools_and_docs");
    let log_file = sandbox.path().join("dev.log");
    let cargo_log_file = sandbox.path().join("cargo.log");

    sandbox.write_arg_logger_tool("greentic-dev", &log_file, 0);
    sandbox.write_exit_tool("greentic-operator", 0);
    sandbox.write_cargo_binstall_tool(&cargo_log_file, None);

    let mock_root = sandbox.path().join("mock-dist");
    fs::create_dir_all(&mock_root).expect("mock root");

    let tool_zip = mock_root.join("tool.zip");
    write_tool_zip(
        &tool_zip,
        "greentic-enterprise-tool",
        b"#!/bin/sh\necho tool\n",
    );
    let tool_sha256 = sha256_file(&tool_zip);

    let doc_path = mock_root.join("guide.md");
    fs::write(&doc_path, b"# enterprise guide\n").expect("doc write");

    let tool_manifest = serde_json::json!({
        "$schema": "https://raw.githubusercontent.com/greentic-biz/customers-tools/main/schemas/tool.schema.json",
        "schema_version": "1",
        "id": "greentic-enterprise-tool",
        "name": "Enterprise Tool",
        "description": "fixture",
        "install": {
            "type": "release-binary",
            "binary_name": "greentic-enterprise-tool",
            "targets": [
                {
                    "os": "macos",
                    "arch": "x86_64",
                    "url": format!("file://{}", tool_zip.display()),
                    "sha256": tool_sha256.clone()
                },
                {
                    "os": "macos",
                    "arch": "aarch64",
                    "url": format!("file://{}", tool_zip.display()),
                    "sha256": tool_sha256.clone()
                },
                {
                    "os": "linux",
                    "arch": "x86_64",
                    "url": format!("file://{}", tool_zip.display()),
                    "sha256": tool_sha256.clone()
                },
                {
                    "os": "linux",
                    "arch": "aarch64",
                    "url": format!("file://{}", tool_zip.display()),
                    "sha256": tool_sha256
                }
            ]
        },
        "docs": []
    });

    let doc_manifest = serde_json::json!({
        "$schema": "https://raw.githubusercontent.com/greentic-biz/customers-tools/main/schemas/doc.schema.json",
        "schema_version": "1",
        "id": "enterprise-guide",
        "title": "Enterprise Guide",
        "source": {
            "type": "download",
            "url": format!("file://{}", doc_path.display())
        },
        "download_file_name": "enterprise-guide.md",
        "default_relative_path": "guides"
    });

    let manifest = serde_json::json!({
        "$schema": "https://raw.githubusercontent.com/greentic-biz/customers-tools/main/schemas/tenant-tools.schema.json",
        "schema_version": "1",
        "tenant": "acme",
        "tools": [
            {
                "id": "greentic-enterprise-tool",
                "url": format!("file://{}", mock_root.join("tool-manifest.json").display())
            }
        ],
        "docs": [
            {
                "id": "enterprise-guide",
                "url": format!("file://{}", mock_root.join("doc-manifest.json").display())
            }
        ]
    });

    fs::write(
        mock_root.join("tool-manifest.json"),
        serde_json::to_vec_pretty(&tool_manifest).expect("tool manifest json"),
    )
    .expect("tool manifest write");

    fs::write(
        mock_root.join("doc-manifest.json"),
        serde_json::to_vec_pretty(&doc_manifest).expect("doc manifest json"),
    )
    .expect("doc manifest write");

    let manifest_path = mock_root.join("manifest.json");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).expect("manifest json"),
    )
    .expect("manifest write");

    let cargo_home = sandbox.path().join("cargo-home");
    let home = sandbox.path().join("home");
    fs::create_dir_all(&cargo_home).expect("cargo_home");
    fs::create_dir_all(&home).expect("home");

    let mut extra = HashMap::new();
    extra.insert(
        "GTC_TENANT_MANIFEST_URL_TEMPLATE".to_string(),
        format!("file://{}", manifest_path.display()),
    );
    extra.insert("GREENTIC_ACME_KEY".to_string(), "secret-token".to_string());
    extra.insert("CARGO_HOME".to_string(), cargo_home.display().to_string());
    extra.insert("HOME".to_string(), home.display().to_string());

    let output = sandbox.run_gtc_capture(["install", "--tenant", "acme"], extra);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let installed_tool = cargo_home.join("bin").join("greentic-enterprise-tool");
    assert!(
        installed_tool.exists(),
        "tool should be installed to cargo bin"
    );

    let installed_doc = home
        .join(".greentic")
        .join("artifacts")
        .join("docs")
        .join("guides")
        .join("enterprise-guide.md");
    assert!(
        installed_doc.exists(),
        "doc should be installed to artifacts root"
    );

    let logged = fs::read_to_string(log_file).expect("read dev log");
    assert!(logged.contains("install tools"));
    let cargo_logged = fs::read_to_string(cargo_log_file).expect("read cargo log");
    assert!(cargo_logged.contains("binstall -y --version 0.4 greentic-dev"));
    assert!(cargo_logged.contains("binstall -y --version 0.4 greentic-operator"));

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("secret-token"));
}

#[test]
fn install_skips_tenant_when_public_install_fails() {
    let sandbox = TestSandbox::new("install_skips_tenant_when_public_install_fails");
    let cargo_log_file = sandbox.path().join("cargo.log");

    sandbox.write_exit_tool("greentic-dev", 23);
    sandbox.write_exit_tool("greentic-operator", 0);
    sandbox.write_cargo_binstall_tool(&cargo_log_file, None);

    let mock_root = sandbox.path().join("mock-dist");
    fs::create_dir_all(&mock_root).expect("mock root");
    fs::write(mock_root.join("index.json"), b"{}").expect("index write");

    let mut extra = HashMap::new();
    extra.insert(
        "GTC_DIST_MOCK_ROOT".to_string(),
        mock_root.display().to_string(),
    );
    extra.insert("GREENTIC_ACME_KEY".to_string(), "secret-token".to_string());

    let output = sandbox.run_gtc_capture(["install", "--tenant", "acme"], extra);
    assert_eq!(output.status.code(), Some(23));
    let cargo_logged = fs::read_to_string(cargo_log_file).expect("read cargo log");
    assert!(cargo_logged.contains("binstall -y --version 0.4 greentic-dev"));
    assert!(cargo_logged.contains("binstall -y --version 0.4 greentic-operator"));
}

#[test]
fn update_calls_binstall_force_for_all_companions() {
    let sandbox = TestSandbox::new("update_calls_binstall_force_for_all_companions");
    let cargo_log_file = sandbox.path().join("cargo.log");

    sandbox.write_cargo_binstall_tool(&cargo_log_file, None);
    sandbox.write_exit_tool("greentic-dev", 0);
    sandbox.write_exit_tool("greentic-operator", 0);

    let output = sandbox.run_gtc_capture(["update"], HashMap::new());
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let cargo_logged = fs::read_to_string(&cargo_log_file).expect("read cargo log");
    assert!(cargo_logged.contains("binstall -y --force --version 0.4 greentic-dev"));
    assert!(cargo_logged.contains("binstall -y --force --version 0.4 greentic-operator"));
    assert!(cargo_logged.contains("binstall -y --force --version 0.4 greentic-bundle"));
    assert!(cargo_logged.contains("binstall -y --force --version 0.4 greentic-setup"));
    assert!(cargo_logged.contains("binstall -y --force --version 0.4 greentic-deployer"));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Updating greentic-dev"));
    assert!(stdout.contains("Updated greentic-dev: OK"));
}

#[test]
fn update_reports_failure_and_continues() {
    let sandbox = TestSandbox::new("update_reports_failure_and_continues");
    let cargo_log_file = sandbox.path().join("cargo.log");

    sandbox.write_cargo_binstall_tool(&cargo_log_file, Some("greentic-bundle"));
    sandbox.write_exit_tool("greentic-dev", 0);
    sandbox.write_exit_tool("greentic-operator", 0);

    let output = sandbox.run_gtc_capture(["update"], HashMap::new());
    assert_eq!(
        output.status.code(),
        Some(1),
        "should exit 1 when any package fails"
    );

    let cargo_logged = fs::read_to_string(&cargo_log_file).expect("read cargo log");
    assert!(cargo_logged.contains("greentic-dev"));
    assert!(cargo_logged.contains("greentic-bundle"));
    assert!(cargo_logged.contains("greentic-deployer"));

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("greentic-bundle: FAIL")
            || stderr.contains("Updated greentic-bundle: FAIL")
    );
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

#[test]
#[cfg(unix)]
#[cfg_attr(target_os = "macos", ignore)]
fn start_single_vm_creates_artifact_spec_and_state() {
    let sandbox = TestSandbox::new("start_single_vm_creates_artifact_spec_and_state");
    let bundle_dir = sandbox.path().join("bundle");
    create_minimal_bundle_dir(&bundle_dir);

    let setup_log = sandbox.path().join("setup.log");
    let deployer_log = sandbox.path().join("deployer.log");
    sandbox.write_exit_tool("greentic-dev", 0);
    sandbox.write_setup_bundle_tool("greentic-setup", &setup_log);
    sandbox.write_single_vm_deployer_tool("greentic-deployer", &deployer_log);

    let state_home = sandbox.path().join("xdg-state");
    let data_home = sandbox.path().join("xdg-data");
    fs::create_dir_all(&state_home).expect("state home");
    fs::create_dir_all(&data_home).expect("data home");

    let mut extra = HashMap::new();
    extra.insert(
        "XDG_STATE_HOME".to_string(),
        state_home.display().to_string(),
    );
    extra.insert("XDG_DATA_HOME".to_string(), data_home.display().to_string());

    let output = sandbox.run_gtc_capture(
        [
            "start",
            bundle_dir.to_str().expect("bundle utf8"),
            "--target",
            "single-vm",
            "--tenant",
            "acme",
            "--team",
            "ops",
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

    let setup_logged = fs::read_to_string(&setup_log).expect("read setup log");
    assert!(setup_logged.contains("bundle build"));

    let deployer_logged = fs::read_to_string(&deployer_log).expect("read deployer log");
    assert!(deployer_logged.contains("single-vm status"));
    assert!(deployer_logged.contains("single-vm apply"));

    let specs_dir = state_home.join("greentic").join("gtc").join("single-vm");
    let spec_path = find_file_named(&specs_dir, "single-vm.deployment.yaml")
        .expect("single-vm spec should be written");
    let spec = fs::read_to_string(&spec_path).expect("read spec");
    assert!(spec.contains("target: single-vm"));
    assert!(spec.contains("name: acme-ops-"));
    assert!(spec.contains("source: 'file://"));

    let deployments_dir = state_home.join("greentic").join("gtc").join("deployments");
    let state_path = find_single_json_file(&deployments_dir).expect("deployment state file");
    let state_raw = fs::read_to_string(&state_path).expect("read deployment state");
    let state: serde_json::Value = serde_json::from_str(&state_raw).expect("deployment state json");
    assert_eq!(
        state.get("target").and_then(serde_json::Value::as_str),
        Some("single-vm")
    );
    let artifact_path = state
        .get("artifact_path")
        .and_then(serde_json::Value::as_str)
        .expect("artifact path");
    assert!(
        Path::new(artifact_path).exists(),
        "artifact path should exist"
    );
}

#[test]
#[cfg(unix)]
#[cfg_attr(target_os = "macos", ignore)]
fn stop_single_vm_destroy_removes_saved_state() {
    let sandbox = TestSandbox::new("stop_single_vm_destroy_removes_saved_state");
    let bundle_dir = sandbox.path().join("bundle");
    create_minimal_bundle_dir(&bundle_dir);

    let setup_log = sandbox.path().join("setup.log");
    let deployer_log = sandbox.path().join("deployer.log");
    sandbox.write_exit_tool("greentic-dev", 0);
    sandbox.write_setup_bundle_tool("greentic-setup", &setup_log);
    sandbox.write_single_vm_deployer_tool("greentic-deployer", &deployer_log);

    let state_home = sandbox.path().join("xdg-state");
    let data_home = sandbox.path().join("xdg-data");
    fs::create_dir_all(&state_home).expect("state home");
    fs::create_dir_all(&data_home).expect("data home");

    let mut extra = HashMap::new();
    extra.insert(
        "XDG_STATE_HOME".to_string(),
        state_home.display().to_string(),
    );
    extra.insert("XDG_DATA_HOME".to_string(), data_home.display().to_string());

    let start = sandbox.run_gtc_capture(
        [
            "start",
            bundle_dir.to_str().expect("bundle utf8"),
            "--target",
            "single-vm",
        ],
        extra.clone(),
    );
    assert_eq!(
        start.status.code(),
        Some(0),
        "start stderr: {}",
        String::from_utf8_lossy(&start.stderr)
    );

    let deployments_dir = state_home.join("greentic").join("gtc").join("deployments");
    let state_path =
        find_single_json_file(&deployments_dir).expect("deployment state file after start");
    assert!(
        state_path.exists(),
        "deployment state should exist after start"
    );

    let stop = sandbox.run_gtc_capture(
        [
            "stop",
            bundle_dir.to_str().expect("bundle utf8"),
            "--target",
            "single-vm",
            "--destroy",
        ],
        extra,
    );
    assert_eq!(
        stop.status.code(),
        Some(0),
        "stop stderr: {}",
        String::from_utf8_lossy(&stop.stderr)
    );

    let deployer_logged = fs::read_to_string(&deployer_log).expect("read deployer log");
    assert!(deployer_logged.contains("single-vm destroy"));
    assert!(
        !state_path.exists(),
        "deployment state should be removed after single-vm destroy"
    );
}

fn write_tool_zip(path: &Path, file_name: &str, contents: &[u8]) {
    let file = fs::File::create(path).expect("zip create");
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file(format!("bin/{file_name}"), options)
        .expect("zip start file");
    zip.write_all(contents).expect("zip write");
    zip.finish().expect("zip finish");
}

fn sha256_file(path: &Path) -> String {
    let bytes = fs::read(path).expect("read file for sha256");
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn create_minimal_bundle_dir(path: &Path) {
    fs::create_dir_all(path.join("packs")).expect("packs dir");
    fs::write(path.join("bundle.yaml"), "bundle_id: demo\n").expect("bundle.yaml");
    fs::write(path.join("packs").join("default.gtpack"), b"fixture-pack").expect("default pack");
}

fn find_file_named(root: &Path, file_name: &str) -> Option<PathBuf> {
    if root.is_file() {
        return root
            .file_name()
            .filter(|name| *name == file_name)
            .map(|_| root.to_path_buf());
    }
    let entries = fs::read_dir(root).ok()?;
    for entry in entries {
        let path = entry.ok()?.path();
        if path.is_dir() {
            if let Some(found) = find_file_named(&path, file_name) {
                return Some(found);
            }
        } else if path.file_name().is_some_and(|name| name == file_name) {
            return Some(path);
        }
    }
    None
}

fn find_single_json_file(root: &Path) -> Option<PathBuf> {
    let mut matches = Vec::new();
    let entries = fs::read_dir(root).ok()?;
    for entry in entries {
        let path = entry.ok()?.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            matches.push(path);
        }
    }
    if matches.len() == 1 {
        matches.pop()
    } else {
        None
    }
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

    fn write_setup_bundle_tool(&self, name: &str, log_file: &Path) {
        let path = self.binary_path(name);
        self.compile_rust_tool_at(&path, &rust_setup_bundle_tool_program(log_file));
    }

    fn write_single_vm_deployer_tool(&self, name: &str, log_file: &Path) {
        let path = self.binary_path(name);
        self.compile_rust_tool_at(&path, &rust_single_vm_deployer_tool_program(log_file));
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
        let current_path = env::var_os("PATH").unwrap_or_default();
        let mut path_entries = vec![self.root.clone()];
        path_entries.extend(env::split_paths(&current_path));
        let merged_path = env::join_paths(path_entries).expect("join PATH");

        let mut cmd = Command::new(env!("CARGO_BIN_EXE_gtc"));
        cmd.args(args).env("PATH", merged_path);

        for (binary, env_key) in [
            ("greentic-dev", "GREENTIC_DEV_BIN"),
            ("greentic-operator", "GREENTIC_OPERATOR_BIN"),
            ("greentic-setup", "GREENTIC_SETUP_BIN"),
            ("greentic-bundle", "GREENTIC_BUNDLE_BIN"),
            ("greentic-deployer", "GREENTIC_DEPLOYER_BIN"),
        ] {
            let path = self.binary_path(binary);
            if path.is_file() {
                cmd.env(env_key, path);
            }
        }

        for (k, v) in extra_env {
            cmd.env(k, v);
        }

        cmd.output().expect("run gtc")
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

fn rust_exit_tool_program(exit_code: i32) -> String {
    format!("fn main() {{ std::process::exit({exit_code}); }}",)
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
        println!("cargo-binstall = \"1.0.0\" # mock");
        return;
    }}
    if args.first().map(String::as_str) == Some("binstall")
        && args.get(1).map(String::as_str) == Some("--version")
    {{
        println!("cargo-binstall 1.0.0");
        return;
    }}
{fail_check}
}}
"#
    )
}

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

fn rust_single_vm_deployer_tool_program(log_file: &Path) -> String {
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

    match args.get(1).map(String::as_str) {{
        Some("status") => {{
            println!("{{{{\"status\":\"missing\"}}}}");
        }}
        Some("apply") | Some("destroy") | None | Some(_) => {{}}
    }}
}}
"#
    )
}
