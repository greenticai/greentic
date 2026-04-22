Status: Operational guidance in this repo
Scope: How this repo uses i18n, QA/schema patterns, and distributor-client
Implementation owner: Mixed ownership across `gtc`, `greentic-i18n`, `greentic-distributor-client`, and adjacent Greentic tooling

# i18n, QA, and Distributor Client

Use this document to understand how these platform capabilities show up in this
repo.

It is operational guidance for contributors working in `gtc`. It is not a
claim that every underlying implementation lives here.

## What This Repo Owns

This repo directly owns:

- CLI locale selection and locale forwarding behavior
- repo-owned config/env defaults such as `GTC_LOCALE`
- bundle-reference mapping for `repo://...` and `store://...`
- install and start flows that call into distributor-client-backed behavior

This repo does not own the full implementation of:

- the broader i18n crate behavior beyond how `gtc` uses it
- QA component internals or cross-repo QA tooling
- distributor service internals

## i18n In This Repo

### What It Is

In this repo, i18n means the CLI and related subprocesses can choose a locale,
normalize it, and pass it through to repo-owned or companion-tool flows.

### What Current Code Proves

Current repo-local code shows that:

- `gtc` can take `--locale`
- `GTC_LOCALE` provides a default when a CLI locale is not passed
- locale values are normalized before use
- subprocesses are commonly invoked with `GREENTIC_LOCALE=<locale>`

The relevant repo-owned surfaces are:

- [`docs/config.md`](../config.md) for `GTC_LOCALE`
- `src/perf_targets.rs` for locale detection helpers
- `src/bin/gtc/i18n.rs` for embedded locale usage
- `src/bin/gtc/process.rs` for subprocess locale forwarding

### What To Verify Before You Edit i18n Behavior

Verify:

1. whether the change is about repo-owned locale routing or a companion tool's own translation behavior
2. whether `--locale` must be preserved in a passthrough command
3. whether `GTC_LOCALE` or `GREENTIC_LOCALE` semantics would change
4. whether docs and examples need updating in the same PR

## QA, Schemas, and Answers In This Repo

### What This Repo Can Say Safely

This repo already uses a schema-first authoring model in its docs:

- inspect the current schema first
- make `answers.json` match the schema
- treat emitted answers and schema-derived docs as stronger than remembered examples

That pattern is visible in:

- [`docs/02-cli/gtc-wizard.md`](../02-cli/gtc-wizard.md)
- [`docs/03-authoring/answers-json-patterns.md`](./answers-json-patterns.md)
- [`docs/02-cli/greentic-flow.md`](../02-cli/greentic-flow.md)
- [`docs/03-authoring/flow-step-schema-mapping.md`](./flow-step-schema-mapping.md)

### What This Repo Does Not Prove Yet

This repo does not currently prove the full standalone internals of QA specs,
QA answer schemas, or every QA workflow across Greentic repos. Treat broader QA
behavior as adjacent-platform capability, not repo-owned implementation truth.

### Practical Rule

When QA-style authoring or validation depends on schemas:

- trust current schema output before prose
- trust repo-local generated schema docs when they exist
- avoid inventing answer-file keys from old demos
- document current limitations instead of implying support

### Setup and Wizard Relationship

Current repo-local docs show that wizard/setup flows rely on schema and answer
patterns, even when the deeper implementation is delegated to companion tools.

In practice:

- authoring begins with the current schema
- answer files should be derived from that schema
- setup/runtime docs should only promise what current CLI behavior and tests support

## Distributor Client In This Repo

### What It Is For

This repo depends on distributor-client behavior for install and remote artifact
resolution flows.

You can see that dependency in:

- `Cargo.toml`
- `src/dist.rs`
- `src/bin/gtc/install.rs`
- `src/bin/gtc/deploy/bundle_resolution.rs`

### What Current Code Proves

Current code shows that `gtc` can:

- resolve bundle references such as local paths, `file://`, `http(s)://`, `oci://`, `repo://`, and `store://`
- map `repo://...` and `store://...` references through env-configured registry bases
- use distributor-client-backed resolution logic during install and distribution-oriented flows

Repo-owned config for these mappings is documented in
[`docs/config.md`](../config.md):

- `GREENTIC_REPO_REGISTRY_BASE`
- `GREENTIC_STORE_REGISTRY_BASE`
- tenant install key patterns such as `GREENTIC_<TENANT>_KEY`

### When Repo-Local Docs Are Enough

Repo-local docs here are enough when you need to know:

- which bundle-reference schemes `gtc` accepts
- which env vars `gtc` reads for registry mapping
- whether a command in this repo expects remote bundle resolution

### When To Consult Another Repo

Consult adjacent distributor or registry repos when you need:

- distributor protocol internals
- registry authentication behavior beyond the env/config surface owned here
- artifact publishing semantics owned outside `gtc`

## Decision Support

- Need to change CLI locale routing or locale defaults -> stay in this repo first.
- Need to change answer-file examples or schema-first guidance -> stay in this repo first, then verify downstream schemas.
- Need to change distributor protocol behavior or remote registry internals -> this repo is not the final implementation owner.

## Common Mistakes

- Treating demo answer files as more current than schema output
- Documenting QA behavior from memory instead of from current schema/tests
- Assuming `repo://` or `store://` references work without the registry-base env vars configured
- Mixing repo-owned locale handling with broader translation/catalog ownership in other repos
