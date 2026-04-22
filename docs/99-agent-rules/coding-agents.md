Status: Canonical in this repo
Scope: Local documentation and verification rules for coding agents
Implementation owner: gtc documentation in this repo

# Coding Agents

Use these rules whenever you change code, docs, examples, or schemas in this
repository.

## Core Rules

- Do not infer CLI syntax from examples in other repos.
- Do not invent `answers.json` keys if a schema is available.
- Do not treat demos as canonical unless a repo-local doc says they are.
- If schema and prose disagree, trust schema first and update the prose.
- Prefer current terminology from
  [`docs/00-start-here/current-terms-and-deprecations.md`](../00-start-here/current-terms-and-deprecations.md),
  not older aliases.
- Validate doc examples where possible.
- When changing behavior, update the relevant canonical docs in the same PR.

## Verification Rules

- Check repo-local canonical docs before consulting other repos.
- Treat `docs/04-schemas/` as the first stop for schema-derived truth.
- Use current code in this repo when docs and behavior still look ambiguous.
- Run `gtc docs sync-schemas --best-effort` or
  `bash ci/sync_schema_docs.sh --best-effort` after schema-affecting changes.
- Run `bash ci/validate_doc_examples.sh` after changing canonical structured
  examples.
- If you consult another repo for context, re-check the final answer against
  this repo’s code, docs, and schemas before editing.

## Anti-Patterns To Avoid

- Copying command examples from demo repos without re-validation
- Treating screenshots or older README fragments as proof of current behavior
- Filling in missing schema details from memory
- Writing “official” guidance without stating whether it is canonical here or
  only operational guidance here
- Leaving docs drift for later after changing repo-owned behavior

## Maintenance Rule

When you touch CLI behavior, local workflows, or repo-owned structured examples,
make the canonical docs and examples match the implementation before you stop.
Check whether
[`docs/04-schemas/drift-report.md`](../04-schemas/drift-report.md),
[`docs/examples/`](../examples/README.md), and
[`docs/00-start-here/current-terms-and-deprecations.md`](../00-start-here/current-terms-and-deprecations.md)
also need updates.
