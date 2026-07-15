Status: Generated from current tooling
Scope: Fixture-backed example component schema for `greentic-flow`
Implementation owner: `greentic-flow` fixture resolver

# Acme Widget Fixture Schema

This file records one verified `greentic-flow component-schema` output captured
through the installed fixture resolver.

It is included as **fixture coverage**, not as a claim that `acme/widget:1` is
one of this repo's real canonical production components.

## Provenance

- Tool version: `greentic-flow 1.1.4`
- Command:

```bash
greentic-flow component-schema oci://acme/widget:1 \
  --resolver fixture:///Users/maarten/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/greentic-flow-1.1.4/tests/fixtures/registry \
  --format json
```

- Raw artifact: [`acme-widget-fixture-default.json`](./acme-widget-fixture-default.json)

## Current Output Shape

The emitted schema is currently:

- `type: object`
- `additionalProperties: false`
- no declared top-level properties

This means the verified fixture default-mode contract in this environment is an
empty strict object schema.

## Why This Is Still Useful

This proves that:

- `greentic-flow component-schema` is available locally
- fixture-based component-schema capture works in this environment
- later automation can expand this folder once real component references are
  available or pinned for local verification
