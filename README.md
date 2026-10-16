<img src="assets/icon.svg" width="64" height="64" alt="">

# anti-slopr

Opinionated lints that reject low-evidence and low-signal Rust: the code that compiles, passes review at a glance, and quietly rots. Much of it is what coding agents produce when they optimise for "it builds".

anti-slopr is the Rust sibling of [anti-slop](https://github.com/dmmulroy/anti-slop) (Oxlint rules for TypeScript). It reflects a taste rather than a universal standard: every rule cites the practice it encodes, and every rule can be configured, suppressed with a reason, or turned off.

## What you get

| Layer | What it is | Toolchain |
| --- | --- | --- |
| [Lint profile](profile/lints.toml) | 140 curated rustc/Clippy lint levels plus a [`clippy.toml`](profile/clippy.toml), each non-obvious entry commented with why | stable |
| [Dylint library](lints/) | 17 custom lints, most of them type-aware, for what Clippy cannot express | nightly, pinned and installed automatically by Dylint |
| [Dependency bans](profile/deny.toml) | cargo-deny bans for mocking frameworks | stable |

The profile alone is worth adopting; the Dylint library adds the rules that need type information Clippy's configuration cannot reach.

## Install with an agent skill

```bash
npx skills add oliverjames2024/anti-slopr --skill install-anti-slopr
```

Then ask your coding agent to install anti-slopr in the current repository. The skill merges the profile into your manifest without overwriting local choices, wires up Dylint, adds CI, and reports a per-lint baseline with an incremental adoption plan instead of mass-suppressing findings. Ask it to **update anti-slopr** later to pick up new releases while preserving local changes.

## Manual installation

### 1. Lint profile (stable)

Merge [`profile/lints.toml`](profile/lints.toml) into your workspace `Cargo.toml` and opt each member in:

```toml
[lints]
workspace = true
```

For a single-crate package, rename `[workspace.lints.*]` to `[lints.*]`. Copy [`profile/clippy.toml`](profile/clippy.toml) to the workspace root, then:

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Highlights of the policy:

- **Honest suppression.** `#[allow]` is denied in favour of `#[expect(lint, reason = "...")]`; `unfulfilled_lint_expectations` fails the build once the suppressed problem is gone, so suppressions cannot rot.
- **No hidden panics.** `unwrap()` is denied. `expect()` stays legal, but its message must state the invariant (see `low_signal_panic_message`).
- **No swallowed errors.** `map_err(|_| ..)`, `x.ok();`, and `let _ = must_use()` are denied.
- **Documented `unsafe`.** Every unsafe block needs a `SAFETY:` comment, one operation per block.
- **`pedantic` on, with the noisy, boilerplate-producing lints off.** The opt-outs follow what ruff, uv, oxc, and rerun disable, and each is justified inline.

### 2. Dylint library

```bash
cargo install cargo-dylint dylint-link   # or: cargo binstall cargo-dylint dylint-link
```

Add the library to your workspace `Cargo.toml`, pinning a release:

```toml
[workspace.metadata.dylint]
libraries = [
  { git = "https://github.com/oliverjames2024/anti-slopr", tag = "v0.1.0", pattern = "lints" },
]
```

```bash
cargo dylint --all -- --all-targets
cargo dylint --all --fix                     # apply machine-applicable suggestions
DYLINT_RUSTFLAGS="-D warnings" cargo dylint --all -- --all-targets   # CI
```

The first run downloads the pinned nightly and builds the library; later runs are cached. Your project keeps building on its own toolchain.

To vendor instead, copy `lints/` into your repository (for example `tools/anti-slopr/`), exclude it from your workspace, and reference it with `{ path = "tools/anti-slopr" }`. The library is then yours to edit.

### 3. Dependency bans

Merge [`profile/deny.toml`](profile/deny.toml) into your `deny.toml` and run `cargo deny check bans`.

### Configuration and suppression

Lints with options read them from `dylint.toml` in the workspace root:

```toml
[vague_symbol_name]
extra_segments = ["processor"]

[blocking_call_in_async]
extra_paths = ["my_crate::db::query_sync"]
```

Suppress a single finding on the narrowest item, with a reason that records the evidence. `cfg_attr` keeps plain `cargo build` from warning about an unknown lint:

```rust
#[cfg_attr(dylint_lib = "anti_slopr", expect(untyped_value_in_signature, reason = "forwards the webhook body to the audit log unchanged"))]
fn forward(body: serde_json::Value) { /* ... */ }
```

Disable a lint for the whole workspace with `DYLINT_RUSTFLAGS="-A <lint>"`.

## Lints

All lints default to `warn`. Apart from the comment and layout lints, they use rustc's type information, so they see through type aliases, generic code, and re-exports.

### Errors

#### `stringly_typed_error`

`Result<_, E>` where `E` is `String`, `&str`, `Box<str>`, `Cow<str>`, `Arc<str>`, or `Rc<str>`, in function and trait signatures.

```rust
fn parse_port(input: &str) -> Result<u16, String> { /* ... */ }
```

A string is not an error: it implements no `Error`, carries no `source()` chain, and forces callers to parse prose. Define an error enum or struct. Trait implementations are skipped; the trait declaration is flagged instead. ([Effective Rust, Item 4](https://www.lurklurk.org/effective-rust/errors.html), [API Guidelines C-GOOD-ERR](https://rust-lang.github.io/api-guidelines/interoperability.html#c-good-err))

#### `error_to_string`

`map_err` (and `Option::ok_or`/`ok_or_else`) that turns a typed error into a string.

```rust
let port: u16 = raw.parse().map_err(|e| e.to_string())?;
let port: u16 = raw.parse().map_err(|e| format!("bad port: {e}"))?;
```

The conversion site throws away the type and the cause chain. Add a variant that keeps the cause as `#[source]`, or use `anyhow::Context` in binaries.

#### `opaque_public_error`

A function exported from a library crate returns `Box<dyn Error>`, `anyhow::Error`, or `eyre::Report`.

```rust
pub fn load(path: &Path) -> anyhow::Result<Config> { /* ... */ }
```

Callers cannot match on failure modes. `anyhow` is excellent in binaries and private code; libraries should expose a typed error. Private and `pub(crate)` functions, binaries, and trait impls are not linted.

#### `silent_error_default`

The error of a `Result` is replaced with a value or dropped without being looked at.

```rust
let port = raw.parse::<u16>().unwrap_or_default();
let n = s.parse::<i64>().unwrap_or_else(|_| 0);
let ok: Vec<_> = results.into_iter().filter_map(Result::ok).collect();
let first = lines.next().and_then(|l| l.ok()).flatten();
```

The failure becomes indistinguishable from a legitimate value. Propagate with `?`, or match and handle (or log) the error. Closures that use the error, `Option` methods, and errors that carry no information (`VarError`, `TryFromIntError`, `Infallible`) are not linted. Configure more with `allowed_error_types`.

#### `low_signal_panic_message`

`expect`, `panic!`, and `unreachable!` messages that state no invariant: fewer than three words, or a generic phrase such as `"failed"`, `"should never happen"`, or `"something went wrong"`.

```rust
let config = load().expect("failed");
```

```rust
let re = Regex::new(r"^\d+$").expect("digit-only pattern is a valid regex");
```

With `unwrap()` denied by the profile, `expect` is how code documents an invariant. Its message should say why it cannot fail ([std: "expect messages"](https://doc.rust-lang.org/std/error/index.html#common-message-styles), [burntsushi on unwrap](https://burntsushi.net/unwrap/)). Test code is skipped. Configure `min_words` and `extra_phrases`.

### Types

#### `untyped_value_in_signature`

Function parameters, return types, and struct or enum fields whose type contains `serde_json::Value`/`Map`, `toml::Value`/`Table`, `serde_yaml::Value`/`Mapping`, `rmpv::Value`, or `dyn Any`, at any depth.

```rust
pub fn apply(config: &serde_json::Value) -> Result<(), ApplyError> { /* ... */ }
```

The Rust counterpart of anti-slop's `no-unknown-parameters` and `no-unsafe-dictionary-type`. An untyped value pushes parsing to every consumer. Parse once at the boundary into a typed struct and pass that ([Effective Rust, Item 1](https://www.lurklurk.org/effective-rust/use-types.html)). `#[serde(flatten)]` catch-all fields, `as_any`-style methods, and panic payloads are allowed. Configure `allowed_paths` for genuine pass-through boundaries and `extra_types` for your own dynamic types.

#### `deref_polymorphism`

`Deref` implemented on a multi-field struct (or an enum) to inherit the target's methods.

```rust
struct Admin { user: User, permissions: Vec<Permission> }
impl Deref for Admin { type Target = User; fn deref(&self) -> &User { &self.user } }
```

Deref is for smart pointers; using it as inheritance makes method resolution surprising and leaks the inner API ([Rust Design Patterns: Deref polymorphism](https://rust-unofficial.github.io/patterns/anti_patterns/deref.html), [API Guidelines C-DEREF](https://rust-lang.github.io/api-guidelines/predictability.html#c-deref)). Single-field newtypes, guards with `Drop`, and `Cow`-like enums are not linted.

### Async

#### `blocking_call_in_async`

Blocking `std` calls (`thread::sleep`, `std::fs`, `std::net`, `Command::output`, `mpsc::Receiver::recv`, `Condvar`, ...) and `reqwest::blocking` directly inside an `async fn`, block, or closure.

```rust
async fn load(path: &Path) -> io::Result<String> {
    std::fs::read_to_string(path)
}
```

A blocking call parks the executor thread and stalls every task on it ([Alice Ryhl: Async: What is blocking?](https://ryhl.io/blog/async-what-is-blocking/)). Use the runtime's async API or `spawn_blocking`; code in a synchronous closure is not linted. Add your own blocking APIs with `extra_paths`.

#### `discarded_join_handle`

A `tokio::task::JoinHandle` or `std::thread::JoinHandle` dropped as a statement or with `let _ =`.

```rust
tokio::spawn(sync_inventory(db.clone()));
```

The task's panics and errors vanish and nothing waits for it at shutdown. Keep handles in a `JoinSet` and await them, use `std::thread::scope`, or `drop(handle)` explicitly with a comment when detaching is intended.

### Tests

#### `test_without_assertion`

A `#[test]` function with no evidence of checking anything.

```rust
#[test]
fn parses_config() {
    let _ = parse_config(SAMPLE);
}
```

Tests that assert nothing pass regardless of behaviour, and agents generate them to look productive. Evidence includes any `assert*`/`prop_assert*`/`panic!` macro, `unwrap`/`expect`, `?`, calls to `assert*`/`check`/`verify`/`ensure` helpers (including local helpers up to three calls deep), and `#[should_panic]`.

### Suppression hygiene

#### `blanket_lint_suppression`

Inner `#![allow(..)]`/`#![expect(..)]` at crate or module level, and any `allow`/`expect` naming a lint group (`unused`, `warnings`, `clippy::pedantic`, `clippy::all`, ...).

```rust
#![allow(dead_code, clippy::pedantic)]
```

Suppressing by scope or group hides every future instance too, including real bugs. Clippy's `allow_attributes` does not see inner attributes. Fix the code, or put a narrow `#[expect(lint, reason = "...")]` on the item.

#### `low_signal_lint_reason`

A `reason` that explains nothing: fewer than three words, or placeholders like `"needed"`, `"false positive"`, `"temporary"`, `"it's fine for now"`.

```rust
#[expect(clippy::cast_possible_truncation, reason = "needed")]
```

```rust
#[expect(clippy::cast_possible_truncation, reason = "length is bounded by MAX_FRAME (u16::MAX) in the decoder")]
```

A placeholder reason is an `allow` that tricks `allow_attributes_without_reason`. Reasons that cite an issue or URL are accepted when short. Configure `min_words` and `extra_phrases`.

#### `deny_warnings_in_source`

`#![deny(warnings)]` or `forbid(warnings)` in source code.

Every new compiler or Clippy release can then break your build and your dependents' builds ([Rust Design Patterns: `#![deny(warnings)]`](https://rust-unofficial.github.io/patterns/anti_patterns/deny-warnings.html)). Deny warnings in CI with `RUSTFLAGS="-D warnings"` or `cargo clippy -- -D warnings`, and set levels in `[lints]`.

### Naming

#### `vague_symbol_name`

Types, traits, functions, methods, modules, and constants whose name contains a whole word that names no responsibility: `util(s)`, `helper(s)`, `misc`, `stuff`, `thing(s)`, `manager`, `obj`, `foo`/`bar`/`baz`, `temp`/`tmp`, `dummy`, `my`, `shape` (kept from anti-slop), and `Impl` in type names.

```rust
mod utils;
struct UserManager;
fn do_stuff() {}
```

A vague name hides what code owns or does, and attracts unrelated code over time. Words are matched whole (`Template` and `utilize` are fine). Trait-impl items, FFI names, and test code are skipped. Configure `forbidden_segments` and `extra_segments`.

### Comments and layout

These lints read comments, so they work from the source text. A finding is attached to the innermost item, statement, or expression that contains the comment, so `#[expect]` on that node suppresses it; comments between two items can only be suppressed at crate level.

#### `slop_comment`

Comments that are artifacts of generation rather than documentation, in four categories:

| Category | Examples |
| --- | --- |
| `elision` | `// ...`, `// ... rest of the code`, `// existing code unchanged`, `// your code here`, `// add more variants as needed` |
| `narration` | `// Added error handling`, `// Fixed: off-by-one`, `// Now uses a HashMap`, `// Previously, ...` |
| `hedging` | `// In a real application you would ...`, `// For simplicity, ...`, `// This is a placeholder`, `// Replace this with a real ...` |
| `unfinished` | `// TODO: implement`, `// TODO: add error handling`, `// not yet implemented` |

Elision comments stand in for code that is not there; narration describes the edit, which belongs in the commit message; hedges admit the code is not the real thing; bare unfinished markers record no plan. Comments describe the code as it is. TODOs with context (`TODO(#123): ...`), long explanatory comments, doc comments recording API history, and string literals are not linted. Configure `extra_patterns` and `disabled_categories`.

#### `low_signal_safety_comment`

`SAFETY:` comments with fewer than four words of justification, or only generic clauses such as `this is safe`, `trust me`, or `the pointer is always valid`.

```rust
// SAFETY: this is safe
unsafe { *bytes.as_ptr() }
```

```rust
// SAFETY: `bytes` is non-empty (checked above), so its pointer is valid for a one-byte read.
unsafe { *bytes.as_ptr() }
```

Clippy's `undocumented_unsafe_blocks` only checks that the comment exists; the justification is the proof obligation. Short cross-references such as `Guaranteed by caller.` are flagged too: spell out what the caller guarantees. Configure `min_words`.

#### `missing_item_spacing`

A multi-line item or statement that touches its neighbour without a blank line. The fix is machine-applicable (`cargo dylint --all --fix`).

```rust
struct Config {
    retries: u32,
}
impl Config {
    fn new() -> Self { Self { retries: 3 } }
}
```

The Rust counterpart of anti-slop's `require-readable-spacing`; rustfmt preserves blank lines but never adds them. Comments and attributes travel with the item they precede. Adjacent single-line items such as `use` and `mod x;` groups may stay together. Set `statements = false` to check only items.

## Analysis boundaries

The lints run inside rustc with full type information, but they reason locally: they do not follow values across function boundaries (except `test_without_assertion`'s helper search) and do not resolve what an arbitrary helper function does. Each lint's documentation, and its UI test's `// Not linted:` cases, record what it intentionally leaves alone.

## Sources

The policy draws on [Effective Rust](https://www.lurklurk.org/effective-rust/), the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/), [Rust Design Patterns: anti-patterns](https://rust-unofficial.github.io/patterns/anti_patterns/index.html), the [Clippy lint list](https://rust-lang.github.io/rust-clippy/master/index.html), [RFC 2383 (lint reasons)](https://github.com/rust-lang/rfcs/blob/master/text/2383-lint-reasons.md), writing by [matklad](https://matklad.github.io/2021/05/31/how-to-test.html), [burntsushi](https://burntsushi.net/unwrap/), and [Alice Ryhl](https://ryhl.io/blog/async-what-is-blocking/), the lint configurations of [ruff](https://github.com/astral-sh/ruff/blob/main/Cargo.toml), [uv](https://github.com/astral-sh/uv/blob/main/Cargo.toml), [oxc](https://github.com/oxc-project/oxc/blob/main/Cargo.toml), [rerun](https://github.com/rerun-io/rerun/blob/main/Cargo.toml), [zed](https://github.com/zed-industries/zed/blob/main/Cargo.toml), [biome](https://github.com/biomejs/biome/blob/main/Cargo.toml), [codex](https://github.com/openai/codex/blob/main/codex-rs/Cargo.toml), and [Embark](https://github.com/EmbarkStudios/rust-ecosystem/blob/main/lints.toml), and community reports on LLM-written Rust ([HN: 100K lines of Rust with AI](https://news.ycombinator.com/item?id=48205415), [HN](https://news.ycombinator.com/item?id=43443064), [users.rust-lang.org](https://users.rust-lang.org/t/recent-explosion-in-rust-rewrites-and-reimplementations-with-llms-thoughts/142939)).

## Development

```bash
scripts/check.sh          # build, UI tests, clippy, profile validation, end-to-end demo
scripts/bless.sh <lint>   # regenerate lints/ui/<lint>.stderr, then review the diff
```

Each lint is one module in `lints/src/` with a UI example in `lints/ui/`; `scripts/sync-registry.py` (run from `lints/`) regenerates the module list and `[[example]]` tables. [`demo/`](demo/) is a deliberately sloppy crate, and `scripts/e2e.sh` checks that every lint fires on it. The library tracks the nightly and `clippy_utils` revision pinned in `lints/`; upgrade both with `cargo dylint upgrade lints`. See [AGENTS.md](AGENTS.md).

## License

MIT. Inspired by [anti-slop](https://github.com/dmmulroy/anti-slop) by Dillon Mulroy.
