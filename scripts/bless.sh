#!/usr/bin/env bash
# Regenerates ui/<lint>.stderr for the given lints (default: all) from the
# output of the current build. Review the diff before committing.
set -euo pipefail
cd "$(dirname "$0")/../lints"

lints=("$@")
if [ ${#lints[@]} -eq 0 ]; then
    mapfile -t lints < <(ls ui/*.rs | xargs -n1 basename | sed 's/\.rs$//')
fi

for lint in "${lints[@]}"; do
    rm -f "/tmp/$lint.stage-id.stderr" "/tmp/$lint.stage-id.fixed"
    touch "ui/$lint.stderr"
    cargo test --quiet --lib "$lint::ui" -- --exact >/dev/null 2>&1 || true
    # Examples with `// run-rustfix` also record the fixed source.
    if [ -f "ui/$lint.fixed" ] && [ -f "/tmp/$lint.stage-id.fixed" ]; then
        cp "/tmp/$lint.stage-id.fixed" "ui/$lint.fixed"
    fi
    if [ -f "/tmp/$lint.stage-id.stderr" ]; then
        cp "/tmp/$lint.stage-id.stderr" "ui/$lint.stderr"
        echo "blessed $lint"
    else
        echo "unchanged $lint"
    fi
done
