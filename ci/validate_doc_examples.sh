#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
schema_path="$repo_root/docs/04-schemas/wizard-schema.json"
examples_dir="$repo_root/docs/examples"

if ! command -v jq >/dev/null 2>&1; then
  echo "error: jq is required to validate canonical doc examples" >&2
  exit 1
fi

if [[ ! -f "$schema_path" ]]; then
  echo "error: missing wizard schema at $schema_path; run schema sync first" >&2
  exit 1
fi

wizard_id="$(jq -r '.properties.wizard_id.const' "$schema_path")"
schema_id="$(jq -r '.properties.schema_id.const' "$schema_path")"
schema_version="$(jq -r '.properties.schema_version.const' "$schema_path")"
selected_actions="$(jq -r '.properties.answers.properties.selected_action.enum[]?' "$schema_path")"

validate_wizard_example() {
  local path="$1"

  jq empty "$path" >/dev/null

  local actual_wizard_id actual_schema_id actual_schema_version locale selected_action
  actual_wizard_id="$(jq -r '.wizard_id' "$path")"
  actual_schema_id="$(jq -r '.schema_id' "$path")"
  actual_schema_version="$(jq -r '.schema_version' "$path")"
  locale="$(jq -r '.locale' "$path")"
  selected_action="$(jq -r '.answers.selected_action // empty' "$path")"

  [[ "$actual_wizard_id" == "$wizard_id" ]] || {
    echo "error: $path has wizard_id=$actual_wizard_id, expected $wizard_id" >&2
    exit 1
  }
  [[ "$actual_schema_id" == "$schema_id" ]] || {
    echo "error: $path has schema_id=$actual_schema_id, expected $schema_id" >&2
    exit 1
  }
  [[ "$actual_schema_version" == "$schema_version" ]] || {
    echo "error: $path has schema_version=$actual_schema_version, expected $schema_version" >&2
    exit 1
  }
  [[ -n "$locale" && "$locale" != "null" ]] || {
    echo "error: $path is missing non-empty locale" >&2
    exit 1
  }

  if [[ -n "$selected_action" ]]; then
    if ! grep -Fxq "$selected_action" <<<"$selected_actions"; then
      echo "error: $path has answers.selected_action=$selected_action, expected one of: $(echo "$selected_actions" | paste -sd ',' -)" >&2
      exit 1
    fi
  fi

  echo "validated: ${path#$repo_root/}"
}

validate_wizard_example "$examples_dir/wizard-launcher-minimal.answers.json"
validate_wizard_example "$examples_dir/wizard-launcher-bundle.answers.json"

echo "example validation completed"
