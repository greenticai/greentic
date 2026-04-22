Status: Operational guidance in this repo
Scope: How contributors in this repo should choose and compose extension packs
Implementation owner: Mixed ownership across `gtc`, extension repos, and runtime/tooling repos

# Extension Pack Patterns

Use this guide when you need to decide which extension pack to add, how it
should relate to the application pack, and when `gtc`'s extension handoff
paths are the right tool.

This doc is practical on purpose. It is not a full reference for every
extension implementation in the wider Greentic ecosystem.

## What An Extension Pack Is For

An extension pack adds cross-cutting platform capability around the
business-facing application pack.

In this repo's current model:

- application packs carry the business logic
- extension packs carry channel, runtime, auth, state, or observability support
- bundles assemble both into one runnable system

## The Basic Composition Rule

Start from the application need, then add only the extension capability needed
to make that application runnable in the right environment.

Do not push every cross-cutting concern into the application pack by default.

## Common Extension Types

### Messaging

Use a messaging-oriented extension when the worker needs ongoing interactive
conversation.

Typical signals:

- multi-step chat
- user replies that drive the next step
- channel-aware presentation
- richer UI such as cards or chat surfaces

Current repo context prominently points to:

- WebChat
- Teams
- Slack
- Webex
- WhatsApp
- Telegram

### Events

Use an event-oriented extension when the worker is mainly triggered by external
systems rather than by an interactive user session.

Typical signals:

- webhook intake
- timers
- email or SMS style triggers
- fire-and-forget processing

### State

Use state support when the worker must remember something across steps,
messages, or sessions.

Typical signals:

- conversation continuity
- resumable workflows
- approval processes with later continuation

### Secrets

Use secrets support when the runtime or components need protected credentials
that should not live in normal config or example payloads.

### OAuth

Use OAuth support when the worker must act on behalf of a user or tenant with
delegated identity.

### Telemetry

Use telemetry support when you need runtime visibility, auditability, or
operational traces.

### UI / WebChat

Use UI-oriented extension capability when the worker needs a browser-facing
entrypoint or a richer end-user surface.

## Recommended Defaults In Current Repo Context

These are cautious defaults supported by the current repo context:

- For interactive user journeys, start with a messaging-oriented path.
- For public or browser-facing interaction, treat WebChat as the clearest current default example.
- For trigger-driven workflows, start with events rather than messaging.
- Add state, secrets, OAuth, and telemetry only when the flow actually needs those concerns.

These are local working defaults, not permanent global Greentic law. Re-check
current implementation and adjacent capability docs before turning them into
hard-coded assumptions.

## Need X -> Use Y

- Need a chat-style worker -> use a messaging extension path.
- Need a browser-facing interaction surface -> add WebChat-style UI support.
- Need webhooks, timers, or trigger-first execution -> use an events extension path.
- Need cross-step memory or resumable state -> add state support.
- Need protected credentials -> add secrets support.
- Need delegated login or provider access -> add OAuth support.
- Need runtime observability -> add telemetry support.

## How App Packs And Extension Packs Coexist

Use this layering:

1. keep the business flow in the application pack
2. attach extension packs for cross-cutting capability
3. assemble both in the bundle
4. let setup/start operate on the resulting bundle

That keeps the application pack focused on what the worker does, and the
extension pack focused on what the worker needs around it.

## How To Attach Extension Capability In Practice

There are two patterns that matter in this repo.

### Bundle Composition Pattern

Use normal bundle composition when the required packs are already known and you
are assembling a runnable bundle.

This is the default pattern for stable pack composition.

### Extension Launcher And Handoff Pattern

Use the extension launcher flow when you need `gtc` to route through an
extension registry and produce normalized handoff data.

Current repo-owned behavior proves support for:

- `gtc wizard --extensions ...`
- `--extension-registry <path>`
- `--emit-extension-handoff <path>`
- `gtc setup --extension-setup-handoff <path>`
- `gtc start --extension-start-handoff <path>`

That means this repo is opinionated about the routing and handoff shape, even
though the deeper extension implementation usually lives elsewhere.

## Precedence And Conflict Guidance

Current repo-local code does not prove a rich conflict-resolution model for
overlapping extension packs, so keep the guidance simple:

- do not add two extensions for the same job unless you know why
- avoid overlapping channel ownership without an explicit reason
- treat duplicate cross-cutting capability as a design smell until proven otherwise
- prefer one clear extension path over multiple partially overlapping ones

## When To Choose Extension Launcher Versus Normal Bundle Editing

Use extension launcher and handoff when:

- extension selection comes from a registry
- an extension-specific wizard must run first
- you need normalized setup/start handoff documents

Use normal bundle editing when:

- the packs are already known
- no extension-specific wizard is needed
- you are just composing a concrete runnable bundle

## What Should An Agent Verify First?

Before choosing an extension pattern, verify:

1. whether the need is application logic or cross-cutting capability
2. whether an existing extension already covers it
3. whether the flow is interactive, trigger-driven, browser-facing, or delegated-auth-heavy
4. whether the current path should use plain bundle composition or the extension launcher and handoff flow
5. whether a default recommendation here is still supported by current repo docs and code

## Common Mistakes

- putting messaging/provider behavior inside the application pack by default
- adding OAuth when simple config or secrets would have been enough
- adding multiple overlapping extensions without a clear ownership split
- assuming extension launcher mode is the same thing as normal wizard passthrough
- documenting extension precedence rules more strongly than current code proves
