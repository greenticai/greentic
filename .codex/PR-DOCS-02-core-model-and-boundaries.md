# PR-DOCS-02 — Core Model And Boundaries

## Goal

Document the Greentic core model for this repo’s audience and classify which related repos and examples are safe to consult, inspirational only, or implementation-authoritative elsewhere.

## Why This PR Exists

Agents and contributors need two things before they can reason safely:

- how components, flows, packs, bundles, and extensions fit together
- which repos and examples they may trust for what

Without that, later command docs still encourage cargo-culting from demos and adjacent repos.

## Status Model

- `Canonical in this repo` for repo-local framing and trust rules
- `Operational guidance in this repo` for cross-repo ownership boundaries

## Implementation Owner(s)

- Mixed ownership
- This repo owns the local documentation of boundaries and trust rules
- Related repos may own the underlying implementation for some concepts

## In Scope

- Canonical architecture doc for components, flows, packs, bundles
- Extension overview with current high-level families
- Demo map
- Repo compatibility map
- Conservative trust classification for adjacent repos

## Out Of Scope

- Detailed command syntax for `gtc`
- Generated schemas
- Sync tooling
- Troubleshooting deep dives

## Inputs To Verify First

- Current `README.md`
- `docs/architecture.mmd`
- `docs/repository_catalog.md`
- `docs/repository_catalog_en.md`
- Current code references to packs, bundles, start/setup behavior, extension handling
- Existing references to `greentic-demo`, `greentic-flow`, `greentic-pack`, `greentic-operator`, `greentic-mcp`, `greentic-oauth`, and distributor-client

## Files To Add

- `docs/01-core-model/components-flows-packs-bundles.md`
- `docs/01-core-model/extensions-overview.md`
- `docs/05-examples/demo-map.md`
- `docs/05-examples/repo-compatibility-map.md`

## Files To Update

- `docs/00-start-here.md`
- `README.md` only if a new important canonical doc referral is needed and remains concise

## Files To Redirect Or Deprecate

- None required unless an existing repo-catalog section should be explicitly referred to rather than duplicated

## Content Requirements

`components-flows-packs-bundles.md` must explain:

- what a component is
- what a flow is
- what a pack is
- what a bundle is
- how they compose
- difference between application packs and extension packs
- how a bundle can contain application packs plus extensions
- lifecycle from authoring to setup to start
- decision-support guidance:
  - when to create a component
  - when to create a flow
  - when to create a pack
  - when to create a bundle

`extensions-overview.md` must explain:

- common extension families at a high level
- messaging
- events
- state
- secrets
- OAuth
- observability/telemetry if relevant in current repo context
- static/public UI if relevant in current repo context
- current default or recommended messaging/UI direction only if grounded in current docs/code
- how extensions relate to bundles and app packs

`demo-map.md` must explain:

- what `greentic-demo` is useful for
- what is safe to copy conceptually
- what must always be re-validated against current schema/docs/code in this repo

`repo-compatibility-map.md` must classify related repos by:

- canonical for what
- example-only for what
- may lag current APIs
- safe to consult for inspiration
- must be re-checked against current repo schema/docs

## Acceptance Criteria

- A contributor can answer “what should I create here?” from the docs
- Repo trust boundaries are conservative and explicit
- Demos are framed as inspiration, not silent authority
- Cross-repo ownership is called out instead of implied away

## Verification

- Inspect current code and README terminology
- Verify every trust classification is supported by current repo context
- Confirm no doc claims ownership this repo does not actually have

## Risks / Ambiguities

- Some concepts may be described differently across related repos
- Extension families may evolve faster than current repo-local prose
- Existing catalogs may contain useful but non-canonical descriptions that need careful reuse

## Follow-up PRs

- `PR-DOCS-03-cli-setup-and-start.md`
- `PR-DOCS-04-cli-wizard-and-answers.md`
- `PR-DOCS-06-platform-capabilities.md`

