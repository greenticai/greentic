#!/usr/bin/env bash
set -euo pipefail

#echo "[1/5] tools/i18n.sh validate"
#tools/i18n.sh validate

echo "[2/5] cargo fmt --all -- --check"
cargo fmt --all -- --check

echo "[3/5] cargo clippy --all-targets --all-features -- -D warnings"
cargo clippy --all-targets --all-features -- -D warnings

echo "[4/5] cargo test --all-targets --all-features"
cargo test --all-targets --all-features

echo "[5/5] cargo publish --dry-run --allow-dirty"
cargo publish --dry-run --allow-dirty

echo "local_check completed"
