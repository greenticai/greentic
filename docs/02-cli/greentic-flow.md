Status: Operational guidance in this repo
Scope: How contributors in this repo should use `greentic-flow component-schema`
Implementation owner: `greentic-flow` for the deeper tool behavior; this doc is repo-local usage guidance

# `greentic-flow component-schema`

Use `greentic-flow component-schema` when you need the current contract for a
component before wiring it into a flow.

This is operational guidance in this repo, not a claim that `greentic-flow`
implementation details live here.

## Why This Matters

Agents and contributors frequently guess the shape of component inputs and
outputs from:

- old examples
- remembered YAML fragments
- adjacent repos
- screenshots or docs that predate current schema changes

That is exactly the kind of drift this command helps prevent.

## Basic Command

The repo-local guidance is:

```bash
greentic-flow component-schema <component>.wasm
```

Use this before you wire a component into a flow if the step shape, input
contract, or output contract is not already proven elsewhere.

## What It Returns

From current repo context, the safe claim is:

- it returns schema information for the component contract
- that schema is more trustworthy than prose examples when the two disagree

This repo does not currently prove the exact full output format of
`component-schema` in code or checked-in generated docs yet, so do not overstate
the exact JSON structure unless you verify it from the current tool output.

## How To Use It In Practice

Treat the command as a pre-wiring validation step:

1. identify the component you want to call
2. inspect its current schema
3. map the flow step inputs to what the component actually expects
4. map the component outputs back into the flow context intentionally

If you skip step 2, you are much more likely to invent field names or assume the
wrong payload shape.

## What Should An Agent Do Before Wiring A Component?

Before adding or changing a flow step, verify:

1. the component’s current expected inputs
2. the component’s current produced outputs
3. whether config-like values belong in config rather than payload
4. whether error outputs need explicit mapping

## Components Worth Checking First

Current repo-local catalogs make these especially relevant examples to inspect
when they are part of your workflow:

- `component-templates`
- `component-script-rhai`
- `component-flow2flow`
- `component-adaptive-card`
- `component-llm-openai`
- `component-oauth-card`

These are not the only valid components. They are just the most visible ones in
the current repo-local docs.

## Safe Rule

If prose docs and current `component-schema` output disagree:

- trust the current schema output
- fix the prose
