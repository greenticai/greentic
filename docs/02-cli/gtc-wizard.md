Status: Canonical in this repo
Scope: Repo-owned `gtc wizard` command surface and current routing behavior
Implementation owner: `gtc` for routing and extension-launcher behavior; downstream wizard tooling for deeper generation semantics

# `gtc wizard`

`gtc wizard` is a canonical entrypoint in this repo, but it has an important
ownership boundary:

- normal wizard usage is routed to `greentic-dev wizard ...`
- extension-launcher usage is owned locally by `gtc`

Do not treat this repo as the owner of every downstream wizard question,
generated artifact, or answer key unless the current schema proves it.

## What This Command Is

Use `gtc wizard` when you want to begin a structured, reproducible creation flow
instead of building everything by hand.

Current repo-local sources show two wizard modes:

- **legacy passthrough mode**
- **extension-launcher mode**

## Legacy Passthrough Mode

In the normal case, `gtc wizard ...` routes to:

```text
greentic-dev wizard ...
```

`gtc` adds `--locale` for the downstream wizard if one was not already present,
then forwards the remaining wizard arguments.

That means `gtc` owns the entrypoint and routing, but the deeper wizard schema
and generation behavior are downstream.

## Extension-Launcher Mode

If you pass extension flags such as `--extensions`, `gtc` switches to a local
extension-launcher flow instead of plain passthrough.

Current extension-launcher flags include:

- `--extensions <id[,id...]>`
- `--extension-registry <path>`
- `--emit-extension-handoff <path>`

In that mode, `gtc`:

- resolves extension descriptors
- launches the configured extension wizard binary or binaries
- writes a normalized launcher handoff document

This is repo-owned behavior in the current implementation.

## `gtc wizard --schema`

Current tests confirm that:

```bash
gtc wizard --schema
```

is passed through to the downstream dev wizard and emits the downstream schema
JSON to stdout.

This is the correct first step when you need to know what answer structure the
current wizard expects.

## `gtc wizard --answers <answers.json>`

Current README and test coverage support the repo-local pattern:

```bash
gtc wizard --answers <answers.json>
```

In the current implementation, `gtc` forwards that request to the downstream
wizard owner rather than validating the answer structure itself.

That means the safe rule is:

- inspect the current wizard schema first
- make your answer file match the schema
- do not invent keys from memory or from older demo repos

## `gtc wizard --emit-answers <path>`

Current integration tests also confirm a downstream passthrough pattern for:

```bash
gtc wizard --emit-answers <path>
```

At minimum, the generated answer document currently appears to include:

- `schema_version`
- `answers`
- `events`

Treat that as observed current behavior, not a permanent schema guarantee unless
the current emitted schema confirms it.

## Current Ownership Boundary

This repo currently owns:

- the `gtc wizard` entrypoint
- locale forwarding into the downstream wizard
- extension-launcher mode
- extension registry resolution
- extension launcher handoff generation

This repo does **not** currently own:

- every downstream wizard question
- every create/update mode semantic
- every answer key name
- the full generation behavior behind normal non-extension wizard flows

## Current Practical Flow

The current repo-local flow remains:

1. `gtc wizard`
2. `gtc setup`
3. `gtc start`

Use `wizard` to create or update the structured inputs, then move into setup and
start once the bundle or generated artifacts exist.

## Current Limitations And Safe Wording

Be careful with claims like:

- “wizard definitely supports X create/update mode”
- “these exact answer keys always exist”
- “all bundle/pack/component mutations are owned by `gtc`”

Those claims may belong to downstream tooling rather than this repo. If the
current schema or code does not prove them, describe them as downstream wizard
behavior and re-check before documenting specifics.

## What Should An Agent Verify First?

Before editing wizard docs or examples, verify:

1. whether the call is normal passthrough mode or extension-launcher mode
2. whether the current downstream schema is available via `gtc wizard --schema`
3. whether the example depends on `--answers` or `--emit-answers`
4. whether the behavior is repo-owned here or only routed here
