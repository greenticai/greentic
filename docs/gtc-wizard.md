# gtc Routing

`gtc` is a thin router. Business logic stays in downstream binaries.

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
- `gtc wizard ...` -> `greentic-dev wizard ...`
- `gtc op ...` -> `greentic-operator ...`
- `gtc install ...` -> `greentic-dev install tools ...`

## Binary Discovery

PR-GTC-01 uses PATH-only discovery:

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
