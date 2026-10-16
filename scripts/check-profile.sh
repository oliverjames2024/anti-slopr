#!/usr/bin/env bash
# Validates profile/ against the stable toolchain: every lint name must exist and
# clippy.toml must parse. Builds a throwaway workspace that uses the profile.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

mkdir -p "$scratch/probe/src"
{ printf '[workspace]\nmembers = ["probe"]\nresolver = "3"\n\n'; cat "$root/profile/lints.toml"; } > "$scratch/Cargo.toml"
cp "$root/profile/clippy.toml" "$scratch/clippy.toml"
cat > "$scratch/probe/Cargo.toml" <<'TOML'
[package]
name = "probe"
version = "0.1.0"
edition = "2024"
description = "anti-slopr profile probe"
license = "MIT"
repository = "https://example.invalid"
readme = false
keywords = []
categories = []

[lints]
workspace = true
TOML
echo 'fn main() {}' > "$scratch/probe/src/main.rs"

output="$(cd "$scratch" && cargo +stable clippy --quiet -- -D warnings 2>&1)" || { echo "$output"; exit 1; }
if grep -qiE "unknown lint|unknown field|error" <<<"$output"; then
    echo "$output"
    exit 1
fi
echo "profile ok"
