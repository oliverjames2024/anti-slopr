use clippy_utils::diagnostics::span_lint_and_help;
use clippy_utils::macros::{is_panic, root_macro_call_first_node};
use clippy_utils::source::snippet_opt;
use clippy_utils::{is_in_test, sym};
use rustc_ast::LitKind;
use rustc_hir::{Expr, ExprKind};
use rustc_lint::{LateContext, LateLintPass, Lint, LintStore};
use rustc_middle::ty;
use rustc_session::{declare_lint, impl_lint_pass};
use rustc_span::Span;
use serde::Deserialize;

declare_lint! {
    /// ### What it does
    /// Flags string-literal messages of `.expect(..)`/`.expect_err(..)` on
    /// `Option`/`Result`, `panic!(..)`, and `unreachable!(..)` that state no
    /// invariant: fewer than `min_words` words, or a generic phrase such as
    /// "failed", "should never happen", "impossible", or "something went wrong".
    ///
    /// ### Why is this bad?
    /// The message of an `expect` is the documentation of why the value cannot
    /// be absent; the message of a panic is the only clue whoever reads the
    /// crash report gets. "should not happen" says neither what was expected
    /// nor why. The standard library recommends `expect` messages that describe
    /// the expectation, e.g. `.expect("env variable `PATH` should be set")`.
    ///
    /// ### Known problems
    /// Only literal messages are checked; messages with format arguments are
    /// assumed to carry context. Code in tests is skipped, and so are
    /// `assert!`-family and `todo!` messages.
    ///
    /// ### Example
    /// ```rust,ignore
    /// let re = Regex::new(r"^\d+$").expect("failed");
    /// let first = items.first().expect("should never happen");
    /// unreachable!("impossible");
    /// ```
    /// Use instead:
    /// ```rust,ignore
    /// let re = Regex::new(r"^\d+$").expect("digit-only regex literal should be valid");
    /// let first = items.first().expect("`items` is checked to be non-empty above");
    /// unreachable!("the tokenizer only produces ASCII digits here");
    /// ```
    ///
    /// ### Configuration
    /// ```toml
    /// [low_signal_panic_message]
    /// min_words = 3                       # messages with fewer words are flagged
    /// extra_phrases = ["todo fix", "bug"] # more generic phrases (case-insensitive)
    /// ```
    pub LOW_SIGNAL_PANIC_MESSAGE,
    Warn,
    "`expect`/`panic!`/`unreachable!` message that does not state the violated invariant"
}

pub static LINTS: &[&Lint] = &[LOW_SIGNAL_PANIC_MESSAGE];

pub fn register(lint_store: &mut LintStore) {
    lint_store.register_late_lint_pass(Box::new(|_| Box::new(LowSignalPanicMessage::new())));
}

/// Compared after lowercasing, trimming surrounding punctuation, and collapsing whitespace.
const GENERIC_PHRASES: &[&str] = &[
    "failed",
    "error",
    "should not happen",
    "should never happen",
    "this should not happen",
    "this should never happen",
    "impossible",
    "unreachable",
    "oops",
    "unwrap failed",
    "expect failed",
    "bad",
    "invalid",
    "something went wrong",
    "no value",
    "none",
    "is none",
    "failed to unwrap",
    "should work",
    "should be ok",
    "must succeed",
    "can't fail",
    "cannot fail",
    "will never fail",
];

#[derive(Deserialize)]
#[serde(default)]
struct Config {
    min_words: usize,
    extra_phrases: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            min_words: 3,
            extra_phrases: Vec::new(),
        }
    }
}

struct LowSignalPanicMessage {
    min_words: usize,
    extra_phrases: Vec<String>,
}

impl LowSignalPanicMessage {
    fn new() -> Self {
        let config: Config = dylint_linting::config_or_default(LOW_SIGNAL_PANIC_MESSAGE.name_lower().as_str());

        Self {
            min_words: config.min_words,
            extra_phrases: config.extra_phrases.iter().map(|phrase| normalize(phrase)).collect(),
        }
    }

    fn is_low_signal(&self, message: &str) -> bool {
        let normalized = normalize(message);
        let words = normalized
            .split(' ')
            .filter(|word| word.chars().any(char::is_alphanumeric))
            .count();

        words < self.min_words
            || GENERIC_PHRASES.contains(&normalized.as_str())
            || self.extra_phrases.contains(&normalized)
    }

    fn report(&self, cx: &LateContext<'_>, span: Span, what: &str, message: &str, help: &'static str) {
        if self.is_low_signal(message) {
            span_lint_and_help(
                cx,
                LOW_SIGNAL_PANIC_MESSAGE,
                span,
                format!("{what} message {message:?} does not state the violated invariant"),
                None,
                help,
            );
        }
    }
}

impl_lint_pass!(LowSignalPanicMessage => [LOW_SIGNAL_PANIC_MESSAGE]);

impl<'tcx> LateLintPass<'tcx> for LowSignalPanicMessage {
    fn check_expr(&mut self, cx: &LateContext<'tcx>, expr: &'tcx Expr<'tcx>) {
        if let ExprKind::MethodCall(segment, receiver, [arg], _) = expr.kind
            && matches!(segment.ident.name, sym::expect | sym::expect_err)
            && !expr.span.from_expansion()
            && let ExprKind::Lit(lit) = arg.kind
            && let LitKind::Str(message, _) = lit.node
            && !arg.span.from_expansion()
            && let ty::Adt(adt, _) = cx.typeck_results().expr_ty(receiver).kind()
            && matches!(cx.tcx.get_diagnostic_name(adt.did()), Some(sym::Option | sym::Result))
            && !is_in_test(cx.tcx, expr.hir_id)
        {
            self.report(
                cx,
                arg.span,
                &format!("`{}`", segment.ident.name),
                message.as_str(),
                "say why this cannot fail, e.g. `.expect(\"digit-only regex literal should be valid\")`",
            );
            return;
        }

        let Some(macro_call) = root_macro_call_first_node(cx, expr) else {
            return;
        };

        let name = if is_panic(cx, macro_call.def_id) {
            "panic!"
        } else if cx.tcx.is_diagnostic_item(sym::unreachable_macro, macro_call.def_id) {
            "unreachable!"
        } else {
            return;
        };

        // The macro call itself must be written by the user, not produced by another macro.
        if macro_call.span.from_expansion() || is_in_test(cx.tcx, expr.hir_id) {
            return;
        }

        let Some(message) = snippet_opt(cx, macro_call.span).and_then(|snippet| literal_macro_message(&snippet)) else {
            return;
        };

        self.report(
            cx,
            macro_call.span,
            &format!("`{name}`"),
            &message,
            "describe the invariant that was violated and the relevant state, e.g. `unreachable!(\"the tokenizer only produces ASCII digits here\")`",
        );
    }
}

/// Lowercases, trims surrounding punctuation and whitespace, and collapses inner whitespace.
fn normalize(message: &str) -> String {
    message
        .trim_matches(|character: char| !character.is_alphanumeric())
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// The message of `name!("literal")`, if the macro's only argument is a string
/// literal without format placeholders.
fn literal_macro_message(snippet: &str) -> Option<String> {
    let (_, arguments) = snippet.split_once('!')?;
    let arguments = arguments.trim();
    let inner = arguments
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
        .or_else(|| arguments.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')))
        .or_else(|| arguments.strip_prefix('{').and_then(|rest| rest.strip_suffix('}')))?;
    let inner = inner.trim();
    let literal = inner.strip_suffix(',').unwrap_or(inner).trim_end();

    let content = parse_string_literal(literal)?;

    // `{name}` and `{}` placeholders mean the message carries runtime context.
    let without_escapes = content.replace("{{", "").replace("}}", "");
    if without_escapes.contains('{') {
        return None;
    }

    Some(content.replace("{{", "{").replace("}}", "}"))
}

/// The contents of a single plain (`"..."`) or raw (`r#"..."#`) string literal
/// spanning all of `literal`. Escapes are kept verbatim except `\"` and `\\`.
fn parse_string_literal(literal: &str) -> Option<String> {
    if let Some(raw) = literal.strip_prefix('r') {
        let hashes = raw.len() - raw.trim_start_matches('#').len();
        let fence = "#".repeat(hashes);
        let body = raw
            .strip_prefix(&fence)?
            .strip_prefix('"')?
            .strip_suffix(&fence)?
            .strip_suffix('"')?;
        return Some(body.to_owned());
    }

    let body = literal.strip_prefix('"')?.strip_suffix('"')?;
    let mut content = String::with_capacity(body.len());
    let mut characters = body.chars();

    while let Some(character) = characters.next() {
        match character {
            '\\' => match characters.next()? {
                escaped @ ('"' | '\\') => content.push(escaped),
                'n' | 't' | 'r' => content.push(' '),
                other => {
                    content.push('\\');
                    content.push(other);
                },
            },
            // An unescaped quote ends the literal early: more tokens follow.
            '"' => return None,
            other => content.push(other),
        }
    }

    Some(content)
}

#[test]
fn ui() {
    crate::ui_test_with_flags("low_signal_panic_message", &["--test"]);
}
