Status: Canonical in this repo
Scope: Repo-owned `gtc platform` command surface and its ownership boundary
Implementation owner: `gtc` for the routing only; `greentic-deploy-platform` for every verb, flag and default

# `gtc platform`

Installs the Greentic platform itself — the admin, the designer, the
tenant-manager, the edge and the datastore they share.

It is deliberately **not** under `gtc deploy`. That group is about a bundle
reaching an environment that already exists; this is about bringing the
environment into being. The two share a verb in English and nothing in
implementation, and nesting one inside the other made `gtc deploy` mean two
unrelated things.

## What this repo owns

Routing, and nothing more. `gtc` resolves `platform` to the companion binary
`greentic-deploy-platform` and forwards everything after it **verbatim** — no
token added, none removed — so the string the operator typed is the string that
binary parses. Its usage and error lines name `gtc platform ...`, which is the
only form anyone is told to run.

The subcommand declares no flags of its own. Every verb, option and default
lives in `greentic-deploy-platform`; mirroring them here would be a second copy
that goes stale the first time that binary adds a flag. A flag this router has
never heard of is forwarded rather than rejected — whether it means anything is
the companion's question to answer.

## Getting the binary

`greentic-deploy-platform` is **not** part of the toolchain `gtc install`
places, and `gtc doctor` does not probe for it — a machine that never installs
the platform has no reason to carry it, and failing doctor over an absent one
would train operators to ignore the check.

It is released as a single binary from the greentic-deploy-platform repository.
Put it anywhere `gtc` looks — `~/.cargo/bin`, or beside `gtc` itself — and the
subcommand starts working. For a local build, point `gtc` straight at it:

```bash
export GREENTIC_PLATFORM_BIN="/path/to/target/debug/greentic-deploy-platform"
```

When it is missing, `gtc platform` says so by name and repeats both of those
options rather than reporting a generic exec failure.

### The dev channel

`gtc-dev` looks for `greentic-deploy-platform-dev`, not
`greentic-deploy-platform` — the launcher's suffix propagates to every companion,
which is what keeps a dev toolchain from silently reaching for stable tools. The
release attaches both names for that reason; installing only the unsuffixed one
leaves `gtc-dev platform` unable to find anything.

The missing-binary message names whichever one was actually looked for, and says
so explicitly when the two differ.

## Verbs

The pipeline, in order; each stage writes what the next reads.

```
init  →  render  →  [bundle]  →  apply  →  bootstrap  →  verify
```

`apply` is aliased `deploy`. `destroy` uninstalls, and `openapi` writes the
deploy-facing OpenAPI document.

The install target — Kubernetes or AWS ECS — is chosen once, at `init`, and
recorded in `platform.yaml`:

```bash
gtc platform init --target aws-ecs --region ap-southeast-1
gtc platform deploy
```

No later verb takes `--target`; every one of them reads it from the spec. On the
AWS target there is nothing to `render` or `bundle` and both refuse it: it
creates AWS resources directly, and Fargate cannot side-load an image.

Run `gtc platform --help` for the current list — it is answered by the companion
binary, so it cannot drift from what is actually installed.
