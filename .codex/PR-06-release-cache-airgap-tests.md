# PR-06: Release Cache Air-Gap Tests

Repo: `gtc`

## Goal

Cover install-time prefetch, release-index generation, and air-gapped export/import behavior.

This PR should land after PR-03 and PR-04, or be split so each earlier PR
carries its own focused tests. Current `main` has no release-index writer,
`release-cache` command, or release-index-backed offline mutable-ref resolver.

Sibling repo update: `../greentic-dev` has local changes that add version-only
`extension_packs` and `components` sections to generated toolchain manifests,
and `../greentic-distributor-client` has local changes that add
release-context-aware OCI resolution. These tests should target those contracts
if they are available as dependencies.

## Test Cases

### Install

- Manifest or release metadata with extension packs/components resolves versions
  to digests. If the greentic-dev PR lands first, use its
  `extension_packs`/`components` fields rather than inventing a test-only shape.
- `gtc install --release ... --channel ...` is accepted if PR-03 keeps that CLI
  shape; today those flags conflict.
- Generated release index contains mutable refs for the selected channel.
- Every indexed digest has a corresponding `artifacts/sha256/.../blob` and `entry.json`.
- Partial install failure does not corrupt the previous release index.

### Air-Gap Export/Import

- `gtc release-cache export` and `gtc release-cache import` are registered in
  the clap command tree and dispatcher.
- Export collects release index plus all referenced artifacts.
- Import verifies checksums before mutating the target cache.
- Import restores the real `DistClient` cache layout.
- Imported cache supports offline resolution through the resolver path chosen by
  PR-03. Prefer the distributor-client `resolve_with_release_context` /
  `resolve_oci_ref_with_context` API from the sibling release-context PR when
  available.

### Corruption

- Missing release index fails export.
- Missing referenced blob fails export.
- Checksum mismatch fails import.
- Invalid digest in index fails import validation.

## Notes

Do not assert against a fictional `blobs/` or `entries/` cache layout. The runtime cache layout is:

```text
<cache_dir>/artifacts/sha256/<aa>/<remaining-62-hex>/blob
<cache_dir>/artifacts/sha256/<aa>/<remaining-62-hex>/entry.json
```

Use temporary cache and state roots in tests. The current distributor client
honors `GREENTIC_CACHE_DIR` / `GREENTIC_DIST_CACHE_DIR`; release-current state
should have an explicit test override if PR-03 writes outside the cache root.
