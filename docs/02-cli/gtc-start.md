Status: Canonical in this repo
Scope: Repo-owned `gtc start` orchestration and current local-vs-deployer behavior
Implementation owner: `gtc` for bundle resolution, target selection, and orchestration; `greentic-start` and deployer-backed tools after handoff

# `gtc start`

`gtc start` is the canonical start entrypoint in this repo.

Unlike `gtc setup`, `gtc start` is not just a thin passthrough. This repo owns
substantial start-time behavior, including:

- bundle reference resolution
- start-argument parsing
- deployment-target selection
- local runtime versus deployer-backed branching
- extension start handoff integration

## What This Command Is

Use `gtc start` when you want to launch a prepared bundle either:

- in the local runtime path, or
- through a deployer-backed target such as `aws`, `gcp`, or `azure`

## When Should I Use It?

Use `gtc start` after setup or when you already have a bundle workspace and know
the execution path you want.

## What Should I Use Instead If Not This?

- Need to prepare the bundle first? Use [`gtc setup`](./gtc-setup.md).
- Need to build or modify the bundle itself? Stay in the authoring path.
- Need extension-specific normalized handoff? Use
  `gtc start --extension-start-handoff <path>`.

## Basic Usage

The `gtc` CLI describes the main argument as:

```text
BUNDLE_REF
```

Current repo-owned help text defines that as a bundle path or reference such as:

- local path
- `file://`
- `http://` or `https://`
- `oci://`
- `repo://`
- `store://`

A normal local start looks like:

```bash
gtc start ./some-bundle
```

## What `gtc` Owns Before Handoff

Current code paths show that `gtc start` does all of the following before
handing control elsewhere:

1. parse deploy-specific CLI options such as `--target`,
   `--environment`, `--provider-pack`, `--app-pack`, and
   `--deploy-bundle-source`
2. resolve the bundle reference into a concrete bundle directory
3. prepare one warmed deployable bundle from the post-setup workspace
4. parse runtime-facing start arguments
5. prepare admin certificates when admin mode is requested
6. select the effective start target
7. choose between local runtime mode and deployer-backed mode

## Prepared Bundle Contract

`gtc start` always runs a warmed prepared bundle. The prepared bundle is built
from the resolved post-setup workspace before the command branches into local
runtime or deployer-backed behavior.

Target selection changes only where that prepared bundle runs:

- `gtc start <bundle>` runs the prepared bundle locally.
- `gtc start <bundle> --target aws` deploys that prepared bundle to AWS.
- `gtc start <bundle> --target azure` deploys that prepared bundle to Azure.
- `gtc start <bundle> --target gcp` deploys a **published** bundle to Cloud Run
  through env-packs — it does not prepare the bundle locally. See
  [GCP Cloud Run Start](#gcp-cloud-run-start).

Packs own their setup-derived runtime config. If setup generates non-secret
pack-owned files, those files should be written into the bundle workspace under
normal bundle files/assets so they are included in the warmed prepared bundle.
Deployer targets receive the prepared bundle artifact/root/digest; they should
not parse pack-specific runtime config.

## Local Runtime Start

If the selected target is `runtime`, `gtc`:

- resolves the bundle
- builds and warms the prepared bundle
- builds the runtime start request
- prints the selected target and resolved bundle directory
- invokes `greentic-start` with serialized runtime arguments pointing at the
  prepared bundle root

In this mode, `gtc` prints:

- `Selected deployment target: runtime`
- `Deployment mode: local runtime`

This is the clearest path when you want local execution rather than deployment
through a cloud or deployer-backed path.

## GCP Cloud Run Start

`--target gcp` does not use the Terraform/IaC layer that `aws` and `azure` use.
It is an **env-packs** deploy, the same mechanism as a local start: `gtc`
synthesizes a `greentic.env-manifest.v1` binding the environment's deployer slot
to `greentic.deployer.gcp-cloudrun`, then runs `op env up`. No Terraform, no
provider pack, and no `.greentic/deployment-targets.json` entry.

Because Cloud Run pulls the bundle over the network at container boot, the bundle
must already be published as an `oci://` artifact — a local path is unreachable
from it, and `oci://` is the only scheme the env-apply engine will fetch:

```bash
oras push ghcr.io/YOU/YOUR-BUNDLE:v1 ./YOUR-BUNDLE.gtbundle
gtc start oci://ghcr.io/YOU/YOUR-BUNDLE:v1 --target gcp
```

An `oci://` bundle ref is used as its own deploy source, so nothing else is
needed. To deploy a copy published elsewhere, name it explicitly with
`--deploy-bundle-source oci://…`.

Project and region are read from `GOOGLE_CLOUD_PROJECT` / `CLOUDSDK_CORE_PROJECT`
and `GOOGLE_CLOUD_REGION` / `CLOUDSDK_RUN_REGION` / `CLOUDSDK_COMPUTE_REGION`,
falling back to `gcloud config`. Neither has a default — a guessed region decides
latency, egress cost and data residency. Access mode is `public`, which `gtc`
prints before deploying.

Credentials are the ambient Application Default Credentials
(`gcloud auth application-default login`); `gtc` never takes a key file.

Teardown goes back through env-packs — `gtc stop <bundle> --target gcp --destroy`
runs `op env destroy`, which deletes the cloud resources before removing local
state. This destroys the whole environment, not one bundle: Cloud Run has no
per-bundle teardown verb.

> **The operator must carry the Cloud Run deployer.** `gtc op` delegates to
> `greentic-operator`, and the `deploy-gcp-cloudrun` feature is only on the
> develop lane. A stable operator binds the kind, plans a green apply, and then
> refuses at reconcile with `op env up is only supported for the
> greentic.deployer.k8s deployer` — after the store has been mutated. Check with
> `gtc op env doctor <env>`: the kind appears under `unknown_kinds` when the
> binary cannot execute it.

## Deployer-Backed Start

If the selected target is `aws` or `azure`, `gtc` switches into deployer-backed
behavior.

Current supported target values are:

- `runtime`
- `aws`
- `gcp`
- `azure`

In deployer-backed mode, `gtc`:

- resolves the bundle
- builds and warms the prepared bundle
- chooses the target
- passes the prepared bundle root/artifact/digest into deploy/start
  orchestration
- returns once the deploy/start path succeeds or fails

This is why `gtc start` is not just "run the local runtime binary." It owns the
decision about whether execution is local or deployer-backed.

## How Target Selection Works

Current target selection order is:

1. explicit `--target`
2. default target from `.greentic/deployment-targets.json` if one is marked as default
3. explicit target list from `.greentic/deployment-targets.json`
4. fallback to `runtime` if no deploy targets are declared

If multiple deployment targets are available and there is no explicit target:

- interactive TTY mode prompts the user to choose
- non-interactive mode fails and tells the caller to rerun with `--target`

## Setup And Start Relationship

Treat the relationship this way:

- `setup` prepares the bundle and setup-time inputs
- `start` chooses the execution path and launches the prepared system

Do not collapse them into one generic “deploy” idea. The current implementation
keeps them distinct.

## What Start Owns vs What Deployers Own

`gtc start` owns:

- the top-level command surface
- bundle reference resolution
- target selection
- local-vs-deployer branching
- serialization of start arguments for local runtime mode

Deployer-backed tools own:

- the deeper deployment mechanics after `gtc` chooses a non-runtime target
- cloud/provider-specific provisioning and apply/destroy behavior

`greentic-start` owns:

- the local runtime start behavior after `gtc` hands off runtime-mode arguments

## Common Confusion Points

### `gtc start` Is Not Always Local

If the selected target is not `runtime`, `gtc start` runs deployer-backed logic
instead of the plain local runtime path.

### `--bundle` Is Not The Main Interface

Current parser behavior explicitly rejects passing `--bundle` as the main start
selector. The bundle reference should be passed as the main argument to
`gtc start`.

### Non-Interactive Calls Can Still Need `--target`

If a bundle advertises multiple possible targets and there is no explicit
default, automation should pass `--target` rather than relying on interactive
selection.

### Remote References Are Resolved Before Start

`gtc` can fetch or map remote bundle references before execution. This means the
bundle reference itself may not always be a local directory at the moment the
user types the command.

## Extension Start Handoff

This repo also supports:

```bash
gtc start --extension-start-handoff <path>
```

In that mode, `gtc`:

- loads a normalized start handoff document
- merges start-tail arguments from that handoff
- re-enters the normal `gtc start` orchestration path with the resolved bundle reference

This keeps extension-specific start inputs normalized while still using the same
repo-owned start logic.

## What Should An Agent Verify First?

Before editing start docs or examples, verify:

1. which target is actually selected
2. whether the call is local runtime or deployer-backed
3. whether the bundle reference is local or remote
4. whether the example depends on interactive target selection
5. whether the example is using extension start handoff rather than the normal path
