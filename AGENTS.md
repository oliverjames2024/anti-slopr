# Repository guidance

- `lints/` is the Dylint library (crate `anti_slopr`). It builds on the nightly pinned in `lints/rust-toolchain.toml` together with the `clippy_utils` rev in `lints/Cargo.toml`; bump both together with `cargo dylint upgrade lints`.
- `profile/` is the stable-toolchain policy (rustc/Clippy lint levels, `clippy.toml`, cargo-deny bans). Every non-obvious entry carries a comment saying why.
- `skills/install-anti-slopr/` is the agent skill consumers install. Keep it in sync with `profile/` and the README.
- Keep lints generic and suitable for any repository. No application-specific names, paths, or exceptions.
- One lint per module in `lints/src/`, registered in the `modules!` list in `lints/src/lib.rs`, with a UI example `lints/ui/<lint>.rs` registered as an `[[example]]`. Each UI example covers true positives and labelled `// Not linted:` negatives for every known false-positive risk.
- Regenerate expected output with `scripts/bless.sh <lint>` and read the diff before committing.
- Document every lint in the README (what, why, example, fix) and cite the source of the practice.
- Run `scripts/check.sh` before committing.
