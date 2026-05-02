Status: Canonical in this repo
Scope: Repo-owned `gtc install` command surface and release artifact cache behavior
Implementation owner: `gtc` for install routing, release manifest resolution, and release cache indexing

# `gtc install`

`gtc install` is the canonical install entrypoint in this repo.

Use it before normal `gtc wizard`, `gtc setup`, or `gtc start` work so the
companion binaries and release artifacts expected by the current toolchain are
available locally.

## Default Stable Install

The default command is:

```bash
gtc install
```

In the current implementation, that means the stable channel unless the command
is invoked through a channel-named launcher:

- `gtc` -> `stable`
- `gtc-dev` -> `dev`
- `gtc-rnd` -> `rnd`

For normal user and agent guidance, treat plain `gtc install` as:

- install the latest stable Greentic toolchain
- install the required companion binaries
- cache the stable release packs declared by the release manifest
- cache the stable release components declared by the release manifest

The release cache matters because stable packs and components can then resolve
through the local distributor cache instead of depending on whatever mutable
`latest` tag happens to contain.

## Release And Channel Install

Use this form when you need a specific release/channel pair:

```bash
gtc install --release <version> --channel <channel>
```

Supported channels are:

- `stable`
- `dev`
- `rnd`

`rnd` means research and development.

If `--release` is provided without `--channel`, the channel defaults to the
current launcher's channel.

## Release Context Checks

Before `gtc wizard` and `gtc setup` hand off to downstream tooling, `gtc` checks
whether the installed toolchain release context is current for the launcher's
channel:

- `gtc wizard` and `gtc setup` check the latest `stable` release.
- `gtc-dev wizard` and `gtc-dev setup` check the latest `dev` release.
- `gtc-rnd wizard` and `gtc-rnd setup` check the latest `rnd` release.

When the installed release context is missing, on a different channel, or behind
the latest release for that channel, the default behavior is to print a warning
and continue. The warning tells the user to run the matching launcher install
command, such as:

```bash
gtc install
gtc-dev install
gtc-rnd install
```

Use strict mode when automation should fail instead of continuing with a stale
or mismatched release context:

```bash
gtc wizard --strict-release-context
gtc setup --strict-release-context
```

Use ignore mode when the caller intentionally wants to skip the release-context
check:

```bash
gtc wizard --ignore-release-context
gtc setup --ignore-release-context
```

These flags are owned by `gtc`; they are stripped before arguments are forwarded
to downstream wizard or setup tools.

## Phase Selectors

The install command can restrict work to one or more phases:

```bash
gtc install --install-binaries-only
gtc install --install-packs-only
gtc install --install-components-only
```

With no phase selector, `gtc install` runs the full default install path for
binaries, release packs, and release components.

## OCI References In Docs And Examples

When adding an existing OCI-hosted pack or component to repo-local docs, bundles,
or examples, prefer the release channel tag that matches the intended lane.

For stable guidance, use:

```text
oci://ghcr.io/<owner-or-org>/<artifact>:stable
```

Do not use:

```text
oci://ghcr.io/<owner-or-org>/<artifact>:latest
```

for canonical pack or component guidance unless the example is explicitly about
testing an unverified moving target. `latest` can point at artifacts that have
not gone through the stable release path.

## What Should An Agent Verify First?

Before editing install docs, bundle examples, or component/pack references,
verify:

1. whether the artifact is meant to track `stable`, `dev`, or `rnd`
2. whether a checked-in example still uses `:latest` where `:stable` is intended
3. whether the release manifest and cache behavior are relevant to the change
4. whether docs that mention installing only binaries now need to mention cached
   packs and components as well
