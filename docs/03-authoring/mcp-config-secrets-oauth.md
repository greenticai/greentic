Status: Operational guidance in this repo
Scope: How this repo should frame config, secrets, and OAuth around MCP-style capabilities
Implementation owner: Mixed ownership across `gtc`, `greentic-secrets`, `greentic-oauth`, `greentic-mcp`, and adjacent runtime tooling

# MCP Config, Secrets, and OAuth

Use this guide when a bundle or flow uses MCP-oriented capability and you need
to decide where config, secrets, and OAuth responsibilities belong.

This repo does not own every implementation detail here. It does own the local
guidance for how contributors should reason about the boundaries.

## First Principle

Do not collapse config, secrets, and OAuth into one generic "settings" bucket.

Treat them as different responsibilities:

- config tells the system what shape of behavior to use
- secrets provide protected credentials or tokens
- OAuth manages delegated identity and token lifecycle

## Config Handling

### What Belongs In Config

Config is the right place for:

- non-secret endpoints
- bundle or runtime mode selection
- adapter behavior toggles
- file paths or references passed through `gtc start --config ...`
- registry-base env vars and repo-owned routing inputs documented in [`docs/config.md`](../config.md)

### What Does Not Belong In Config

Do not put long-lived credentials, API keys, or refresh tokens into normal
config files just because they are easy to pass around.

### Repo-Owned Surfaces To Check

Check:

- [`docs/config.md`](../config.md)
- `src/config.rs`
- `src/start_stop_parsing.rs`
- `src/bin/gtc/process.rs`

## Secrets Handling

### What This Repo Proves

Current repo-local evidence shows that `gtc` already treats secret-like inputs
carefully in several places:

- prompt helpers use zeroizing strings
- cloud deploy flows distinguish secret and optional-secret prompt fields
- fingerprinting tests avoid leaking `.dev.secrets.env` contents into hashes

That is enough to document a local rule:

- secrets should stay out of normal payload examples and out of casual checked-in config

### Authoring Responsibility

At authoring time:

- identify which values are secrets
- keep them out of example payloads and prose where possible
- document the boundary, not the literal secret value

At setup/runtime time:

- let the appropriate secret backend or prompt flow provide the value
- avoid rewriting secret-handling behavior in flow docs unless current code proves it

## OAuth Handling

### What OAuth Is For

OAuth belongs to delegated identity cases:

- a user logs in to an external provider
- the system receives or refreshes tokens on that user's or tenant's behalf
- runtime steps later use that delegated access

### What This Repo Can Say Safely

This repo can safely say:

- OAuth is not just another config field
- OAuth often pairs with card/UI components and setup/runtime flows
- deeper broker/token-storage behavior is likely owned outside this repo

The repo catalog and existing flow guidance make `greentic-oauth` and
`component-oauth-card` relevant adjacent references, but not local
implementation truth.

## Runtime Expectations

When MCP-oriented capability needs config, secrets, or OAuth:

- config should describe the integration shape
- secrets should provide protected values
- OAuth should handle delegated access flows
- the adapter layer should make those inputs usable by the Greentic component contract

If those boundaries are blurred, contributors tend to build fragile examples and
invent unsupported wiring.

## Authoring Versus Setup Responsibility

At authoring time:

- define the contract clearly
- decide which inputs are config, secrets, and OAuth-driven
- keep flow payloads focused on business data

At setup time:

- gather or point to environment-specific values
- apply setup answers or runtime config that the current toolchain expects

At runtime:

- rely on the resolved config and secret/OAuth inputs already prepared for the component path

## Common Failure Modes

- putting secrets into checked-in config or example payloads
- treating OAuth tokens as static config
- documenting a generic MCP auth flow as if it were the Greentic contract
- mixing adapter config with business payload fields
- assuming setup automatically creates OAuth or secret backends unless current code/docs say so

## What To Verify First

1. whether the value is really config, really a secret, or really delegated auth
2. whether the boundary is owned in this repo or by an adjacent capability repo
3. whether current schema/docs prove the input shape
4. whether the example belongs in prose docs or should instead point to generated schema docs later
