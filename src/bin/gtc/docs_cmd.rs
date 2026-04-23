use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use clap::ArgMatches;

pub(super) fn run_docs(sub_matches: &ArgMatches, debug: bool, _locale: &str) -> i32 {
    match sub_matches.subcommand() {
        Some(("sync-schemas", sync_matches)) => run_sync_schemas(sync_matches, debug),
        _ => {
            eprintln!("usage: gtc docs sync-schemas [--best-effort|--strict]");
            2
        }
    }
}

fn run_sync_schemas(sub_matches: &ArgMatches, debug: bool) -> i32 {
    let Some(script_path) = resolve_sync_script_path() else {
        eprintln!(
            "failed to locate ci/sync_schema_docs.sh; run this command from the repo root or a workspace checkout"
        );
        return 1;
    };

    let mode = if sub_matches.get_flag("strict") {
        "--strict"
    } else {
        "--best-effort"
    };

    if debug {
        eprintln!(
            "gtc: running schema sync via bash {} {}",
            script_path.display(),
            mode
        );
    }

    match Command::new("bash")
        .arg(&script_path)
        .arg(mode)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
    {
        Ok(status) => status.code().unwrap_or(1),
        Err(err) => {
            eprintln!(
                "failed to execute schema sync script {}: {err}",
                script_path.display()
            );
            1
        }
    }
}

fn resolve_sync_script_path() -> Option<PathBuf> {
    let cwd_candidate = env::current_dir()
        .ok()?
        .join("ci")
        .join("sync_schema_docs.sh");
    if cwd_candidate.is_file() {
        return Some(cwd_candidate);
    }

    let exe_candidate = env::current_exe()
        .ok()
        .and_then(|path| resolve_repo_root_from_exe(&path))
        .map(|root| root.join("ci").join("sync_schema_docs.sh"));
    if let Some(path) = exe_candidate
        && path.is_file()
    {
        return Some(path);
    }

    None
}

fn resolve_repo_root_from_exe(current_exe: &Path) -> Option<PathBuf> {
    Some(current_exe.parent()?.parent()?.parent()?.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::{resolve_repo_root_from_exe, resolve_sync_script_path};
    use std::env;
    use std::path::Path;

    #[test]
    fn resolve_repo_root_from_target_debug_exe() {
        let root =
            resolve_repo_root_from_exe(Path::new("/tmp/repo/target/debug/gtc")).expect("repo root");
        assert_eq!(root, Path::new("/tmp/repo"));
    }

    #[test]
    fn resolve_sync_script_path_prefers_repo_cwd() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ci_dir = dir.path().join("ci");
        std::fs::create_dir_all(&ci_dir).expect("mkdir");
        let script = ci_dir.join("sync_schema_docs.sh");
        std::fs::write(&script, "#!/usr/bin/env bash\n").expect("write");

        let old = env::current_dir().expect("cwd");
        env::set_current_dir(dir.path()).expect("set cwd");
        let resolved = resolve_sync_script_path().expect("script path");
        env::set_current_dir(old).expect("restore cwd");

        // macOS tempdirs live under /var which is a symlink to /private/var;
        // `current_dir()` returns the canonical form, so compare canonicalized paths.
        assert_eq!(
            resolved.canonicalize().expect("canonicalize resolved"),
            script.canonicalize().expect("canonicalize script"),
        );
    }
}
