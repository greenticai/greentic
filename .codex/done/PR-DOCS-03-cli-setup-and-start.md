# PR-DOCS-03 — CLI Setup And Start

## Goal

Document `gtc setup` and `gtc start` as the canonical operational flows for this repo, with especially clear local-versus-remote behavior and ownership boundaries.

## Why This PR Exists

Setup and start are the most likely places for contributors and agents to infer behavior from demos, adjacent repos, or wishful abstractions. This PR should replace that guesswork with current implementation-grounded docs.

## Status Model

`Canonical in this repo`

## Implementation Owner(s)

- `gtc` for command surface, local orchestration, and local defaults
- External tools or deployer paths may own parts of execution after handoff

## In Scope

- `docs/02-cli/gtc-setup.md`
- `docs/02-cli/gtc-start.md`
- explicit documentation of setup/start relationship
- local runtime versus remote/deployer execution model
- current ownership boundaries and common confusion points

## Out Of Scope

- Wizard semantics beyond what setup/start depends on
- Generated schemas
- Sync tooling
- Full troubleshooting compendium

## Inputs To Verify First

- Current `gtc` CLI help for `setup` and `start`
- `src/bin/gtc/cli.rs`
- `src/bin/gtc.rs`
- `src/bin/gtc/deploy/start_stop.rs`
- `src/bin/gtc/deploy/bundle_resolution.rs`
- `src/bin/gtc/deploy/cloud_deploy.rs`
- `README.md`
- Existing `docs/gtc-wizard.md` only where it mentions setup/start relationships

## Files To Add

- `docs/02-cli/gtc-setup.md`
- `docs/02-cli/gtc-start.md`

## Files To Update

- `docs/00-start-here.md`
- `README.md` only with a concise referral if needed

## Files To Redirect Or Deprecate

- None required in this PR

## Content Requirements

`gtc-setup.md` must explain:

- how to use `gtc setup ./<name>.gtbundle`
- whether `gtc setup ./<name>-bundle` is applicable and under what currently implemented conditions
- `with UI` versus `without UI` if actually implemented
- what setup is responsible for
- what setup persists or outputs
- what belongs in authoring versus setup versus runtime
- schema-driven setup behavior only where supported now

`gtc-start.md` must explain:

- how `gtc start <bundle>` runs locally
- how `gtc start <bundle>` can use deployer paths for remote/cloud execution if currently implemented
- what `start` owns versus what deployers own
- relationship between setup and start
- local versus remote/deployer execution model
- common confusion points and practical troubleshooting notes

Both docs must:

- document only currently implemented behavior
- avoid aspirational language
- use TODO markers only when unavoidable and clearly justified
- answer:
  - what is this
  - when should I use it
  - what should I use instead if not this
  - what should I verify before proceeding

## Acceptance Criteria

- Local versus remote/deployer behavior is explicit
- A coding agent can follow the docs without consulting other repos first
- The docs describe actual implemented ownership boundaries

## Verification

- Inspect CLI help and relevant code paths
- Verify examples against current accepted arguments
- Verify terminology matches actual command surface

## Risks / Ambiguities

- Some remote behavior may depend on external tools or deployer assets
- README examples may currently oversimplify the ownership model
- Setup/start handoff behavior may need careful wording to avoid overclaiming

## Follow-up PRs

- `PR-DOCS-04-cli-wizard-and-answers.md`
- `PR-DOCS-12-troubleshooting-and-maintenance-hardening.md`

