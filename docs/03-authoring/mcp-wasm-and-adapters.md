Status: Operational guidance in this repo
Scope: How contributors in this repo should reason about MCP WASM and adapter composition
Implementation owner: Mixed ownership across `gtc`, `greentic-mcp`, component repos, and adjacent runtime tooling

# MCP WASM and Adapters

Use this guide when you need to document or author Greentic flows that rely on
MCP-style capabilities.

This is not a generic MCP reference. It explains the safer operating model for
contributors in this repo.

## What An MCP WASM Means Here

In Greentic context, an "MCP WASM" should be read as a component-oriented,
WASM-executable unit that helps expose or consume MCP-style capability inside a
Greentic flow or pack.

The important framing is:

- MCP gives you a protocol and transport model
- Greentic needs a flow-compatible component contract
- the wrapper or adapter layer bridges those two worlds

Do not assume that a raw MCP server is already a drop-in Greentic component.

## Composition Pattern

The safe mental model is:

1. an MCP-facing capability exists somewhere
2. an adapter or wrapper translates it into the contract Greentic expects
3. the resulting component is wired into a flow like any other Greentic step

That means the adapter boundary is where you should expect concerns such as:

- transport selection
- request/response normalization
- config and secret injection
- mapping external MCP semantics into flow-friendly payloads

## What Belongs To MCP Versus The Greentic Wrapper

Usually treat these as MCP-side concerns:

- protocol-level expectations
- transport specifics such as stdio or SSE
- server/tool capability semantics

Usually treat these as Greentic-wrapper concerns:

- step payload shape expected by the flow
- config conventions used by the bundle or runtime
- secret resolution at setup/runtime boundaries
- error handling and mapping needed for flow execution

## What Agents Must Not Assume

Do not assume:

- that generic MCP examples imply the correct Greentic flow shape
- that raw MCP request/response payloads are already the right step contract
- that config, secrets, or OAuth belong inside the same layer by default
- that a demo in another repo proves the current authoring contract here

When in doubt, verify the current component schema and the current bundle/flow
structure before writing docs or examples.

## Practical Composition Guidance

If you are introducing an MCP-backed capability into a flow:

1. identify the user-facing or flow-facing contract you actually need
2. verify whether a Greentic wrapper component already exists
3. put transport and protocol specifics behind the adapter layer
4. keep the flow step focused on business payloads and mapping, not raw protocol chatter

## Current Repo Context

This repo references MCP as part of the broader Greentic platform story:

- README describes MCP as a lighter-weight integration path
- repo catalogs classify `greentic-mcp` as the likely owner for deeper MCP internals

What this repo does not currently prove on its own is the full exact adapter
contract for every MCP composition path. Keep docs conservative and
schema-first.

## When To Use This Pattern

Use MCP-plus-adapter composition when:

- you want to expose an external MCP capability through a Greentic flow
- the capability should look like a normal flow step after adaptation
- the flow should stay deterministic at the orchestration level even if the underlying integration is more dynamic

## When To Use Something Else

Use a regular Greentic component without MCP layering when:

- the capability is already natively modeled as a Greentic component
- you do not need MCP protocol compatibility
- adding an adapter would only add complexity without improving reuse

## Common Failure Modes

- pushing raw MCP protocol details directly into flow YAML
- treating adapter configuration as ad hoc payload data
- assuming generic MCP auth examples cover Greentic secrets and OAuth boundaries
- copying examples from another repo without re-checking current schema and pack structure
