# PR-04: Air-Gapped Release Cache Export/Import

Repo: `gtc`

## Goal

Support portable release cache bundles for air-gapped environments.

This PR depends on PR-03 defining and writing a release index. Current `main`
does not have a `release-cache` command, release-index model, or mutable-ref
offline resolver.

Sibling repo update: `../greentic-distributor-client` has local release-context
changes that define `ReleaseIndex`, `ReleaseIndexEntry`, and
`ReleaseResolutionContext`, plus context-aware resolution against
`<cache_dir>/release-index/v1/<channel>/<release>.json`. If that lands first,
this PR should reuse that schema and validate imported indexes against it.

## CLI

Export:

```bash
gtc release-cache export \
  --release 1.0.16 \
  --channel stable \
  --output cache.tar.zst
```

Import:

```bash
gtc release-cache import \
  --input cache.tar.zst
```

Implementation must add the `release-cache` command to both the clap tree and
the dispatcher, plus English i18n strings at minimum. Follow the existing
repo-owned command pattern in `src/bin/gtc/cli.rs` and `src/bin/gtc/commands.rs`.

## Archive Contents

The archive may use a transport-friendly structure, but it must contain enough data to restore the real `DistClient` cache layout.

Recommended archive structure:

```text
manifest.json
checksums.json
release-index/v1/<channel>/<release>.json
artifacts/sha256/<aa>/<remaining-62-hex>/blob
artifacts/sha256/<aa>/<remaining-62-hex>/entry.json
```

Only include `legacy-components/` or `legacy-packs/` if a referenced flow still requires those raw OCI subcaches. The primary runtime cache is `artifacts/sha256/...`.

## Export Logic

- Load `<cache_dir>/release-index/v1/<channel>/<release>.json`.
- Collect all referenced digests.
- For each digest, include:
  - `artifacts/sha256/.../blob`
  - `artifacts/sha256/.../entry.json`
- Generate checksums for every archive payload file.
- Compress as `tar.zst`, or explicitly change the extension/format. Current
  dependencies include `tar` and `flate2`, but not a zstd crate.

## Import Logic

- Unpack to a temporary directory.
- Verify checksums before touching the target cache.
- Validate release index schema.
- Validate each indexed digest has a blob and `entry.json`.
- Restore into the configured cache root.
- Use atomic replacement for the release index.

## Current-Code Alignment Notes

- Use the same cache root policy as `DistOptions::default()`:
  `GREENTIC_CACHE_DIR`, then `GREENTIC_DIST_CACHE_DIR`, then the distributor
  client's default cache root.
- `greentic-distributor-client` 0.5.0 stores primary artifacts at
  `artifacts/sha256/<aa>/<remaining-62-hex>/blob` and metadata at
  `entry.json`. Its lower-level cache path helpers are private, so this PR must
  either add small local path helpers that exactly match that layout or move
  archive support into distributor-client and consume public APIs.
- If the local distributor-client release-context PR is available as a
  dependency, use its public release-index structs for archive validation, but
  still expect gtc to own archive creation/import unless distributor-client also
  adds export/import APIs.
- `legacy-components/` and `legacy-packs/` are used by the underlying OCI
  resolver/fetcher during network pulls, but primary runtime reopening goes
  through the `artifacts/sha256/...` cache.
- The test claim that imported cache resolves `:stable` offline is not true
  with published distributor-client 0.5.0 alone; it becomes correct if gtc uses
  the local/upcoming distributor-client context-aware resolver and supplies the
  matching `ReleaseResolutionContext`.

## Future Optional Files

- `signature.json`
- SBOMs
- attestations

## Tests

- Export then import into an empty cache.
- Imported cache resolves `:stable` offline through `greentic-distributor-client`.
- Missing blob in archive fails validation.
- Checksum mismatch fails before cache mutation.
