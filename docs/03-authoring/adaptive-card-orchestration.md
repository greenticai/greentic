Status: Operational guidance in this repo
Scope: Recommended adaptive-card architecture and flow patterns for contributors in this repo
Implementation owner: Mixed ownership across component repos, messaging/runtime repos, and flow authoring in Greentic

# Adaptive Card Orchestration

Use this guide when a worker needs card-based interaction and you need to
decide what belongs in the component, what belongs in the flow, and how to keep
the orchestration maintainable.

This repo does not own every adaptive-card implementation detail, so this guide
stays pattern-focused and conservative.

## Recommended Architecture

Treat adaptive-card handling as a composition of concerns:

1. a flow decides when a card should be shown
2. a rendering-oriented component produces the card payload
3. the channel or messaging layer delivers the card or a channel-specific equivalent
4. the next flow step handles the reply or callback

That separation keeps orchestration readable and keeps rendering logic out of
the main flow where possible.

## Where Adaptive Cards Fit

Adaptive cards fit best when:

- the user must choose from structured options
- the workflow needs a guided input step
- the UI should be richer than plain text but still deterministic

They do not replace the whole flow. They are one interaction surface inside the
flow.

## Keep This Split

### Put This In The Component

Use a component for:

- rendering card JSON or channel-specific card payloads
- templating reusable card structure
- normalizing card output for multiple channels
- input/output transformation that is tightly coupled to card rendering

### Put This In The Flow

Use the flow for:

- deciding when the card should appear
- branching after the user responds
- coordinating state updates
- handling retries, timeouts, or fallback paths
- choosing what happens when a richer card channel is unavailable

If the logic is about process control, it belongs in the flow, not in the card
component.

## State Handling

Adaptive-card flows usually need state when:

- the card represents one step in a longer conversation
- the reply must be matched to a previous prompt
- the workflow must resume after an approval or selection

A safe pattern is:

1. store the minimum context needed before rendering the card
2. send the card
3. wait for a reply or callback
4. map the reply back into the next deterministic flow step

Do not hide important workflow state only inside the card payload if the flow
will need it later.

## Reply And Response Handling

Treat replies as structured flow inputs, not as ad hoc UI side effects.

Recommended pattern:

1. render the card from explicit flow state
2. receive the response through the messaging/channel path
3. normalize the response into a flow-friendly shape
4. branch in the flow based on that normalized result

This matters because channel behavior can differ. The flow should operate on
normalized semantics, not on raw per-channel response quirks.

## Validation And Templating Boundaries

Keep these boundaries clear:

- card layout and reusable structure -> component/template concern
- business validation and allowed transitions -> flow concern
- channel fallback or downscaling -> adapter/runtime concern unless the flow must explicitly react

Do not bury business-policy validation inside a rendering template.

## Common Orchestration Patterns

### Guided Choice Pattern

Use when the user must select one approved option from a known set.

Good fit for:

- approvals
- routing choices
- environment or target selection

### Confirm Or Cancel Pattern

Use when a workflow reaches a checkpoint and needs explicit human confirmation
before continuing.

Good fit for:

- privileged actions
- irreversible operations
- escalation checkpoints

### Collect Structured Input Pattern

Use when the workflow needs several related fields but still wants a guided,
card-based UX instead of free-form text.

Good fit for:

- ticket enrichment
- guided intake
- change-request capture

## Anti-Patterns

- putting the whole workflow decision tree inside one card component
- encoding critical workflow state only in the front-end payload
- branching directly on raw channel-specific response formats
- mixing business-policy validation with card rendering logic
- assuming every channel supports the same rich-card feature set

## When To Use A Card At All

Use a card when structured user interaction improves determinism or clarity.

Use plain text or a simpler interaction when:

- a normal prompt is enough
- the channel support is weak or uncertain
- the extra rendering layer would not improve the workflow outcome

## What Should An Agent Verify First?

Before adding adaptive-card orchestration, verify:

1. whether the interaction really needs a card instead of plain text
2. which part is rendering logic and which part is process logic
3. whether the flow needs stored state before and after the card interaction
4. how replies will be normalized across channels
5. whether current repo docs or component schemas prove the contract you are documenting
