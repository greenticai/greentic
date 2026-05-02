Status: Canonical in this repo
Scope: Local behavior + current operational guidance
Implementation owner: gtc documentation in this repo

# Start Here

This repo now treats a small set of repo-local documents as the fastest way to
find implementation truth and avoid stale examples.

## What Is Canonical Here

Use the following source-of-truth order when you need the current answer for
this repository:

1. Generated schema docs under [`docs/04-schemas/`](./04-schemas/README.md)
2. Repo-local canonical docs in `docs/`
3. Current code in this repo
4. Curated examples and demos
5. Other repos only when a repo-local doc explicitly tells you to consult them

Generated schema docs are the highest-priority source when they exist for the
topic you are working on. If schema and prose disagree, trust schema first and
update the prose.

## Status Vocabulary

This repo uses these labels deliberately:

- `Canonical in this repo`
  Use this for local implementation truth, local workflows, local defaults, and
  local policy.
- `Operational guidance in this repo`
  Use this when the underlying implementation may live elsewhere, but
  contributors in this repo still need current guidance on how to use it here.
- `Generated from current tooling`
  Use this for machine-derived schema docs under `docs/04-schemas/`.

## What Is Not Automatically Authoritative

Treat these as useful inputs, not default truth:

- demo repositories
- old screenshots
- older blog posts or release notes
- examples copied from adjacent Greentic repos
- local habits or remembered CLI syntax

`greentic-demo` and older repos can still be helpful, but they are not
canonical unless a repo-local doc explicitly points you there for a specific
reason.

## Happy Path For Coding Agents

If you are changing behavior or docs in this repo, read in this order:

1. [`.codex/global_rules.md`](../.codex/global_rules.md)
2. [`docs/99-agent-rules/coding-agents.md`](./99-agent-rules/coding-agents.md)
3. [`docs/00-start-here/current-terms-and-deprecations.md`](./00-start-here/current-terms-and-deprecations.md)
4. [`docs/04-schemas/README.md`](./04-schemas/README.md)
5. Current repo-local canonical docs for the area you are touching
6. The relevant code paths if anything is still ambiguous

## Current Doc Tree

Start with these repo-local docs:

- [`docs/99-agent-rules/coding-agents.md`](./99-agent-rules/coding-agents.md)
  Rules for coding agents and contributors working in this repo.
- [`docs/00-start-here/current-terms-and-deprecations.md`](./00-start-here/current-terms-and-deprecations.md)
  Current terminology and wording guardrails.
- [`docs/04-schemas/README.md`](./04-schemas/README.md)
  Canonical location for generated schema docs.
- [`docs/04-schemas/wizard-schema.md`](./04-schemas/wizard-schema.md)
  Current generated wizard-schema summary plus raw JSON artifact.
- [`docs/04-schemas/setup-schema.md`](./04-schemas/setup-schema.md)
  Current setup-schema availability note for the installed toolchain.
- [`docs/04-schemas/component-schemas/README.md`](./04-schemas/component-schemas/README.md)
  Optional expanded component-schema coverage and current gaps.
- `gtc docs sync-schemas --best-effort`
  Supported refresh command for the generated schema docs in this repo.
- [`docs/examples/README.md`](./examples/README.md)
  Canonical validated structured examples referenced by the docs.
- [`docs/01-core-model/components-flows-packs-bundles.md`](./01-core-model/components-flows-packs-bundles.md)
  Canonical explanation of how the main Greentic building blocks fit together.
- [`docs/01-core-model/extensions-overview.md`](./01-core-model/extensions-overview.md)
  Repo-local guidance for extension families and bundle composition.
- [`docs/02-cli/gtc-setup.md`](./02-cli/gtc-setup.md)
  Canonical setup entrypoint and current ownership boundary.
- [`docs/02-cli/gtc-start.md`](./02-cli/gtc-start.md)
  Canonical start entrypoint, target selection, and local-vs-deployer behavior.
- [`docs/02-cli/gtc-wizard.md`](./02-cli/gtc-wizard.md)
  Canonical wizard entrypoint, routing behavior, and current limitations.
- [`docs/02-cli/gtc-install.md`](./02-cli/gtc-install.md)
  Canonical install entrypoint, release channels, and stable OCI reference guidance.
- [`docs/02-cli/greentic-flow.md`](./02-cli/greentic-flow.md)
  Repo-local guidance for using `greentic-flow component-schema` before wiring steps.
- [`docs/03-authoring/happy-path-build-an-app.md`](./03-authoring/happy-path-build-an-app.md)
  Linear repo-local path from authoring into setup and start.
- [`docs/03-authoring/answers-json-patterns.md`](./03-authoring/answers-json-patterns.md)
  Conservative guidance for `answers.json` documents.
- [`docs/03-authoring/flow-step-schema-mapping.md`](./03-authoring/flow-step-schema-mapping.md)
  Practical schema-first guidance for flow-step mapping.
- [`docs/03-authoring/i18n-qa-distributor-client.md`](./03-authoring/i18n-qa-distributor-client.md)
  Operational guidance for repo-local i18n behavior, schema/QA patterns, and distributor-client usage boundaries.
- [`docs/03-authoring/mcp-wasm-and-adapters.md`](./03-authoring/mcp-wasm-and-adapters.md)
  Operational guidance for composing MCP-oriented capability into flow-compatible components.
- [`docs/03-authoring/mcp-config-secrets-oauth.md`](./03-authoring/mcp-config-secrets-oauth.md)
  Operational guidance for config, secrets, and OAuth boundaries around MCP-style integrations.
- [`docs/03-authoring/extension-pack-patterns.md`](./03-authoring/extension-pack-patterns.md)
  Practical guidance for choosing extension packs, composing them with app packs, and using extension handoff flows.
- [`docs/03-authoring/adaptive-card-orchestration.md`](./03-authoring/adaptive-card-orchestration.md)
  Practical guidance for adaptive-card rendering, reply handling, state, and flow/component boundaries.
- [`docs/gtc-wizard.md`](./gtc-wizard.md)
  Redirect stub kept only for old links and search habits.
- [`docs/config.md`](./config.md)
  Repo-owned environment/config surface for `gtc`.
- [`docs/ci.md`](./ci.md)
  Repo-local CI and workflow notes.
- [`docs/release-verification.md`](./release-verification.md)
  Release verification flow for packaged artifacts.
- [`docs/repository_catalog_en.md`](./repository_catalog_en.md)
  English catalog of adjacent Greentic repos and components. Useful context, but
  not a replacement for current repo-local canonical docs.
- [`docs/05-examples/demo-map.md`](./05-examples/demo-map.md)
  How to use `greentic-demo` safely without treating it as canonical.
- [`docs/05-examples/repo-compatibility-map.md`](./05-examples/repo-compatibility-map.md)
  Conservative trust map for adjacent Greentic repos.
- [`docs/06-troubleshooting/common-authoring-and-runtime-issues.md`](./06-troubleshooting/common-authoring-and-runtime-issues.md)
  Practical troubleshooting guide for current authoring, validation, setup, and runtime confusion.

## Working Rule

When behavior changes in this repo, update the relevant canonical docs in the
same PR. Do not leave docs drift for a follow-up unless the PR clearly calls out
the gap and the missing work is unavoidable.

## Documentation Maintenance Policy

Treat documentation maintenance as part of the implementation work, not as a
cleanup step after the fact.

Update generated docs when:

- wizard-schema output changes
- component-schema coverage changes
- the schema sync tool or its captured outputs change

Update prose docs when:

- CLI behavior, routing, defaults, or ownership boundaries change
- setup/start/wizard guidance would otherwise describe stale behavior
- troubleshooting advice changes because the current failure modes changed

Update examples when:

- canonical structured examples no longer match current schema expectations
- a doc points at an example that is no longer validated

Update terminology docs when:

- a current term replaces an older alias
- a deprecated name still appears often enough to confuse contributors or
  agents

Verify changes with the repo-local path that fits the work:

- `gtc docs sync-schemas --best-effort` or
  `bash ci/sync_schema_docs.sh --best-effort` for generated docs
- `bash ci/validate_doc_examples.sh` for canonical structured examples
- `bash ci/local_check.sh` for the broader repo check, while noting any
  unrelated pre-existing failures separately

If generated docs, prose docs, examples, and implementation diverge, fix the
divergence in the same PR whenever reasonably possible.
