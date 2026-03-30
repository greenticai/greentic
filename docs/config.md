# GTC Config Inventory

This document lists the repository-owned environment variables that `gtc`
recognizes directly through `GtcConfig`.

## Distribution and install

- `GTC_LOCALE`
  Default locale override used when `--locale` is not passed on the CLI.
- `GTC_DIST_MOCK_ROOT`
  Override the local mock root used by `src/dist.rs` tests and local fixture flows.
- `GTC_TENANT_MANIFEST_URL_TEMPLATE`
  Override the tenant tool manifest URL template used by install flows.
- `CARGO_HOME`
  Override the cargo home used when resolving install destinations and companion binaries.
- `GREENTIC_<TENANT>_KEY`
  Tenant-scoped install key used by `gtc install --tenant ...` when `--key` is not provided.
  Non-alphanumeric characters in the tenant name are normalized to `_`.

## Bundle and deploy source mapping

- `GREENTIC_DEPLOY_BUNDLE_SOURCE`
  Default remote bundle source used by deploy flows when `--deploy-bundle-source` is omitted.
- `GREENTIC_REPO_REGISTRY_BASE`
  Base URL used to resolve `repo://...` bundle references.
- `GREENTIC_STORE_REGISTRY_BASE`
  Base URL used to resolve `store://...` bundle references.

## Terraform operator defaults

- `GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE`
  Override the operator image injected into deploy subprocesses.
- `GREENTIC_DEPLOY_TERRAFORM_VAR_OPERATOR_IMAGE_DIGEST`
  Override the operator image digest injected into deploy subprocesses.

## Companion binary overrides

- `GREENTIC_DEV_BIN`
  Override the `greentic-dev` binary path.
- `GREENTIC_OPERATOR_BIN`
  Override the `greentic-operator` binary path.
- `GREENTIC_BUNDLE_BIN`
  Override the `greentic-bundle` binary path.
- `GREENTIC_DEPLOYER_BIN`
  Override the `greentic-deployer` binary path.
- `GREENTIC_SETUP_BIN`
  Override the `greentic-setup` binary path.

Cloud-provider credentials and generic process environment values such as `PATH`
are intentionally not listed here. They are still supported, but they are treated
as external runtime inputs rather than repo-owned configuration knobs.
