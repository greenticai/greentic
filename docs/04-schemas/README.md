Status: Canonical in this repo
Scope: Reserved location for generated schema docs
Implementation owner: gtc documentation in this repo

# Schema Docs

This directory is the canonical location for generated schema docs in this repo.

Use this folder for machine-derived outputs and the minimal markdown wrappers
that explain what was generated, when it was generated, and which command
produced it.

## Refresh Command

Use either of these supported entrypoints to refresh the schema docs:

```bash
gtc docs sync-schemas --best-effort
```

or, if you need the repo-local path directly:

```bash
bash ci/sync_schema_docs.sh --best-effort
```

Use `--strict` when you want missing optional companion coverage to be treated
as an error instead of a warning.

## What Belongs Here

This folder can contain:

- raw generated JSON schema artifacts
- markdown summaries that point to those artifacts
- partial-coverage notes when a command or companion binary is unavailable
- optional companion-tool schema coverage when it is verifiable in the current environment

## Repo-Owned Versus Optional Coverage

Treat the outputs here in two groups:

- **Repo-owned schema outputs**
  These are the highest-priority generated references for behavior exposed by
  `gtc` itself.
- **Optional expanded coverage**
  These come from companion tools such as `greentic-flow` when those tools are
  available locally and the referenced component can be verified.

When optional expanded coverage is missing, say so explicitly rather than
inventing or copying schemas from another repo.

## Current Baseline Outputs

- [`wizard-schema.md`](./wizard-schema.md)
  Generated summary for the current installed `gtc wizard --schema` output.
- [`wizard-schema.json`](./wizard-schema.json)
  Raw emitted JSON schema captured from current tooling.
- [`setup-schema.md`](./setup-schema.md)
  Current status note for setup-schema coverage in the installed toolchain.
- [`component-schemas/README.md`](./component-schemas/README.md)
  Notes for optional expanded component-schema coverage.
- [`drift-report.md`](./drift-report.md)
  Heuristic summary of changed schema outputs and likely follow-up review targets.

## Drift Review

After running schema sync, review:

- [`drift-report.md`](./drift-report.md)
- any changed files under `docs/04-schemas/`
- the canonical prose docs or examples listed in the drift report

This is a warning-first enforcement path today: local check refreshes schema
docs, emits the drift report, and warns when generated schema docs changed.

## Example Validation

Canonical structured examples that depend on current schema should be validated
through the repo-local example validator:

```bash
bash ci/validate_doc_examples.sh
```

The current validated example set lives under
[`docs/examples/`](../examples/README.md).

The full generated-doc workflow will be added in a later PR. Until then:

- treat this path as the stable destination for schema-derived docs
- prefer live schema output from current tooling when available
- treat repo-local canonical prose docs as the next-best source when generated
  schema docs have not landed yet for a topic

Human-authored docs should link to generated schema docs here rather than
embedding large schema dumps inline.
