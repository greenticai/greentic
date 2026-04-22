Status: Canonical in this repo
Scope: Current terminology for docs and code changes
Implementation owner: gtc documentation in this repo

# Current Terms And Deprecations

Use the current terms below when editing docs or code comments in this repo.

| Current term | Older or deprecated synonym | Status | Use in new docs/code? | Notes |
| --- | --- | --- | --- | --- |
| `update` | `upgrade` | Current term | Yes | The current CLI command is `gtc update`. Do not invent an `upgrade` command in docs. |
| `bundle ref` / `bundle-ref` | `--bundle` for `gtc start` or `gtc stop` | Current term | Yes | Current parsing expects the bundle reference as the main argument. Existing parser errors explicitly tell users not to pass `--bundle`. |
| `gtc op` | `gtc operator` | Current term | Yes, use `gtc op` | The current CLI tree exposes `op`, not an `operator` top-level command. |
| `Canonical in this repo` | generic “official” or “authoritative” without scope | Current term | Yes | Use this label when the implementation truth is owned here. |
| `Operational guidance in this repo` | treating cross-repo behavior as if this repo owns it | Current term | Yes | Use this label when guidance is local but implementation ownership is elsewhere. |
| `generated schema docs` | prose-only schema descriptions | Current term | Yes | Schema-derived docs outrank prose when both exist for the same topic. |
| `extensions` / `extension handoff` | ad hoc “plugin setup” wording | Current term | Yes | Current CLI surfaces use `--extensions`, `--extension-registry`, `--extension-setup-handoff`, and `--extension-start-handoff`. |
| `setup` and `start` | vague “deploy the bundle” wording for every path | Current term | Yes | Keep setup and start as distinct phases unless the current implementation truly combines them. |

## Usage Rule

If an older term still appears in historical docs or examples, prefer the
current term in new writing and update the old wording when you touch the file.
