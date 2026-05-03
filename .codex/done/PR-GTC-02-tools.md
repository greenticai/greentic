PR-GTC-02 — Add gtc install (Public Tools + Tenant-Authorized Installs, locked contracts)
Summary

This PR introduces a new command:

gtc install

The command installs Greentic tools and optionally tenant-authorized commercial components.

Behavior depends on whether a tenant is specified.

The goal is to keep gtc consistent with the existing philosophy:

Thin CLI launcher

No business logic duplication

Use existing infrastructure (greentic-distribution-client)

Full i18n compliance

Behavior
Mode 1 — Public Install (default)
gtc install

If no --tenant is specified:

Install public Greentic tools only

Do not request a key

Do not download commercial artifacts

This mode simply installs the current public CLI tools using the same installation mechanism already used today.

Example output:

Installing public Greentic tools…
Installed:
  greentic-dev
  greentic-operator
  gtc
Mode 2 — Tenant Install
gtc install --tenant <tenant>

Tenant mode installs authorized commercial artifacts in addition to public tools.

Artifacts may include:

tools

components

packs

bundles

These artifacts are retrieved via OCI from GHCR using greentic-distribution-client.

Key Resolution

When --tenant is specified, a tenant key is required.

Resolution order:

--key <KEY>

environment variable GREENTIC_<TENANT>_KEY

interactive prompt (masked input)

If the key cannot be resolved → installation fails.

Environment Variable Format

Environment variable name is derived from the tenant name:

GREENTIC_<TENANT>_KEY

Tenant names are normalized:

Tenant Input	Env Variable
acme	GREENTIC_ACME_KEY
acme-dev	GREENTIC_ACME_DEV_KEY
Acme.Dev-01	GREENTIC_ACME_DEV_01_KEY

Normalization rules:

uppercase

non-alphanumeric → _

collapse repeated _

trim leading/trailing _

Interactive Key Prompt

If the key is not provided via flag or env variable, gtc prompts the user:

Enter key for tenant 'acme':

Input is hidden (no characters echoed).

Implementation uses:

rpassword::prompt_password()

This ensures secure masked input across Linux, macOS, and Windows.

Installation Process
Step 1 — Install Public Tools

gtc install always installs public tools first.

These are installed to the same location used by cargo binstall.

Default binary location:

OS	Directory
Linux/macOS	~/.cargo/bin
Windows	%USERPROFILE%\.cargo\bin

Resolution order:

$CARGO_HOME/bin

fallback ~/.cargo/bin

Step 2 — Tenant Authorized Artifacts (optional)

If --tenant is specified:

gtc pulls a tenant authorization manifest:

oci://ghcr.io/greentic-biz/<tenant>/install-manifest:latest

via:

greentic-distribution-client

The manifest contains OCI references to authorized artifacts.

Example manifest:

{
  "schema": "greentic.install.manifest.v1",
  "items": [
    {
      "kind": "tool",
      "name": "greentic-enterprise-tool",
      "oci": "oci://ghcr.io/greentic-biz/tools/enterprise-tool:1.0.0"
    },
    {
      "kind": "pack",
      "name": "enterprise-routing",
      "oci": "oci://ghcr.io/greentic-biz/packs/enterprise-routing:0.4.0"
    }
  ]
}

For each item:

Kind	Install Location
tools	Cargo bin dir
components	~/.greentic/artifacts/components
packs	~/.greentic/artifacts/packs
bundles	~/.greentic/artifacts/bundles
CLI
Command
gtc install
Options
--tenant <TENANT>
--key <KEY>
Examples

Public install:

gtc install

Tenant install (env key):

export GREENTIC_ACME_KEY=ghp_xxxxxx
gtc install --tenant acme

Tenant install (flag):

gtc install --tenant acme --key ghp_xxxxxx

Tenant install (prompt):

gtc install --tenant acme
Enter key for tenant 'acme':
i18n Requirements

All CLI text must use greentic-i18n tags.

English strings live in:

assets/i18n/en.json

Translations are bundled into the binary.

Example keys:

gtc.cmd.install.about
gtc.arg.tenant.help
gtc.arg.key.help
gtc.install.public_mode
gtc.install.tenant_mode
gtc.install.prompt_key
gtc.install.using_env_key
gtc.err.key_required
gtc.err.invalid_key
gtc.err.distribution_client_missing
gtc.err.pull_failed

Example en.json entries:

{
  "gtc.cmd.install.about": "Install Greentic tools and optionally tenant-authorized artifacts.",
  "gtc.arg.tenant.help": "Tenant identifier for authorized installs.",
  "gtc.arg.key.help": "Authorization key for the specified tenant.",
  "gtc.install.public_mode": "Installing public Greentic tools.",
  "gtc.install.tenant_mode": "Tenant mode enabled for '{tenant}'.",
  "gtc.install.prompt_key": "Enter key for tenant '{tenant}':",
  "gtc.install.using_env_key": "Using tenant key from environment.",
  "gtc.err.key_required": "A key is required for tenant installs."
}
Dependencies

New crate dependency:

rpassword

Used for masked password input.

Error Handling
Error	Behavior
Missing distribution client	clear error message
Invalid key	fail install
OCI pull failure	fail artifact install
Partial install failures	reported individually

Public install should still succeed even if commercial install fails.

Testing
Unit Tests

tenant env var normalization

key resolution priority

env variable lookup

error on empty key

Integration Tests

Using fake binaries in PATH:

fake greentic-distribution-client

fake artifact downloads

Tests verify:

correct CLI routing

correct environment variable usage

correct install directories

correct argument passing

CI must not require real GHCR access.

Documentation Updates

Update README:

gtc install

Add section:

Installing Greentic Tools

Explain:

public install

tenant install

env variable keys

artifact locations

Definition of Done

gtc install implemented

public install works without tenant

tenant install resolves key via flag/env/prompt

masked input for key prompt

artifacts downloaded via greentic-distribution-client

binaries installed to Cargo bin dir

i18n tags used for all CLI text

unit + integration tests passing

documentation updated

Codex Prompt

Implement gtc install in the gtc CLI. If --tenant is not specified, install public tools only and do not prompt for keys. If --tenant is specified, resolve the key via --key, then GREENTIC_<TENANT>_KEY, then a masked interactive prompt using rpassword. Use greentic-distribution-client to fetch a tenant authorization manifest from OCI and install listed artifacts. Tools go into the Cargo bin directory, other artifacts go into ~/.greentic/artifacts. All CLI strings must use greentic-i18n tags with English text stored in assets/i18n/en.json.

Resolved Q&A (Locked Contracts)

1) Distributor contract
- Do not shell out to a distributor CLI.
- Use `greentic-distributor-client v0.4` as a Rust library.
- Keep usage behind a gtc adapter with two operations:
  - `pull_bytes(oci_ref, key) -> Vec<u8>`
  - `pull_to_dir(oci_ref, key, out_dir) -> ()`
- Keep key handling in-memory only.

2) Tool install semantics
- gtc owns install semantics.
- Distributor client fetches/materializes artifacts only.
- gtc unpacks tool artifacts (`zip`/`tar`/`tar.gz`) and copies binaries into Cargo bin dir.
- Cargo bin dir resolution:
  - `$CARGO_HOME/bin` if `CARGO_HOME` is set
  - fallback `~/.cargo/bin`

3) Public mode
- `gtc install` with no tenant is pure passthrough:
  - `greentic-dev install tools`

4) Public failure policy
- If public install fails, tenant install is skipped.
- Return the same non-zero exit code.

5) Tenant partial failures
- Continue best-effort per item.
- Print per-item status (`ok`/`fail`).
- Final exit is non-zero if any tenant item failed.

6) Artifact roots and paths
- Use `directories` crate for user path roots.
- Artifacts root:
  - Unix/macOS: `~/.greentic/artifacts`
  - Windows: `%USERPROFILE%\\.greentic\\artifacts`
- Kind folders:
  - `components/<name>/...`
  - `packs/<name>/...`
  - `bundles/<name>/...`

Tenant key resolution (locked order)
1. `--key <KEY>`
2. env `GREENTIC_<TENANT>_KEY` (tenant normalized)
3. masked prompt `rpassword::prompt_password`

Tenant normalization
- uppercase
- non-alphanumeric -> `_`
- collapse repeated `_`
- trim leading/trailing `_`
