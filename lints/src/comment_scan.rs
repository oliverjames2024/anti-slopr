//! Comment discovery for the textual comment lints (`slop_comment`,
//! `low_signal_safety_comment`).
//!
//! Comments are not part of the AST, so these lints lex the crate's own module
//! files with `rustc_lexer`. To keep `#[allow]`/`#[expect]` on an enclosing item,
//! statement, or expression working, a finding is not emitted from `check_crate`
//! directly: it is attached to the innermost attributed AST node whose source range
//! contains it, and emitted when the early lint visitor reaches that node, where
//! the node's lint levels are in effect.

use std::collections::HashMap;
use std::ops::Range;

use rustc_ast::visit::{self, AssocCtxt, Visitor};
use rustc_ast::{Arm, AssocItem, Attribute, Crate, Expr, HasAttrs, Inline, Item, ItemKind, ModKind, NodeId, Stmt, Variant};
use rustc_lexer::{FrontmatterAllowed, TokenKind};
use rustc_lint::{EarlyContext, LintContext};
use rustc_span::def_id::LOCAL_CRATE;
use rustc_span::{BytePos, FileName, Pos, Span};

/// A run of comments that reads as one unit: consecutive line comments of the
/// same kind with nothing but a single line break between them, or one block
/// comment.
pub struct CommentGroup {
    /// Whether the group is a doc comment (`///`, `//!`, `/** */`, `/*! */`).
    pub doc: bool,
    pub lines: Vec<CommentLine>,
}

pub struct CommentLine {
    /// The comment text without its markers and surrounding whitespace.
    pub text: String,
    /// The text of the line within the source file.
    pub span: Span,
    /// Whether the line is inside (or is a fence of) a fenced code block.
    pub in_code_block: bool,
}

/// Every comment in the local crate's module files.
pub fn comment_groups(cx: &EarlyContext<'_>, krate: &Crate) -> Vec<CommentGroup> {
    let source_map = cx.sess().source_map();

    let mut module_files = vec![source_map.lookup_source_file(krate.spans.inner_span.lo()).start_pos];
    let mut collector = ModuleFiles(Vec::new());
    visit::walk_crate(&mut collector, krate);
    module_files.extend(
        collector
            .0
            .iter()
            .map(|span| source_map.lookup_source_file(span.lo()).start_pos),
    );

    let mut groups = Vec::new();

    for file in source_map.files().iter() {
        if file.cnum != LOCAL_CRATE
            || !matches!(file.name, FileName::Real(_))
            || !module_files.contains(&file.start_pos)
        {
            continue;
        }

        if let Some(src) = &file.src {
            lex_comments(src, file.start_pos, &mut groups);
        }
    }

    groups
}

/// The source ranges of outlined (`mod foo;`) module bodies.
struct ModuleFiles(Vec<Span>);

impl<'ast> Visitor<'ast> for ModuleFiles {
    fn visit_item(&mut self, item: &'ast Item) {
        if let ItemKind::Mod(_, _, ModKind::Loaded(_, Inline::No { .. }, spans)) = &item.kind {
            self.0.push(spans.inner_span);
        }

        visit::walk_item(self, item);
    }
}

fn lex_comments(src: &str, file_start: BytePos, groups: &mut Vec<CommentGroup>) {
    let mut offset = rustc_lexer::strip_shebang(src).unwrap_or(0);
    let span_of = |range: Range<usize>| {
        Span::with_root_ctxt(
            file_start + BytePos::from_usize(range.start),
            file_start + BytePos::from_usize(range.end),
        )
    };

    // A line comment joins the current group only if nothing but whitespace
    // with a single line break separates it from the previous one.
    let mut joinable = false;
    let mut current: Option<(TokenKind, CommentGroup)> = None;

    for token in rustc_lexer::tokenize(&src[offset..], FrontmatterAllowed::No) {
        let range = offset..offset + token.len as usize;
        let text = &src[range.clone()];
        offset = range.end;

        match token.kind {
            TokenKind::Whitespace => {
                if text.matches('\n').count() > 1 {
                    joinable = false;
                }
            },
            TokenKind::LineComment { doc_style } => {
                let line = CommentLine {
                    text: strip_line_comment(text).to_owned(),
                    span: span_of(range),
                    in_code_block: false,
                };

                match &mut current {
                    Some((kind, group)) if joinable && *kind == token.kind => group.lines.push(line),
                    _ => {
                        groups.extend(current.take().map(|(_, group)| group));
                        current = Some((token.kind, CommentGroup {
                            doc: doc_style.is_some(),
                            lines: vec![line],
                        }));
                    },
                }

                joinable = true;
            },
            TokenKind::BlockComment { doc_style, terminated } => {
                groups.extend(current.take().map(|(_, group)| group));
                groups.push(block_comment_group(text, range.start, doc_style.is_some(), terminated, &span_of));
                joinable = false;
            },
            _ => {
                groups.extend(current.take().map(|(_, group)| group));
                joinable = false;
            },
        }
    }

    groups.extend(current.map(|(_, group)| group));

    for group in groups.iter_mut() {
        mark_code_blocks(group);
    }
}

fn strip_line_comment(text: &str) -> &str {
    let body = text.strip_prefix("//").unwrap_or(text);
    let body = body.strip_prefix('!').unwrap_or_else(|| body.trim_start_matches('/'));
    body.trim()
}

fn block_comment_group(
    text: &str,
    start: usize,
    doc: bool,
    terminated: bool,
    span_of: &impl Fn(Range<usize>) -> Span,
) -> CommentGroup {
    let marker_len = if doc { 3 } else { 2 };
    let end = if terminated { text.len() - 2 } else { text.len() };
    let body = text.get(marker_len..end).unwrap_or_default();
    let multiline = body.contains('\n');

    let mut lines = Vec::new();
    let mut line_start = start + marker_len;

    for raw in body.split('\n') {
        let mut content = raw.trim_start();
        // ` * text` continuation lines.
        if multiline && let Some(rest) = content.strip_prefix('*') {
            content = rest.trim_start();
        }
        let content = content.trim_end();

        let content_start = if content.is_empty() {
            line_start
        } else {
            line_start + (content.as_ptr() as usize - raw.as_ptr() as usize)
        };

        lines.push(CommentLine {
            text: content.to_owned(),
            span: span_of(content_start..content_start + content.len()),
            in_code_block: false,
        });

        line_start += raw.len() + 1;
    }

    CommentGroup { doc, lines }
}

fn mark_code_blocks(group: &mut CommentGroup) {
    let mut in_block = false;

    for line in &mut group.lines {
        let fence = line.text.starts_with("```") || line.text.starts_with("~~~");
        line.in_code_block = in_block || fence;
        if fence {
            in_block = !in_block;
        }
    }
}

/// The innermost attributed AST nodes, by source range, so that a finding at a
/// span can be emitted while the lint levels of its enclosing node are active.
pub struct LintScopes {
    scopes: Vec<(BytePos, BytePos, NodeId)>,
}

impl LintScopes {
    pub fn new(krate: &Crate) -> Self {
        let mut collector = ScopeCollector(Vec::new());
        visit::walk_crate(&mut collector, krate);
        Self { scopes: collector.0 }
    }

    /// The innermost attributed node containing `span`, or `None` if only the
    /// crate-level attributes apply.
    pub fn innermost(&self, span: Span) -> Option<NodeId> {
        self.scopes
            .iter()
            .filter(|(lo, hi, _)| *lo <= span.lo() && span.hi() <= *hi)
            // `min_by_key` keeps the first minimum; scopes are recorded parent
            // first, so reverse to prefer the innermost of equally sized ones.
            .rev()
            .min_by_key(|(lo, hi, _)| *hi - *lo)
            .map(|(_, _, id)| *id)
    }
}

struct ScopeCollector(Vec<(BytePos, BytePos, NodeId)>);

impl ScopeCollector {
    fn record(&mut self, id: NodeId, span: Span, attrs: &[Attribute]) {
        if attrs.is_empty() || span.from_expansion() {
            return;
        }

        let lo = attrs
            .iter()
            .map(|attr| attr.span)
            .filter(|attr| !attr.from_expansion() && attr.lo() < span.lo())
            .map(Span::lo)
            .min()
            .unwrap_or(span.lo());
        self.0.push((lo, span.hi(), id));
    }
}

impl<'ast> Visitor<'ast> for ScopeCollector {
    fn visit_item(&mut self, item: &'ast Item) {
        self.record(item.id, item.span, &item.attrs);
        // The body of an outlined module lives in another file.
        if let ItemKind::Mod(_, _, ModKind::Loaded(_, Inline::No { .. }, spans)) = &item.kind
            && !item.attrs.is_empty()
        {
            self.0.push((spans.inner_span.lo(), spans.inner_span.hi(), item.id));
        }
        visit::walk_item(self, item);
    }

    fn visit_assoc_item(&mut self, item: &'ast AssocItem, ctxt: AssocCtxt) {
        self.record(item.id, item.span, &item.attrs);
        visit::walk_assoc_item(self, item, ctxt);
    }

    fn visit_stmt(&mut self, stmt: &'ast Stmt) {
        self.record(stmt.id, stmt.span, stmt.attrs());
        visit::walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, expr: &'ast Expr) {
        self.record(expr.id, expr.span, &expr.attrs);
        visit::walk_expr(self, expr);
    }

    fn visit_arm(&mut self, arm: &'ast Arm) {
        self.record(arm.id, arm.span, &arm.attrs);
        visit::walk_arm(self, arm);
    }

    fn visit_variant(&mut self, variant: &'ast Variant) {
        self.record(variant.id, variant.span, &variant.attrs);
        visit::walk_variant(self, variant);
    }
}

/// Findings waiting for the early lint visitor to reach their node.
pub struct Deferred<T> {
    pending: HashMap<NodeId, Vec<T>>,
}

impl<T> Default for Deferred<T> {
    fn default() -> Self {
        Self { pending: HashMap::new() }
    }
}

impl<T> Deferred<T> {
    pub fn push(&mut self, id: NodeId, finding: T) {
        self.pending.entry(id).or_default().push(finding);
    }

    pub fn take(&mut self, id: NodeId) -> Vec<T> {
        self.pending.remove(&id).unwrap_or_default()
    }
}

/// Implements the early-pass callbacks that flush deferred findings for every
/// node kind `LintScopes` records, by calling the pass's own
/// `fn flush(&mut self, cx: &EarlyContext<'_>, id: NodeId)`.
macro_rules! flush_deferred_findings {
    () => {
        fn check_item(&mut self, cx: &rustc_lint::EarlyContext<'_>, item: &rustc_ast::Item) {
            self.flush(cx, item.id);
        }

        fn check_impl_item(&mut self, cx: &rustc_lint::EarlyContext<'_>, item: &rustc_ast::AssocItem) {
            self.flush(cx, item.id);
        }

        fn check_trait_item(&mut self, cx: &rustc_lint::EarlyContext<'_>, item: &rustc_ast::AssocItem) {
            self.flush(cx, item.id);
        }

        fn check_stmt(&mut self, cx: &rustc_lint::EarlyContext<'_>, stmt: &rustc_ast::Stmt) {
            self.flush(cx, stmt.id);
        }

        fn check_expr(&mut self, cx: &rustc_lint::EarlyContext<'_>, expr: &rustc_ast::Expr) {
            self.flush(cx, expr.id);
        }

        fn check_arm(&mut self, cx: &rustc_lint::EarlyContext<'_>, arm: &rustc_ast::Arm) {
            self.flush(cx, arm.id);
        }

        fn check_variant(&mut self, cx: &rustc_lint::EarlyContext<'_>, variant: &rustc_ast::Variant) {
            self.flush(cx, variant.id);
        }
    };
}

pub(crate) use flush_deferred_findings;
