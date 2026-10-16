use std::collections::HashMap;

use clippy_utils::diagnostics::span_lint_and_then;
use rustc_ast::{AssocItem, Attribute, Block, Crate, HasAttrs, Inline, Item, ItemKind, ModKind, NodeId, Stmt, StmtKind};
use rustc_errors::Applicability;
use rustc_lexer::{FrontmatterAllowed, TokenKind};
use rustc_lint::{EarlyContext, EarlyLintPass, Lint, LintContext, LintStore};
use rustc_session::{declare_lint, impl_lint_pass};
use rustc_span::{BytePos, Pos, Span};
use serde::Deserialize;

declare_lint! {
    /// ### What it does
    /// Requires a blank line between two adjacent items (in a module, `impl`, or
    /// trait) when at least one of them spans multiple lines. With `statements`
    /// enabled (the default) the same rule applies to adjacent statements in a
    /// block: a multi-line statement must be separated from its neighbours by a
    /// blank line. Single-line neighbours (a run of consts or short `let`s) may
    /// stay grouped, and declarations (`use`, `extern crate`, `mod name;`) never
    /// need separating from each other, even when a `use` spans several lines.
    ///
    /// ### Why is this bad?
    /// Blank lines are how code shows its structure. A multi-line function,
    /// `impl`, or statement glued to its neighbour reads as one block, so the
    /// reader has to find the boundary by matching braces. Generated code often
    /// emits definitions back to back.
    ///
    /// ### Known problems
    /// Only the item or statement itself decides whether it is multi-line; its
    /// attributes and doc comments do not (`#[derive(..)]` above a unit struct
    /// does not make it multi-line). A comment between the two neighbours is kept
    /// with the following one: the blank line is inserted after the previous
    /// neighbour's last line. Pairs are skipped when anything other than
    /// whitespace and comments separates them (for example a macro invocation or
    /// a `#[cfg]`-disabled item), and when either comes from a macro expansion.
    ///
    /// ### Example
    /// ```rust,ignore
    /// fn parse(input: &str) -> Config {
    ///     let raw = input.trim();
    ///     let config = Config {
    ///         name: raw.to_owned(),
    ///     };
    ///     config
    /// }
    /// fn print(config: &Config) {
    ///     println!("{config:?}");
    /// }
    /// ```
    /// Use instead:
    /// ```rust,ignore
    /// fn parse(input: &str) -> Config {
    ///     let raw = input.trim();
    ///
    ///     let config = Config {
    ///         name: raw.to_owned(),
    ///     };
    ///
    ///     config
    /// }
    ///
    /// fn print(config: &Config) {
    ///     println!("{config:?}");
    /// }
    /// ```
    ///
    /// ### Configuration
    /// ```toml
    /// [missing_item_spacing]
    /// statements = true   # also check statements inside blocks
    /// ```
    pub MISSING_ITEM_SPACING,
    Warn,
    "multi-line item or statement not separated from its neighbour by a blank line"
}

pub static LINTS: &[&Lint] = &[MISSING_ITEM_SPACING];

pub fn register(lint_store: &mut LintStore) {
    lint_store.register_early_lint_pass(Box::new(|| Box::new(MissingItemSpacing::new())));
}

#[derive(Deserialize)]
struct Config {
    #[serde(default = "default_statements")]
    statements: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            statements: default_statements(),
        }
    }
}

fn default_statements() -> bool {
    true
}

/// One element of an item list or block, reduced to what the check needs.
struct Entry {
    id: NodeId,
    /// The item or statement without its attributes.
    span: Span,
    /// Where its attributes and doc comments start, or `span.lo()`.
    start: BytePos,
    /// A `use`, `extern crate`, or `mod name;` declaration.
    is_declaration: bool,
}

impl Entry {
    fn new(id: NodeId, span: Span, attrs: &[Attribute], is_declaration: bool) -> Option<Self> {
        if span.from_expansion() || span.is_dummy() || attrs.iter().any(|attr| attr.span.from_expansion()) {
            return None;
        }

        let start = attrs
            .iter()
            .map(|attr| attr.span.lo())
            .filter(|lo| *lo < span.lo())
            .min()
            .unwrap_or(span.lo());

        Some(Self {
            id,
            span,
            start,
            is_declaration,
        })
    }

    fn item(item: &Item) -> Option<Self> {
        Self::new(item.id, item.span, &item.attrs, is_declaration(item))
    }

    fn assoc_item(item: &AssocItem) -> Option<Self> {
        Self::new(item.id, item.span, &item.attrs, false)
    }

    fn stmt(stmt: &Stmt) -> Option<Self> {
        if matches!(stmt.kind, StmtKind::Empty) {
            return None;
        }
        let declaration = matches!(&stmt.kind, StmtKind::Item(item) if is_declaration(item));
        Self::new(stmt.id, stmt.span, stmt.attrs(), declaration)
    }
}

fn is_declaration(item: &Item) -> bool {
    matches!(
        item.kind,
        ItemKind::Use(_)
            | ItemKind::ExternCrate(..)
            | ItemKind::Mod(_, _, ModKind::Loaded(_, Inline::No { .. }, _) | ModKind::Unloaded)
    )
}

struct Finding {
    /// The first line of the entry that needs a blank line before it.
    span: Span,
    /// Where to insert the line break: the end of the previous entry's last line.
    insert_at: BytePos,
    what: &'static str,
    previous_multiline: bool,
}

struct MissingItemSpacing {
    config: Config,
    pending: HashMap<NodeId, Finding>,
}

impl MissingItemSpacing {
    fn new() -> Self {
        Self {
            config: dylint_linting::config_or_default(MISSING_ITEM_SPACING.name_lower().as_str()),
            pending: HashMap::new(),
        }
    }

    fn check_list(&mut self, cx: &EarlyContext<'_>, entries: impl Iterator<Item = Option<Entry>>, what: &'static str) {
        let source_map = cx.sess().source_map();
        let entries: Vec<Entry> = entries.flatten().collect();

        for pair in entries.windows(2) {
            let [previous, next] = pair else {
                continue;
            };

            if previous.is_declaration && next.is_declaration {
                continue;
            }

            let previous_multiline = source_map.is_multiline(previous.span);
            if !previous_multiline && !source_map.is_multiline(next.span) {
                continue;
            }

            if previous.span.hi() > next.start {
                continue;
            }

            let gap = Span::with_root_ctxt(previous.span.hi(), next.start);
            let Ok(text) = source_map.span_to_snippet(gap) else {
                continue;
            };

            let Some(offset) = line_break_offset(&text) else {
                continue;
            };

            let first_line = source_map.span_until_char(next.span.with_lo(next.start), '\n');
            self.pending.insert(next.id, Finding {
                span: first_line,
                insert_at: gap.lo() + BytePos::from_usize(offset),
                what,
                previous_multiline,
            });
        }
    }

    fn flush(&mut self, cx: &EarlyContext<'_>, id: NodeId) {
        let Some(finding) = self.pending.remove(&id) else {
            return;
        };

        let what = finding.what;
        span_lint_and_then(
            cx,
            MISSING_ITEM_SPACING,
            finding.span,
            format!("missing blank line before this {what}"),
            |diag| {
                if finding.previous_multiline {
                    diag.note(format!("the previous {what} spans multiple lines"));
                } else {
                    diag.note(format!("this {what} spans multiple lines"));
                }
                diag.span_suggestion_verbose(
                    Span::with_root_ctxt(finding.insert_at, finding.insert_at),
                    "insert a blank line",
                    "\n",
                    Applicability::MachineApplicable,
                );
            },
        );
    }
}

/// For the text between two neighbours, the offset at which inserting `\n`
/// creates a blank line directly after the previous neighbour's last line, or
/// `None` if a blank line already exists, the neighbours share a line, or
/// something other than whitespace and comments separates them.
fn line_break_offset(gap: &str) -> Option<usize> {
    let mut offset = 0;
    let mut first_break = None;

    for token in rustc_lexer::tokenize(gap, FrontmatterAllowed::No) {
        let text = &gap[offset..offset + token.len as usize];

        match token.kind {
            TokenKind::Whitespace => {
                if text.matches('\n').count() > 1 {
                    return None;
                }
                if first_break.is_none()
                    && let Some(index) = text.find('\n')
                {
                    first_break = Some(offset + index);
                }
            },
            TokenKind::LineComment { .. } | TokenKind::BlockComment { terminated: true, .. } => {},
            _ => return None,
        }

        offset += token.len as usize;
    }

    first_break
}

impl_lint_pass!(MissingItemSpacing => [MISSING_ITEM_SPACING]);

impl EarlyLintPass for MissingItemSpacing {
    fn check_crate(&mut self, cx: &EarlyContext<'_>, krate: &Crate) {
        self.check_list(cx, krate.items.iter().map(|item| Entry::item(item)), "item");
    }

    fn check_item(&mut self, cx: &EarlyContext<'_>, item: &Item) {
        self.flush(cx, item.id);

        if item.span.from_expansion() {
            return;
        }

        match &item.kind {
            ItemKind::Mod(_, _, ModKind::Loaded(items, ..)) => {
                self.check_list(cx, items.iter().map(|item| Entry::item(item)), "item");
            },
            ItemKind::Impl(impl_) => {
                self.check_list(cx, impl_.items.iter().map(|item| Entry::assoc_item(item)), "item");
            },
            ItemKind::Trait(trait_) => {
                self.check_list(cx, trait_.items.iter().map(|item| Entry::assoc_item(item)), "item");
            },
            _ => {},
        }
    }

    fn check_impl_item(&mut self, cx: &EarlyContext<'_>, item: &AssocItem) {
        self.flush(cx, item.id);
    }

    fn check_trait_item(&mut self, cx: &EarlyContext<'_>, item: &AssocItem) {
        self.flush(cx, item.id);
    }

    fn check_block(&mut self, cx: &EarlyContext<'_>, block: &Block) {
        if self.config.statements && !block.span.from_expansion() {
            self.check_list(cx, block.stmts.iter().map(Entry::stmt), "statement");
        }
    }

    fn check_stmt(&mut self, cx: &EarlyContext<'_>, stmt: &Stmt) {
        self.flush(cx, stmt.id);
    }
}

#[test]
fn ui() {
    crate::ui_test("missing_item_spacing");
}
