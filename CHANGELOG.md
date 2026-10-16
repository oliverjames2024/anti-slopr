# Changelog

## Unreleased

- `install-anti-slopr` skill: adoption guidance for large workspaces (`references/adoption.md`). Per-crate opt-in; a committed per-lint baseline with a check that refuses growth; report-only checks that refuse from a stated date; Clippy and Dylint runs scoped to changed packages with `--no-deps`; tool lookup outside a login shell with a typed skip; and agent rules against gaming measures or running repository-wide fixes.

## v0.1.0 — 2026-10-15

Initial release.

- Lint profile: 140 rustc/Clippy lint levels (`profile/lints.toml`), `clippy.toml`, and cargo-deny bans for mocking frameworks.
- Dylint library (nightly-2026-08-20, clippy_utils `21baba9c`) with 17 lints:
  - errors: `stringly_typed_error`, `error_to_string`, `opaque_public_error`, `silent_error_default`, `low_signal_panic_message`
  - types: `untyped_value_in_signature`, `deref_polymorphism`
  - async: `blocking_call_in_async`, `discarded_join_handle`
  - tests: `test_without_assertion`
  - suppression hygiene: `blanket_lint_suppression`, `low_signal_lint_reason`, `deny_warnings_in_source`
  - naming: `vague_symbol_name`
  - comments and layout: `slop_comment`, `low_signal_safety_comment`, `missing_item_spacing` (autofix)
- `install-anti-slopr` agent skill.
