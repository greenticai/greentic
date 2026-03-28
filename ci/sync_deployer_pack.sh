#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEPLOYER_DIR="${1:-$ROOT_DIR/../greentic-deployer}"
TARGET_PACK="${2:-$ROOT_DIR/assets/deployer/terraform.gtpack}"

if [ ! -f "$DEPLOYER_DIR/Cargo.toml" ]; then
  echo "greentic-deployer repo not found at: $DEPLOYER_DIR" >&2
  exit 1
fi

cargo run --manifest-path "$DEPLOYER_DIR/Cargo.toml" --features internal-tools --bin build_fixture_gtpacks

mkdir -p "$(dirname "$TARGET_PACK")"
cp -f "$DEPLOYER_DIR/dist/terraform.gtpack" "$TARGET_PACK"

echo "synced terraform.gtpack to $TARGET_PACK"
