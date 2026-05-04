# PR-DOCS-08 — Generated Schema Docs Baseline

## Goal

Introduce the generated schema docs area and initial machine-derived outputs that become canonical references for schema-driven behavior.

## Why This PR Exists

The repo needs a repeatable place where schema-derived truth can live independently from prose docs. This PR establishes that structure before automation and enforcement are added.

## Status Model

`Canonical in this repo`

## Implementation Owner(s)

- `gtc` for repo-owned schema outputs
- Companion tools may optionally provide expanded coverage

## In Scope

- `docs/04-schemas/README.md`
- initial generated outputs for repo-owned schemas
- initial curated common-component schema docs if available
- clear generated-doc conventions

## Out Of Scope

- Full sync automation
- Drift report generation
- CI enforcement

## Inputs To Verify First

- Current availability of `gtc wizard --schema`
- Current availability of any repo-owned setup schema output
- Whether companion binaries are available locally for optional expanded coverage
- Existing common component references in README/docs

## Files To Add

- `docs/04-schemas/README.md`
- `docs/04-schemas/wizard-schema.md`
- `docs/04-schemas/setup-schema.md` if currently available
- `docs/04-schemas/component-schemas/` outputs for any curated set actually verifiable in current environment

## Files To Update

- `docs/00-start-here.md`
- `README.md` only with a concise referral to generated schemas

## Files To Redirect Or Deprecate

- None required

## Content Requirements

Generated docs must capture:

- command used to generate them
- date/time or version marker if appropriate
- clear note that they are generated from current tooling
- structured presentation rather than raw unreadable dumps when formatting can be improved cheaply
- explicit indication when output is partial because optional companion binaries were unavailable

`docs/04-schemas/README.md` must explain:

- what belongs in this folder
- which outputs are repo-owned versus optional expanded coverage
- how human docs should link to generated docs instead of embedding large schema dumps

## Acceptance Criteria

- `docs/04-schemas/` exists as an explicit canonical schema area
- Generated docs are referenced from the repo’s canonical entrypoints
- Outputs are repeatable and clearly marked as generated

## Verification

- Run the generation commands available in the current environment
- Verify generated outputs are understandable and deterministic enough for later automation
- Verify human docs link to generated docs instead of duplicating them

## Risks / Ambiguities

- Companion binaries may not be present locally
- Formatting may need a lightweight helper even in the baseline pass
- Component coverage must stay conservative if the current environment cannot verify all candidates

## Follow-up PRs

- `PR-DOCS-09-schema-sync-tool.md`
- `PR-DOCS-10-drift-report-and-policy-enforcement.md`
- `PR-DOCS-11-example-validation.md`

