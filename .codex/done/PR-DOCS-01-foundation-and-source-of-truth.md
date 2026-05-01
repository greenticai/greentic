# PR-DOCS-01 — Foundation And Source Of Truth

## Goal

Create the canonical documentation entrypoints, source-of-truth policy, terminology baseline, and agent guidance that every later documentation PR depends on.

## Why This PR Exists

The repo currently has useful material, but a contributor or coding agent still has to infer where truth lives. This PR establishes that decision path up front so later docs can be written against one explicit contract.

## Status Model

Primary output is `Canonical in this repo`.

## Implementation Owner(s)

- `gtc` for local CLI behavior and repo-local policy
- `.codex/global_rules.md` for authoritative agent rules in this repo

## In Scope

- Add canonical entrypoint doc under `docs/00-start-here.md`
- Add human-readable agent rules doc under `docs/99-agent-rules/coding-agents.md`
- Update `.codex/global_rules.md` rather than creating a second rules file
- Add a concise coding-agent referral section near the top of `README.md`
- Define source-of-truth ordering
- Define documentation status vocabulary
- Define terminology/deprecations baseline early in the program
- Add initial documentation maintenance policy language

## Out Of Scope

- Deep command semantics
- Generated schema tooling
- Cross-repo docs beyond trust classification and guidance
- Updating non-English docs

## Inputs To Verify First

- Current `README.md`
- Existing `.codex/global_rules.md`
- Existing `docs/gtc-wizard.md`
- Existing repo catalogs and architecture artifacts that may need referral links rather than duplication
- Current CLI help for commands owned in this repo

## Files To Add

- `docs/00-start-here.md`
- `docs/00-start-here/current-terms-and-deprecations.md`
- `docs/99-agent-rules/coding-agents.md`

## Files To Update

- `README.md`
- `.codex/global_rules.md`

## Files To Redirect Or Deprecate

- None required in this PR

## Content Requirements

`docs/00-start-here.md` must include:

- what this repo considers canonical
- source-of-truth order:
  - generated schema docs
  - repo-local canonical docs
  - current code in this repo
  - curated examples and demos
  - other repos only when explicitly referenced
- warning that demos and older repos are not automatically authoritative
- links to the doc tree
- happy-path reading order for coding agents
- explicit distinction between:
  - `Canonical in this repo`
  - `Operational guidance in this repo`

`docs/00-start-here/current-terms-and-deprecations.md` must include:

- current term
- deprecated or older synonym
- status
- whether new docs/code should use it
- notes grounded in current code/docs rather than memory

`docs/99-agent-rules/coding-agents.md` must include explicit rules:

- do not infer CLI syntax from examples in other repos
- do not invent `answers.json` keys if schema exists
- do not treat demos as canonical unless docs say so
- if schema and prose disagree, trust schema first and update docs
- prefer current terminology over deprecated aliases
- validate doc examples where possible
- when changing behavior, update canonical docs in the same PR

`README.md` must:

- remain onboarding-oriented
- add a visible “For coding agents and contributors” section
- refer agents to:
  - `docs/00-start-here.md`
  - `.codex/global_rules.md`
  - `docs/04-schemas/`
- avoid duplicating detailed command semantics

`.codex/global_rules.md` must:

- reinforce doc-sync expectations
- state that if CLI/schema/common component behavior changes, docs must be updated in the same PR
- state that generated docs drift should be fixed by running the sync tool once it exists
- avoid becoming a duplicate human-facing contribution guide

## Acceptance Criteria

- A new contributor or agent can identify canonical docs in under 30 seconds
- README points agents to the correct sources without turning into an ops manual
- Agent rules explicitly forbid the common failure modes
- Terminology guidance exists before later doc PRs start
- Only one authoritative rules file exists for agents in this repo

## Verification

- Inspect current repo docs and current CLI help
- Verify links from `README.md` and `docs/00-start-here.md`
- Verify the source-of-truth order is consistent across all new files
- Verify no non-English docs were changed

## Risks / Ambiguities

- Existing docs may already imply different ownership boundaries
- Terminology may have drifted across README, CLI help, and prior planning docs
- `.codex/global_rules.md` must remain aligned with existing repo-overview workflow obligations

## Follow-up PRs

- `PR-DOCS-02-core-model-and-boundaries.md`
- `PR-DOCS-03-cli-setup-and-start.md`

