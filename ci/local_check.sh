#!/usr/bin/env bash
set -euo pipefail

#echo "[0/6] tools/i18n.sh validate"
#tools/i18n.sh validate

echo "[1/6] schema docs sync (best effort)"
bash ci/sync_schema_docs.sh --best-effort
if ! git diff --quiet -- docs/04-schemas; then
  echo "warning: schema docs changed during local_check; review docs/04-schemas/drift-report.md and update affected prose docs/examples as needed" >&2
fi

echo "[1b/6] validate canonical doc examples"
bash ci/validate_doc_examples.sh

echo "[2/6] cargo fmt --all -- --check"
cargo fmt --all -- --check

echo "[3/6] cargo clippy --all-targets --all-features -- -D warnings"
cargo clippy --all-targets --all-features -- -D warnings

echo "[4/6] cargo test --all-targets --all-features"
cargo test --all-targets --all-features

echo "[4b/6] cargo test --test perf_scaling hashing_scaling_should_not_collapse -- --ignored --exact"
cargo test --test perf_scaling hashing_scaling_should_not_collapse -- --ignored --exact

echo "[5/6] cargo publish --dry-run --allow-dirty"
cargo publish --dry-run --allow-dirty

echo "local_check completed"
