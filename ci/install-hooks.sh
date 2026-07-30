#!/usr/bin/env bash
# Point git at the tracked hooks in .githooks. Run once per clone:
#
#     ci/install-hooks.sh
#
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
git config core.hooksPath .githooks
echo "installed: core.hooksPath = .githooks (pre-commit runs fmt + clippy + tests)"
