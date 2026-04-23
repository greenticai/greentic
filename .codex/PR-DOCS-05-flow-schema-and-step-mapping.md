# PR-DOCS-05 — Flow Schema And Step Mapping

## Goal

Explain how component schemas are discovered and how they map into practical flow-step authoring, using current terminology and behavior rather than historical phrasing.

## Why This PR Exists

This is a high-drift area. Agents often guess how component schemas, payload mappings, and flow steps relate, especially when examples are incomplete or cross-repo. This PR should make schema-first flow authoring explicit.

## Status Model

`Operational guidance in this repo`

## Implementation Owner(s)

- Likely mixed ownership
- This repo owns the operational guidance for how contributors here should use component schemas and flow mappings
- `greentic-flow` or related tooling may own deeper implementation semantics

## In Scope

- `docs/02-cli/greentic-flow.md`
- `docs/03-authoring/flow-step-schema-mapping.md`
- practical guidance for component-schema-first flow work

## Out Of Scope

- Full flow engine internals
- Generated schema automation
- Generic architecture restatement already covered in PR-DOCS-02

## Inputs To Verify First

- Current references to `greentic-flow`
- Current usage or docs for `component-schema`
- Existing flow-related material in repo catalogs and README
- Any current example flows or tests referenced from this repo
- Any current schema docs once PR-DOCS-08 exists

## Files To Add

- `docs/02-cli/greentic-flow.md`
- `docs/03-authoring/flow-step-schema-mapping.md`

## Files To Update

- `docs/00-start-here.md`
- `docs/03-authoring/happy-path-build-an-app.md` only if a referral is needed later

## Files To Redirect Or Deprecate

- None required

## Content Requirements

`greentic-flow.md` must explain:

- `greentic-flow component-schema <component>.wasm`
- what it returns
- how agents should use it before wiring a component into a flow
- common documented components to inspect only if grounded in current repo usage

`flow-step-schema-mapping.md` must explain:

- how component input/output schema relates to step authoring
- how payload/state/config map into steps
- `in_map`, `out_map`, `err_map`
- when explicit mapping is needed
- when mapping may be omitted
- how to unify outputs across heterogeneous components
- practical pitfalls and validation steps

## Acceptance Criteria

- Agents can treat component schema as a first-class truth source
- Mapping guidance is practical enough to support authoring decisions
- Terms align with current flow implementation, not older docs

## Verification

- Inspect current implementation and schema outputs where available
- Confirm terminology against actual tool output and code, not prior prose alone
- Verify the docs are marked as operational guidance when implementation is elsewhere

## Risks / Ambiguities

- Some required verification may depend on companion tooling not always installed locally
- Current examples may be sparse or stale
- Historical naming drift is likely

## Follow-up PRs

- `PR-DOCS-06-platform-capabilities.md`
- `PR-DOCS-11-example-validation.md`

