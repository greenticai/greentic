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

## Install And Artifact Channel Rules

See [`docs/02-cli/gtc-install.md`](../02-cli/gtc-install.md) for the canonical
install entrypoint and release-cache behavior.

- Treat `gtc install` as the default stable install path. It installs the latest
  stable toolchain and, when the release manifest contains them, caches stable
  release packs and components as well as binaries.
- Use `gtc install --release <version> --channel <channel>` when a specific
  release must be installed. Supported channels are `stable`, `dev`, and `rnd`
  (`rnd` means research and development).
- Remember the launcher-to-channel mapping: `gtc` uses `stable`, `gtc-dev` uses
  `dev`, and `gtc-rnd` uses `rnd`.
- `gtc wizard` and `gtc setup` warn when the installed release context is not
  current for the launcher's channel. Use `--strict-release-context` for
  automation that must fail on mismatch, or `--ignore-release-context` when the
  caller intentionally wants to skip the check.
- When adding an existing OCI-hosted pack or component to docs, bundles, or
  examples, prefer the channel tag that matches the intended release lane. For
  production-ready repo-local guidance, use `oci://ghcr.io/...:stable`.
- Do not use `oci://ghcr.io/...:latest` for pack or component references in
  canonical guidance unless the point is explicitly to test an unverified moving
  target. `latest` can resolve to artifacts that have not gone through the stable
  release path.

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
