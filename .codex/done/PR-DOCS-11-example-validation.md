# PR-DOCS-11 — Example Validation

## Goal

Validate the highest-value canonical examples so docs do not drift into aspirational syntax or stale structured artifacts.

## Why This PR Exists

Even well-written prose drifts if the example files behind it are never checked. This PR makes example correctness part of the documentation system rather than an honor system.

## Status Model

`Canonical in this repo`

## Implementation Owner(s)

- Repo-local validation tooling and example set selection

## In Scope

- Validate canonical `answers.json` examples
- Validate any canonical flow fragments tied to current schemas where feasible
- Validate documented config snippets or structured examples where feasible
- Start with the examples used in canonical docs first

## Out Of Scope

- Exhaustive validation of every historical example in the repo
- Validation of non-canonical non-English docs

## Inputs To Verify First

- Canonical examples introduced in earlier PRs
- Current available schemas and sync outputs
- Current local check and CI posture

## Files To Add

- Validation script or tests
- Canonical example artifacts under docs/examples if needed

## Files To Update

- `ci/local_check.sh` if validation belongs there
- canonical docs to point at validated examples

## Files To Redirect Or Deprecate

- Any stale canonical structured example that is replaced by a validated equivalent

## Content Requirements

Validate, where feasible:

- sample `answers.json`
- sample flow fragments
- sample config snippets
- any structured examples directly tied to current schemas in canonical docs

Implementation should:

- start small
- prioritize correctness over breadth
- make it obvious which examples are canonical and validated

## Acceptance Criteria

- Highest-value canonical examples are validated automatically
- Canonical docs no longer depend on purely aspirational structured examples
- Validation coverage is clearly documented

## Verification

- Run validation locally
- Verify failing examples point clearly to the stale artifact
- Confirm validated examples match the canonical docs that reference them

## Risks / Ambiguities

- Some examples may be easier to validate structurally than behaviorally
- Over-scoping validation can slow iteration without much value
- Examples tied to companion tooling may need careful optional handling

## Follow-up PRs

- `PR-DOCS-12-troubleshooting-and-maintenance-hardening.md`

