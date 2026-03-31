use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};

use directories::BaseDirs;
use gtc::config::GtcConfig;
use gtc::error::{GtcError, GtcResult};

use super::deploy::{ChildProcessEnv, StartTarget, default_operator_image_for_target};
use super::{BUNDLE_BIN, DEFAULT_OPERATOR_IMAGE_DIGEST, DEPLOYER_BIN, DEV_BIN, OP_BIN, SETUP_BIN};
use crate::i18n_support::{t, t_or};

pub(super) fn run_binary_checked(
    binary: &str,
    args: &[String],
    debug: bool,
    locale: &str,
    operation: &str,
) -> GtcResult<()> {
    run_binary_checked_with_target_and_env(binary, args, debug, locale, operation, None, None)
}

pub(super) fn run_binary_checked_with_target(
    binary: &str,
    args: &[String],
    debug: bool,
    locale: &str,
    operation: &str,
    target: Option<StartTarget>,
) -> GtcResult<()> {
    run_binary_checked_with_target_and_env(binary, args, debug, locale, operation, target, None)
}

pub(super) fn run_binary_checked_with_target_and_env(
    binary: &str,
    args: &[String],
    debug: bool,
    locale: &str,
    operation: &str,
    target: Option<StartTarget>,
    extra_env: Option<&ChildProcessEnv>,
) -> GtcResult<()> {
    let status =
        run_binary_status_with_target_and_env(binary, args, debug, locale, target, extra_env)?;
    if status.success() {
        return Ok(());
    }
    Err(GtcError::message(format!(
        "{operation} failed via {binary} with status {}",
        status.code().unwrap_or(1)
    )))
}

pub(super) fn run_binary_capture(
    binary: &str,
    args: &[String],
    debug: bool,
    locale: &str,
) -> GtcResult<String> {
    run_binary_capture_with_target(binary, args, debug, locale, None)
}

pub(super) fn run_binary_capture_with_target(
    binary: &str,
    args: &[String],
    debug: bool,
    locale: &str,
    target: Option<StartTarget>,
) -> GtcResult<String> {
    if debug {
        eprintln!("{} {} {:?}", t(locale, "gtc.debug.exec"), binary, args);
    }
    let command = resolve_binary_command(binary);
    let mut process = ProcessCommand::new(&command);
    process.args(args).env("GREENTIC_LOCALE", locale);
    apply_default_deploy_env_for_target(&mut process, target);
    let output = process
        .output()
        .map_err(|err| GtcError::io(format!("failed to execute {binary}"), err))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            return Err(GtcError::message(format!(
                "{binary} exited with status {}",
                output.status.code().unwrap_or(1)
            )));
        }
        return Err(GtcError::message(stderr));
    }
    String::from_utf8(output.stdout)
        .map_err(|err| GtcError::message(format!("invalid UTF-8 from {binary}: {err}")))
}

pub(super) fn run_binary_status_with_target_and_env(
    binary: &str,
    args: &[String],
    debug: bool,
    locale: &str,
    target: Option<StartTarget>,
    extra_env: Option<&ChildProcessEnv>,
) -> GtcResult<std::process::ExitStatus> {
    if debug {
        eprintln!("{} {} {:?}", t(locale, "gtc.debug.exec"), binary, args);
    }
    let command = resolve_binary_command(binary);
    let mut process = ProcessCommand::new(&command);
    process
        .args(args)
        .env("GREENTIC_LOCALE", locale)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    apply_default_deploy_env_for_target(&mut process, target);
    if let Some(extra_env) = extra_env {
        extra_env.apply(&mut process);
    }
    process
        .status()
        .map_err(|err| GtcError::io(format!("failed to execute {binary}"), err))
}

pub(super) fn apply_default_deploy_env_for_target(
    process: &mut ProcessCommand,
    target: Option<StartTarget>,
) {
    let cfg = GtcConfig::from_env();
    if cfg.terraform_operator_image().is_none()
        && let Some(target) = target
        && let Some(image) = default_operator_image_for_target(target)
    {
        process.env("GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE", image);
    }
    if cfg.terraform_operator_image_digest().is_none() {
        process.env(
            "GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE_DIGEST",
            DEFAULT_OPERATOR_IMAGE_DIGEST,
        );
    }
}

pub(super) fn resolve_cargo_bin_dir() -> GtcResult<PathBuf> {
    if let Some(cargo_home) = GtcConfig::from_env().cargo_home() {
        return Ok(cargo_home.join("bin"));
    }

    let base =
        BaseDirs::new().ok_or_else(|| GtcError::message("failed to resolve home directory"))?;
    Ok(base.home_dir().join(".cargo").join("bin"))
}

pub(super) fn passthrough(binary: &str, args: &[String], debug: bool, locale: &str) -> i32 {
    if debug {
        eprintln!("{} {} {:?}", t(locale, "gtc.debug.exec"), binary, args);
    }

    let command = resolve_binary_command(binary);

    match ProcessCommand::new(&command)
        .args(args)
        .env("GREENTIC_LOCALE", locale)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
    {
        Ok(status) => status.code().unwrap_or(1),
        Err(err) => {
            if err.kind() == std::io::ErrorKind::NotFound {
                match binary {
                    DEV_BIN => print_missing_dev_message(locale),
                    OP_BIN => print_missing_op_message(locale),
                    SETUP_BIN => eprintln!("{}", t(locale, "gtc.err.bin_missing_setup")),
                    _ => eprintln!("{}", t(locale, "gtc.err.exec_failed")),
                }
            } else {
                eprintln!("{}: {err}", t(locale, "gtc.err.exec_failed"));
            }
            1
        }
    }
}

pub(super) fn run_doctor(locale: &str) -> i32 {
    let mut failed = false;

    for binary in [DEV_BIN, OP_BIN, BUNDLE_BIN, SETUP_BIN, DEPLOYER_BIN] {
        let command = resolve_binary_command(binary);
        match ProcessCommand::new(&command).arg("--version").output() {
            Ok(output) => {
                let status_label = if output.status.success() {
                    t(locale, "gtc.doctor.ok")
                } else {
                    t(locale, "gtc.doctor.warn")
                };
                let version = first_non_empty_line(&String::from_utf8_lossy(&output.stdout))
                    .or_else(|| first_non_empty_line(&String::from_utf8_lossy(&output.stderr)))
                    .unwrap_or_else(|| t(locale, "gtc.doctor.version_unavailable").into_owned());
                println!("{binary}: {status_label} ({version}) [{}]", command);
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                failed = true;
                println!(
                    "{binary}: {} [{}]",
                    t(locale, "gtc.doctor.missing"),
                    command
                );
                match binary {
                    DEV_BIN => print_missing_dev_message(locale),
                    OP_BIN => print_missing_op_message(locale),
                    BUNDLE_BIN => eprintln!(
                        "{}",
                        t_or(
                            locale,
                            "gtc.err.bin_missing_bundle",
                            "greentic-bundle not found in PATH. Install with: cargo install greentic-bundle",
                        )
                    ),
                    SETUP_BIN => eprintln!("{}", t(locale, "gtc.err.bin_missing_setup")),
                    DEPLOYER_BIN => eprintln!(
                        "{}",
                        t_or(
                            locale,
                            "gtc.err.bin_missing_deployer",
                            "greentic-deployer not found in PATH. Install with: cargo install greentic-deployer",
                        )
                    ),
                    _ => {}
                }
            }
            Err(err) => {
                failed = true;
                println!(
                    "{binary}: {} ({err}) [{}]",
                    t(locale, "gtc.doctor.missing"),
                    command
                );
            }
        }
    }

    if failed { 1 } else { 0 }
}

fn first_non_empty_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToOwned::to_owned)
}

fn print_missing_dev_message(locale: &str) {
    eprintln!("{}", t(locale, "gtc.err.bin_missing_dev"));
    eprintln!(
        "{}",
        t_or(
            locale,
            "gtc.err.bin_missing_dev_install_hint",
            "Run `gtc install` first."
        )
    );
}

fn print_missing_op_message(locale: &str) {
    eprintln!("{}", t(locale, "gtc.err.bin_missing_op"));
    eprintln!(
        "{}",
        t_or(
            locale,
            "gtc.err.bin_missing_op_install_hint",
            "Run `gtc install` first."
        )
    );
}

fn resolve_binary_command(binary: &str) -> String {
    if let Some(path) = resolve_companion_binary(binary) {
        return path.display().to_string();
    }
    match binary {
        DEV_BIN => GtcConfig::from_env()
            .dev_bin_override()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| binary.to_string()),
        _ => binary.to_string(),
    }
}

pub(super) fn resolve_companion_binary(binary: &str) -> Option<PathBuf> {
    resolve_companion_binary_from(env::current_exe().ok().as_deref(), binary)
}

pub(super) fn resolve_companion_binary_from(
    current_exe: Option<&Path>,
    binary: &str,
) -> Option<PathBuf> {
    let env_override = companion_binary_env_override(binary);
    if let Some(path) = env_override {
        return Some(PathBuf::from(path));
    }

    if let Some(current_exe) = current_exe {
        let exe_dir = current_exe.parent()?;
        if let Some(sibling) = resolve_binary_in_dir(exe_dir, binary) {
            return Some(sibling);
        }

        if let Some(workspace_candidate) = resolve_workspace_local_binary(current_exe, binary) {
            return Some(workspace_candidate);
        }
    }

    if let Ok(cargo_bin_dir) = resolve_cargo_bin_dir()
        && let Some(cargo_candidate) = resolve_binary_in_dir(&cargo_bin_dir, binary)
    {
        return Some(cargo_candidate);
    }
    None
}

pub(super) fn resolve_binary_in_dir(dir: &Path, binary: &str) -> Option<PathBuf> {
    binary_file_candidates(dir, binary)
        .into_iter()
        .find(|candidate| candidate.is_file())
}

fn binary_file_candidates(dir: &Path, binary: &str) -> Vec<PathBuf> {
    let mut candidates = vec![dir.join(binary)];
    let exe_suffix = env::consts::EXE_SUFFIX;
    if !exe_suffix.is_empty() && !binary.ends_with(exe_suffix) {
        candidates.push(dir.join(format!("{binary}{exe_suffix}")));
    }
    candidates
}

fn resolve_workspace_local_binary(current_exe: &Path, binary: &str) -> Option<PathBuf> {
    let repo_dir = current_exe.parent()?.parent()?.parent()?;
    let workspace_root = repo_dir.parent()?;
    let repo_name = match binary {
        DEV_BIN => "greentic-dev",
        OP_BIN => "greentic-operator",
        BUNDLE_BIN => "greentic-bundle",
        DEPLOYER_BIN => "greentic-deployer",
        SETUP_BIN => "greentic-setup",
        _ => return None,
    };
    let candidate = workspace_root
        .join(repo_name)
        .join("target")
        .join("debug")
        .as_path()
        .to_path_buf();
    resolve_binary_in_dir(&candidate, binary)
}

fn companion_binary_env_override(binary: &str) -> Option<std::ffi::OsString> {
    let cfg = GtcConfig::from_env();
    match binary {
        DEV_BIN => cfg.dev_bin_override(),
        OP_BIN => cfg.operator_bin_override(),
        BUNDLE_BIN => cfg.bundle_bin_override(),
        DEPLOYER_BIN => cfg.deployer_bin_override(),
        SETUP_BIN => cfg.setup_bin_override(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::{apply_default_deploy_env_for_target, run_binary_capture, run_binary_checked};
    use super::{first_non_empty_line, resolve_cargo_bin_dir};
    #[cfg(unix)]
    use crate::deploy::StartTarget;
    #[cfg(unix)]
    use crate::tests::env_test_lock;
    #[cfg(unix)]
    use std::env;
    use std::process::Command as ProcessCommand;

    #[test]
    fn first_non_empty_line_skips_blank_lines() {
        assert_eq!(
            first_non_empty_line("\n  \nhello\nworld"),
            Some("hello".to_string())
        );
        assert_eq!(first_non_empty_line(" \n\t"), None);
    }

    #[test]
    fn resolve_cargo_bin_dir_prefers_env_override() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        unsafe {
            env::set_var("CARGO_HOME", dir.path());
        }

        let resolved = resolve_cargo_bin_dir().expect("cargo bin");

        unsafe {
            env::remove_var("CARGO_HOME");
        }
        assert_eq!(resolved, dir.path().join("bin"));
    }

    #[test]
    #[cfg(unix)]
    fn apply_default_deploy_env_for_target_sets_expected_vars() {
        let _guard = env_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            env::remove_var("GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE");
            env::remove_var("GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE_DIGEST");
        }

        let mut cmd = ProcessCommand::new("env");
        apply_default_deploy_env_for_target(&mut cmd, Some(StartTarget::Aws));
        let envs: Vec<_> = cmd.get_envs().collect();

        assert!(envs.iter().any(|(key, value)| {
            key.to_string_lossy() == "GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE"
                && value.is_some()
        }));
        assert!(envs.iter().any(|(key, value)| {
            key.to_string_lossy() == "GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE_DIGEST"
                && value.is_some()
        }));
    }

    #[cfg(unix)]
    #[test]
    fn run_binary_capture_returns_stdout() {
        let output = run_binary_capture(
            "/bin/sh",
            &["-c".to_string(), "printf ok".to_string()],
            false,
            "en",
        )
        .expect("capture");
        assert_eq!(output, "ok");
    }

    #[cfg(unix)]
    #[test]
    fn run_binary_checked_reports_non_zero_exit() {
        let err = run_binary_checked(
            "/bin/sh",
            &["-c".to_string(), "exit 7".to_string()],
            false,
            "en",
            "demo operation",
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("demo operation failed via /bin/sh with status 7")
        );
    }
}
