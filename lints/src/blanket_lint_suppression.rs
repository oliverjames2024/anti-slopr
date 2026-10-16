use clippy_utils::diagnostics::span_lint_and_help;
use rustc_ast::{AttrStyle, Attribute, Crate, Item, ItemKind, MetaItemInner};
use rustc_lint::{EarlyContext, EarlyLintPass, Lint, LintStore};
use rustc_session::{declare_lint, declare_lint_pass};
use rustc_span::{Symbol, sym};

declare_lint! {
    /// ### What it does
    /// Flags lint suppressions with a blanket scope:
    ///
    /// - an inner `#![allow(..)]` or `#![expect(..)]` at crate or module level, and
    /// - any `allow`/`expect` (inner or outer) naming a lint *group* such as
    ///   `warnings`, `unused`, `clippy::all` or `clippy::pedantic`.
    ///
    /// ### Why is this bad?
    /// A suppression at crate or module scope, or of a whole group, hides every
    /// future instance of the lint, including the ones that are real bugs. It is
    /// the quickest way to make a warning go away without understanding it,
    /// which is exactly what coding agents tend to do.
    ///
    /// ### Known problems
    /// Some crate-level suppressions are deliberate (for example a style lint the
    /// project has decided against). Prefer configuring those in the `[lints]`
    /// table of `Cargo.toml`, which states the policy in one obvious place.
    /// Inner attributes inside function bodies are only flagged when they name a
    /// group. Attributes produced by macros are ignored. An `allow(warnings)`
    /// also silences this lint's own report about it (lint levels apply to the
    /// attribute's own item), so `#![allow(warnings)]` cannot be reported.
    ///
    /// ### Example
    /// ```rust,ignore
    /// #![allow(dead_code)]
    ///
    /// #[allow(clippy::pedantic)]
    /// fn parse(input: &str) -> u32 { .. }
    /// ```
    /// Use instead:
    /// ```rust,ignore
    /// #[expect(clippy::cast_possible_truncation, reason = "input is at most 4 ASCII digits")]
    /// fn parse(input: &str) -> u32 { .. }
    /// ```
    pub BLANKET_LINT_SUPPRESSION,
    Warn,
    "lint suppression that covers a whole crate, module, or lint group"
}

pub static LINTS: &[&Lint] = &[BLANKET_LINT_SUPPRESSION];

pub fn register(lint_store: &mut LintStore) {
    lint_store.register_early_lint_pass(Box::new(|| Box::new(BlanketLintSuppression)));
}

/// Lint groups whose suppression silences many unrelated lints at once.
const LINT_GROUPS: &[&str] = &[
    "warnings",
    "unused",
    "nonstandard_style",
    "rust_2018_idioms",
    "rust_2018_compatibility",
    "rust_2021_compatibility",
    "rust_2024_compatibility",
    "future_incompatible",
    "let_underscore",
    "clippy::all",
    "clippy::pedantic",
    "clippy::nursery",
    "clippy::restriction",
    "clippy::style",
    "clippy::complexity",
    "clippy::perf",
    "clippy::correctness",
    "clippy::suspicious",
    "clippy::cargo",
    "rustdoc::all",
];

const HELP: &str = "fix the code, or put a narrow `#[expect(lint, reason = \"..\")]` on the specific item";
const CRATE_HELP: &str = "fix the code, or put a narrow `#[expect(lint, reason = \"..\")]` on the specific item; \
     a deliberate crate-wide policy belongs in the `[lints]` table of Cargo.toml, next to the rest of the policy";

declare_lint_pass!(BlanketLintSuppression => [BLANKET_LINT_SUPPRESSION]);

impl EarlyLintPass for BlanketLintSuppression {
    fn check_attribute(&mut self, cx: &EarlyContext<'_>, attr: &Attribute) {
        let Some((level, items)) = suppression(attr) else {
            return;
        };

        for (item, name) in lint_names(&items) {
            if LINT_GROUPS.contains(&name.as_str()) {
                span_lint_and_help(
                    cx,
                    BLANKET_LINT_SUPPRESSION,
                    item.span(),
                    format!(
                        "`{}` suppresses every lint in the `{name}` group",
                        render(attr.style, level, &name)
                    ),
                    None,
                    HELP,
                );
            }
        }
    }

    fn check_crate(&mut self, cx: &EarlyContext<'_>, krate: &Crate) {
        check_scope(cx, &krate.attrs, "crate");
    }

    fn check_item(&mut self, cx: &EarlyContext<'_>, item: &Item) {
        if let ItemKind::Mod(_, ident, _) = item.kind
            && !item.span.from_expansion()
        {
            check_scope(cx, &item.attrs, &format!("module `{ident}`"));
        }
    }
}

/// Reports inner suppressions of individual lints in `attrs`, which apply to
/// the whole `scope`. Groups are reported by `check_attribute` instead.
fn check_scope(cx: &EarlyContext<'_>, attrs: &[Attribute], scope: &str) {
    for attr in attrs.iter().filter(|attr| attr.style == AttrStyle::Inner) {
        let Some((level, items)) = suppression(attr) else {
            continue;
        };

        for (item, name) in lint_names(&items) {
            if !LINT_GROUPS.contains(&name.as_str()) {
                span_lint_and_help(
                    cx,
                    BLANKET_LINT_SUPPRESSION,
                    item.span(),
                    format!(
                        "`{}` suppresses `{name}` for the whole {scope}",
                        render(attr.style, level, &name)
                    ),
                    None,
                    if scope == "crate" { CRATE_HELP } else { HELP },
                );
            }
        }
    }
}

/// The level (`allow` or `expect`) and arguments of a hand-written suppression attribute.
fn suppression(attr: &Attribute) -> Option<(Symbol, Vec<MetaItemInner>)> {
    let level = attr.name()?;
    if !matches!(level, sym::allow | sym::expect) || attr.span.from_expansion() {
        return None;
    }
    Some((level, attr.meta_item_list()?.into_iter().collect()))
}

/// The lint paths named by a lint attribute, such as `dead_code` or `clippy::all`,
/// skipping the `reason = ".."` argument.
fn lint_names(items: &[MetaItemInner]) -> impl Iterator<Item = (&MetaItemInner, String)> {
    items.iter().filter_map(|item| {
        let meta = item.meta_item().filter(|meta| meta.is_word())?;
        let name = meta
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.as_str())
            .collect::<Vec<_>>()
            .join("::");
        Some((item, name))
    })
}

fn render(style: AttrStyle, level: Symbol, name: &str) -> String {
    let bang = if style == AttrStyle::Inner { "!" } else { "" };
    format!("#{bang}[{level}({name})]")
}

#[test]
fn ui() {
    crate::ui_test("blanket_lint_suppression");
}
