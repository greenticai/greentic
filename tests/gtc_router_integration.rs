#![cfg(unix)]

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
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
fn wizard_default_routes_to_greentic_pack_wizard_with_locale() {
    let sandbox = TestSandbox::new("wizard_default_routes_to_greentic_pack_wizard_with_locale");
    let log_file = sandbox.path().join("pack.log");
    let op_log_file = sandbox.path().join("op.log");

    let pack_script = format!(
        "#!/bin/sh\nprintf 'args:%s\\n' \"$*\" >> '{}'\nprintf 'locale:%s\\n' \"$GREENTIC_LOCALE\" >> '{}'\nexit 0\n",
        log_file.display(),
        log_file.display()
    );

    sandbox.write_script("greentic-dev", "#!/bin/sh\nexit 0\n");
    sandbox.write_script("greentic-pack", &pack_script);
    let op_script = format!(
        "#!/bin/sh\nprintf 'args:%s\\n' \"$*\" >> '{}'\nprintf 'locale:%s\\n' \"$GREENTIC_LOCALE\" >> '{}'\nexit 0\n",
        op_log_file.display(),
        op_log_file.display()
    );
    sandbox.write_script("greentic-operator", &op_script);

    let status = sandbox.run_gtc(["wizard", "foo", "--bar"], HashMap::new());
    assert_eq!(
        status.code(),
        Some(0),
        "wizard should still route via dev when args are present"
    );

    let default_output = sandbox.run_gtc_capture_with_input(["wizard"], HashMap::new(), "1\n");
    assert_eq!(default_output.status.code(), Some(0));

    let logged = fs::read_to_string(log_file).expect("read pack log");
    assert!(logged.contains("args:wizard --locale en"));
    assert!(logged.contains("locale:en"));

    let operator_output =
        sandbox.run_gtc_capture_with_input(["wizard"], HashMap::new(), "2\ncustom.gtbundle\n");
    assert_eq!(operator_output.status.code(), Some(0));
    let op_logged = fs::read_to_string(op_log_file).expect("read operator log");
    assert!(op_logged.contains("args:demo new custom.gtbundle --locale en"));
    assert!(op_logged.contains("locale:en"));
}

#[test]
fn wizard_with_answers_routes_to_operator_demo_new() {
    let sandbox = TestSandbox::new("wizard_with_answers_routes_to_operator_demo_new");
    let log_file = sandbox.path().join("op.log");

    let op_script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit 0\n",
        log_file.display()
    );

    sandbox.write_script("greentic-dev", "#!/bin/sh\nexit 0\n");
    sandbox.write_script("greentic-operator", &op_script);

    let status = sandbox.run_gtc(["wizard", "--answers", "oci://example"], HashMap::new());
    assert_eq!(status.code(), Some(0));

    let logged = fs::read_to_string(log_file).expect("read op log");
    assert!(logged.contains("demo new myfirst.gtbundle"));
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
    assert!(cargo_logged.contains("binstall -y --version 0.4 greentic-dev greentic-operator"));
}

#[test]
fn install_tenant_mode_uses_env_key_and_installs_artifacts() {
    let sandbox = TestSandbox::new("install_tenant_mode_uses_env_key_and_installs_artifacts");
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

    let pack_dir = mock_root.join("pack");
    fs::create_dir_all(&pack_dir).expect("pack dir");
    fs::write(pack_dir.join("README.txt"), b"enterprise pack").expect("pack write");

    let manifest = serde_json::json!({
        "schema": "greentic.install.manifest.v1",
        "items": [
            {
                "kind": "tool",
                "name": "greentic-enterprise-tool",
                "oci": "oci://ghcr.io/greentic-biz/tools/enterprise-tool:1.0.0"
            },
            {
                "kind": "pack",
                "name": "enterprise-routing",
                "oci": "oci://ghcr.io/greentic-biz/packs/enterprise-routing:0.4.0"
            }
        ]
    });

    let manifest_path = mock_root.join("manifest.json");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).expect("manifest json"),
    )
    .expect("manifest write");

    let index = serde_json::json!({
        "oci://ghcr.io/greentic-biz/customers-tools/acme:latest": "manifest.json",
        "oci://ghcr.io/greentic-biz/tools/enterprise-tool:1.0.0": "tool.zip",
        "oci://ghcr.io/greentic-biz/packs/enterprise-routing:0.4.0": "pack"
    });
    fs::write(
        mock_root.join("index.json"),
        serde_json::to_vec_pretty(&index).expect("index json"),
    )
    .expect("index write");

    let cargo_home = sandbox.path().join("cargo-home");
    let home = sandbox.path().join("home");
    fs::create_dir_all(&cargo_home).expect("cargo_home");
    fs::create_dir_all(&home).expect("home");

    let mut extra = HashMap::new();
    extra.insert(
        "GTC_DIST_MOCK_ROOT".to_string(),
        mock_root.display().to_string(),
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

    let installed_pack = home
        .join(".greentic")
        .join("artifacts")
        .join("packs")
        .join("enterprise-routing")
        .join("README.txt");
    assert!(
        installed_pack.exists(),
        "pack should be installed to artifacts root"
    );

    let logged = fs::read_to_string(log_file).expect("read dev log");
    assert!(logged.contains("install tools"));
    let cargo_logged = fs::read_to_string(cargo_log_file).expect("read cargo log");
    assert!(cargo_logged.contains("binstall -y --version 0.4 greentic-dev greentic-operator"));

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
    assert!(cargo_logged.contains("binstall -y --version 0.4 greentic-dev greentic-operator"));
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

        for (k, v) in extra_env {
            cmd.env(k, v);
        }

        cmd.output().expect("run gtc")
    }

    fn run_gtc_capture_with_input<const N: usize>(
        &self,
        args: [&str; N],
        extra_env: HashMap<String, String>,
        input: &str,
    ) -> std::process::Output {
        let current_path = env::var("PATH").unwrap_or_default();
        let merged_path = format!("{}:{}", self.root.display(), current_path);

        let mut cmd = Command::new(env!("CARGO_BIN_EXE_gtc"));
        cmd.args(args)
            .env("PATH", merged_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        for (k, v) in extra_env {
            cmd.env(k, v);
        }

        let mut child = cmd.spawn().expect("spawn gtc");
        let mut stdin = child.stdin.take().expect("stdin");
        stdin
            .write_all(input.as_bytes())
            .expect("write stdin to gtc");
        drop(stdin);
        child.wait_with_output().expect("wait gtc")
    }
}

impl Drop for TestSandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
