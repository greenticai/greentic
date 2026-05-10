Status: Generated from current tooling
Scope: Current setup-schema availability in the installed toolchain
Implementation owner: `greentic-setup` for command support; this doc records current observed behavior

# Setup Schema

This document records the current setup-schema status for the installed
toolchain used during schema sync.

## Provenance

- Tool version: `greentic-setup 0.5.12`
- Command attempted:

```bash
gtc setup --schema
```

## Current Result

The installed toolchain does **not** currently support `--schema` on the setup
path.

Observed stderr:

```text
warning: Greentic toolchain release context is 1.0.18 (stable), but the latest stable release is 1.0.15. Run `gtc install` to upgrade.
error: unexpected argument '--schema' found

  tip: to pass '--schema' as a value, use '-- --schema'

Usage: greentic-setup [OPTIONS] [BUNDLE] [COMMAND]

For more information, try '--help'.
```

Exit status: `2`

## What To Do Instead Right Now

For current setup behavior, use:

- [`../02-cli/gtc-setup.md`](../02-cli/gtc-setup.md) for repo-local guidance
- `greentic-setup --help` for the installed command surface
- `greentic-setup --dry-run --emit-answers <file> <bundle>` when you need an
  answers template for a concrete bundle

## Why This File Exists

Schema sync should record missing coverage explicitly rather than silently
pretending setup-schema generation already exists.
