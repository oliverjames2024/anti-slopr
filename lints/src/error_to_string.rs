use clippy_utils::diagnostics::span_lint_and_help;
use clippy_utils::sym;
use rustc_hir::{Expr, ExprKind};
use rustc_lint::{LateContext, LateLintPass, Lint, LintStore};
use rustc_middle::ty;
use rustc_session::{declare_lint, declare_lint_pass};

use crate::stringly_typed_error::{is_string_like, result_err_ty};

declare_lint! {
    /// ### What it does
    /// Flags conversions that turn an error, or a missing value, into a string
    /// error: `Result::map_err` whose closure or function returns a `String`,
    /// `&str`, `Box<str>`, `Cow<str>`, or `Arc<str>`/`Rc<str>` (for example
    /// `.map_err(|e| e.to_string())`, `.map_err(|e| format!("ctx: {e}"))`,
    /// `.map_err(ToString::to_string)`), and `Option::ok_or`/`ok_or_else`
    /// producing such a string.
    ///
    /// ### Why is this bad?
    /// Stringifying an error throws away its type and its `source()` chain:
    /// callers can no longer match on the failure, and reports lose the
    /// underlying cause. This lint catches the conversion site itself, which
    /// `stringly_typed_error` misses when the enclosing signature is generic or
    /// the string is converted again later (for example by `?` into
    /// `Box<dyn Error>`).
    ///
    /// ### Known problems
    /// `map_err` from an error that is already a string is not flagged; the
    /// problem lies upstream. Code that also returns `Result<_, String>` is
    /// reported by `stringly_typed_error` as well.
    ///
    /// ### Example
    /// ```rust,ignore
    /// let port: u16 = input.parse().map_err(|e| format!("invalid port: {e}"))?;
    /// let user = users.get(id).ok_or("no such user")?;
    /// ```
    /// Use instead:
    /// ```rust,ignore
    /// let port: u16 = input.parse().map_err(|source| ConfigError::InvalidPort { input, source })?;
    /// let user = users.get(id).ok_or(LookupError::UnknownUser(id))?;
    /// // or, in a binary: `input.parse().context("invalid port")?`
    /// ```
    pub ERROR_TO_STRING,
    Warn,
    "error converted into a string, losing its type and source chain"
}

pub static LINTS: &[&Lint] = &[ERROR_TO_STRING];

pub fn register(lint_store: &mut LintStore) {
    lint_store.register_late_lint_pass(Box::new(|_| Box::new(ErrorToString)));
}

declare_lint_pass!(ErrorToString => [ERROR_TO_STRING]);

impl<'tcx> LateLintPass<'tcx> for ErrorToString {
    fn check_expr(&mut self, cx: &LateContext<'tcx>, expr: &'tcx Expr<'tcx>) {
        let ExprKind::MethodCall(segment, receiver, [_], call_span) = expr.kind else {
            return;
        };

        if expr.span.from_expansion() {
            return;
        }

        let method = segment.ident.name;
        let typeck = cx.typeck_results();
        let receiver_ty = typeck.expr_ty(receiver);

        let ty::Adt(receiver_adt, _) = receiver_ty.kind() else {
            return;
        };
        let receiver_kind = cx.tcx.get_diagnostic_name(receiver_adt.did());

        let Some(output_err) = result_err_ty(cx, typeck.expr_ty(expr)) else {
            return;
        };

        if !is_string_like(cx, output_err) {
            return;
        }

        match (receiver_kind, method) {
            (Some(sym::Result), sym::map_err) => {
                let Some(input_err) = result_err_ty(cx, receiver_ty) else {
                    return;
                };

                // Already a string: nothing is lost here, the problem is upstream.
                if is_string_like(cx, input_err) {
                    return;
                }

                span_lint_and_help(
                    cx,
                    ERROR_TO_STRING,
                    call_span,
                    format!("`map_err` turns the `{input_err}` error into `{output_err}`, dropping its type and source chain"),
                    None,
                    "add a variant to a dedicated error type that keeps the cause as `#[source]`, or use `anyhow::Context` in a binary",
                );
            },
            (Some(sym::Option), sym::ok_or | sym::ok_or_else) => {
                span_lint_and_help(
                    cx,
                    ERROR_TO_STRING,
                    call_span,
                    format!("`{method}` turns `None` into a `{output_err}` error"),
                    None,
                    "return a dedicated error type (for example an enum variant naming the missing value), or use `anyhow::Context` in a binary",
                );
            },
            _ => {},
        }
    }
}

#[test]
fn ui() {
    crate::ui_test("error_to_string");
}
