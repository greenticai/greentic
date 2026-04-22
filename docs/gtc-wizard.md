# gtc Routing

`gtc` is a thin router. Business logic stays in downstream binaries.
The wizard path now has two modes:

- legacy passthrough mode
- extension-launcher mode

## Commands

```bash
gtc dev <args...>
gtc op <args...>
gtc wizard <args...>
gtc install <args...>
gtc setup <args...>
gtc start <args...>
gtc stop <args...>
```

## Routing Rules

- `gtc dev ...` -> `greentic-dev ...`
- `gtc dev wizard ...` -> `greentic-dev wizard ...`
- `gtc wizard ...` -> `greentic-dev wizard ...` in legacy passthrough mode
- `gtc wizard --extensions <id[,id...]> ...` -> descriptor-based extension wizard launch
- `gtc op ...` -> `greentic-operator ...`
- `gtc install ...` -> `greentic-dev install tools ...`

## Binary Discovery

Legacy passthrough mode still uses PATH/sibling/cargo-bin discovery for built-in
Greentic companion binaries.

Extension-launcher mode resolves extension descriptors from one of:

- `--extension-registry <path>`
- `--emit-extension-handoff <path>` to override the generated aggregate handoff location
- `GTC_EXTENSION_REGISTRY`
- `./extension-registry.json`
- `~/.greentic/artifacts/store_assets/extensions/registry.json`

The descriptor then declares which binary to launch and, optionally, the working
directory and default wizard arguments.

After all requested extension wizards run successfully, `gtc` writes a
normalized aggregate launcher handoff JSON document. By default it is written to:

- `./.greentic/wizard/extensions/launcher-handoff.json`

Built-in companion discovery uses PATH-only resolution plus the existing sibling
and cargo-bin fallback logic:

- `greentic-dev` must be present in PATH
- `greentic-operator` must be present in PATH
- `greentic-setup` must be present in PATH for `gtc setup`
- `terraform` must be present in PATH for `gtc start` when the bundle includes the Terraform deployer
- cloud deploy flows also require cloud credentials/tooling for the selected target:
  - AWS for `--target aws`
  - Azure for `--target azure`
  - GCP for `--target gcp`

If missing, `gtc` prints install guidance.

## Practical flow

The current practical operator path is:

1. `gtc wizard`
2. `gtc setup`
3. `gtc start --target runtime|aws|azure|gcp`
4. `gtc stop --target runtime|aws|azure|gcp`

Extension-launcher examples:

```bash
gtc wizard --extensions telco-x --extension-registry ./extension-registry.json
gtc wizard --extensions telco-x,greentic-dw --extension-registry ./extension-registry.json --dry-run
gtc wizard --extensions telco-x,greentic-dw --emit-extension-handoff /tmp/extensions-handoff.json
```

The ownership boundary for the extension model stays explicit:

- X owns X logic
- DW owns DW logic
- `gtc` owns discovery and routing
- `setup` owns setup
- `start` owns readiness and launch

The same contract-first model now extends into setup/start handoff as well:

- `gtc setup --extension-setup-handoff <path>`
- `gtc start --extension-start-handoff <path>`

Those flags are generic handoff entrypoints. `gtc` only validates the common
contract and forwards the normalized inputs into the existing setup/start
owners. It still does not own extension-specific setup or runtime semantics.

Reference docs:

- `/home/vgrishkyan/greentic/demo/MEETING_FLOW.md`
- `/home/vgrishkyan/greentic/demo/AWS_DEPLOY.md`
- `/home/vgrishkyan/greentic/demo/AZURE_DEPLOY.md`
- `/home/vgrishkyan/greentic/demo/GCP_DEPLOY.md`
- `/home/vgrishkyan/greentic/demo/MULTICLOUD_E2E_MATRIX.md`

## Doctor

`gtc doctor` checks:

- `greentic-dev` presence in PATH
- `greentic-operator` presence in PATH
- Optional `--version` output display

No compatibility enforcement is performed in PR-GTC-01.
