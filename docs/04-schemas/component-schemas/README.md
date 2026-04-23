Status: Generated from current tooling
Scope: Optional expanded component-schema coverage for this repo
Implementation owner: `greentic-flow` for component-schema emission; this folder records what was verifiable locally

# Component Schemas

This folder is reserved for component-schema outputs captured through
`greentic-flow component-schema`.

## Coverage Rule

Only check in component-schema outputs here when the referenced component was
actually verifiable in the current environment.

That means:

- prefer current real component references when they are available locally
- use fixture-backed examples only when they are clearly labeled as fixtures
- do not copy component schemas from another repo or from memory

## Current Baseline Coverage

- [`acme-widget-fixture-default.md`](./acme-widget-fixture-default.md)
  Verified through the installed `greentic-flow` fixture resolver when that
  fixture source is available locally.

## Current Gaps

The commonly referenced real components in repo prose, such as adaptive-card,
templates, and llm-openai, are still intentionally absent here until they can
be regenerated from verifiable local or pinned inputs.
