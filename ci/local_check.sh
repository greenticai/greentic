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

echo "[4b/5] cargo test --test perf_scaling hashing_scaling_should_not_collapse -- --ignored --exact"
cargo test --test perf_scaling hashing_scaling_should_not_collapse -- --ignored --exact

echo "[5/5] cargo publish --dry-run --allow-dirty"
cargo publish --dry-run --allow-dirty

echo "local_check completed"
