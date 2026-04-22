# PR-DOCS-04 — CLI Wizard And Answers

## Goal

Create the canonical wizard and `answers.json` docs for this repo, and migrate the existing wizard doc into the new `docs/02-cli/` hierarchy with a stub at the old path.

## Why This PR Exists

`gtc wizard` is a primary entrypoint in current onboarding material. It is also where agents are most likely to invent answer keys, over-trust demos, or assume support that is not really implemented. This PR should make wizard usage deterministic and current.

## Status Model

`Canonical in this repo`

## Implementation Owner(s)

- `gtc` for the command surface and routing behavior documented here
- Related tools may own deeper generation behavior after handoff

## In Scope

- Canonical `docs/02-cli/gtc-wizard.md`
- Linear authoring “happy path” doc
- `answers.json` patterns doc with minimal valid examples
- Redirect stub at old `docs/gtc-wizard.md`
- Consolidation of overlapping wizard material rather than duplication

## Out Of Scope

- Flow schema mapping in depth
- Generated schema tooling
- Broad extension architecture docs

## Inputs To Verify First

- Current `gtc` CLI help for `wizard`
- `src/bin/gtc/cli.rs`
- `src/bin/gtc.rs`
- Existing `docs/gtc-wizard.md`
- README wizard sections
- Any current schema or help output for `gtc wizard --schema`
- Existing answer examples referenced from this repo

## Files To Add

- `docs/02-cli/gtc-wizard.md`
- `docs/03-authoring/happy-path-build-an-app.md`
- `docs/03-authoring/answers-json-patterns.md`

## Files To Update

- `docs/00-start-here.md`
- `README.md` only if a referral link is needed

## Files To Redirect Or Deprecate

- Replace `docs/gtc-wizard.md` with a short redirect stub to `docs/02-cli/gtc-wizard.md`

## Content Requirements

`gtc-wizard.md` must explain:

- `gtc wizard --schema`
- `gtc wizard --answers answers.json`
- how `answers.json` should follow the wizard schema
- current implemented create/update/setup/remove terminology if present
- current supported behavior only
- clear current limitations where support is incomplete

`happy-path-build-an-app.md` must provide a linear path:

- create or prepare component
- inspect schema where relevant
- create or update flow
- add or update steps
- create or update pack
- create or update bundle
- run setup
- run start

`answers-json-patterns.md` must include:

- minimal valid examples
- practical examples
- examples for:
  - component generation from `./src` if currently supported
  - flow create/update
  - add-step/update-step
  - pack create/update
  - bundle create/update
- common mistakes section

## Acceptance Criteria

- Agents can stop inventing `answers.json` structure
- Wizard docs are migrated into the new hierarchy without leaving duplicate full copies
- Docs are end-to-end, practical, and clearly limited to actual current behavior

## Verification

- Inspect current CLI help and schema output where available
- Validate example structure where feasible
- Verify old wizard path becomes a stub, not a second canonical copy

## Risks / Ambiguities

- Some create/update flows may be implemented downstream rather than fully in this repo
- Current README examples may rely on demo-hosted answers that require careful framing
- Exact supported modes may differ from older planning assumptions

## Follow-up PRs

- `PR-DOCS-05-flow-schema-and-step-mapping.md`
- `PR-DOCS-11-example-validation.md`

