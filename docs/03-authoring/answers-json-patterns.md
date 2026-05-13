Status: Canonical in this repo
Scope: Repo-local guidance for wizard answer documents
Implementation owner: Mixed ownership; schema details are downstream-owned unless proven here

# `answers.json` Patterns

This doc is intentionally conservative.

The single safest rule is:

- if `gtc wizard --schema` is available, use it before writing `answers.json`

This repo should not invent answer keys from memory, from demo repos, or from
older screenshots.

## Minimum Safe Pattern

There are two different levels of confidence to keep separate:

- **observed emitted shape**
  Current integration tests in this repo confirm that downstream
  `--emit-answers` behavior can produce a document containing top-level fields
  such as `schema_version`, `answers`, and `events`.
- **validated canonical examples**
  This repo now keeps its canonical checked-in wizard examples under
  [`docs/examples/`](../examples/README.md), and those are validated against the
  current generated launcher schema.

Treat the emitted shape as useful observation, not as a universal contract for
every downstream wizard flow.

## Practical Pattern

A practical workflow is:

1. inspect the schema
2. emit or prepare an answer file
3. fill only the keys the current schema expects
4. rerun the wizard using that answer file

## Recommended Workflow

Use this sequence:

```bash
gtc wizard --schema
gtc wizard --emit-answers ./answers.json
gtc wizard --answers ./answers.json
```

`gtc wizard --answers` and `gtc setup --answers` accept the same source forms:
plain local paths, `file://...`, `http://...`, `https://...`, `oci://...`,
`store://...`, and `repo://...`. The referenced JSON must parse as an object.

If the downstream wizard does not support `--emit-answers` for your exact flow,
fall back to using the schema output directly.

## Validated Examples

Use these checked-in examples when you want a concrete starting point that this
repo validates automatically:

- [`docs/examples/wizard-launcher-minimal.answers.json`](../examples/wizard-launcher-minimal.answers.json)
- [`docs/examples/wizard-launcher-bundle.answers.json`](../examples/wizard-launcher-bundle.answers.json)

These examples are intentionally launcher-scoped because that is the current
generated schema this repo can validate locally. The second example reflects the
current `answers.selected_action` enum including values such as `pack` and
`bundle`.

## What This Repo Does Not Yet Prove

This repo does **not** currently prove, on its own, the full exact answer shape
for all of these downstream intents:

- component generation from `./src`
- flow create/update
- add-step/update-step
- pack create/update
- bundle create/update

Those topics are real workflow goals, but the exact keys and modes should be
taken from the current downstream schema, not assumed from this prose doc.

## Common Mistakes

Do not:

- invent keys because they “sound right”
- copy answer files from `greentic-demo` without re-checking schema
- assume old answer files still match the current downstream wizard
- treat README examples as proof of the exact JSON structure
- confuse setup answers with wizard answers

## Working Rule

If the schema and a handwritten example disagree:

- trust the schema
- fix the example
