Status: Generated from current tooling
Scope: Current installed `gtc wizard --schema` output
Implementation owner: `gtc` / installed companion tooling

# Wizard Schema

This document summarizes the current generated wizard schema captured from the
installed toolchain in this environment.

## Provenance

- Tool version: `gtc 1.0.9`
- Command:

```bash
gtc wizard --schema
```

- Raw artifact: [`wizard-schema.json`](./wizard-schema.json)

## Stable Top-Level Facts

- `title`: `greentic-dev launcher wizard answers`
- `wizard_id`: `greentic-dev.wizard.launcher.main`
- `schema_id`: `greentic-dev.launcher.main`
- `schema_version`: `1.0.0`
- `selected_action` enum:
  - `pack`
  - `bundle`

## What This Schema Represents

The emitted schema is the launcher-level answer contract exposed by the current
installed `gtc` toolchain.

It is not only a tiny top-level wrapper. The raw JSON also embeds nested schema
material for delegated pack and bundle answers, which is why the captured raw
artifact is substantially larger than a simple launcher contract.

## How To Use It

- Use this schema before creating or validating wizard `answers.json` input for
  the launcher path.
- Treat the raw JSON artifact as the canonical machine-derived reference.
- Use prose docs such as [`../02-cli/gtc-wizard.md`](../02-cli/gtc-wizard.md)
  only as interpretation and guidance around the emitted contract.

## Current Limitations

- This sync path captures the installed toolchain output, not yet a fully
  repo-pinned internal schema emitter.
- Later drift-report tooling can build on this generated baseline without
  requiring contributors to hand-edit schema summaries.
