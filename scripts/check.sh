#!/usr/bin/env bash
# Full local gate: lint library build + UI tests + clippy, profile validation, and
# the end-to-end demo.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"

(cd "$root/lints" && cargo build && cargo test && cargo clippy --lib --tests -- -D warnings)
"$root/scripts/check-profile.sh"
"$root/scripts/e2e.sh"
