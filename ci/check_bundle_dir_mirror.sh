#!/usr/bin/env bash
set -euo pipefail

# ci/check_bundle_dir_mirror.sh
#
# Verifies that gtc's mirror copies of greentic-bundle's bundle-directory
# prediction functions have not drifted from the canonical implementation
# in the wizard (greentic-bundle/src/wizard/mod.rs).
#
# Compared functions:
#   - normalize_bundle_id
#   - default_bundle_output_dir
#
# The output_dir trim/empty-check chain in predict_bundle_dir is structurally
# different from normalized_request_from_document (different function shape,
# same semantics) and is covered by the unit tests in up.rs rather than by
# this source-level check.
#
# Fails closed: exits non-zero when the sibling greentic-bundle checkout is
# missing, when a source file is absent, or when a function body cannot be
# extracted.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bundle_repo="${GREENTIC_BUNDLE_REPO:-$repo_root/../greentic-bundle}"

if [[ ! -d "$bundle_repo" ]]; then
  echo "error: greentic-bundle checkout not found at $bundle_repo" >&2
  echo "  Set GREENTIC_BUNDLE_REPO or check out greentic-bundle as a sibling." >&2
  exit 1
fi

gtc_source="$repo_root/src/bin/gtc/up.rs"
bundle_source="$bundle_repo/src/wizard/mod.rs"

for f in "$gtc_source" "$bundle_source"; do
  if [[ ! -f "$f" ]]; then
    echo "error: source file not found: $f" >&2
    exit 1
  fi
done

# Extract a Rust function body by name: from "fn $name(" through its closing
# "}" at brace-depth zero.  Handles nested blocks and balanced braces in
# format strings.  Does NOT parse strings/comments, but the target functions
# contain only balanced braces in both contexts, so this is safe.
extract_fn() {
  local file="$1" name="$2"
  awk -v name="$name" '
    BEGIN { found = 0; depth = 0 }
    !found && $0 ~ "fn " name "\\(" { found = 1 }
    found {
      print
      for (i = 1; i <= length($0); i++) {
        c = substr($0, i, 1)
        if (c == "{") depth = depth + 1
        else if (c == "}") {
          depth = depth - 1
          if (depth == 0) exit
        }
      }
    }
  ' "$file"
}

failures=0

check_fn() {
  local name="$1"
  local gtc_body bundle_body

  gtc_body="$(extract_fn "$gtc_source" "$name")"
  bundle_body="$(extract_fn "$bundle_source" "$name")"

  if [[ -z "$gtc_body" ]]; then
    echo "error: could not extract $name from $gtc_source" >&2
    failures=$((failures + 1))
    return
  fi
  if [[ -z "$bundle_body" ]]; then
    echo "error: could not extract $name from $bundle_source" >&2
    failures=$((failures + 1))
    return
  fi

  if [[ "$gtc_body" != "$bundle_body" ]]; then
    echo "DRIFT: $name differs between gtc and greentic-bundle" >&2
    echo "" >&2
    echo "--- greentic-bundle (canonical)" >&2
    echo "$bundle_body" >&2
    echo "" >&2
    echo "+++ gtc (mirror)" >&2
    echo "$gtc_body" >&2
    echo "" >&2
    diff <(echo "$bundle_body") <(echo "$gtc_body") >&2 || true
    failures=$((failures + 1))
  else
    echo "ok: $name matches"
  fi
}

check_fn "normalize_bundle_id"
check_fn "default_bundle_output_dir"

if [[ "$failures" -gt 0 ]]; then
  echo "" >&2
  echo "error: $failures function(s) drifted from greentic-bundle" >&2
  echo "  Update the mirror in src/bin/gtc/up.rs to match" >&2
  echo "  $bundle_source" >&2
  exit 1
fi

echo "bundle-dir mirror check passed"
