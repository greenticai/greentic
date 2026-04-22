#!/usr/bin/env bash
set -euo pipefail

mode="best-effort"
for arg in "$@"; do
  case "$arg" in
    --strict)
      mode="strict"
      ;;
    --best-effort)
      mode="best-effort"
      ;;
    *)
      echo "unknown argument: $arg" >&2
      echo "usage: bash ci/sync_schema_docs.sh [--best-effort|--strict]" >&2
      exit 2
      ;;
  esac
done

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
schema_dir="$repo_root/docs/04-schemas"
component_dir="$schema_dir/component-schemas"

mkdir -p "$schema_dir" "$component_dir"

gtc_bin="${GTC_SCHEMA_SYNC_GTC_BIN:-gtc}"
flow_bin="${GTC_SCHEMA_SYNC_FLOW_BIN:-greentic-flow}"

updated=0
unchanged=0
warnings=0
failures=0
drift_changed=0

schema_changes_file="$(mktemp)"
notable_changes_file="$(mktemp)"
docs_review_file="$(mktemp)"
examples_review_file="$(mktemp)"
warnings_file="$(mktemp)"
meta_changes_file="$(mktemp)"

say() {
  printf '%s\n' "$*"
}

warn() {
  printf 'warning: %s\n' "$*" >&2
  printf '%s\n' "$*" >>"$warnings_file"
  warnings=$((warnings + 1))
  if [[ "$mode" == "strict" ]]; then
    failures=$((failures + 1))
  fi
}

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

write_if_changed() {
  local src="$1"
  local dest="$2"
  local label="$3"
  local old_tmp=""

  if [[ -f "$dest" ]] && cmp -s "$src" "$dest"; then
    rm -f "$src"
    say "unchanged: $label"
    unchanged=$((unchanged + 1))
    return 0
  fi

  if [[ -f "$dest" ]]; then
    old_tmp="$(mktemp)"
    cp "$dest" "$old_tmp"
  fi

  mv "$src" "$dest"
  say "updated: $label"
  updated=$((updated + 1))

  case "$dest" in
    *.json)
      if [[ -n "$old_tmp" ]]; then
        record_json_drift "$dest" "$old_tmp" "$dest"
      fi
      ;;
    "$schema_dir/setup-schema.md")
      record_meta_change "$dest"
      ;;
  esac

  [[ -n "$old_tmp" ]] && rm -f "$old_tmp"
}

first_line() {
  "$@" 2>/dev/null | sed -n '1p'
}

have_cmd() {
  command -v "$1" >/dev/null 2>&1
}

extract_json_field() {
  local path="$1"
  local jq_expr="$2"
  if have_cmd jq; then
    jq -r "$jq_expr // empty" "$path" 2>/dev/null || true
    return 0
  fi
  return 1
}

append_unique_line() {
  local target="$1"
  local value="$2"
  [[ -z "$value" ]] && return 0
  if [[ -f "$target" ]] && grep -Fqx "$value" "$target"; then
    return 0
  fi
  printf '%s\n' "$value" >>"$target"
}

record_review_targets() {
  local target="$1"
  shift
  local item
  for item in "$@"; do
    append_unique_line "$target" "$item"
  done
}

record_schema_review_targets() {
  local dest="$1"
  case "$(basename "$dest")" in
    wizard-schema.json)
      record_review_targets "$docs_review_file" \
        "docs/02-cli/gtc-wizard.md" \
        "docs/03-authoring/answers-json-patterns.md" \
        "docs/03-authoring/happy-path-build-an-app.md"
      record_review_targets "$examples_review_file" \
        "docs/03-authoring/answers-json-patterns.md" \
        "docs/03-authoring/happy-path-build-an-app.md"
      ;;
    setup-schema.json)
      record_review_targets "$docs_review_file" \
        "docs/02-cli/gtc-setup.md" \
        "docs/03-authoring/happy-path-build-an-app.md"
      record_review_targets "$examples_review_file" \
        "docs/03-authoring/happy-path-build-an-app.md"
      ;;
    acme-widget-fixture-default.json)
      record_review_targets "$docs_review_file" \
        "docs/02-cli/greentic-flow.md" \
        "docs/03-authoring/flow-step-schema-mapping.md"
      ;;
  esac
}

record_meta_change() {
  local dest="$1"
  append_unique_line "$meta_changes_file" "$dest"
  case "$(basename "$dest")" in
    setup-schema.md)
      record_review_targets "$docs_review_file" "docs/02-cli/gtc-setup.md"
      ;;
  esac
}

record_json_drift() {
  local dest="$1"
  local old_file="$2"
  local new_file="$3"
  local section_tmp old_paths new_paths

  drift_changed=1
  append_unique_line "$schema_changes_file" "$dest"
  record_schema_review_targets "$dest"

  if ! have_cmd jq; then
    return 0
  fi

  section_tmp="$(mktemp)"
  old_paths="$(mktemp)"
  new_paths="$(mktemp)"

  jq -r 'paths | map(tostring) | join(".")' "$old_file" 2>/dev/null | sort -u >"$old_paths" || true
  jq -r 'paths | map(tostring) | join(".")' "$new_file" 2>/dev/null | sort -u >"$new_paths" || true

  {
    printf '### `%s`\n\n' "$dest"

    local added removed
    added="$(comm -13 "$old_paths" "$new_paths" | sed -n '1,8p')"
    removed="$(comm -23 "$old_paths" "$new_paths" | sed -n '1,8p')"

    if [[ -n "$added" ]]; then
      printf 'Added paths:\n'
      while IFS= read -r line; do
        [[ -n "$line" ]] && printf -- '- `%s`\n' "$line"
      done <<<"$added"
      printf '\n'
    fi

    if [[ -n "$removed" ]]; then
      printf 'Removed paths:\n'
      while IFS= read -r line; do
        [[ -n "$line" ]] && printf -- '- `%s`\n' "$line"
      done <<<"$removed"
      printf '\n'
    fi

    if [[ -n "$added" && -n "$removed" ]]; then
      printf 'Possible rename signal:\n'
      printf -- '- This schema changed with both added and removed paths. Review adjacent docs for renamed or restructured fields.\n\n'
    fi
  } >"$section_tmp"

  cat "$section_tmp" >>"$notable_changes_file"
  rm -f "$section_tmp" "$old_paths" "$new_paths"
}

generate_drift_report() {
  local report_tmp report_path
  report_tmp="$(mktemp)"
  report_path="$schema_dir/drift-report.md"

  {
    cat <<'EOF'
Status: Generated from current tooling
Scope: Drift summary for generated schema docs in this repo
Implementation owner: repo-local schema sync tooling

# Drift Report

EOF

    if [[ "$drift_changed" -eq 0 && ! -s "$meta_changes_file" && ! -s "$warnings_file" ]]; then
      cat <<'EOF'
No schema drift was detected during the latest sync run.

This means the generated schema artifacts and their markdown wrappers were
already in sync with the current tool outputs used by the refresh command.
EOF
    else
      if [[ -s "$schema_changes_file" ]]; then
        cat <<'EOF'
## Schemas Changed

EOF
        sort -u "$schema_changes_file" | while IFS= read -r line; do
          [[ -n "$line" ]] && printf -- '- `%s`\n' "$line"
        done
        printf '\n'
      fi

      if [[ -s "$meta_changes_file" ]]; then
        cat <<'EOF'
## Generated Status Docs Changed

EOF
        sort -u "$meta_changes_file" | while IFS= read -r line; do
          [[ -n "$line" ]] && printf -- '- `%s`\n' "$line"
        done
        printf '\n'
      fi

      if [[ -s "$notable_changes_file" ]]; then
        cat <<'EOF'
## Notable Field Changes

EOF
        cat "$notable_changes_file"
      fi

      if [[ -s "$docs_review_file" ]]; then
        cat <<'EOF'
## Docs Likely Needing Manual Review

EOF
        sort -u "$docs_review_file" | while IFS= read -r line; do
          [[ -n "$line" ]] && printf -- '- `%s`\n' "$line"
        done
        printf '\n'
      fi

      if [[ -s "$examples_review_file" ]]; then
        cat <<'EOF'
## Examples Likely Now Stale

EOF
        sort -u "$examples_review_file" | while IFS= read -r line; do
          [[ -n "$line" ]] && printf -- '- `%s`\n' "$line"
        done
        printf '\n'
      fi

      if [[ -s "$warnings_file" ]]; then
        cat <<'EOF'
## Environment Warnings

EOF
        sort -u "$warnings_file" | while IFS= read -r line; do
          [[ -n "$line" ]] && printf -- '- %s\n' "$line"
        done
        printf '\n'
      fi
    fi
  } >"$report_tmp"

  write_if_changed "$report_tmp" "$report_path" "docs/04-schemas/drift-report.md"
}

find_fixture_registry() {
  local matches=()
  shopt -s nullglob
  matches=( "$HOME"/.cargo/registry/src/*/greentic-flow-*/tests/fixtures/registry/index.json )
  shopt -u nullglob

  if [[ "${#matches[@]}" -eq 0 ]]; then
    return 1
  fi

  printf '%s\n' "${matches[@]}" | sort -V | tail -n 1 | xargs dirname
}

generate_wizard_schema() {
  have_cmd "$gtc_bin" || fail "required command not found: $gtc_bin"

  local raw_tmp
  raw_tmp="$(mktemp)"
  if ! "$gtc_bin" wizard --schema >"$raw_tmp"; then
    rm -f "$raw_tmp"
    fail "failed to generate wizard schema via '$gtc_bin wizard --schema'"
  fi
  write_if_changed "$raw_tmp" "$schema_dir/wizard-schema.json" "docs/04-schemas/wizard-schema.json"

  local tool_version title wizard_id schema_id schema_version selected_actions facts_block
  tool_version="$(first_line "$gtc_bin" --version)"
  title="$(extract_json_field "$schema_dir/wizard-schema.json" '.title')"
  wizard_id="$(extract_json_field "$schema_dir/wizard-schema.json" '.properties.wizard_id.const')"
  schema_id="$(extract_json_field "$schema_dir/wizard-schema.json" '.properties.schema_id.const')"
  schema_version="$(extract_json_field "$schema_dir/wizard-schema.json" '.properties.schema_version.const')"
  selected_actions=""
  if have_cmd jq; then
    selected_actions="$(jq -r '.properties.answers.properties.selected_action.enum[]?' "$schema_dir/wizard-schema.json" 2>/dev/null || true)"
  fi

  facts_block=""
  if [[ -n "$title" || -n "$wizard_id" || -n "$schema_id" || -n "$schema_version" ]]; then
    facts_block+="## Stable Top-Level Facts"$'\n\n'
    [[ -n "$title" ]] && facts_block+="- \`title\`: \`$title\`"$'\n'
    [[ -n "$wizard_id" ]] && facts_block+="- \`wizard_id\`: \`$wizard_id\`"$'\n'
    [[ -n "$schema_id" ]] && facts_block+="- \`schema_id\`: \`$schema_id\`"$'\n'
    [[ -n "$schema_version" ]] && facts_block+="- \`schema_version\`: \`$schema_version\`"$'\n'
    if [[ -n "$selected_actions" ]]; then
      facts_block+="- \`selected_action\` enum:"$'\n'
      while IFS= read -r action; do
        [[ -n "$action" ]] && facts_block+="  - \`$action\`"$'\n'
      done <<<"$selected_actions"
    fi
    facts_block+=$'\n'
  fi

  local md_tmp
  md_tmp="$(mktemp)"
  cat >"$md_tmp" <<EOF
Status: Generated from current tooling
Scope: Current installed \`gtc wizard --schema\` output
Implementation owner: \`gtc\` / installed companion tooling

# Wizard Schema

This document summarizes the current generated wizard schema captured from the
installed toolchain in this environment.

## Provenance

- Tool version: \`${tool_version:-unknown}\`
- Command:

\`\`\`bash
gtc wizard --schema
\`\`\`

- Raw artifact: [\`wizard-schema.json\`](./wizard-schema.json)

${facts_block}## What This Schema Represents

The emitted schema is the launcher-level answer contract exposed by the current
installed \`gtc\` toolchain.

It is not only a tiny top-level wrapper. The raw JSON also embeds nested schema
material for delegated pack and bundle answers, which is why the captured raw
artifact is substantially larger than a simple launcher contract.

## How To Use It

- Use this schema before creating or validating wizard \`answers.json\` input for
  the launcher path.
- Treat the raw JSON artifact as the canonical machine-derived reference.
- Use prose docs such as [\`../02-cli/gtc-wizard.md\`](../02-cli/gtc-wizard.md)
  only as interpretation and guidance around the emitted contract.

## Current Limitations

- This sync path captures the installed toolchain output, not yet a fully
  repo-pinned internal schema emitter.
- Later drift-report tooling can build on this generated baseline without
  requiring contributors to hand-edit schema summaries.
EOF
  write_if_changed "$md_tmp" "$schema_dir/wizard-schema.md" "docs/04-schemas/wizard-schema.md"
}

generate_setup_schema_status() {
  have_cmd "$gtc_bin" || fail "required command not found: $gtc_bin"

  local stdout_tmp stderr_tmp exit_code tool_line md_tmp
  stdout_tmp="$(mktemp)"
  stderr_tmp="$(mktemp)"

  set +e
  "$gtc_bin" setup --schema >"$stdout_tmp" 2>"$stderr_tmp"
  exit_code=$?
  set -e

  tool_line="$(first_line greentic-setup --version || true)"
  md_tmp="$(mktemp)"

  if [[ "$exit_code" -eq 0 ]]; then
    write_if_changed "$stdout_tmp" "$schema_dir/setup-schema.json" "docs/04-schemas/setup-schema.json"
    cat >"$md_tmp" <<EOF
Status: Generated from current tooling
Scope: Current setup-schema output in the installed toolchain
Implementation owner: \`greentic-setup\` for command support; this doc records current observed behavior

# Setup Schema

This document records the current setup-schema output for the installed
toolchain used during schema sync.

## Provenance

- Tool version: \`${tool_line:-unknown}\`
- Command:

\`\`\`bash
gtc setup --schema
\`\`\`

- Raw artifact: [\`setup-schema.json\`](./setup-schema.json)

## Current Result

The installed toolchain accepted \`--schema\` on the setup path during this run.

For current semantics, treat the raw artifact as canonical machine-derived
output and use [\`../02-cli/gtc-setup.md\`](../02-cli/gtc-setup.md) for repo-local
operational guidance around it.
EOF
  else
    rm -f "$stdout_tmp"
    rm -f "$schema_dir/setup-schema.json"
    cat >"$md_tmp" <<EOF
Status: Generated from current tooling
Scope: Current setup-schema availability in the installed toolchain
Implementation owner: \`greentic-setup\` for command support; this doc records current observed behavior

# Setup Schema

This document records the current setup-schema status for the installed
toolchain used during schema sync.

## Provenance

- Tool version: \`${tool_line:-unknown}\`
- Command attempted:

\`\`\`bash
gtc setup --schema
\`\`\`

## Current Result

The installed toolchain does **not** currently support \`--schema\` on the setup
path.

Observed stderr:

\`\`\`text
$(sed -n '1,20p' "$stderr_tmp")
\`\`\`

Exit status: \`${exit_code}\`

## What To Do Instead Right Now

For current setup behavior, use:

- [\`../02-cli/gtc-setup.md\`](../02-cli/gtc-setup.md) for repo-local guidance
- \`greentic-setup --help\` for the installed command surface
- \`greentic-setup --dry-run --emit-answers <file> <bundle>\` when you need an
  answers template for a concrete bundle

## Why This File Exists

Schema sync should record missing coverage explicitly rather than silently
pretending setup-schema generation already exists.
EOF
  fi

  rm -f "$stderr_tmp"
  write_if_changed "$md_tmp" "$schema_dir/setup-schema.md" "docs/04-schemas/setup-schema.md"
}

generate_component_fixture_docs() {
  local fixture_root flow_version raw_tmp md_tmp json_path md_path

  json_path="$component_dir/acme-widget-fixture-default.json"
  md_path="$component_dir/acme-widget-fixture-default.md"

  if ! have_cmd "$flow_bin"; then
    warn "optional companion binary not found: $flow_bin; leaving existing component fixture docs unchanged"
    return 0
  fi

  if ! fixture_root="$(find_fixture_registry)"; then
    warn "optional greentic-flow fixture registry not found; leaving existing component fixture docs unchanged"
    return 0
  fi

  raw_tmp="$(mktemp)"
  if ! "$flow_bin" component-schema "oci://acme/widget:1" \
    --resolver "fixture://$fixture_root" \
    --format json >"$raw_tmp"; then
    rm -f "$raw_tmp"
    warn "failed to refresh optional greentic-flow fixture component schema"
    return 0
  fi

  write_if_changed "$raw_tmp" "$json_path" "docs/04-schemas/component-schemas/acme-widget-fixture-default.json"

  flow_version="$(first_line "$flow_bin" --version)"
  md_tmp="$(mktemp)"
  cat >"$md_tmp" <<EOF
Status: Generated from current tooling
Scope: Fixture-backed example component schema for \`greentic-flow\`
Implementation owner: \`greentic-flow\` fixture resolver

# Acme Widget Fixture Schema

This file records one verified \`greentic-flow component-schema\` output captured
through the installed fixture resolver.

It is included as **fixture coverage**, not as a claim that \`acme/widget:1\` is
one of this repo's real canonical production components.

## Provenance

- Tool version: \`${flow_version:-unknown}\`
- Command:

\`\`\`bash
greentic-flow component-schema oci://acme/widget:1 \\
  --resolver fixture://$fixture_root \\
  --format json
\`\`\`

- Raw artifact: [\`acme-widget-fixture-default.json\`](./acme-widget-fixture-default.json)

## Current Output Shape

The emitted schema is currently:

- \`type: object\`
- \`additionalProperties: false\`
- no declared top-level properties

This means the verified fixture default-mode contract in this environment is an
empty strict object schema.

## Why This Is Still Useful

This proves that:

- \`greentic-flow component-schema\` is available locally
- fixture-based component-schema capture works in this environment
- later automation can expand this folder once real component references are
  available or pinned for local verification
EOF
  write_if_changed "$md_tmp" "$md_path" "docs/04-schemas/component-schemas/acme-widget-fixture-default.md"
}

generate_component_readme() {
  local md_tmp
  md_tmp="$(mktemp)"
  cat >"$md_tmp" <<'EOF'
Status: Generated from current tooling
Scope: Optional expanded component-schema coverage for this repo
Implementation owner: `greentic-flow` for component-schema emission; this folder records what was verifiable locally

# Component Schemas

This folder is reserved for component-schema outputs captured through
`greentic-flow component-schema`.

## Coverage Rule

Only check in component-schema outputs here when the referenced component was
actually verifiable in the current environment.

That means:

- prefer current real component references when they are available locally
- use fixture-backed examples only when they are clearly labeled as fixtures
- do not copy component schemas from another repo or from memory

## Current Baseline Coverage

- [`acme-widget-fixture-default.md`](./acme-widget-fixture-default.md)
  Verified through the installed `greentic-flow` fixture resolver when that
  fixture source is available locally.

## Current Gaps

The commonly referenced real components in repo prose, such as adaptive-card,
templates, and llm-openai, are still intentionally absent here until they can
be regenerated from verifiable local or pinned inputs.
EOF
  write_if_changed "$md_tmp" "$component_dir/README.md" "docs/04-schemas/component-schemas/README.md"
}

generate_wizard_schema
generate_setup_schema_status
generate_component_fixture_docs
generate_component_readme
generate_drift_report

say "schema sync summary: updated=$updated unchanged=$unchanged warnings=$warnings mode=$mode"
if [[ "$drift_changed" -eq 1 || -s "$meta_changes_file" ]]; then
  say "review: docs/04-schemas/drift-report.md"
fi

if [[ "$failures" -gt 0 ]]; then
  exit 1
fi
