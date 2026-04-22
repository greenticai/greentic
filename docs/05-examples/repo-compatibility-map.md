Status: Operational guidance in this repo
Scope: Conservative trust map for adjacent Greentic repos
Implementation owner: Mixed ownership across Greentic repos

# Repo Compatibility Map

This map is intentionally conservative.

Use it to decide which repos are safe to consult, what they are useful for, and
what must always be re-checked against this repo’s current docs, schemas, and
code.

| Repo | Canonical for what | Example-only for what | May lag current APIs? | Safe for inspiration? | Must re-check against this repo? |
| --- | --- | --- | --- | --- | --- |
| `greentic` | `gtc` behavior, repo-local workflows, local docs, local packaging and release flow | No | No for repo-owned behavior | Yes | N/A |
| `greentic-demo` | Not canonical here | End-to-end scenarios, sample compositions, sample answer flows | Yes | Yes | Yes |
| `greentic-dev` | Likely canonical for its own deeper dev-tool behavior, not for `gtc` docs here | Dev-tool usage patterns | Yes from this repo’s point of view | Yes | Yes |
| `greentic-flow` | Likely canonical for flow-tool implementation details, not for local repo wording | Flow authoring patterns | Yes from this repo’s point of view | Yes | Yes |
| `greentic-pack` | Likely canonical for pack-tool implementation details, not for local repo wording | Pack authoring patterns | Yes from this repo’s point of view | Yes | Yes |
| `greentic-operator` | Likely canonical for operator/runtime internals, not for `gtc` command semantics here | Runtime behavior context | Yes from this repo’s point of view | Yes | Yes |
| `greentic-mcp` | Likely canonical for MCP integration internals, not for local operational guidance wording | MCP capability examples | Yes from this repo’s point of view | Yes | Yes |
| `greentic-oauth` | Likely canonical for OAuth broker internals, not for local usage wording | OAuth capability examples | Yes from this repo’s point of view | Yes | Yes |
| `greentic-types` | Shared types and schemas across repos | Not primarily example material | Less likely, but still re-check local usage | Yes | Yes |
| `greentic-distributor-client` | Client behavior for distributor access patterns | Usage context | Yes from this repo’s point of view | Yes | Yes |
| `component-*` repos | Component-specific implementation details in their own repos | Concrete component ideas and capability patterns | Yes from this repo’s point of view | Yes | Yes |

## How To Use This Map

Follow this rule:

- use this repo first for repo-owned behavior
- use adjacent repos to understand their own implementation or find inspiration
- always re-check the final answer against this repo before changing docs or code here

## Strong Warning

The fact that another Greentic repo is authoritative for its own implementation
does not make it authoritative for:

- current `gtc` syntax in this repo
- repo-local docs in this repo
- repo-local setup/start expectations
- repo-local terminology in this repo
