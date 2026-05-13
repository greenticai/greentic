Status: Canonical in this repo
Scope: Repo-owned `gtc setup` command surface and current ownership boundaries
Implementation owner: `gtc` for the command entrypoint; `greentic-setup` for the deeper setup behavior after handoff

# `gtc setup`

`gtc setup` is the canonical setup entrypoint in this repo.

In the current implementation, `gtc` owns the command surface and routing, but
the underlying setup work is handed off to `greentic-setup` unless you are using
the explicit extension handoff path.

Before handoff, `gtc setup` checks whether the installed release context is
current for this launcher's channel (`gtc` -> `stable`, `gtc-dev` -> `dev`,
`gtc-rnd` -> `rnd`). A mismatch prints a warning and explains that the user
should run the matching install command, such as `gtc install`,
`gtc-dev install`, or `gtc-rnd install`.

Use `--strict-release-context` to turn that warning into an error. Use
`--ignore-release-context` to skip the check. Both flags are owned by `gtc` and
are stripped before setup arguments are forwarded to `greentic-setup`.

## What This Command Is

Use `gtc setup` when you need to prepare a bundle for execution.

In practical terms, `setup` is the phase between authoring and running:

- authoring produces or updates the bundle and its related artifacts
- setup prepares environment-specific or deployment-specific inputs
- start launches the prepared bundle locally or via a deployer-backed path

## When Should I Use It?

Use `gtc setup` when:

- you already have a bundle or bundle directory
- you need to apply setup answers or setup-time configuration
- you are moving from authoring into a runnable environment

If you are still creating components, flows, packs, or the bundle itself, you
are still in authoring rather than setup.

## What Should I Use Instead If Not This?

- Need to build or modify the bundle structure first? Use the authoring path,
  not `setup`.
- Need to actually run the prepared bundle? Use [`gtc start`](./gtc-start.md).
- Need extension-specific setup handoff? Use
  `gtc setup --extension-setup-handoff <path>`.

## Current Ownership Boundary

This repo currently owns:

- the `gtc setup` command entrypoint
- passthrough routing to `greentic-setup`
- the optional `--extension-setup-handoff <path>` handoff path

This repo does **not** currently own the full semantics of every setup flag.
For normal setup usage, `gtc` forwards the trailing arguments to
`greentic-setup`.

`gtc` does resolve and validate `--answers` before forwarding. `--answers`
accepts JSON object documents from plain local paths, `file://...`,
`http://...`, `https://...`, `oci://...`, `store://...`, and `repo://...`.
Distributor-backed schemes are resolved through `greentic-distributor-client`
and forwarded to `greentic-setup` as a temporary local answers file.

Other syntax such as `--no-ui` may be valid in current workflows, but the
deeper flag behavior is owned by the downstream setup tool, not re-parsed by
`gtc` itself.

## Basic Usage

Current repo-local docs and README use setup in patterns such as:

```bash
gtc setup ./some-bundle --answers <setup-answers.json>
```

The README also shows:

```bash
gtc setup --no-ui ./cloud-deploy-demo-bundle --answers <setup-answers.json>
```

Those forms are passed through to `greentic-setup` after answer source
resolution succeeds.

## Does `./<name>.gtbundle` Work?

This repo’s current README and setup-facing examples mostly show local bundle
directories such as `./helpdesk-itsm-demo-bundle`.

Because `gtc setup` is currently a passthrough route, the exact supported input
forms depend on `greentic-setup`. This repo does not currently add extra local
parsing for `./<name>.gtbundle` versus `./<name>-bundle` at the `gtc` layer.

So the safe repo-local guidance is:

- use the bundle form that the current downstream setup tool accepts
- prefer the directory-style examples already shown in this repo until schema or
  canonical setup docs say otherwise

## With UI vs Without UI

Current repo-local material shows both interactive and non-interactive setup
patterns:

- interactive setup through normal `gtc setup ...`
- non-interactive setup through `gtc setup --no-ui ...`

At the `gtc` layer, this is currently passthrough behavior. `gtc` does not
implement its own UI mode logic here; it forwards the arguments to
`greentic-setup`.

## What Setup Is Responsible For

From the current repo-local framing, setup is responsible for preparing a bundle
for execution, including setup-time inputs that should exist before `start`.

Treat setup as the place for:

- applying setup answers
- environment-specific preparation
- preparing data or configuration needed before execution starts

Do **not** treat setup as the phase that owns actual execution. That belongs to
`start`.

## What Setup Outputs Or Persists

This repo does not currently re-document all downstream persisted outputs at the
`gtc` layer.

The safe repo-local claim is:

- setup may prepare bundle-local state or deployment-local inputs needed by
  later execution
- `gtc` itself is primarily responsible for routing to the setup owner

Where exact persisted outputs matter, verify the current downstream setup tool
or its schema before documenting more detail.

## Extension Setup Handoff

This repo does own one setup-specific structured handoff path:

```bash
gtc setup --extension-setup-handoff <path>
```

In that mode, `gtc`:

- loads a normalized setup handoff document
- builds setup args from that handoff
- forwards the resulting setup request to `greentic-setup`

This is still a handoff mechanism, not full extension-specific setup logic owned
by `gtc`.

## What Should An Agent Verify First?

Before editing setup docs or examples, verify:

1. whether the behavior is owned here or by `greentic-setup`
2. whether the example uses passthrough flags such as `--answers` or `--no-ui`
3. whether the bundle form is documented in current repo-local canonical docs
4. whether a structured extension setup handoff is actually being used
