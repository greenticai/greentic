# PR-DOCS-12 — Troubleshooting And Maintenance Hardening

## Goal

Add the final hardening layer: a practical troubleshooting guide and a durable documentation maintenance policy that makes the new system sustainable.

## Why This PR Exists

Once the canonical docs, generated schemas, sync tooling, and validation exist, contributors still need two things:

- help when those flows fail
- a clear maintenance policy so drift does not come back

## Status Model

- `Canonical in this repo` for maintenance policy and local troubleshooting
- `Operational guidance in this repo` for cross-repo failure cases

## Implementation Owner(s)

- This repo for documentation maintenance expectations
- Mixed ownership for some runtime or cross-repo failure modes

## In Scope

- Common authoring/runtime issue guide
- Final maintenance policy language
- Guidance for agents and humans on keeping docs/examples/schemas in sync

## Out Of Scope

- New sync tooling features unless required for documentation accuracy
- Broad architecture rewrites

## Inputs To Verify First

- Actual failure modes encountered in current Greentic workflows
- Current local check and schema-sync behavior
- Canonical docs and examples created in earlier PRs

## Files To Add

- `docs/06-troubleshooting/common-authoring-and-runtime-issues.md`

## Files To Update

- `docs/00-start-here.md`
- `docs/99-agent-rules/coding-agents.md`
- `README.md` only if a concise maintenance referral is needed
- `CONTRIBUTING.md` if it exists by then, otherwise keep the policy in canonical docs

## Files To Redirect Or Deprecate

- None required

## Content Requirements

`common-authoring-and-runtime-issues.md` must include issues like:

- `gtc wizard --answers` not generating expected artifacts
- component schema and flow mappings disagree
- bundle runs locally but not via deployer
- i18n keys missing
- config/secrets/OAuth not resolving in MCP/WASM flows
- extension pack composition confusion
- generated schema docs out of date

Maintenance policy must explain:

- canonical docs must be kept in sync
- when to update generated docs
- when to update prose docs
- when to update examples
- when to update terminology/deprecations docs
- how agents and humans should verify changes

## Acceptance Criteria

- Troubleshooting advice is practical and based on real failure modes
- New contributors understand documentation maintenance expectations
- The documentation process is durable beyond the initial rollout

## Verification

- Verify troubleshooting guidance against real observed or code-grounded failure modes
- Verify maintenance policy matches actual local check and sync workflows
- Confirm no policy text silently conflicts with `.codex/global_rules.md`

## Risks / Ambiguities

- It is easy to write generic troubleshooting advice instead of repo-grounded guidance
- Some failures may belong more to companion repos than this one and need careful ownership wording
- Policy duplication risk exists if `README.md`, start-here docs, and `.codex/global_rules.md` are not aligned

## Follow-up PRs

- Ongoing maintenance, no additional planned PR in this program

