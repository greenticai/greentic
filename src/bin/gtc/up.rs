//! `gtc up` — the one-line bootstrap.
//!
//! Composes the four commands the README documents (`install`, `wizard`,
//! `setup`, `start`) into one. It composes **gtc's own entry points**, not the
//! companion binaries' argv: `gtc start <dir>` in particular is not a
//! passthrough — it resolves the bundle ref, prepares it, generates admin
//! certs, runs `greentic-setup env-deploy`, and only then starts
//! `greentic-start` *bundle-less*. Rebuilding that argv here would start a
//! server with nothing deployed, and it would fail at runtime rather than at
//! parse time. Calling [`run_start`] keeps `up` correct for free as those
//! steps change.

use std::path::{Path, PathBuf};

use clap::ArgMatches;
use serde_json::{Map, Value};

use crate::cli::build_cli;
use crate::commands::{
    ResolvedAnswersArgs, answers_error_exit_code, check_release_context, resolve_answers_args,
};
use crate::deploy::run_start;
use crate::i18n_support::t_or;
use crate::install::run_install;
use crate::process::passthrough;
use crate::router::route_passthrough_subcommand;

/// Everything `run_up` needs from the command line, resolved once.
struct UpPlan {
    /// Absolute — `output_dir` in the create document is CWD-relative, and a
    /// later step must not re-resolve it against a different directory.
    bundle_dir: PathBuf,
    create_answers: String,
    setup_answers: String,
    start_tail: Vec<String>,
    install: bool,
    start: bool,
    dry_run: bool,
    force: bool,
    /// Held only for their `Drop`: a materialized `oci://` / `store://` /
    /// `repo://` document lives in a tempdir that must outlive the child
    /// process reading it. Dropping either early turns into a confusing
    /// "file not found" from a companion binary.
    _answers_tempdirs: Vec<ResolvedAnswersArgs>,
}

pub(super) fn run_up(
    sub_matches: &ArgMatches,
    default_channel: &'static str,
    invocation: Option<&str>,
    debug: bool,
    locale: &str,
) -> i32 {
    let plan = match build_plan(sub_matches) {
        Ok(plan) => plan,
        Err(UpError { message, code }) => {
            eprintln!("{message}");
            return code;
        }
    };

    // The bundle-directory guard runs before anything else mutates the
    // machine: `up` hides which of its steps is destructive, so it must
    // refuse earlier than the four commands it replaces would.
    if plan.bundle_dir.exists() && !plan.force {
        eprintln!(
            "{} already exists. `up` runs the wizard in create mode, which \
             overwrites a bundle you may have edited and orphans a tunnel \
             still running against it. Pass --force to overwrite it, \
             --bundle-dir <PATH> to build elsewhere, or run `gtc setup` and \
             `gtc start` directly against it.",
            plan.bundle_dir.display()
        );
        return 2;
    }

    // Once, not once per delegated step. `gtc wizard` and `gtc setup` each run
    // this check today; the companion binaries know nothing about it, so there
    // is no flag to suppress downstream — simply don't call it three times.
    // It performs network I/O, so the dedup is a latency win too.
    if !sub_matches.get_flag("ignore-release-context")
        && let Some(status) = check_release_context(default_channel, invocation, debug, locale)
    {
        if sub_matches.get_flag("strict-release-context") {
            eprintln!("error: {status}");
            return 1;
        }
        eprintln!("warning: {status}");
    }

    if plan.dry_run {
        print_dry_run(&plan);
        return 0;
    }

    if plan.install
        && let status = run_install_step(default_channel, debug, locale)
        && status != 0
    {
        return fail(status, "install", "gtc install");
    }

    let wizard_tail = vec!["--answers".to_string(), plan.create_answers.clone()];
    let status = run_passthrough_step("wizard", &wizard_tail, debug, locale);
    if status != 0 {
        return fail(
            status,
            "wizard",
            &format!("gtc wizard --answers {}", plan.create_answers),
        );
    }

    // The predicted path is a second implementation of a rule owned by
    // greentic-bundle (`output_dir`, else `default_bundle_output_dir` over a
    // normalized `bundle_id`). Confirm it rather than discovering the drift
    // three steps later as "bundle not found".
    if !plan.bundle_dir.is_dir() {
        eprintln!(
            "the wizard succeeded but {} does not exist. `up` predicts the \
             bundle directory from the create-answers document; this one \
             disagrees with what the wizard actually created. Re-run with \
             --bundle-dir <PATH>, or run `gtc setup` and `gtc start` against \
             the directory the wizard reported.",
            plan.bundle_dir.display()
        );
        return 1;
    }

    let setup_tail = setup_tail(&plan.bundle_dir, &plan.setup_answers);
    let status = run_passthrough_step("setup", &setup_tail, debug, locale);
    if status != 0 {
        return fail(
            status,
            "setup",
            &format!(
                "gtc setup {} --answers {} --non-interactive",
                plan.bundle_dir.display(),
                plan.setup_answers
            ),
        );
    }

    if !plan.start {
        // The tail is only ever forwarded to start, so with --no-start it
        // would silently vanish. Put it in the printed command instead.
        let tail = start_tail_suffix(&plan.start_tail);
        println!(
            "{} is set up. Start it with:\n  gtc start {}{tail}",
            plan.bundle_dir.display(),
            plan.bundle_dir.display()
        );
        return 0;
    }

    let mut start_tail = vec![plan.bundle_dir.display().to_string()];
    start_tail.extend(plan.start_tail.iter().cloned());
    run_start(&start_tail, debug, locale)
}

struct UpError {
    message: String,
    code: i32,
}

impl UpError {
    fn usage(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: 2,
        }
    }
}

fn build_plan(sub_matches: &ArgMatches) -> Result<UpPlan, UpError> {
    let create_source = sub_matches
        .get_one::<String>("answers")
        .expect("clap requires --answers");
    let setup_source = sub_matches
        .get_one::<String>("setup-answers")
        .expect("clap requires --setup-answers");

    // Route both documents through the same resolver `gtc wizard` and
    // `gtc setup` use, so `oci://`, `store://` and `repo://` behave here
    // exactly as they do there.
    let create = resolve_one(create_source)?;
    let setup = resolve_one(setup_source)?;
    let create_answers = answers_value(&create);
    let setup_answers = answers_value(&setup);

    let bundle_dir = match sub_matches.get_one::<String>("bundle-dir") {
        Some(explicit) => PathBuf::from(explicit),
        None => {
            let document = crate::answer_resolver::load_answers(&create_answers)
                .map_err(|err| UpError::usage(format!("--answers {create_answers}: {err}")))?;
            predict_bundle_dir(&document).map_err(UpError::usage)?
        }
    };
    let bundle_dir = absolutize(&bundle_dir)
        .map_err(|err| UpError::usage(format!("resolving {}: {err}", bundle_dir.display())))?;

    Ok(UpPlan {
        bundle_dir,
        create_answers,
        setup_answers,
        start_tail: sub_matches
            .get_many::<String>("args")
            .map(|values| values.cloned().collect())
            .unwrap_or_default(),
        install: !sub_matches.get_flag("no-install"),
        start: !sub_matches.get_flag("no-start"),
        dry_run: sub_matches.get_flag("dry-run"),
        force: sub_matches.get_flag("force"),
        _answers_tempdirs: vec![create, setup],
    })
}

fn resolve_one(source: &str) -> Result<ResolvedAnswersArgs, UpError> {
    resolve_answers_args(&["--answers".to_string(), source.to_string()]).map_err(|err| UpError {
        message: format!("{err}"),
        code: answers_error_exit_code(&err),
    })
}

/// The resolved path `resolve_answers_args` rewrote `--answers <src>` into.
fn answers_value(resolved: &ResolvedAnswersArgs) -> String {
    resolved
        .args
        .get(1)
        .cloned()
        .expect("resolve_answers_args preserves the --answers value")
}

/// Predict the directory the bundle wizard will create.
///
/// The wizard child inherits stdio, so the path it creates cannot be read back
/// from it — but the create-answers document already carries it, so `up` reads
/// it up front. Mirrors `greentic-bundle`'s `normalize_bundle_id` /
/// `default_bundle_output_dir`; the caller confirms the directory exists after
/// the wizard runs, because this mirror can drift.
pub(crate) fn predict_bundle_dir(document: &Map<String, Value>) -> Result<PathBuf, String> {
    let answers = document.get("answers").and_then(Value::as_object).ok_or(
        "create-answers document has no `answers` object — is this a \
         greentic-dev launcher document?",
    )?;

    // `selected_action` is required by the launcher schema. A `pack` run
    // delegates to the pack wizard, which creates no bundle directory, so
    // there is nothing for setup or start to act on.
    match answers.get("selected_action").and_then(Value::as_str) {
        Some("bundle") => {}
        Some(other) => {
            return Err(format!(
                "create-answers selects `{other}`, but `up` needs \
                 `selected_action: \"bundle\"` — only a bundle run produces a \
                 directory to set up and start."
            ));
        }
        None => return Err("create-answers document declares no `selected_action`".to_string()),
    }

    let delegate = answers
        .get("delegate_answer_document")
        .and_then(|value| value.get("answers"))
        .and_then(Value::as_object)
        .ok_or(
            "create-answers document has no `answers.delegate_answer_document.answers` \
             object — `up` needs a non-interactive bundle document to predict the \
             output directory",
        )?;

    if let Some(dir) = delegate
        .get("output_dir")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|dir| !dir.is_empty())
    {
        return Ok(PathBuf::from(dir));
    }

    let bundle_id = delegate
        .get("bundle_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or(
            "create-answers document declares neither `output_dir` nor \
             `bundle_id`, so the bundle directory cannot be predicted — pass \
             --bundle-dir <PATH>",
        )?;

    Ok(default_bundle_output_dir(bundle_id))
}

/// Mirror of `greentic-bundle`'s `normalize_bundle_id`.
fn normalize_bundle_id(raw: &str) -> String {
    let normalized = raw
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>();
    normalized.trim_matches('-').to_string()
}

/// Mirror of `greentic-bundle`'s `default_bundle_output_dir`.
fn default_bundle_output_dir(bundle_id: &str) -> PathBuf {
    let normalized = normalize_bundle_id(bundle_id);
    if normalized.is_empty() {
        PathBuf::from("./bundle")
    } else {
        PathBuf::from(format!("./{normalized}-bundle"))
    }
}

/// Pin the predicted directory to the CWD once. `output_dir` is a relative
/// path in the document, and `run_start` and the setup passthrough must not be
/// able to resolve it against anything else.
///
/// Lexical only — no `canonicalize`, which fails on a path that does not exist
/// yet, and the whole point is that this directory has not been created.
fn absolutize(path: &Path) -> std::io::Result<PathBuf> {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };

    // Rebuild from `components()`, which drops the `.` a document's `./foo`
    // leaves behind. Worth doing because this path is printed in every error
    // message and passed to two later steps, and `/work/./foo` reads like a
    // bug in the tool. `Components` is what normalizes — there is deliberately
    // no `CurDir` arm here, because adding one would be dead code.
    Ok(joined.components().collect())
}

/// Build the `install` sub-matches from the real flag definitions rather than
/// fabricating them, so `up` cannot drift from `gtc install`.
///
/// `--skip-self-update` is not optional here: `run_install` otherwise replaces
/// the running gtc binary on disk mid-command, leaving the rest of `up` to
/// finish on the old in-memory image.
fn install_matches(locale: &str) -> Option<ArgMatches> {
    build_cli(locale)
        .try_get_matches_from(["gtc", "install", "--skip-self-update"])
        .ok()
        .and_then(|matches| matches.subcommand_matches("install").cloned())
}

fn run_install_step(default_channel: &'static str, debug: bool, locale: &str) -> i32 {
    match install_matches(locale) {
        Some(matches) => run_install(&matches, default_channel, debug, locale),
        None => {
            eprintln!("internal error: could not build the install step");
            1
        }
    }
}

/// Delegate through the same router `gtc wizard` / `gtc setup` use.
fn run_passthrough_step(name: &str, tail: &[String], debug: bool, locale: &str) -> i32 {
    let Some((binary, args)) = route_passthrough_subcommand(name, tail, locale) else {
        eprintln!("internal error: no route for `{name}`");
        return 1;
    };
    passthrough(binary, &args, debug, locale)
}

/// A composite that fails opaquely is worse than the commands it replaces, so
/// always name the step and how to re-run it by hand.
fn fail(status: i32, step: &str, rerun: &str) -> i32 {
    eprintln!("`gtc up` stopped at the {step} step (exit {status}). Re-run it with:\n  {rerun}");
    status
}

/// The setup step's argv.
///
/// `--non-interactive` is mandatory, not defensive. Unlike the wizard —
/// which infers `--yes --non-interactive` from `--answers` — `greentic-setup`
/// does not: without the flag it launches a web UI or prompts on stdin, and
/// the composite hangs with no indication why.
fn setup_tail(bundle_dir: &Path, setup_answers: &str) -> Vec<String> {
    vec![
        bundle_dir.display().to_string(),
        "--answers".to_string(),
        setup_answers.to_string(),
        "--non-interactive".to_string(),
    ]
}

/// Render the `--` tail for a printed command line.
fn start_tail_suffix(tail: &[String]) -> String {
    if tail.is_empty() {
        String::new()
    } else {
        format!(" {}", tail.join(" "))
    }
}

fn print_dry_run(plan: &UpPlan) {
    println!("bundle directory: {}", plan.bundle_dir.display());
    println!("steps:");
    if plan.install {
        println!("  install  gtc install --skip-self-update");
    }
    println!("  wizard   gtc wizard --answers {}", plan.create_answers);
    println!("  verify   {} exists", plan.bundle_dir.display());
    println!(
        "  setup    gtc setup {} --answers {} --non-interactive",
        plan.bundle_dir.display(),
        plan.setup_answers
    );
    if plan.start {
        let tail = start_tail_suffix(&plan.start_tail);
        println!("  start    gtc start {}{tail}", plan.bundle_dir.display());
    }
}

/// `up --help` after-text. Kept next to the implementation so the limitation
/// and the code that imposes it move together.
pub(super) fn after_help(locale: &str) -> String {
    t_or(
        locale,
        "gtc.cmd.up.after_help",
        "Runs install -> wizard -> setup -> start as one command, ending in a \
         foreground server (Ctrl+C to stop). Both answer documents are \
         required; `up` never guesses the second one. Setup flags (--tenant, \
         --team, --env, --advanced) are NOT forwarded — run the four commands \
         separately if you need them. Everything after `--` is forwarded to \
         the start step.",
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::SETUP_BIN;

    fn create_doc(answers: Value) -> Value {
        json!({
            "wizard_id": "greentic-dev.wizard.launcher.main",
            "schema_id": "greentic-dev.launcher.main",
            "schema_version": "1.0.0",
            "locale": "en",
            "answers": answers,
        })
    }

    fn bundle_doc(delegate: Value) -> Value {
        create_doc(json!({
            "selected_action": "bundle",
            "delegate_answer_document": { "answers": delegate },
        }))
    }

    #[test]
    fn output_dir_is_taken_verbatim_from_the_create_document() {
        let doc = bundle_doc(json!({
            "bundle_id": "helpdesk-itsm-demo",
            "output_dir": "helpdesk-itsm-demo-bundle",
        }));
        assert_eq!(
            predict_bundle_dir(doc.as_object().unwrap()).unwrap(),
            PathBuf::from("helpdesk-itsm-demo-bundle")
        );
    }

    #[test]
    fn output_dir_wins_over_the_derived_default() {
        // The two disagree on purpose: whatever the document says the wizard
        // will use, so a derived guess must never override it.
        let doc = bundle_doc(json!({
            "bundle_id": "helpdesk-itsm-demo",
            "output_dir": "/opt/bundles/helpdesk",
        }));
        assert_eq!(
            predict_bundle_dir(doc.as_object().unwrap()).unwrap(),
            PathBuf::from("/opt/bundles/helpdesk")
        );
    }

    #[test]
    fn an_absent_output_dir_falls_back_to_the_normalized_bundle_id() {
        let doc = bundle_doc(json!({ "bundle_id": "helpdesk-itsm-demo" }));
        assert_eq!(
            predict_bundle_dir(doc.as_object().unwrap()).unwrap(),
            PathBuf::from("./helpdesk-itsm-demo-bundle")
        );
    }

    #[test]
    fn a_blank_output_dir_falls_back_rather_than_predicting_the_cwd() {
        // greentic-bundle maps an empty output_dir to "."; predicting the CWD
        // would make the exists-guard fire on every run.
        let doc = bundle_doc(json!({
            "bundle_id": "quickstart",
            "output_dir": "   ",
        }));
        assert_eq!(
            predict_bundle_dir(doc.as_object().unwrap()).unwrap(),
            PathBuf::from("./quickstart-bundle")
        );
    }

    #[test]
    fn bundle_id_normalization_matches_greentic_bundle() {
        assert_eq!(
            default_bundle_output_dir("  Helpdesk ITSM Demo  "),
            PathBuf::from("./helpdesk-itsm-demo-bundle")
        );
        assert_eq!(
            default_bundle_output_dir("--weird--"),
            PathBuf::from("./weird-bundle")
        );
        assert_eq!(default_bundle_output_dir("!!!"), PathBuf::from("./bundle"));
    }

    #[test]
    fn a_pack_run_is_refused_because_it_creates_no_bundle_directory() {
        let doc = create_doc(json!({ "selected_action": "pack" }));
        let err = predict_bundle_dir(doc.as_object().unwrap()).unwrap_err();
        assert!(err.contains("selected_action"), "unexpected error: {err}");
    }

    #[test]
    fn a_document_with_no_delegate_is_refused_before_any_step_runs() {
        let doc = create_doc(json!({ "selected_action": "bundle" }));
        let err = predict_bundle_dir(doc.as_object().unwrap()).unwrap_err();
        assert!(
            err.contains("delegate_answer_document"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn neither_output_dir_nor_bundle_id_is_an_error_not_a_guess() {
        let doc = bundle_doc(json!({ "bundle_name": "helpdesk" }));
        let err = predict_bundle_dir(doc.as_object().unwrap()).unwrap_err();
        assert!(err.contains("--bundle-dir"), "unexpected error: {err}");
    }

    #[test]
    fn a_relative_prediction_is_pinned_to_the_cwd() {
        let pinned = absolutize(Path::new("./demo-bundle")).unwrap();
        assert!(pinned.is_absolute());
        assert!(pinned.ends_with("demo-bundle"));
    }

    #[test]
    fn an_absolute_prediction_is_left_alone() {
        let pinned = absolutize(Path::new("/opt/bundles/demo")).unwrap();
        assert_eq!(pinned, PathBuf::from("/opt/bundles/demo"));
    }

    #[test]
    fn a_leading_dot_component_is_dropped_from_the_pinned_path() {
        // `./demo-bundle` is the shape greentic-bundle's own default emits, so
        // without this every printed path and both delegated argvs carry a
        // `/./` that reads as a bug in the tool.
        let pinned = absolutize(Path::new("./demo-bundle")).unwrap();
        assert!(
            !pinned.to_string_lossy().contains("/./"),
            "{} still carries a dot component",
            pinned.display()
        );
    }

    #[test]
    fn the_setup_step_always_passes_non_interactive() {
        // greentic-setup does NOT infer this from --answers the way the wizard
        // does. Without it the composite opens a web UI or blocks on stdin,
        // which in CI looks like a hang rather than a failure.
        let tail = setup_tail(Path::new("/work/demo-bundle"), "/tmp/setup.json");
        assert_eq!(
            tail,
            vec![
                "/work/demo-bundle".to_string(),
                "--answers".to_string(),
                "/tmp/setup.json".to_string(),
                "--non-interactive".to_string(),
            ]
        );
    }

    #[test]
    fn the_setup_step_routes_to_greentic_setup() {
        // Guards the composition boundary: `up` must reach setup through the
        // same router `gtc setup` uses, not a hand-built argv.
        let tail = setup_tail(Path::new("/work/demo-bundle"), "/tmp/setup.json");
        let (binary, args) = route_passthrough_subcommand("setup", &tail, "en").unwrap();
        assert_eq!(binary, SETUP_BIN);
        assert_eq!(args, tail);
    }

    #[test]
    fn the_install_step_skips_the_self_update() {
        // run_install would otherwise replace the running gtc binary on disk
        // mid-command, leaving wizard/setup/start to finish on the old image.
        let matches = install_matches("en").expect("install matches");
        assert!(matches.get_flag("skip-self-update"));
        // and nothing else is narrowed: a phase selector here would silently
        // install less than `gtc install` does.
        for selector in [
            "install-binaries-only",
            "install-packs-only",
            "install-components-only",
            "install-tenant-only",
        ] {
            assert!(!matches.get_flag(selector), "{selector} should be unset");
        }
        assert!(!matches.get_flag("dry-run"));
    }

    #[test]
    fn the_start_tail_is_rendered_only_when_present() {
        assert_eq!(start_tail_suffix(&[]), "");
        assert_eq!(
            start_tail_suffix(&["--cloudflared".to_string(), "off".to_string()]),
            " --cloudflared off"
        );
    }
}
