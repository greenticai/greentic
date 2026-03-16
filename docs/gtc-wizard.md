# gtc Routing

`gtc` is a thin router. Business logic stays in downstream binaries.

## Commands

```bash
gtc dev <args...>
gtc op <args...>
gtc wizard <args...>
gtc install <args...>
```

## Routing Rules

- `gtc dev ...` -> `greentic-dev ...`
- `gtc dev wizard ...` -> `greentic-dev wizard ...`
- `gtc wizard ...` -> `greentic-operator wizard ...` (always)
- `gtc op ...` -> `greentic-operator ...`
- `gtc install ...` -> `greentic-dev install tools ...`

## Binary Discovery

PR-GTC-01 uses PATH-only discovery:

- `greentic-dev` must be present in PATH
- `greentic-operator` must be present in PATH
- `greentic-setup` must be present in PATH for `gtc setup`
- `terraform` must be present in PATH for `gtc start` when the bundle includes the Terraform deployer

If missing, `gtc` prints install guidance.

## Doctor

`gtc doctor` checks:

- `greentic-dev` presence in PATH
- `greentic-operator` presence in PATH
- Optional `--version` output display

No compatibility enforcement is performed in PR-GTC-01.
