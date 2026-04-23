# PR-DOCS-07 — Extension And Adaptive Card Patterns

## Goal

Document practical extension-pack composition and adaptive-card orchestration patterns so contributors can choose the right pattern without reverse-engineering scattered examples.

## Why This PR Exists

Extension composition and adaptive-card orchestration are both places where agents tend to improvise. This PR should replace improvisation with practical, opinionated guidance.

## Status Model

- `Canonical in this repo` where the repo owns local defaults or command-facing behavior
- `Operational guidance in this repo` where implementation is elsewhere

## Implementation Owner(s)

- Mixed ownership depending on the exact extension or card runtime path

## In Scope

- Common extension types and usage guidance
- Recommended defaults where current repo guidance supports them
- Decision matrix: “Need X -> use Y”
- How extension packs attach to bundles
- How application packs and extension packs coexist
- Adaptive-card orchestration guidance

## Out Of Scope

- Full extension implementation references
- Generated schema automation
- Broad trust-boundary restatement already covered in PR-DOCS-02

## Inputs To Verify First

- Current extension-related CLI help and code paths
- README extension references
- Existing architecture and catalog docs
- Any current references to adaptive-card components, state handling, UI rendering, or orchestration

## Files To Add

- `docs/03-authoring/extension-pack-patterns.md`
- `docs/03-authoring/adaptive-card-orchestration.md`

## Files To Update

- `docs/01-core-model/extensions-overview.md`
- `docs/00-start-here.md` only if additional key referrals are needed

## Files To Redirect Or Deprecate

- None required

## Content Requirements

`extension-pack-patterns.md` must explain:

- common extension types
- when to use each
- recommended defaults
- how to attach them to bundles
- how app packs and extension packs coexist
- precedence/order/conflict guidance if current implementation supports such claims
- decision matrix for common needs

`adaptive-card-orchestration.md` must explain:

- recommended architecture
- how adaptive-card rendering and orchestration fit into components and flows
- state handling
- reply/response handling
- validation and templating boundaries
- when logic belongs in a component versus a flow
- common orchestration patterns
- common mistakes and anti-patterns

## Acceptance Criteria

- An agent can decide which extension approach to use
- Adaptive-card guidance is concrete and opinionated rather than generic
- Docs reduce ambiguity around defaults without inventing unsupported mechanisms

## Verification

- Inspect current repo context before making any recommendation sound canonical
- Verify decision matrix recommendations against real implementation or current trusted guidance
- Confirm any “default” recommendation is still current

## Risks / Ambiguities

- “Default” messaging/UI direction may change over time
- Adaptive-card behavior may be distributed across multiple repos/components
- It is easy to accidentally write aspirational architecture instead of current patterns

## Follow-up PRs

- `PR-DOCS-08-generated-schema-docs-baseline.md`
- `PR-DOCS-12-troubleshooting-and-maintenance-hardening.md`

