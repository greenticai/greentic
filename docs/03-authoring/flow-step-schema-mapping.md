Status: Operational guidance in this repo
Scope: Practical mapping guidance for flow-step authoring
Implementation owner: Mixed ownership; deeper flow semantics likely belong to `greentic-flow` and related tooling

# Flow Step Schema Mapping

This doc explains how to think about flow-step mapping in a schema-first way.

It is intentionally practical and conservative. Use the current tool output and
current implementation as the final authority when exact structure matters.

## The Core Idea

A flow step is where you connect:

- the flow’s current context
- the component’s expected input schema
- the component’s produced output schema

If those three things do not line up, the step is fragile even if the example
looks plausible.

## Input, Output, And Context

Use this mental model:

- **payload**
  The main data you are sending into the component for this step.
- **state**
  Data carried across steps or sessions that the flow needs to preserve.
- **config**
  Stable configuration or environment-specific values that should not be mixed
  into ad hoc payload data unless the component contract truly expects that.

The safest rule is:

- put request-specific data in payload
- keep long-lived workflow memory in state
- keep environment or runtime settings in config

## `in_map`, `out_map`, And `err_map`

Current Greentic terminology frequently uses mapping concepts such as:

- `in_map`
- `out_map`
- `err_map`

Use them as follows:

- **`in_map`**
  Shapes or routes flow context into the component input expected for the step.
- **`out_map`**
  Shapes or routes component success output back into the flow context.
- **`err_map`**
  Shapes or routes component error output into an explicit error-handling path.

The exact syntax should come from the current flow tooling or schema, but the
decision logic above is the stable part.

## When Explicit Mapping Is Needed

Use explicit mapping when:

- the flow context field names do not match the component schema
- only part of the flow context should be passed through
- different components produce different output shapes
- you need a stable shared shape for downstream steps
- the error path needs structured handling rather than generic failure

## When Mapping May Be Omitted

Mapping may be omitted only when the current tool and current schema make the
implicit shape unambiguous and compatible.

Do not omit mapping just because an older example did.

If you are unsure, explicit mapping is safer than relying on hidden alignment.

## How Payload, State, And Config Usually Fit Together

Use this rule of thumb:

- if the value changes per request or per step, it probably belongs in payload
- if the value must survive across steps, it probably belongs in state
- if the value describes environment or stable runtime setup, it probably
  belongs in config

One common failure mode is stuffing config-like values into payload just because
it is easy. That makes flows harder to reason about and easier to break.

## Unifying Outputs Across Different Components

When multiple components feed the same downstream step, normalize their outputs
before the merge point.

That often means:

1. inspect each component schema
2. decide the shared shape the downstream step should see
3. use explicit output mapping so each component converges to that shape

Without that normalization, downstream steps become tightly coupled to whichever
component happened to be wired first.

## Common Pitfalls

- guessing field names from memory instead of inspecting schema
- using demo examples as proof of current mappings
- mixing config values into payload without need
- relying on implicit mapping when component contracts differ
- forgetting to map structured errors separately
- assuming heterogeneous components produce interchangeable outputs

## Practical Validation Steps

Before treating a flow-step mapping as correct, verify:

1. the current component schema
2. the current expected flow-step structure
3. whether success and error outputs need separate handling
4. whether downstream steps expect a normalized shape

## Working Rule

If you have a choice between:

- a convenient undocumented assumption, or
- a mapping supported by current schema/tool output

choose the schema-supported mapping every time.
