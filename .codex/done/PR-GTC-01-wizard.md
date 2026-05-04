PR-GTC-01 (Rewritten) — gtc thin CLI router with bundled i18n
Outcomes

Single entrypoint binary: gtc

Pure passthrough routing:

gtc dev ... → greentic-dev ...

gtc dev wizard ... → greentic-dev wizard ...

gtc wizard ... → greentic-dev wizard ... (always)

gtc op ... → greentic-operator ...

No wizard logic in gtc (no target routing, no “demo wizard” fallback, no token translation).

Fully i18n-compliant CLI:

All clap-visible strings come from greentic-i18n tags.

English strings live in assets/i18n/en.json.

Translations are bundled into the binary (no runtime file dependency).

Non-goals

No “smart wizard” target routing in gtc.

No operator wizard fallback logic.

No bundle/pack format logic.

No version enforcement.

CLI Design
Commands

gtc --help

gtc version

gtc doctor

gtc dev <args…> passthrough

gtc op <args…> passthrough

gtc wizard <args…> passthrough to greentic-dev wizard <args…>

Optional: global --locale <bcp47> only affects gtc’s own UI/help strings (routing is unchanged)

Routing rules
User command	Routed command
gtc dev …	greentic-dev …
gtc dev wizard …	greentic-dev wizard …
gtc wizard …	greentic-dev wizard …
gtc op …	greentic-operator …
Doctor

Minimal checks only:

verify greentic-dev exists in PATH

verify greentic-operator exists in PATH

optionally run <bin> --version and print

Repo layout (suggested)
crates/gtc/
  Cargo.toml
  build.rs
  src/
    main.rs
    cli.rs
    i18n.rs
    router.rs
  assets/
    i18n/
      en.json
      locales.json
  tests/
    routing_fake_bins.rs
    unit_router.rs
i18n requirements (how to make clap strings tag-driven)
Key constraint

clap derive macros want 'static literals for many fields; runtime i18n is easiest with builder-style clap (Command), not derive.

Approach

Pre-scan args to find --locale (or GTC_LOCALE env var).

Build a localized clap::Command using t(locale, "gtc.*").

Parse normally.

Execute routing.

This gives you:

Localized --help output

Localized error messages you control (your own), plus clap’s (mostly English) parsing errors — you can mitigate by keeping parsing shallow at gtc level (gtc is a router).

i18n tag scheme (example)

Use stable keys, no English in code.

Examples:

gtc.app.name

gtc.app.about

gtc.cmd.dev.about

gtc.cmd.op.about

gtc.cmd.wizard.about

gtc.cmd.doctor.about

gtc.arg.locale.help

gtc.err.bin_missing_dev

gtc.err.bin_missing_op

gtc.err.exec_failed

gtc.doctor.ok

gtc.doctor.missing

Minimal en.json (example content)

assets/i18n/en.json:

{
  "gtc.app.name": "gtc",
  "gtc.app.about": "Greentic CLI router (thin launcher).",
  "gtc.cmd.version.about": "Print version information.",
  "gtc.cmd.doctor.about": "Check that required Greentic binaries are available in PATH.",
  "gtc.cmd.dev.about": "Pass through to greentic-dev.",
  "gtc.cmd.op.about": "Pass through to greentic-operator.",
  "gtc.cmd.wizard.about": "Pass through to greentic-dev wizard.",
  "gtc.arg.locale.help": "UI locale for gtc help/output (BCP-47 tag).",
  "gtc.arg.debug_router.help": "Print resolved binary + args to stderr before exec.",
  "gtc.err.bin_missing_dev": "greentic-dev not found in PATH.",
  "gtc.err.bin_missing_op": "greentic-operator not found in PATH.",
  "gtc.err.exec_failed": "Failed to execute command.",
  "gtc.doctor.ok": "OK",
  "gtc.doctor.missing": "MISSING"
}

assets/i18n/locales.json:

{
  "default": "en",
  "supported": ["en"]
}
Implementation blueprint (Rust)
src/i18n.rs

Embed JSON via include_str!

Parse once

t(locale, key) -> Cow<'static, str> (fallback to en, then key)

Key point: clap builder accepts impl Into<Str>, so you can pass owned String / Cow.

src/cli.rs

Build clap::Command using i18n strings:

Root:

--locale

--debug-router

subcommands: version, doctor, dev, op, wizard

Parsing strategy:

Step 1: detect_locale_from_args(&[String]) -> String

Step 2: build_cli(locale)

Step 3: get_matches()

src/router.rs

resolve_in_path("greentic-dev") and resolve_in_path("greentic-operator")

exec(bin, args, debug):

Command::new(bin).args(args).stdin(Stdio::inherit())...

propagate exit code (use std::process::exit(code))

Routing:

dev: exec greentic-dev with remaining args

op: exec greentic-operator with remaining args

wizard: exec greentic-dev with args ["wizard", ...]

src/main.rs

parse

handle version/doctor locally

otherwise route

Tests
Unit tests

route_wizard_always_dev

route_dev_passthrough

route_op_passthrough

missing_binary_errors_are_clean_and_localized_keyed (assert on your error text)

Fake-binary integration tests (CI-safe)

Create temp dir

Write small executable scripts named:

greentic-dev

greentic-operator

Put temp dir at front of PATH for the test process

Scripts should print argv to a file (or stdout)

Assert:

gtc wizard foo bar called greentic-dev with wizard foo bar

gtc dev abc --x called greentic-dev with abc --x

gtc op zzz called greentic-operator with zzz

exit code propagation works

(Keep these scripts cross-platform in tests by generating:

.cmd on Windows

shell script on unix
…and selecting by cfg!(windows).)

Docs updates

README:

gtc is the entrypoint

gtc wizard delegates to greentic-dev wizard

gtc dev/gtc op are raw passthrough

docs/gtc.md:

routing table

i18n note: “help text is localized; clap parsing errors may remain English”

troubleshooting: missing binaries in PATH

Definition of Done

gtc builds and installs

gtc dev/op/wizard passthrough works (no behavioral logic)

gtc help text comes from i18n tags (no English literals in clap-visible strings)

en.json + locales bundled into binary

unit + fake-binary integration tests pass in CI

docs updated

Codex prompt (updated)

Implement gtc as a pure CLI router. gtc dev ... must exec greentic-dev ... unchanged. gtc op ... must exec greentic-operator ... unchanged. gtc wizard ... must exec greentic-dev wizard ... unchanged (no target routing, no operator fallback logic). Use clap builder API (not derive) so all help/about/arg text is resolved via greentic-i18n tags, with English in assets/i18n/en.json and all translations bundled into the binary (via include_str / build.rs as needed). Add unit tests for routing and fake-binary integration tests using temp PATH scripts; CI must not require real greentic binaries. Keep doctor minimal: check both binaries exist in PATH and optionally print --version.

If you want, I can also give you a concrete starter Cargo.toml, build.rs, and the full src/ code skeleton in one paste so you can drop it straight into crates/gtc/.