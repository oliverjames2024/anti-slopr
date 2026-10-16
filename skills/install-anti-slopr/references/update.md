# Update an anti-slopr installation

1. Identify the installed revision: the `tag`/`rev` in `[workspace.metadata.dylint]`, or the provenance note of a vendored copy (`tools/anti-slopr/UPSTREAM.md`).
2. Identify the target revision (default: the latest release tag). Read its `CHANGELOG.md` entries between the two revisions; list new lints, renamed lints, changed defaults, and toolchain bumps for the user.
3. **Git-referenced library**: bump the `tag`. Run `cargo dylint --all -- --all-targets` and report new findings per lint.
4. **Vendored library**: stage the new upstream `lints/` in a temporary directory. If the old upstream snapshot is recoverable (from the recorded revision), perform a three-way merge so local edits survive; otherwise diff the vendored copy against the old upstream revision, show the local edits to the user, and port them onto the new copy. Update `UPSTREAM.md` with the new revision.
5. **Profile**: diff the old and new `profile/lints.toml`, `profile/clippy.toml`, and `profile/deny.toml`. Apply additions and default changes, but keep any entry the repository has deliberately changed. Ask before enabling a newly added `deny`-level lint in a repository that already has findings for it; offer the ratchet strategy from SKILL.md.
6. Run Clippy and Dylint, then summarise: what changed upstream, what was applied, what was kept locally, and the new finding counts.
