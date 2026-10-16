# Adopting in a large workspace

Use this when a workspace has many members or many existing findings, or when a full Clippy run is too slow to gate every change. In a trial on a large numeric workspace, the profile reported about 3,200 findings across six sample crates (122 distinct lints). Most were style, and a small set enforced rules the team already had but did not check. Gating the whole workspace at once would have stalled every change, so adopt in stages.

## 1. Opt crates in one at a time

- Keep `[workspace.lints]` as the policy. Add `[lints] workspace = true` only to members that reach zero findings, starting with new crates and small leaf crates.
- New crates opt in on creation. Record the opted-in set in the repository's agent instructions, or check it in CI, so it only grows.
- For a crate that cannot reach zero yet, keep it out of the opted-in set and track its counts in the baseline (step 2).

## 2. Commit a baseline and refuse growth

`warn` levels and a one-off report do not stop new findings. Commit the counts and check them on every change:

1. Run Clippy (and Dylint, if adopted) with JSON output per crate, for example `cargo clippy -p <crate> --lib --no-deps --message-format=json`.
2. Count findings per `(crate, lint)` and write them to a committed file, such as `lint-baseline.json`.
3. Add a check that recounts the crates a change touches and fails when any `(crate, lint)` count exceeds the baseline. When a count falls, the check rewrites the baseline lower in the same change; it never raises it.
4. Give a rise one route: a dated entry in the baseline file naming the reason and who will reduce it. Review those entries like code.

## 3. Report first, then refuse on a date

A new check can start as report-only so its findings can be triaged. Put the refusal date in the check, for example an environment variable whose default flips from `warn` to `refuse` on a stated date, and say so in its output. Without a date, report-only becomes permanent.

## 4. Keep the cost scoped to the change

- Lint only the packages a change touches: `cargo clippy -p <crate> --no-deps` for each changed package. Without `--no-deps`, Clippy also lints the crate's workspace path dependencies, so one unrelated finding in a shared dependency refuses every change above it.
- Dylint uses a second, pinned nightly and checks dependencies in its own `target/dylint`. In the trial that took about 40 seconds per large crate, plus several gigabytes of disk. Run it on changed, opted-in crates, or as a report-only step, rather than over the whole workspace on every change.
- Measure the gate's wall time when you add it and report it. A gate that grows past a few minutes gets scoped further, not dropped.

## 5. Choose what to adopt first

Classify the baseline before proposing adoption:

- **Correctness and evidence lints** (swallowed errors, discarded causes, stringly typed errors, hash-order-dependent output, wildcard arms over growing enums, suppressions without a reason) usually catch real defects. Adopt them first, even when the rest of the profile waits.
- **Style lints** (pedantic naming, import style, formatting-adjacent rules) are most of the volume and rarely catch defects. Adopt them later, crate by crate, or leave them off by recorded decision.
- **Domain conflicts**: some lints misread a domain. Numeric code is full of intentional index widening and integer-to-float conversions, and domain vocabulary can trip naming lints. Record each such decision in the repository's agent instructions rather than suppressing site by site.
