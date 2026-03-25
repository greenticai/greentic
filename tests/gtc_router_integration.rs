#![cfg(unix)]

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

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
    sandbox.write_script("greentic-dev", "#!/bin/sh\nexit 17\n");
    sandbox.write_script("greentic-operator", "#!/bin/sh\nexit 0\n");

    let status = sandbox.run_gtc(["dev", "flow", "list"], HashMap::new());
    assert_eq!(status.code(), Some(17));
}

#[test]
fn wizard_missing_greentic_operator_prints_install_guidance() {
    let sandbox = TestSandbox::new("wizard_missing_greentic_operator_prints_install_guidance");
    sandbox.write_script("greentic-dev", "#!/bin/sh\nexit 0\n");

    let mut extra = HashMap::new();
    extra.insert(
        "GREENTIC_OPERATOR_BIN".to_string(),
        sandbox
            .path()
            .join("missing-greentic-operator")
            .display()
            .to_string(),
    );

    let output = sandbox.run_gtc_capture(["wizard"], extra);
    assert_eq!(output.status.code(), Some(1));

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("greentic-operator not found in PATH."));
    assert!(stderr.contains("gtc install"));
}

#[test]
fn passthrough_dev_uses_greentic_dev_bin_override() {
    let sandbox = TestSandbox::new("passthrough_dev_uses_greentic_dev_bin_override");
    let log_file = sandbox.path().join("override-dev.log");
    let override_bin = sandbox.path().join("bin").join("greentic-dev-local");
    fs::create_dir_all(override_bin.parent().expect("override parent")).expect("override dir");

    let override_script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit 0\n",
        log_file.display()
    );
    fs::write(&override_bin, override_script).expect("write override dev");
    fs::set_permissions(&override_bin, fs::Permissions::from_mode(0o755))
        .expect("chmod override dev");

    sandbox.write_script("greentic-dev", "#!/bin/sh\nexit 91\n");
    sandbox.write_script("greentic-operator", "#!/bin/sh\nexit 0\n");

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
fn wizard_passthrough_routes_to_greentic_operator_with_all_args() {
    let sandbox = TestSandbox::new("wizard_passthrough_routes_to_greentic_operator_with_all_args");
    let log_file = sandbox.path().join("op.log");

    let op_script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit 0\n",
        log_file.display()
    );

    sandbox.write_script("greentic-dev", "#!/bin/sh\nexit 0\n");
    sandbox.write_script("greentic-operator", &op_script);

    let status = sandbox.run_gtc(
        ["wizard", "--locale", "fr", "--answers", "oci://example"],
        HashMap::new(),
    );
    assert_eq!(status.code(), Some(0));

    let logged = fs::read_to_string(log_file).expect("read op log");
    assert!(logged.contains("wizard --locale fr --answers oci://example"));
}

#[test]
fn wizard_passthrough_preserves_global_locale_for_greentic_operator() {
    let sandbox =
        TestSandbox::new("wizard_passthrough_preserves_global_locale_for_greentic_operator");
    let log_file = sandbox.path().join("op.log");

    let op_script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit 0\n",
        log_file.display()
    );

    sandbox.write_script("greentic-dev", "#!/bin/sh\nexit 0\n");
    sandbox.write_script("greentic-operator", &op_script);

    let status = sandbox.run_gtc(
        ["--locale", "fr", "wizard", "--answers", "oci://example"],
        HashMap::new(),
    );
    assert_eq!(status.code(), Some(0));

    let logged = fs::read_to_string(log_file).expect("read op log");
    assert!(logged.contains("wizard --locale fr --answers oci://example"));
}

#[test]
fn wizard_passthrough_routes_to_greentic_operator_without_args() {
    let sandbox = TestSandbox::new("wizard_passthrough_routes_to_greentic_operator_without_args");
    let log_file = sandbox.path().join("op.log");

    let op_script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit 0\n",
        log_file.display()
    );

    sandbox.write_script("greentic-dev", "#!/bin/sh\nexit 0\n");
    sandbox.write_script("greentic-operator", &op_script);

    let status = sandbox.run_gtc(["wizard"], HashMap::new());
    assert_eq!(status.code(), Some(0));

    let logged = fs::read_to_string(log_file).expect("read op log");
    assert!(logged.contains("wizard"));
}

#[test]
fn wizard_emit_answers_writes_requested_file_via_operator_wizard() {
    let sandbox = TestSandbox::new("wizard_emit_answers_writes_requested_file_via_operator_wizard");
    let answers_path = sandbox.path().join("answers.json");

    sandbox.write_script(
        "greentic-dev",
        "#!/bin/sh\nif [ \"$1\" = \"wizard\" ]; then\n  : > \"$4\"\n  printf '{\"wrong\":\"binary\"}\\n'\nfi\nexit 0\n",
    );
    sandbox.write_script(
        "greentic-operator",
        "#!/bin/sh\nemit=''\nprev=''\nfor arg in \"$@\"; do\n  if [ \"$prev\" = '--emit-answers' ]; then\n    emit=\"$arg\"\n    break\n  fi\n  prev=\"$arg\"\ndone\nif [ -z \"$emit\" ]; then\n  echo 'missing --emit-answers target' >&2\n  exit 9\nfi\nprintf '{\"schema_version\":\"1.0.0\",\"answers\":{},\"events\":[]}\n' > \"$emit\"\nexit 0\n",
    );

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
        "launcher should not fall back to the dev wizard's stdout-only behavior"
    );
}

#[test]
fn setup_help_passthrough_routes_to_greentic_setup() {
    let sandbox = TestSandbox::new("setup_help_passthrough_routes_to_greentic_setup");
    let log_file = sandbox.path().join("setup.log");

    let setup_script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit 0\n",
        log_file.display()
    );

    sandbox.write_script("greentic-setup", &setup_script);
    sandbox.write_script("greentic-dev", "#!/bin/sh\nexit 0\n");
    sandbox.write_script("greentic-operator", "#!/bin/sh\nexit 0\n");

    let status = sandbox.run_gtc(["setup", "--help"], HashMap::new());
    assert_eq!(status.code(), Some(0));

    let logged = fs::read_to_string(log_file).expect("read setup log");
    assert!(logged.contains("--help"));
}

#[test]
fn op_help_passthrough_routes_to_greentic_operator() {
    let sandbox = TestSandbox::new("op_help_passthrough_routes_to_greentic_operator");
    let log_file = sandbox.path().join("op.log");

    let op_script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit 0\n",
        log_file.display()
    );

    sandbox.write_script("greentic-operator", &op_script);
    sandbox.write_script("greentic-dev", "#!/bin/sh\nexit 0\n");

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

    fs::write(
        &override_bin,
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  echo 'greentic-dev local-test'\n  exit 0\nfi\nexit 0\n",
    )
    .expect("write override dev");
    fs::set_permissions(&override_bin, fs::Permissions::from_mode(0o755))
        .expect("chmod override dev");

    sandbox.write_script("greentic-dev", "#!/bin/sh\nexit 92\n");
    sandbox.write_script(
        "greentic-operator",
        "#!/bin/sh\necho 'greentic-operator 0.0.0'\n",
    );
    sandbox.write_script(
        "greentic-bundle",
        "#!/bin/sh\necho 'greentic-bundle 0.0.0'\n",
    );
    sandbox.write_script("greentic-setup", "#!/bin/sh\necho 'greentic-setup 0.0.0'\n");
    sandbox.write_script(
        "greentic-deployer",
        "#!/bin/sh\necho 'greentic-deployer 0.0.0'\n",
    );

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

    let op_script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit 0\n",
        log_file.display()
    );

    sandbox.write_script("greentic-dev", "#!/bin/sh\nexit 0\n");
    sandbox.write_script("greentic-operator", &op_script);

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

    let op_script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit 0\n",
        log_file.display()
    );

    sandbox.write_script("greentic-dev", "#!/bin/sh\nexit 0\n");
    sandbox.write_script("greentic-operator", &op_script);

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

    let dev_script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit 0\n",
        log_file.display()
    );

    sandbox.write_script("greentic-dev", &dev_script);
    sandbox.write_script("greentic-operator", "#!/bin/sh\nexit 0\n");
    let cargo_script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nif [ \"$1\" = \"search\" ]; then\n  echo 'cargo-binstall = \"1.0.0\" # mock'\nfi\nif [ \"$1\" = \"binstall\" ] && [ \"$2\" = \"--version\" ]; then\n  echo 'cargo-binstall 1.0.0'\nfi\nexit 0\n",
        cargo_log_file.display()
    );
    sandbox.write_script("cargo", &cargo_script);

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
fn install_tenant_mode_uses_env_key_and_installs_tools_and_docs() {
    let sandbox = TestSandbox::new("install_tenant_mode_uses_env_key_and_installs_tools_and_docs");
    let log_file = sandbox.path().join("dev.log");
    let cargo_log_file = sandbox.path().join("cargo.log");

    let dev_script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit 0\n",
        log_file.display()
    );

    sandbox.write_script("greentic-dev", &dev_script);
    sandbox.write_script("greentic-operator", "#!/bin/sh\nexit 0\n");
    let cargo_script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nif [ \"$1\" = \"search\" ]; then\n  echo 'cargo-binstall = \"1.0.0\" # mock'\nfi\nif [ \"$1\" = \"binstall\" ] && [ \"$2\" = \"--version\" ]; then\n  echo 'cargo-binstall 1.0.0'\nfi\nexit 0\n",
        cargo_log_file.display()
    );
    sandbox.write_script("cargo", &cargo_script);

    let mock_root = sandbox.path().join("mock-dist");
    fs::create_dir_all(&mock_root).expect("mock root");

    let tool_zip = mock_root.join("tool.zip");
    write_tool_zip(
        &tool_zip,
        "greentic-enterprise-tool",
        b"#!/bin/sh\necho tool\n",
    );

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
                    "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                },
                {
                    "os": "macos",
                    "arch": "aarch64",
                    "url": format!("file://{}", tool_zip.display()),
                    "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                },
                {
                    "os": "linux",
                    "arch": "x86_64",
                    "url": format!("file://{}", tool_zip.display()),
                    "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
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

    sandbox.write_script("greentic-dev", "#!/bin/sh\nexit 23\n");
    sandbox.write_script("greentic-operator", "#!/bin/sh\nexit 0\n");
    let cargo_script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nif [ \"$1\" = \"search\" ]; then\n  echo 'cargo-binstall = \"1.0.0\" # mock'\nfi\nif [ \"$1\" = \"binstall\" ] && [ \"$2\" = \"--version\" ]; then\n  echo 'cargo-binstall 1.0.0'\nfi\nexit 0\n",
        cargo_log_file.display()
    );
    sandbox.write_script("cargo", &cargo_script);

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

fn write_tool_zip(path: &Path, file_name: &str, contents: &[u8]) {
    let file = fs::File::create(path).expect("zip create");
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file(format!("bin/{file_name}"), options)
        .expect("zip start file");
    zip.write_all(contents).expect("zip write");
    zip.finish().expect("zip finish");
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

    fn write_script(&self, name: &str, content: &str) {
        let path = self.root.join(name);
        fs::write(&path, content).expect("write script");
        let mut perms = fs::metadata(&path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).expect("chmod");
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
        let current_path = env::var("PATH").unwrap_or_default();
        let merged_path = format!("{}:{}", self.root.display(), current_path);

        let mut cmd = Command::new(env!("CARGO_BIN_EXE_gtc"));
        cmd.args(args).env("PATH", merged_path);

        for (binary, env_key) in [
            ("greentic-dev", "GREENTIC_DEV_BIN"),
            ("greentic-operator", "GREENTIC_OPERATOR_BIN"),
            ("greentic-setup", "GREENTIC_SETUP_BIN"),
            ("greentic-bundle", "GREENTIC_BUNDLE_BIN"),
            ("greentic-deployer", "GREENTIC_DEPLOYER_BIN"),
        ] {
            let path = self.root.join(binary);
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
