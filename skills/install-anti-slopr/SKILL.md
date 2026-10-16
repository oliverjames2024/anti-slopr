---
name: install-anti-slopr
description: Install, configure, update, or tune anti-slopr in a Rust repository — the strict rustc/Clippy lint profile, the anti-slopr Dylint library, and cargo-deny bans. Use when adding anti-slopr, adopting it incrementally in an existing codebase, upgrading to a newer anti-slopr release, or deciding how to fix or suppress its findings.
---

# Install or update anti-slopr

anti-slopr has three layers. Install all three unless the user narrows the scope:

| Layer | Files it touches | Toolchain |
| --- | --- | --- |
| Lint profile | `[workspace.lints]` in `Cargo.toml`, `clippy.toml` | stable |
| Dylint library | `[workspace.metadata.dylint]` in `Cargo.toml`, optional `dylint.toml` | pinned nightly, installed automatically by Dylint |
| Dependency bans | `deny.toml` | stable (`cargo-deny`) |

The repository owns the result. Preserve existing lint levels, clippy settings, and deny policy; when the profile conflicts with an explicit local choice, keep the local choice and tell the user.

Upstream: `https://github.com/oliverjames2024/anti-slopr`. Profile sources live under `profile/` and the Dylint library under `lints/`. Read the files from the release the user wants (default: the latest tag); do not reproduce them from memory.

## Choose the path

Read the repository's agent instructions, `git status`, the root `Cargo.toml`, any `clippy.toml`/`.clippy.toml`, `deny.toml`, `dylint.toml`, and `rust-toolchain(.toml)`.

- **Existing installation** (a `[workspace.metadata.dylint]` entry naming anti-slopr, or an `anti-slopr` comment in the lint tables): follow [Update](references/update.md).
- **Fresh install**: follow the steps below.
- **Large existing workspace** (many members, or a full Clippy run too slow for every change): follow the steps below, but use the per-crate opt-in, baseline gate and scoped runs in [Adopting in a large workspace](references/adoption.md).

Complete when the target workspace root and any pre-existing uncommitted work are identified.

## Fresh install

1. **Lint profile.** Merge `profile/lints.toml` into the root manifest.
   - Workspace: merge into `[workspace.lints.rust]`, `[workspace.lints.rustdoc]`, `[workspace.lints.clippy]`, and `[workspace.lints.rust.unexpected_cfgs]`. Add `[lints] workspace = true` to every member that lacks a `[lints]` table. A member with its own `[lints]` table cannot also inherit: show the user and ask.
   - In a large workspace with many existing findings, opt members in one at a time instead, starting with new or small crates that can reach zero findings. Record the opted-in set; see [Adopting in a large workspace](references/adoption.md).
   - Single package: use `[lints.*]` instead of `[workspace.lints.*]`.
   - Keep existing entries. Keep the explanatory comments; they are the policy's documentation.
   - Merge `profile/clippy.toml` into the root `clippy.toml`. Clippy rejects unknown keys, so run Clippy afterwards to validate. Merge `disallowed-*` lists by `path` instead of replacing them.
   - If the repository sets `msrv`/`rust-version` below a lint's availability, Clippy reports unknown lints; remove those entries and say so.

   Complete when `cargo clippy --workspace --all-targets` runs without configuration errors or unknown-lint warnings.

2. **Dylint library.** Install the tools if missing, preferring prebuilt binaries:

   ```bash
   cargo binstall --no-confirm cargo-dylint dylint-link || cargo install cargo-dylint dylint-link
   ```

   Then add to the root `Cargo.toml` (or `dylint.toml`), pinning the release tag:

   ```toml
   [workspace.metadata.dylint]
   libraries = [
     { git = "https://github.com/oliverjames2024/anti-slopr", tag = "<release tag>", pattern = "lints" },
   ]
   ```

   For a vendored copy instead (no network at lint time, local edits), copy `lints/` to `tools/anti-slopr/` and use `{ path = "tools/anti-slopr" }`. Exclude that directory from the consuming workspace (`exclude = ["tools/anti-slopr"]`): it builds with its own nightly toolchain.

   Run `cargo dylint --all -- --all-targets`. The first run downloads the pinned nightly and builds the library; later runs are cached.

   Complete when `cargo dylint --all` finishes and reports anti-slopr diagnostics (or none).

3. **Dependency bans.** Merge `profile/deny.toml`'s `[bans]` entries into `deny.toml` (create it with `cargo deny init` if absent and the user agrees to adopt cargo-deny). If a banned crate is already a dependency, do not remove it; report it.

4. **CI.** Add or extend a CI job. The commands below lint the whole workspace, which suits small and medium repositories. When a full run takes more than a few minutes, lint only the packages a change touches (`-p <crate> --no-deps` for each); see [Adopting in a large workspace](references/adoption.md). Measure the job's wall time and report it.

   ```yaml
   - run: cargo clippy --workspace --all-targets --all-features -- -D warnings
   - run: cargo binstall --no-confirm cargo-dylint dylint-link
   - run: cargo dylint --all -- --all-targets
     env:
       DYLINT_RUSTFLAGS: -D warnings
   - run: cargo deny check bans   # if cargo-deny was adopted
   ```

   Cache `~/.cargo`, `~/.rustup/toolchains`, `~/.dylint_drivers`, and `target/`.

   Gate scripts often run outside a login shell (CI runners, hooks, merge bots), where `~/.cargo/bin` may not be on `PATH`. Resolve `cargo`, `cargo-dylint` and `cargo-deny` from `PATH` first, then `$CARGO_HOME/bin` or `~/.cargo/bin`. If a tool is still missing, print a clear skip naming it and exit with a distinct status: never crash, and never pass silently.

   Dylint builds with its own pinned nightly (with `rustc-dev`) and checks dependencies in a separate `target/dylint` directory. Tell the user about the extra toolchain download and disk use before adding it to a gate.

5. **Record the baseline.** Count findings per lint for Clippy and Dylint, and per crate in a workspace. Do not mass-fix or mass-suppress. Present the counts, and classify the high-count lints as correctness or evidence findings versus style before proposing which to adopt first. Offer to commit the counts as a baseline file with a check that fails when any count grows, then offer the adoption strategies below.

## Adopting in an existing codebase

Large existing codebases usually produce many findings. Offer, in order of preference:

1. **Fix by lint**, highest-signal first: suppression lints (`allow_attributes*`, `blanket_lint_suppression`), error-handling lints, then the rest. One commit per lint keeps review tractable.
2. **Ratchet**: temporarily lower specific lints to `"warn"` in the manifest with a `# anti-slopr: ratchet` comment, keep CI on `-D warnings` for everything else, and remove the ratchet entries as the counts reach zero. A `warn` level alone does not stop new findings; pair it with the committed baseline check so that counts can only fall.
3. **Report first, refuse on a date**: when a new gate starts as report-only, write the date it starts refusing into the check itself (for example, an environment default that flips on that date). A report-only check without a switch date stays report-only.
4. **Disable** a Dylint lint for the whole workspace with `DYLINT_RUSTFLAGS="-A <lint>"` only when the team disagrees with the policy; record that decision in the repository's agent instructions.

## Fixing findings

The point of anti-slopr is to change code, not to silence it. When fixing:

- Prefer the replacement the diagnostic suggests. Read the lint's documentation (`README.md` in the upstream repository) when the fix is not obvious.
- A suppression must be `#[expect(lint, reason = "...")]` on the narrowest item, with a reason that records the invariant or evidence. For anti-slopr lints, wrap it so plain `cargo` builds do not warn about an unknown lint:

  ```rust
  #[cfg_attr(dylint_lib = "anti_slopr", expect(untyped_value_in_signature, reason = "forwards arbitrary JSON from the webhook body to the audit log unchanged"))]
  ```

- Never add crate- or module-level `#![allow]`, never suppress a lint group, and never weaken a test to make a lint pass.
- Never game a measure: do not join lines, inline functions or reformat code to get under a line or size threshold, and do not add `#[expect]` across many sites to make a count pass.
- Do not run repository-wide `cargo fmt`, `cargo clippy --fix` or `cargo dylint --fix` as part of adoption. Fix one lint at a time, in the crates in scope, so each change is reviewable.
