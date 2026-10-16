use clippy_utils::diagnostics::span_lint_and_help;
use rustc_ast::Attribute;
use rustc_lint::{EarlyContext, EarlyLintPass, Lint, LintStore};
use rustc_session::{declare_lint, declare_lint_pass};
use rustc_span::{Symbol, sym};

declare_lint! {
    /// ### What it does
    /// Flags `deny(warnings)` and `forbid(warnings)` written in source, at crate
    /// level (`#![deny(warnings)]`) or on an item.
    ///
    /// ### Why is this bad?
    /// The set of warnings grows with every compiler and Clippy release. A crate
    /// that denies all warnings in source stops building for downstream users as
    /// soon as a new release adds a warning, even though nothing is wrong with
    /// it (<https://rust-unofficial.github.io/patterns/anti_patterns/deny-warnings.html>).
    /// Whether warnings fail the build is a CI policy, not a property of the code.
    ///
    /// ### Known problems
    /// Only the `warnings` pseudo-lint is flagged; denying specific lints or
    /// groups is fine. `#![doc(test(attr(deny(warnings))))]`, which only affects
    /// doctests, is not flagged.
    ///
    /// ### Example
    /// ```rust,ignore
    /// #![deny(warnings)]
    /// ```
    /// Use instead:
    /// ```sh
    /// RUSTFLAGS="-D warnings" cargo build   # or: cargo clippy -- -D warnings
    /// ```
    pub DENY_WARNINGS_IN_SOURCE,
    Warn,
    "`deny(warnings)` in source breaks builds whenever the toolchain adds a warning"
}

pub static LINTS: &[&Lint] = &[DENY_WARNINGS_IN_SOURCE];

pub fn register(lint_store: &mut LintStore) {
    lint_store.register_early_lint_pass(Box::new(|| Box::new(DenyWarningsInSource)));
}

declare_lint_pass!(DenyWarningsInSource => [DENY_WARNINGS_IN_SOURCE]);

impl EarlyLintPass for DenyWarningsInSource {
    fn check_attribute(&mut self, cx: &EarlyContext<'_>, attr: &Attribute) {
        let Some(level) = attr.name() else {
            return;
        };

        if !matches!(level, sym::deny | sym::forbid) || attr.span.from_expansion() {
            return;
        }

        let Some(items) = attr.meta_item_list() else {
            return;
        };

        for item in &items {
            if item.meta_item().is_some_and(|meta| meta.is_word() && meta.path == Symbol::intern("warnings")) {
                span_lint_and_help(
                    cx,
                    DENY_WARNINGS_IN_SOURCE,
                    item.span(),
                    format!("`{level}(warnings)` in source fails the build on any new compiler or clippy warning"),
                    None,
                    "enforce it in CI instead (`RUSTFLAGS=\"-D warnings\"` or `cargo clippy -- -D warnings`), \
                     or set specific lint levels in the `[lints]` table of Cargo.toml",
                );
            }
        }
    }
}

#[test]
fn ui() {
    crate::ui_test("deny_warnings_in_source");
}
