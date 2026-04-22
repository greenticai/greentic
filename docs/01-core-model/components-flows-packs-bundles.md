Status: Canonical in this repo
Scope: Local framing of the Greentic composition model
Implementation owner: Mixed ownership across Greentic repos; this doc is canonical for how this repo explains the model

# Components, Flows, Packs, And Bundles

This doc explains how the main Greentic building blocks hang together for work
in this repository.

## The Short Version

Greentic systems are assembled in layers:

```text
Component -> Flow -> Pack -> Bundle -> Setup -> Start
```

- A **component** does one unit of work.
- A **flow** decides how units of work are connected.
- A **pack** bundles the runnable building blocks for distribution.
- A **bundle** selects and configures packs for a concrete runnable system.

## Component

A component is a self-describing executable unit, typically a Wasm component in
the Greentic model.

Use a component when you need to define or reuse one concrete piece of logic,
for example:

- call an API
- render a card
- transform data
- invoke an LLM with explicit permissions
- template a message

In practice, components should stay narrow and composable. If you are trying to
describe sequencing, branching, retries, or orchestration, you probably need a
flow rather than a bigger component.

## Flow

A flow is the orchestration layer. It describes how work moves between steps,
what transitions are allowed, and how the overall task progresses.

Use a flow when you need:

- multiple steps
- branching or deterministic routing
- shared state between steps
- error handling across steps
- a human-in-the-loop checkpoint

If a component is one building block, a flow is the execution plan that wires
those blocks together.

## Pack

A pack is the distribution unit that brings together the artifacts needed to run
part of the system.

From the current repo context and repository catalogs, a pack typically gathers:

- components
- flows
- validation or packaging metadata

Use a pack when you need to package reusable application logic or reusable
platform capability for distribution and validation.

## Bundle

A bundle is the runnable composition layer above packs.

In current repo-local framing, a bundle is where you assemble the packs and
runtime-facing configuration needed for a concrete digital worker or deployment
target.

A bundle can:

- include one or more application packs
- include extension packs
- carry the configuration needed for setup and start flows
- represent the unit that `gtc setup` and `gtc start` act on

## Application Packs vs Extension Packs

Use this distinction consistently:

- **Application pack**
  Contains the business-facing logic of the digital worker: components, flows,
  and related application behavior.
- **Extension pack**
  Adds cross-cutting capability around the application logic, such as messaging,
  state, secrets, telemetry, OAuth, or deployment-related behavior.

Application packs describe what the digital worker does.
Extension packs describe what supporting capability the digital worker needs.

## How They Compose

A common composition path looks like this:

```text
components
  -> wired into flows
  -> packaged into application pack(s)
  -> combined with extension pack(s)
  -> assembled into a bundle
  -> prepared by setup
  -> executed by start
```

Another way to read the same model:

- components are the bricks
- flows are the wiring plan
- packs are the packaged modules
- bundles are the runnable assembly

## Lifecycle From Authoring To Runtime

The repo’s current docs and CLI surface imply this lifecycle:

1. Author or prepare components
2. Define or update flows
3. Package them into packs
4. Assemble the target bundle
5. Run setup for environment-specific preparation
6. Run start for local or deployer-backed execution

This is why `setup` and `start` should not be collapsed into one vague idea of
"deploying everything." They are different phases with different responsibilities.

## When Should I Create What?

Use this decision guide:

- Need one reusable unit of execution? Create a **component**.
- Need to connect multiple steps or decisions? Create a **flow**.
- Need to package reusable application or platform capability? Create a **pack**.
- Need a runnable assembly that setup/start will act on? Create a **bundle**.

## What Should I Use Instead If Not This?

- If you are adding one API call or data transform, do not create a flow first.
  Start with a component.
- If you are trying to describe user journeys, state transitions, or branching,
  do not hide that inside one component. Use a flow.
- If you only need to reference existing logic, do not create a new bundle yet.
  You may only need to update an existing pack or flow.
- If you need cross-cutting runtime capability, do not push that into the
  application pack by default. Consider an extension pack.

## What Should An Agent Verify First?

Before changing any of these layers, verify:

1. whether the current truth is repo-owned here or owned in another Greentic repo
2. whether generated schema docs exist for the structure you are editing
3. whether the current `gtc` workflow in this repo expects a bundle, a pack, or a flow at that step
4. whether an existing pack or bundle already provides what you are about to add
