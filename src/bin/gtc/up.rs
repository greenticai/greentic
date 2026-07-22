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
use tempfile::TempDir;

use crate::OP_BIN;
use crate::answer_resolver::{
    AnswerSourceKind, AnswerSourceLoader, DefaultAnswerSourceLoader, classify_answers_source,
    load_answer_bytes, parse_answers_bytes,
};
use crate::cli::build_cli;
use crate::commands::{answers_error_exit_code, check_release_context};
use crate::deploy::run_start;
use crate::i18n_support::t_or;
use crate::install::run_install;
use crate::process::{passthrough, run_binary_capture, run_binary_checked};
use crate::router::route_passthrough_subcommand;

/// Everything `run_up` needs from the command line, resolved once.
struct UpPlan {
    /// Where the WIZARD will write, from the create document. Always known:
    /// the wizard requires `bundle_id`, so a document it could accept is a
    /// document `up` can predict from. This is the path the overwrite guard
    /// exists to protect, and `--bundle-dir` does not move it.
    wizard_dir: PathBuf,
    /// Where setup and start operate — `wizard_dir` unless `--bundle-dir`
    /// corrects a prediction that drifted. Absolute: `output_dir` is
    /// CWD-relative and a later step must not re-resolve it elsewhere.
    bundle_dir: PathBuf,
    create_answers: String,
    setup_answers: String,
    start_tail: Vec<String>,
    install: bool,
    updates: bool,
    start: bool,
    dry_run: bool,
    force: bool,
    /// Held only for its `Drop`: both documents are snapshotted into it, and
    /// it must outlive the child processes reading them. Dropping it early
    /// turns into a confusing "file not found" from a companion binary.
    _snapshot: TempDir,
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

    // The overwrite guard runs before anything else mutates the machine:
    // `up` hides which of its steps is destructive, so it must refuse earlier
    // than the four commands it replaces would.
    //
    // Every directory a step may write is checked, not just the one setup and
    // start use. `--bundle-dir` does NOT redirect the wizard, so guarding only
    // it would leave the wizard free to overwrite the document's directory
    // without `--force` — the exact data loss this guard exists to prevent.
    for dir in guarded_dirs(&plan) {
        if dir.exists() && !plan.force {
            eprintln!(
                "{} already exists. `up` runs the wizard in create mode, which \
                 overwrites a bundle you may have edited and orphans a tunnel \
                 still running against it. Pass --force to overwrite it, change \
                 `output_dir` in the create-answers document to build elsewhere, \
                 or run `gtc setup` and `gtc start` directly against it.",
                dir.display()
            );
            return 2;
        }
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

    if plan.updates {
        let env_id = resolve_start_env_id(&plan.start_tail);
        let status = run_updates_step(&env_id, &plan._snapshot, debug, locale);
        if status != 0 {
            return status;
        }
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

    // Snapshot BOTH documents once, then hand those files to every step.
    // `gtc wizard --answers <url>` re-fetches on each invocation, and the
    // README's `releases/latest` URLs are mutable — so without this the
    // overwrite guard would be decided on one version of the create document
    // while the wizard acted on another, and setup could receive a version
    // incompatible with the bundle the wizard built.
    let snapshot = TempDir::new()
        .map_err(|err| UpError::usage(format!("creating the answers snapshot directory: {err}")))?;
    let loader = DefaultAnswerSourceLoader;
    let (create_answers, document) =
        snapshot_answers(snapshot.path(), "create", create_source, &loader)?;
    // The setup document's parse is discarded, but not wasted: it is what makes
    // a malformed setup document fail here rather than at the setup step, after
    // the wizard has already created a bundle.
    let (setup_answers, _) = snapshot_answers(snapshot.path(), "setup", setup_source, &loader)?;

    // Predicted unconditionally, even under `--bundle-dir`: this is where the
    // wizard writes, so the guard needs it whatever the later steps use.
    let wizard_dir = pin(&predict_bundle_dir(&document).map_err(UpError::usage)?)?;
    let bundle_dir = match sub_matches.get_one::<String>("bundle-dir") {
        Some(explicit) => pin(Path::new(explicit))?,
        None => wizard_dir.clone(),
    };

    Ok(UpPlan {
        wizard_dir,
        bundle_dir,
        create_answers,
        setup_answers,
        start_tail: sub_matches
            .get_many::<String>("args")
            .map(|values| values.cloned().collect())
            .unwrap_or_default(),
        updates: sub_matches.get_flag("updates"),
        install: !sub_matches.get_flag("no-install"),
        start: !sub_matches.get_flag("no-start"),
        dry_run: sub_matches.get_flag("dry-run"),
        force: sub_matches.get_flag("force"),
        _snapshot: snapshot,
    })
}

/// Every directory a step may write, deduplicated.
///
/// `bundle_dir` differs from `wizard_dir` only under `--bundle-dir`, and then
/// both are live: the wizard writes one, setup and start the other.
fn guarded_dirs(plan: &UpPlan) -> Vec<&Path> {
    let mut dirs = vec![plan.wizard_dir.as_path()];
    if plan.bundle_dir != plan.wizard_dir {
        dirs.push(plan.bundle_dir.as_path());
    }
    dirs
}

/// Read a document once and return the path every later step reads, plus the
/// parsed document for prediction.
///
/// A **remote** source is materialized into `dir` so all three readers see the
/// same bytes: `gtc wizard --answers <url>` re-fetches on every invocation, and
/// the README's `releases/latest` URLs are mutable, so otherwise the overwrite
/// guard could be decided on one version while the wizard acted on another.
/// The snapshot directory is `tempfile`'s 0700, which keeps the setup
/// document's provider credentials off a shared machine.
///
/// A **local** source is validated and passed through at its original path. It
/// must not be relocated: greentic-bundle resolves relative pack and provider
/// references against the answers file's own directory
/// (`local_reference_base_dir`), and only *remote* documents are required to
/// carry absolute references. Copying a local document into a tempdir would
/// silently break every relative reference in it.
fn snapshot_answers(
    dir: &Path,
    name: &str,
    source: &str,
    loader: &dyn AnswerSourceLoader,
) -> Result<(String, Map<String, Value>), UpError> {
    let to_err = |err: gtc::error::GtcError| UpError {
        message: format!("{err}"),
        code: answers_error_exit_code(&err),
    };
    let kind = classify_answers_source(source).map_err(to_err)?;
    let bytes = load_answer_bytes(source, loader).map_err(to_err)?;
    let document = parse_answers_bytes(source, &bytes).map_err(to_err)?;

    if matches!(
        kind,
        AnswerSourceKind::LocalPath | AnswerSourceKind::FileUrl
    ) {
        return Ok((source.to_string(), document));
    }

    let path = dir.join(format!("{name}-answers.json"));
    std::fs::write(&path, &bytes)
        .map_err(|err| UpError::usage(format!("writing {}: {err}", path.display())))?;
    Ok((path.display().to_string(), document))
}

fn pin(path: &Path) -> Result<PathBuf, UpError> {
    absolutize(path).map_err(|err| UpError::usage(format!("resolving {}: {err}", path.display())))
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
    if plan.bundle_dir != plan.wizard_dir {
        // Surface the split: the wizard writes one directory and setup/start
        // read another, and a dry run that showed only one would hide it.
        println!("wizard writes to:  {}", plan.wizard_dir.display());
    }
    println!("steps:");
    if plan.install {
        println!("  install  gtc install --skip-self-update");
    }
    println!("  wizard   gtc wizard --answers {}", plan.create_answers);
    println!("  verify   {} exists", plan.bundle_dir.display());
    println!(
        "  guard    {} must not already exist",
        plan.wizard_dir.display()
    );
    println!(
        "  setup    gtc setup {} --answers {} --non-interactive",
        plan.bundle_dir.display(),
        plan.setup_answers
    );
    if plan.updates {
        let env_id = resolve_start_env_id(&plan.start_tail);
        println!(
            "  updates  greentic-operator op env apply --dry-run (env={env_id}, plan_endpoint={DEFAULT_PLAN_ENDPOINT})"
        );
        println!("           greentic-operator op env apply --non-interactive");
    }
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
         required; `up` never guesses the second one, and both are fetched \
         once and reused by every step. Setup flags (--tenant, --team, --env, \
         --advanced) are NOT forwarded — run the four commands separately if \
         you need them. Everything after `--` is forwarded to the start step.",
    )
}

const DEFAULT_PLAN_ENDPOINT: &str = "https://updates.greentic.cloud/v1/environments/_/plan";

/// The verb whose presence proves the operator can honour an `updates` block.
///
/// Deliberately NOT a version comparison. `greentic-operator` is a thin wrapper
/// crate with its OWN version — 1.1.4 on crates.io — while the capability lives
/// in its `greentic-deployer` dependency, currently 1.1.24. Those numbers do not
/// track each other: `greentic-operator 1.1.4` depends on
/// `greentic-deployer >=1.1.16, <1.2.0-0`, so what a given binary can actually
/// do depends on when it was built, not on what `--version` prints. Gating on
/// `>= 1.1.24` compared against `--version` output refuses EVERY install,
/// including a freshly built one that is perfectly capable.
///
/// `op trust-root add-did` landed in the same deployer release as `trust_did`
/// handling in the env-manifest, so its presence is a direct answer to the
/// question actually being asked. An older binary reports
/// `unrecognized subcommand 'add-did'` and exits non-zero.
const UPDATES_CAPABILITY_PROBE: [&str; 4] = ["op", "trust-root", "add-did", "--help"];

/// Extract the environment id from the start tail args.
///
/// Precedence: `--env <id>` or `--env=<id>` in the tail, then
/// `$GREENTIC_ENV`, then `"local"`.
fn resolve_start_env_id(start_tail: &[String]) -> String {
    // --env=<id> form
    if let Some(id) = start_tail.iter().find_map(|arg| arg.strip_prefix("--env=")) {
        return id.to_string();
    }

    // --env <id> (spaced) form
    if let Some(pair) = start_tail.windows(2).find(|pair| pair[0] == "--env") {
        return pair[1].clone();
    }

    // Environment variable fallback
    if let Ok(val) = std::env::var("GREENTIC_ENV")
        && !val.is_empty()
    {
        return val;
    }

    "local".to_string()
}

/// Run the updates subscription step: version-gate the operator, write a
/// minimal env-manifest, and apply it via `op env apply`.
fn run_updates_step(env_id: &str, snapshot: &TempDir, debug: bool, locale: &str) -> i32 {
    // Capability-gate the companion before writing anything.
    if !operator_supports_updates(debug, locale) {
        eprintln!(
            "{}",
            t_or(
                locale,
                "gtc.up.err.updates_version_gate",
                "--updates needs a greentic-operator built against greentic-deployer 1.1.24 \
                 or newer (it must provide `op trust-root add-did`). Reinstall greentic-operator \
                 and retry.",
            )
        );
        return 1;
    }

    let manifest = updates_manifest(env_id);

    let path = snapshot.path().join("updates-manifest.json");
    if let Err(err) = std::fs::write(&path, manifest.to_string().as_bytes()) {
        eprintln!(
            "{}: {err}",
            t_or(
                locale,
                "gtc.up.err.updates_manifest_write",
                "failed to write updates manifest",
            )
        );
        return 1;
    }

    let path_str = path.display().to_string();
    let apply_args = |extra: &[&str]| -> Vec<String> {
        let mut args = vec![
            "op".to_string(),
            "env".to_string(),
            "apply".to_string(),
            "--answers".to_string(),
            path_str.clone(),
        ];
        args.extend(extra.iter().map(|s| (*s).to_string()));
        args
    };

    // Show the plan BEFORE converging. `--non-interactive` implies `--yes`, so
    // the real apply below cannot stop to ask — a bootstrap must not hang on a
    // prompt. That makes this preview the only thing standing between the
    // operator and an unannounced change to what their environment trusts, so
    // it is not optional and a failure here aborts before anything mutates.
    // `--dry-run` validates, diffs and prints the plan, then exits without
    // touching the store.
    if let Err(err) = run_binary_checked(
        OP_BIN,
        &apply_args(&["--dry-run"]),
        debug,
        locale,
        "updates subscription plan",
    ) {
        eprintln!(
            "{}: {err}",
            t_or(
                locale,
                "gtc.up.err.updates_plan_failed",
                "could not plan the updates subscription",
            )
        );
        return 1;
    }

    let args = apply_args(&["--non-interactive"]);
    if let Err(err) = run_binary_checked(OP_BIN, &args, debug, locale, "updates subscription") {
        eprintln!(
            "{}: {err}",
            t_or(
                locale,
                "gtc.up.err.updates_apply_failed",
                "updates subscription failed",
            )
        );
        return 1;
    }

    0
}

/// The env-manifest that subscribes `env_id` to the fleet update channel.
///
/// `plan_endpoint` is the only field written explicitly because it is a
/// required field (`pub plan_endpoint: String` in `ManifestUpdates`) — an
/// empty `"updates": {}` block fails with `missing field 'plan_endpoint'`.
/// All other fields (`enabled`, `on_notify`, `poll_interval_secs`, etc.)
/// are `Option` and default correctly when absent.
///
/// `trust_root` is deliberately absent. Trust-root writes are the takeover
/// primitive the surrounding design exists to contain, and the implicit
/// `trust_did` anchoring that `plan_endpoint == DEFAULT_PLAN_ENDPOINT`
/// triggers already establishes the only key this environment needs in
/// order to verify fleet plans.
fn updates_manifest(env_id: &str) -> Value {
    serde_json::json!({
        "schema": "greentic.env-manifest.v1",
        "environment": { "id": env_id },
        "updates": {},
    })
}

/// Probe whether the installed operator can honour an `updates` block.
///
/// See [`UPDATES_CAPABILITY_PROBE`] for why this is a capability probe and not
/// a version comparison. Fails closed: any error running the probe is treated
/// as "not capable".
fn operator_supports_updates(debug: bool, locale: &str) -> bool {
    let args: Vec<String> = UPDATES_CAPABILITY_PROBE
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    run_binary_capture(OP_BIN, &args, debug, locale).is_ok()
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
        // Mirrors `normalized_request_from_document` in greentic-bundle, which
        // trims `output_dir`, discards it when empty, and falls back to the
        // derived default. NOT `normalize_output_dir`'s empty-maps-to-"." rule
        // — that guards other seed paths and never sees a value from here.
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

    fn plan_with(wizard: &str, bundle: &str) -> UpPlan {
        UpPlan {
            wizard_dir: PathBuf::from(wizard),
            bundle_dir: PathBuf::from(bundle),
            create_answers: String::new(),
            setup_answers: String::new(),
            start_tail: Vec::new(),
            updates: false,
            install: false,
            start: false,
            dry_run: true,
            force: false,
            _snapshot: TempDir::new().unwrap(),
        }
    }

    #[test]
    fn the_guard_covers_the_wizard_directory_even_under_bundle_dir() {
        // --bundle-dir does NOT redirect the wizard, so guarding only it would
        // leave the document's directory free to be overwritten without
        // --force. Both must be checked.
        let plan = plan_with("/work/from-document", "/work/override");
        assert_eq!(
            guarded_dirs(&plan),
            vec![
                Path::new("/work/from-document"),
                Path::new("/work/override")
            ]
        );
    }

    #[test]
    fn the_guard_does_not_repeat_a_single_directory() {
        let plan = plan_with("/work/demo-bundle", "/work/demo-bundle");
        assert_eq!(guarded_dirs(&plan), vec![Path::new("/work/demo-bundle")]);
    }

    #[derive(Default)]
    struct CountingLoader {
        http_calls: std::cell::Cell<usize>,
    }

    impl crate::answer_resolver::AnswerSourceLoader for CountingLoader {
        fn load_http(&self, _source: &str) -> gtc::error::GtcResult<Vec<u8>> {
            self.http_calls.set(self.http_calls.get() + 1);
            Ok(br#"{"answers":{"selected_action":"bundle"},"note":"fetched once"}"#.to_vec())
        }

        fn load_distributor(&self, _source: &str) -> gtc::error::GtcResult<Vec<u8>> {
            unreachable!("not exercised")
        }
    }

    #[test]
    fn a_remote_document_is_fetched_once_and_materialized() {
        // The point of the snapshot: `gtc wizard --answers <url>` re-fetches on
        // every invocation and `releases/latest` is mutable, so the guard could
        // otherwise be decided on different bytes than the wizard acts on.
        let dir = TempDir::new().unwrap();
        let loader = CountingLoader::default();
        let (path, document) = snapshot_answers(
            dir.path(),
            "create",
            "https://example.com/create-answers.json",
            &loader,
        )
        .map_err(|err| err.message)
        .unwrap();

        assert_eq!(loader.http_calls.get(), 1, "fetched exactly once");
        assert!(
            Path::new(&path).starts_with(dir.path()),
            "{path} is not inside the snapshot directory"
        );
        assert_eq!(document["note"], "fetched once");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            br#"{"answers":{"selected_action":"bundle"},"note":"fetched once"}"#,
            "bytes must be written verbatim, not re-serialized"
        );
    }

    #[test]
    fn a_local_document_is_never_relocated() {
        // greentic-bundle resolves relative pack/provider references against
        // the answers file's OWN directory (`local_reference_base_dir`).
        // Copying a local document into a tempdir would move that base and
        // silently break every relative reference in it.
        let dir = TempDir::new().unwrap();
        let source = dir.path().join("origin.json");
        let raw = br#"{"answers":{"selected_action":"bundle"},"note":"kept verbatim"}"#;
        std::fs::write(&source, raw).unwrap();

        let (path, document) = snapshot_answers(
            dir.path(),
            "create",
            &source.display().to_string(),
            &CountingLoader::default(),
        )
        .map_err(|err| err.message)
        .unwrap();
        assert_eq!(
            path,
            source.display().to_string(),
            "a local document must be passed through at its original path"
        );
        assert_eq!(document["note"], "kept verbatim");
    }

    /// Build a real `UpPlan` the way `run_up` does: through clap, from files
    /// on disk. Without this, the guard's most important property — that
    /// `--bundle-dir` cannot detach it from the wizard — is only asserted
    /// against a hand-built struct, and a `build_plan` that wired the two
    /// together would sail through.
    fn build_plan_from(dir: &Path, extra: &[&str]) -> UpPlan {
        let create = dir.join("create.json");
        let setup = dir.join("setup.json");
        std::fs::write(
            &create,
            serde_json::to_vec(&bundle_doc(json!({
                "bundle_id": "helpdesk-itsm-demo",
                "output_dir": "from-document",
            })))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(&setup, br#"{"tenant":"demo"}"#).unwrap();

        let mut argv = vec![
            "gtc".to_string(),
            "up".to_string(),
            "--answers".to_string(),
            create.display().to_string(),
            "--setup-answers".to_string(),
            setup.display().to_string(),
        ];
        argv.extend(extra.iter().map(|value| value.to_string()));

        let matches = build_cli("en")
            .try_get_matches_from(argv)
            .expect("up parses");
        let sub = matches
            .subcommand_matches("up")
            .expect("up matches")
            .clone();
        build_plan(&sub).map_err(|err| err.message).expect("plan")
    }

    #[test]
    fn bundle_dir_never_detaches_the_guard_from_the_wizard() {
        // The whole point of the guard: `--bundle-dir` moves where setup and
        // start look, NOT where the wizard writes. If build_plan ever sets
        // wizard_dir from the override, the document's directory becomes
        // overwritable without --force.
        let dir = TempDir::new().unwrap();
        let plan = build_plan_from(dir.path(), &["--bundle-dir", "/work/override"]);
        assert!(
            plan.wizard_dir.ends_with("from-document"),
            "wizard_dir must come from the document, got {}",
            plan.wizard_dir.display()
        );
        assert_eq!(plan.bundle_dir, PathBuf::from("/work/override"));
        assert_eq!(guarded_dirs(&plan).len(), 2);
    }

    #[test]
    fn without_an_override_both_directories_are_the_documents() {
        let dir = TempDir::new().unwrap();
        let plan = build_plan_from(dir.path(), &[]);
        assert_eq!(plan.bundle_dir, plan.wizard_dir);
        assert!(plan.wizard_dir.ends_with("from-document"));
        assert_eq!(guarded_dirs(&plan).len(), 1);
    }

    #[test]
    fn build_plan_hands_every_step_a_readable_document() {
        let dir = TempDir::new().unwrap();
        let plan = build_plan_from(dir.path(), &[]);
        for answers in [&plan.create_answers, &plan.setup_answers] {
            assert!(Path::new(answers).is_file(), "{answers} is missing");
        }
    }

    #[test]
    fn the_start_tail_is_rendered_only_when_present() {
        assert_eq!(start_tail_suffix(&[]), "");
        assert_eq!(
            start_tail_suffix(&["--cloudflared".to_string(), "off".to_string()]),
            " --cloudflared off"
        );
    }

    // -- updates flag tests --

    #[test]
    fn updates_flag_defaults_to_off() {
        let dir = TempDir::new().unwrap();
        let plan = build_plan_from(dir.path(), &[]);
        assert!(!plan.updates);
    }

    #[test]
    fn updates_flag_is_parsed_when_present() {
        let dir = TempDir::new().unwrap();
        let plan = build_plan_from(dir.path(), &["--updates"]);
        assert!(plan.updates);
    }

    // -- resolve_start_env_id tests --

    #[test]
    fn resolve_start_env_id_defaults_to_local() {
        // Remove the env var so it doesn't interfere.
        // SAFETY: this test is single-threaded and no other thread reads
        // GREENTIC_ENV concurrently.
        unsafe { std::env::remove_var("GREENTIC_ENV") };
        assert_eq!(resolve_start_env_id(&[]), "local");
    }

    #[test]
    fn resolve_start_env_id_extracts_spaced_flag() {
        let tail = vec!["--env".to_string(), "staging".to_string()];
        assert_eq!(resolve_start_env_id(&tail), "staging");
    }

    #[test]
    fn resolve_start_env_id_extracts_equals_flag() {
        let tail = vec!["--env=production".to_string()];
        assert_eq!(resolve_start_env_id(&tail), "production");
    }

    /// The gate must ask what the binary can DO. `greentic-operator` is a
    /// wrapper crate versioned 1.1.4 while the capability lives in its
    /// `greentic-deployer` dependency at 1.1.24, so any assertion about the
    /// probe being a version comparison would be asserting a bug.
    #[test]
    fn capability_probe_asks_for_the_verb_that_proves_updates_support() {
        assert_eq!(
            UPDATES_CAPABILITY_PROBE,
            ["op", "trust-root", "add-did", "--help"],
        );
    }

    /// The `updates` block must stay EMPTY: every field defaults to the fleet
    /// answer, and `trust_did` anchors implicitly only when nothing pins the
    /// endpoint away from the default. Writing fields out pins today's values.
    #[test]
    fn updates_manifest_declares_an_empty_block_and_never_a_trust_root() {
        let manifest = updates_manifest("local");

        assert_eq!(manifest["schema"], "greentic.env-manifest.v1");
        assert_eq!(manifest["environment"]["id"], "local");
        assert_eq!(
            manifest["updates"],
            json!({}),
            "an explicit field pins today's default into the environment",
        );
        assert!(
            manifest.get("trust_root").is_none(),
            "a bootstrap must not write the trust root: implicit trust_did \
             anchoring already establishes the fleet key",
        );
    }

    #[test]
    fn updates_manifest_names_the_resolved_environment() {
        assert_eq!(
            updates_manifest("production")["environment"]["id"],
            "production"
        );
    }

    #[test]
    fn default_plan_endpoint_matches_the_fleet_url() {
        assert_eq!(
            DEFAULT_PLAN_ENDPOINT,
            "https://updates.greentic.cloud/v1/environments/_/plan"
        );
    }
}
