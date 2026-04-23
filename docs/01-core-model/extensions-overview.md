Status: Operational guidance in this repo
Scope: How this repo frames extension families and their role in bundle composition
Implementation owner: Mixed ownership across Greentic repos

# Extensions Overview

This doc explains the extension families that appear in current repo-local docs
and adjacent Greentic catalogs.

Use it as operational guidance for this repo, not as proof that every extension
implementation lives here.

## What An Extension Is

An extension adds supporting capability around the application logic of a
digital worker.

In current repo-local framing, extensions are usually attached through extension
packs or extension-related runtime/config composition rather than by putting all
cross-cutting behavior directly inside the application pack.

## Common Extension Families

### Messaging

Messaging extensions connect the system to human-facing channels and messaging
providers.

Current repo-local sources point to capabilities such as:

- Teams
- Slack
- Webex
- WhatsApp
- Telegram
- WebChat

Use messaging-oriented extensions when the digital worker needs interactive,
session-based, human-facing communication.

### Events

Event-oriented extensions handle non-conversational triggers such as:

- webhooks
- timers
- email
- SMS
- other trigger-driven input paths

Use event-oriented extensions when the workflow is primarily initiated by a
system trigger rather than an interactive conversation.

### State

State-related extensions support persistence between steps or across sessions.

Use them when flows need:

- session continuity
- saved workflow state
- cross-step data persistence

### Secrets

Secrets-related extensions provide secure access to credentials, API keys, and
other protected values.

Use them when the runtime or components need secret resolution that should stay
outside normal application-pack logic.

### OAuth

OAuth-related extensions provide identity and delegated-access behavior, such as
login flows or token brokerage patterns.

Use them when the digital worker must access third-party systems on behalf of a
user or tenant.

### Observability / Telemetry

Telemetry-related extensions support logs, traces, metrics, auditability, and
runtime visibility.

Use them when you need:

- operational insight
- compliance evidence
- traceability across execution paths

### Static / Public UI

Current repo-local docs also point to web-facing and richer UI experiences such
as WebChat and card rendering.

Use these when the digital worker needs:

- browser-based interaction
- richer channel UX
- public or semi-public user entrypoints

## How Extensions Relate To Bundles And App Packs

Think of the relationship this way:

- the application pack holds the business-facing logic
- the extension pack supplies cross-cutting platform capability
- the bundle assembles both into a runnable system

That means a bundle can contain:

- one or more application packs
- one or more extension packs
- the configuration needed to connect them

## Recommended Defaults From Current Repo Context

Current repo-local material supports these cautious defaults:

- Prefer a messaging-oriented extension path when the user journey is
  conversational and session-based.
- Prefer an event-oriented extension path when the workflow is trigger-driven
  and mostly stateless.
- Treat WebChat as a strong current example of a human-facing interactive
  surface because it appears prominently in the README and current assets.

This is still operational guidance, not a permanent global rule. Re-check the
current docs and implementation before making a hard default assumption in code.

## Need X -> Use Y

- Need an interactive chat-style worker -> start with **messaging** extensions.
- Need webhook or timer triggers -> start with **events** extensions.
- Need continuity across steps or sessions -> add **state** support.
- Need protected credentials or provider keys -> add **secrets** support.
- Need third-party delegated identity -> add **OAuth** support.
- Need logs, traces, or runtime visibility -> add **telemetry** support.
- Need browser-facing or richer UI experiences -> add **UI / WebChat-style** support.

For the more practical bundle-composition and handoff patterns, continue with
[`docs/03-authoring/extension-pack-patterns.md`](../03-authoring/extension-pack-patterns.md).

## What Should An Agent Verify First?

Before choosing an extension path, verify:

1. whether the capability is application logic or cross-cutting support
2. whether an existing extension pack already covers the need
3. whether the current bundle already includes overlapping extension capability
4. whether the repo is documenting local behavior or only operational guidance for a capability owned elsewhere
