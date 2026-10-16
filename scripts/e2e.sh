#!/usr/bin/env bash
# Runs the local anti-slopr library against demo/ and checks that every lint in
# lints/src fires at least once.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"

expected="$(ls "$root/lints/src" | sed -n 's/\.rs$//p' | grep -vx -e lib -e util -e comment_scan | sort)"

# The demo is meant to fail; only the diagnostics matter.
output="$(cd "$root/demo" && cargo dylint --all -- --all-targets --message-format=json 2>/dev/null || true)"
fired="$(grep -o '"code":{"code":"[a-z_]*"' <<<"$output" | sed 's/.*"code":"//; s/"$//' | sort -u)"

missing="$(comm -23 <(echo "$expected") <(echo "$fired"))"
if [ -n "$missing" ]; then
    echo "lints that did not fire on demo/:"
    echo "$missing"
    exit 1
fi
echo "e2e ok: all $(wc -l <<<"$expected") lints fired on demo/"
