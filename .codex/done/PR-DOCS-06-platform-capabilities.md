# PR-DOCS-06 — Platform Capabilities

## Goal

Provide one coherent set of operational-guidance docs for cross-cutting platform capabilities this repo depends on: i18n, QA, distributor-client, MCP/WASM adapters, config, secrets, and OAuth.

## Why This PR Exists

These topics are easy to misunderstand because they cross repo boundaries. Contributors still need them documented here, but not in a way that falsely claims local implementation ownership.

## Status Model

`Operational guidance in this repo`

## Implementation Owner(s)

- Mixed ownership across Greentic repos and companion tools
- This repo owns the local usage guidance and trust boundaries

## In Scope

- i18n operational model for this repo
- QA/schema interplay as used by this repo
- distributor-client usage boundaries
- MCP WASM and adapter composition pattern
- config, secrets, and OAuth authoring/runtime boundaries

## Out Of Scope

- Full reference docs for every external repo
- Generated schema tooling
- Extension decision matrix as a standalone topic

## Inputs To Verify First

- Current README references to i18n, QA, MCP, OAuth, distributor-client
- Current code paths touching distributor-client, config, secrets, OAuth, and extension handoff
- Existing repo catalogs and architecture docs
- Any current docs already covering config or CI/runtime expectations

## Files To Add

- `docs/03-authoring/i18n-qa-distributor-client.md`
- `docs/03-authoring/mcp-wasm-and-adapters.md`
- `docs/03-authoring/mcp-config-secrets-oauth.md`

## Files To Update

- `docs/00-start-here.md`
- `docs/01-core-model/extensions-overview.md` only if a referral is needed

## Files To Redirect Or Deprecate

- None required

## Content Requirements

`i18n-qa-distributor-client.md` must explain:

- how i18n works conceptually and operationally in this repo
- how QA specs, answers, and schema interplay work in current repo usage
- how wizard/setup flows rely on QA/schema patterns if they do
- what distributor-client is for
- how bundles, packs, or components may resolve or publish through distributor-client
- repo-specific ownership boundaries
- when to consult another repo versus when repo-local docs here are enough

`mcp-wasm-and-adapters.md` must explain:

- what an MCP WASM means in Greentic context
- how an MCP component composes with an MCP adapter to become Greentic-compatible
- what belongs to MCP versus the Greentic wrapper
- what agents must not assume from generic MCP docs
- concrete composition examples if current repo context supports them

`mcp-config-secrets-oauth.md` must explain:

- config handling
- secrets handling
- OAuth handling
- runtime expectations
- authoring and setup responsibilities
- common failure modes

## Acceptance Criteria

- Contributors have one place to understand these platform capabilities in repo context
- Docs distinguish local usage guidance from implementation ownership elsewhere
- The docs are operational and decision-supportive, not generic platform marketing

## Verification

- Inspect current code and docs for each capability
- Verify every ownership statement conservatively
- Confirm examples are consistent with current repo behavior

## Risks / Ambiguities

- These topics may evolve across multiple repos on different cadences
- Current repo-local implementation may only touch subsets of the broader platform concepts
- Over-documentation risk is high if scope is not kept operational

## Follow-up PRs

- `PR-DOCS-07-extension-and-adaptive-card-patterns.md`
- `PR-DOCS-12-troubleshooting-and-maintenance-hardening.md`

