# PR-DOCS-09 — Schema Sync Tool

## Goal

Add an officially supported schema-doc refresh command and wire it into local checks in a way that is useful locally and scalable toward stricter automation later.

## Why This PR Exists

Generated docs only help if they are easy and safe to refresh. This PR introduces the actual sync mechanism behind that workflow.

## Status Model

`Canonical in this repo`

## Implementation Owner(s)

- `gtc` or the most idiomatic repo-local command path chosen during implementation

## In Scope

- Add `gtc docs sync-schemas` or equivalent
- Refresh repo-owned schema docs under `docs/04-schemas/`
- Optional expanded coverage from companion binaries when available
- Integrate best-effort invocation into `ci/local_check.sh`
- Document the command

## Out Of Scope

- Full drift report detail beyond basic useful output
- Hard CI enforcement
- Large-scale example validation

## Inputs To Verify First

- Current `gtc` CLI structure and naming conventions
- Existing local check behavior in `ci/local_check.sh`
- Current generated-schema baseline from PR-DOCS-08
- Availability patterns for optional companion binaries

## Files To Add

- New CLI command implementation files as needed
- Supporting library files as needed

## Files To Update

- `src/bin/gtc/cli.rs`
- `src/bin/gtc.rs`
- `ci/local_check.sh`
- `docs/04-schemas/README.md`
- relevant CLI docs and start-here referrals

## Files To Redirect Or Deprecate

- None required

## Content Requirements

The tool must:

- run repo-owned schema generation from this repo alone
- refresh `gtc wizard --schema`
- refresh repo-owned setup schema if implemented here
- optionally run companion-binary schema generation when available
- regenerate docs under `docs/04-schemas/`
- be callable from `ci/local_check.sh`

Preferred behavior:

- best-effort mode for local development
- clear warnings when optional companion binaries are missing
- strict mode or a future-friendly path for CI use
- concise drift summary in command output

## Acceptance Criteria

- Running the sync command updates schema docs deterministically for repo-owned outputs
- Missing companion binaries do not make local usage brittle
- The command is documented and safe for contributors and agents to run locally

## Verification

- Run the command in a local environment with only repo-owned tools
- Verify graceful behavior with and without companion binaries
- Verify local check integration does not become noisy or flaky

## Risks / Ambiguities

- CLI naming may need to fit current command architecture cleanly
- Strict-versus-best-effort behavior needs careful defaults
- Formatting helpers may grow beyond the smallest useful scope if not constrained

## Follow-up PRs

- `PR-DOCS-10-drift-report-and-policy-enforcement.md`
- `PR-DOCS-11-example-validation.md`

