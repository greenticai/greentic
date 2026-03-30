# Release Verification

`gtc` releases now ship with a SHA-256 checksum manifest alongside the packaged
binaries.

## Release assets

Each release includes:

- platform archives such as `gtc-x86_64-unknown-linux-gnu.tgz`
- a checksum manifest named `gtc-<version>-checksums.txt`
- an SPDX JSON SBOM named `gtc-<version>.spdx.json`
- GitHub artifact attestations for the packaged archives and checksum manifest

## Verify a downloaded artifact

Example for Linux/macOS:

```bash
sha256sum -c gtc-0.9.38-checksums.txt --ignore-missing
```

Or verify a single archive explicitly:

```bash
sha256sum gtc-x86_64-unknown-linux-gnu.tgz
grep 'gtc-x86_64-unknown-linux-gnu.tgz' gtc-0.9.38-checksums.txt
```

Example for PowerShell:

```powershell
$actual = (Get-FileHash .\gtc-x86_64-pc-windows-msvc.zip -Algorithm SHA256).Hash.ToLower()
Select-String 'gtc-x86_64-pc-windows-msvc.zip' .\gtc-0.9.38-checksums.txt
```

## Current scope

This verification flow currently covers:

- artifact integrity via checksums
- a machine-readable SPDX SBOM for the crate release
- GitHub-hosted build provenance attestations for release artifacts

The release process does not yet provide:

- code signing
- notarization

## Provenance

The release workflow publishes GitHub build provenance attestations for:

- `gtc-*.tgz`
- `gtc-*.zip`
- `gtc-<version>-checksums.txt`

Those attestations are intended to provide a machine-verifiable link between the
published artifacts and the GitHub Actions release workflow that built them.

## SBOM

The release workflow publishes an SPDX JSON SBOM generated with `cargo-sbom`:

- `gtc-<version>.spdx.json`

That SBOM is intended to make the crate dependency graph and packaged software
components inspectable as part of each release.

Code signing and notarization are still tracked as follow-up hardening work.
