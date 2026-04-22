Status: Canonical in this repo
Scope: Local troubleshooting + current operational guidance
Implementation owner: gtc documentation in this repo

# Common Authoring And Runtime Issues

Use this guide when the current canonical docs are clear in principle but the
actual authoring, setup, start, or validation flow still fails in practice.

## `gtc wizard --answers` does not produce the artifacts you expected

What to check first:

- Verify the shape of your input against
  [`docs/04-schemas/wizard-schema.md`](../04-schemas/wizard-schema.md).
- Compare your file against the validated examples in
  [`docs/examples/`](../examples/README.md).
- Re-read [`docs/02-cli/gtc-wizard.md`](../02-cli/gtc-wizard.md) before
  assuming support for a command path you saw in another repo.

Why this happens here:

- `gtc wizard` routes into `greentic-dev wizard` for normal wizard behavior.
- This repo can prove the launcher-style schema it captures locally, but it
  does not prove every downstream artifact shape from companion tooling.

What to do:

- Start from the smallest validated example and add fields incrementally.
- Run `bash ci/validate_doc_examples.sh` if you are editing canonical examples.
- If your intended field is not in the generated schema, do not invent it.

## Component schema and flow mappings disagree

What to check first:

- Inspect
  [`docs/02-cli/greentic-flow.md`](../02-cli/greentic-flow.md).
- Inspect
  [`docs/03-authoring/flow-step-schema-mapping.md`](../03-authoring/flow-step-schema-mapping.md).

Why this happens here:

- Component-level schemas and flow-level step wiring are related, but not
  identical.
- Heterogeneous components often need explicit `in_map`, `out_map`, or
  `err_map` to normalize shapes.

What to do:

- Treat `payload`, `state`, and `config` as different concerns.
- Add explicit mapping when shapes diverge instead of relying on memory or
  implicit conventions.
- Re-run `greentic-flow component-schema ...` before updating prose examples.

## The bundle starts locally but fails when using a deployer or cloud target

What to check first:

- Review [`docs/02-cli/gtc-start.md`](../02-cli/gtc-start.md).
- Confirm whether you are using the local runtime path or a deployer-backed
  path.

Why this happens here:

- `gtc start` owns local orchestration and target selection in this repo, but
  deploy/install behavior also depends on deployer-side credentials, registry
  access, and remote runtime assumptions.

What to do:

- Verify that bundle references resolve in the target environment.
- If you are using `repo://` or `store://` refs, confirm
  `GREENTIC_REPO_REGISTRY_BASE` or `GREENTIC_STORE_REGISTRY_BASE` are set.
- Treat a successful local run as proof of bundle validity, not proof that the
  remote environment is ready.

## `gtc setup` behavior is unclear or differs from older examples

What to check first:

- Read [`docs/02-cli/gtc-setup.md`](../02-cli/gtc-setup.md).
- Check [`docs/04-schemas/setup-schema.md`](../04-schemas/setup-schema.md).

Why this happens here:

- The installed toolchain currently does not support `gtc setup --schema`, so
  older prose or copied examples can overstate what is machine-verifiable.

What to do:

- Prefer current CLI help and current repo-local docs over older examples.
- When behavior is still ambiguous, inspect the current setup code path before
  writing or updating canonical guidance.

## Locale or i18n behavior is missing or inconsistent

What to check first:

- Review
  [`docs/03-authoring/i18n-qa-distributor-client.md`](../03-authoring/i18n-qa-distributor-client.md).
- Check the current repo config surfaces in [`docs/config.md`](../config.md).

Why this happens here:

- Locale can be influenced by environment and runtime context, and some i18n
  behavior is operational guidance rather than implementation owned here.
- This repo currently also has a compile-time issue around `greentic_i18n`
  imports, which can block verification even when the docs are correct.

What to do:

- Check locale-related env vars such as `GTC_LOCALE` and `GREENTIC_LOCALE`.
- Distinguish missing translated content from a runtime selection problem.
- If repo checks fail before you reach runtime, note the current compile issue
  separately instead of treating it as proof that your doc change is wrong.

## Config, secrets, or OAuth do not resolve in MCP-oriented flows

What to check first:

- Review
  [`docs/03-authoring/mcp-wasm-and-adapters.md`](../03-authoring/mcp-wasm-and-adapters.md)
  and
  [`docs/03-authoring/mcp-config-secrets-oauth.md`](../03-authoring/mcp-config-secrets-oauth.md).

Why this happens here:

- Generic MCP documentation does not define Greentic’s wrapper boundaries.
- Config passing, secret resolution, and OAuth brokerage may depend on adjacent
  services or runtime infrastructure that this repo does not own fully.

What to do:

- Confirm whether the failure is in authoring, setup-time injection, or runtime
  resolution.
- Keep component semantics and secret-provider semantics separate.
- Document unresolved behavior as operational guidance, not local
  implementation truth, unless this repo’s code proves more.

## Extension pack composition is confusing

What to check first:

- Review
  [`docs/03-authoring/extension-pack-patterns.md`](../03-authoring/extension-pack-patterns.md)
  and
  [`docs/01-core-model/extensions-overview.md`](../01-core-model/extensions-overview.md).

Why this happens here:

- This repo supports both normal bundle composition and an extension-handoff
  flow with flags such as `--extensions`, `--extension-setup-handoff`, and
  `--extension-start-handoff`.
- It is easy to mix those paths conceptually.

What to do:

- Decide first whether you need ordinary bundle composition or extension
  launcher handoff.
- Do not assume pack precedence or conflict behavior unless a repo-local doc
  says so explicitly.

## Generated schema docs are out of date

What to check first:

- Review [`docs/04-schemas/README.md`](../04-schemas/README.md).
- Check [`docs/04-schemas/drift-report.md`](../04-schemas/drift-report.md).

What to do:

- Run `gtc docs sync-schemas --best-effort` or
  `bash ci/sync_schema_docs.sh --best-effort`.
- Inspect the drift report and update affected prose docs if the generated
  schema changed.
- If optional companion binaries are unavailable locally, treat the refresh as
  partial and keep the warning in mind when reviewing.

## Validated canonical examples fail local validation

What to check first:

- Review [`docs/examples/README.md`](../examples/README.md).
- Compare your example against
  [`docs/04-schemas/wizard-schema.json`](../04-schemas/wizard-schema.json).

What to do:

- Run `bash ci/validate_doc_examples.sh`.
- Fix the structured example first, then update any prose snippets that refer
  to it.
- Prefer linking to validated example files instead of keeping slightly
  different inline JSON in multiple docs.

## `ci/local_check.sh` fails after doc changes

What to check first:

- Read the output in order. The script now runs schema sync and example
  validation before cargo-based checks.

What to do:

- If schema docs changed, review
  [`docs/04-schemas/drift-report.md`](../04-schemas/drift-report.md).
- If example validation failed, fix the canonical example files before editing
  prose.
- If Rust compilation fails later, separate your docs verification result from
  unrelated pre-existing compile failures.

Current known repo issue:

- `src/perf_targets.rs:7` currently fails with
  `error[E0432]: unresolved import greentic_i18n` during local checks.
