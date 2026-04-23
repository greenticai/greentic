# PR-DOCS-10 — Drift Report And Policy Enforcement

## Goal

Make schema and documentation drift visible and actionable, then add a pragmatic enforcement path that warns or fails in the right places without becoming flaky.

## Why This PR Exists

Refreshing generated files is not enough by itself. Contributors need help understanding what changed and which prose docs or examples likely need review.

## Status Model

`Canonical in this repo`

## Implementation Owner(s)

- `gtc` or the repo-local docs-sync tooling path
- local and CI policy owned in this repo

## In Scope

- Extend sync output into a drift report or equivalent console artifact
- Identify notable schema changes
- Surface docs/examples likely needing review
- Add enforcement path via local check, CI warning, or both

## Out Of Scope

- Perfect semantic diffing
- Broad example validation coverage beyond what PR-DOCS-11 will add

## Inputs To Verify First

- Current sync-tool behavior from PR-DOCS-09
- Current local check and CI workflow structure
- Current generated docs folder conventions
- Current canonical prose docs likely to depend on schemas

## Files To Add

- `docs/04-schemas/drift-report.md` only if a persistent artifact is the best fit

## Files To Update

- Sync-tool implementation
- `ci/local_check.sh`
- relevant CI workflow wiring if straightforward
- `docs/04-schemas/README.md`
- maintenance-policy docs if needed

## Files To Redirect Or Deprecate

- None required

## Content Requirements

The drift report should identify:

- schemas changed
- notable fields added, removed, or renamed when detectable
- docs likely needing manual review
- examples likely now stale

The enforcement path should:

- prefer non-flaky behavior
- allow a clear warning path if hard failure is too invasive initially
- ensure schema-changing PRs do not silently skip doc refresh expectations

## Acceptance Criteria

- A schema change produces a clear signal
- An agent or contributor can use the report to decide which docs/examples to review
- The enforcement path exists, even if initial mode is warning-first rather than hard fail

## Verification

- Simulate or observe changed schema output
- Verify the report is understandable to both humans and coding agents
- Verify local-check or CI behavior is explicit and not surprisingly brittle

## Risks / Ambiguities

- Heuristic diffing may produce false positives or miss meaningful semantic changes
- Hard enforcement too early may create churn
- Persistent report files may create noisy diffs if not designed carefully

## Follow-up PRs

- `PR-DOCS-11-example-validation.md`
- `PR-DOCS-12-troubleshooting-and-maintenance-hardening.md`

