use clippy_utils::diagnostics::span_lint_and_help;
use rustc_hir::{Expr, LetStmt, PatKind, Stmt, StmtKind};
use rustc_lint::{LateContext, LateLintPass, Lint, LintStore};
use rustc_session::{declare_lint, declare_lint_pass};
use rustc_span::Span;

use crate::util::is_adt_with_path;

declare_lint! {
    /// ### What it does
    /// Flags a spawned task or thread whose `JoinHandle` is thrown away: an
    /// expression statement such as `tokio::spawn(work());` or
    /// `std::thread::spawn(..);`, or a `let _ = ..` binding of a handle.
    /// Handles of `tokio::task::JoinHandle` and `std::thread::JoinHandle` are
    /// recognised.
    ///
    /// ### Why is this bad?
    /// Dropping the handle detaches the task. If it panics or returns an error,
    /// nobody finds out; nothing waits for it at shutdown, so its work may be
    /// cut off half-way. Fire-and-forget hides failures.
    ///
    /// ### Known problems
    /// Detaching is sometimes intended (a background reaper that lives for the
    /// whole process). Say so with an explicit `drop(handle)` and a comment;
    /// explicit `drop(..)` calls are not reported. Scoped threads
    /// (`std::thread::scope`) are joined automatically and are not reported.
    ///
    /// ### Example
    /// ```rust,ignore
    /// tokio::spawn(flush_metrics(client));
    /// ```
    /// Use instead:
    /// ```rust,ignore
    /// let mut tasks = tokio::task::JoinSet::new();
    /// tasks.spawn(flush_metrics(client));
    /// // ..
    /// while let Some(result) = tasks.join_next().await {
    ///     result??;
    /// }
    /// ```
    pub DISCARDED_JOIN_HANDLE,
    Warn,
    "the `JoinHandle` of a spawned task or thread is dropped"
}

pub static LINTS: &[&Lint] = &[DISCARDED_JOIN_HANDLE];

pub fn register(lint_store: &mut LintStore) {
    lint_store.register_late_lint_pass(Box::new(|_| Box::new(DiscardedJoinHandle)));
}

declare_lint_pass!(DiscardedJoinHandle => [DISCARDED_JOIN_HANDLE]);

const TOKIO_HANDLE: &str = "tokio::task::JoinHandle";
const STD_HANDLE: &str = "std::thread::JoinHandle";

impl<'tcx> LateLintPass<'tcx> for DiscardedJoinHandle {
    fn check_stmt(&mut self, cx: &LateContext<'tcx>, stmt: &'tcx Stmt<'tcx>) {
        if stmt.span.from_expansion() {
            return;
        }

        match stmt.kind {
            StmtKind::Semi(expr) => check_discarded(cx, expr, stmt.span),
            StmtKind::Let(LetStmt {
                pat,
                init: Some(init),
                els: None,
                ..
            }) if matches!(pat.kind, PatKind::Wild) => check_discarded(cx, init, stmt.span),
            _ => {},
        }
    }
}

fn check_discarded<'tcx>(cx: &LateContext<'tcx>, expr: &'tcx Expr<'tcx>, span: Span) {
    if expr.span.from_expansion() {
        return;
    }

    let ty = cx.typeck_results().expr_ty(expr);
    let (handle, help) = if is_adt_with_path(cx, ty, &[TOKIO_HANDLE]) {
        (
            TOKIO_HANDLE,
            "keep the handle (or spawn into a `tokio::task::JoinSet`) and await it; if detaching is intended, write `drop(handle)` with a comment saying why",
        )
    } else if is_adt_with_path(cx, ty, &[STD_HANDLE]) {
        (
            STD_HANDLE,
            "keep the handle and `join()` it, or use `std::thread::scope`; if detaching is intended, write `drop(handle)` with a comment saying why",
        )
    } else {
        return;
    };

    span_lint_and_help(
        cx,
        DISCARDED_JOIN_HANDLE,
        span,
        format!("`{handle}` is dropped, so the task's panics and result are silently lost"),
        None,
        help,
    );
}

#[test]
fn ui() {
    crate::ui_test("discarded_join_handle");
}
