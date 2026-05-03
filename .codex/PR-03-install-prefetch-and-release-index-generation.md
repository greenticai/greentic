# PR-03: Prefetch Extension Packs/Components and Generate Release Index

Repo: `gtc`

## Goal

Add install-time support for release/channel context:

```text
gtc install --release 1.0.16 --channel stable
```

The install flow should prefetch artifacts and write a release index compatible with `greentic-distributor-client`.

## Responsibilities

- Update the existing `install` CLI contract: current `main` already parses
  `--release` and `--channel`, but they conflict. This PR must either allow
  `--release ... --channel ...` as shown above, or change the examples/tests to
  use a release-only command with an explicit default channel.
- Extend `ToolchainInstallOptions` / `ToolchainSource` so release installs carry
  channel context. Current `ToolchainSource::Release` only stores the release
  string.
- Load the release artifact manifest from a clearly defined source. Current
  `main` resolves the gtc toolchain manifest from GHCR or
  `GTC_TOOLCHAIN_MANIFEST_PATH`; it does not call `greentic-dev` for this.
- Resolve versioned extension packs and components.
- Pull artifacts into the shared `DistClient` cache.
- Generate the release index.
- Save current release context.

## Current-Code Alignment Notes

- The current toolchain manifest schema is only:

```text
schema, toolchain, version, channel, created_at, packages[]
```

Each package has `crate`, `bins`, and `version`. There is no current
`extension_packs`, `components`, or artifact-ref field. This PR must either
extend `greentic.toolchain-manifest.v1` carefully, introduce a separate release
artifact manifest, or document exactly where those refs come from.
- Sibling repo update: `../greentic-dev` has local PR
  `.codex/PR-01-toolchain-manifest-extension-packs-components.md` and code
  changes that extend the toolchain manifest with version-only
  `extension_packs` and `components`. If that lands first, this PR should
  consume those fields instead of defining a separate artifact manifest.
- The install path currently installs Rust crates via `cargo binstall`; it does
  not prefetch runtime artifacts through `DistClient`.
- `greentic-distributor-client` 0.5.0 exposes `parse_source`, `resolve`,
  `fetch`, `open_cached`, and `stat_cache`. Prefer those public APIs over
  duplicating resolver behavior.
- Sibling repo update: `../greentic-distributor-client` has local PR
  `.codex/PR-02-release-context-aware-oci-resolution.md` and code changes that
  introduce `ReleaseChannel`, `ReleaseResolutionContext`, `ReleaseIndex`,
  `ReleaseIndexEntry`, `is_mutable_release_tag`,
  `DistClient::resolve_with_release_context`, and
  `DistClient::resolve_oci_ref_with_context`. If that lands first, this PR
  should depend on and use those public types/APIs.
- The current installed toolchain state is written to
  `~/.greentic/toolchain/installed.json`, with `GTC_TOOLCHAIN_STATE_DIR` as a
  test override. If adding `~/.greentic/releases/current.json`, add an explicit
  state-dir override for tests and document why this is separate from the
  installed toolchain state.

## Cache And Index Paths

Use the same cache root as `greentic-distributor-client` / `DistClient`.

Release index:

```text
<cache_dir>/release-index/v1/<channel>/<release>.json
```

Artifacts:

```text
<cache_dir>/artifacts/sha256/<aa>/<remaining-62-hex>/blob
<cache_dir>/artifacts/sha256/<aa>/<remaining-62-hex>/entry.json
```

Do not introduce `oci-blobs/`, `blobs/`, or `entries/` as primary local layouts. Air-gap archives may have their own archive structure, but import must restore the real cache layout.

## Index Generation

For each versioned manifest ref:

```rust
let mutable_ref = format!("{repo}:{channel}");
let version_ref = format!("{repo}:{version}");
let resolved = fetch(version_ref);

release_index.insert(mutable_ref, version, resolved.digest, resolved.canonical_ref);
```

The canonical ref must be digest-pinned and must match the digest.

Define the release-index JSON schema in this PR. `greentic-distributor-client`
0.5.0 does not currently ship a release-index model, so either:

- keep the schema gtc-owned and implement a gtc resolver/injector that maps
  mutable refs through it, or
- bump to a distributor-client version that owns this schema and use that public
  API.

After reviewing the sibling repo local changes, prefer the second option once
available: use the distributor-client `ReleaseIndex` / `ReleaseIndexEntry`
schema and context-aware resolver API rather than creating a gtc-owned duplicate.

## Current Context

Write:

```text
~/.greentic/releases/current.json
```

```json
{
  "release": "1.0.16",
  "channel": "stable"
}
```

If the cache root is configurable, keep the current context path policy explicit. It should not accidentally follow a temporary cache dir used by tests unless that is intentional.

## Tests

- `gtc install --release ... --channel ...` writes the release index.
- All index digests have corresponding cache blobs and `entry.json`.
- Re-running install is idempotent.
- Partial failure does not replace a previous valid index.
