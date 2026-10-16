use std::collections::HashSet;
use std::ops::ControlFlow;

use clippy_utils::diagnostics::span_lint_and_help;
use clippy_utils::visitors::for_each_expr;
use clippy_utils::{fn_def_id, is_test_function};
use rustc_hir::def::{CtorOf, DefKind, Res};
use rustc_hir::def_id::LocalDefId;
use rustc_hir::intravisit::FnKind;
use rustc_hir::{Body, Expr, ExprKind, FnDecl, MatchSource, QPath, find_attr};
use rustc_lint::{LateContext, LateLintPass, Lint, LintStore};
use rustc_middle::ty::{self, TypeckResults};
use rustc_span::{ExpnKind, MacroKind, Span, sym};
use rustc_session::{declare_lint, declare_lint_pass};

use crate::util::is_adt_with_path;

declare_lint! {
    /// ### What it does
    /// Flags `#[test]` functions whose body contains no evidence of checking
    /// anything. Evidence is any of:
    /// - a macro whose name starts with `assert`, `debug_assert`, or
    ///   `prop_assert`, or is `panic!`, `unreachable!`, `bail!`, `ensure!`, or
    ///   `proptest!` (from any crate: `assert_matches!`, `insta::assert_snapshot!`, ..);
    /// - `unwrap`, `expect`, `unwrap_err`, or `expect_err` on an `Option` or `Result`;
    /// - the `?` operator, or constructing an `Err(..)` in a test returning `Result`;
    /// - a call to a function or method named `assert*`, `check`/`check_*`,
    ///   `expect*`, `verify`/`verify_*`, or `ensure`/`ensure_*`;
    /// - a call to a local helper function whose body contains such evidence;
    /// - `#[should_panic]` on the test.
    ///
    /// ### Why is this bad?
    /// A test without an assertion passes whatever the code does. It inflates
    /// the test count and coverage while protecting nothing; a test should fail
    /// when the behaviour it names breaks.
    ///
    /// ### Known problems
    /// "Does not panic" smoke tests are reported; make the expectation explicit
    /// (assert on the result) or suppress with a reasoned `#[expect]`. Helpers
    /// from other crates that assert internally are only recognised by name.
    /// A test returning `Result` that only ends in `Ok(())` is reported, because
    /// it can never fail.
    ///
    /// ### Example
    /// ```rust,ignore
    /// #[test]
    /// fn parses_config() {
    ///     let _config = Config::parse("port = 80");
    /// }
    /// ```
    /// Use instead:
    /// ```rust,ignore
    /// #[test]
    /// fn parses_config() {
    ///     let config = Config::parse("port = 80").unwrap();
    ///     assert_eq!(config.port, 80);
    /// }
    /// ```
    pub TEST_WITHOUT_ASSERTION,
    Warn,
    "`#[test]` function that checks nothing"
}

pub static LINTS: &[&Lint] = &[TEST_WITHOUT_ASSERTION];

pub fn register(lint_store: &mut LintStore) {
    lint_store.register_late_lint_pass(Box::new(|_| Box::new(TestWithoutAssertion)));
}

declare_lint_pass!(TestWithoutAssertion => [TEST_WITHOUT_ASSERTION]);

/// Prefixes of assertion-like macro names.
const MACRO_PREFIXES: &[&str] = &["assert", "debug_assert", "prop_assert"];
/// Exact assertion-like macro names.
const MACRO_NAMES: &[&str] = &["panic", "unreachable", "bail", "ensure", "proptest"];
/// Function names that count when equal to the word or followed by `_`.
const CALL_WORDS: &[&str] = &["check", "verify", "ensure"];
/// Function name prefixes that count however they continue (`assert_eq`, `expect_err`).
const CALL_PREFIXES: &[&str] = &["assert", "expect"];
/// How many levels of local helper functions are searched for evidence.
const HELPER_DEPTH: u32 = 3;

impl<'tcx> LateLintPass<'tcx> for TestWithoutAssertion {
    fn check_fn(
        &mut self,
        cx: &LateContext<'tcx>,
        kind: FnKind<'tcx>,
        _: &'tcx FnDecl<'tcx>,
        body: &'tcx Body<'tcx>,
        span: Span,
        def_id: LocalDefId,
    ) {
        let FnKind::ItemFn(ident, ..) = kind else {
            return;
        };

        // Tests produced by `macro_rules!` generators are skipped; an
        // attribute-macro test (`#[tokio::test]`) keeps the user's tokens.
        if is_from_bang_macro(span) || ident.span.from_expansion() || !is_test_function(cx.tcx, def_id) {
            return;
        }

        let hir_id = cx.tcx.local_def_id_to_hir_id(def_id);
        if find_attr!(cx.tcx, hir_id, ShouldPanic { .. }) {
            return;
        }

        let returns_result = {
            let output = cx.tcx.fn_sig(def_id).instantiate_identity().skip_binder().output();
            matches!(output.kind(), ty::Adt(adt, _) if cx.tcx.is_diagnostic_item(sym::Result, adt.did()))
        };

        let mut search = EvidenceSearch {
            cx,
            visited: HashSet::from([def_id]),
        };
        if search.has_evidence(body, cx.typeck_results(), returns_result, HELPER_DEPTH) {
            return;
        }

        span_lint_and_help(
            cx,
            TEST_WITHOUT_ASSERTION,
            ident.span,
            format!("test `{ident}` asserts nothing"),
            None,
            "assert on the behaviour the test names (`assert_eq!`, `assert!(matches!(..))`, `.unwrap()` on the result under test), or mark an expected panic with `#[should_panic]`",
        );
    }
}

struct EvidenceSearch<'a, 'tcx> {
    cx: &'a LateContext<'tcx>,
    visited: HashSet<LocalDefId>,
}

impl<'tcx> EvidenceSearch<'_, 'tcx> {
    fn has_evidence(
        &mut self,
        body: &'tcx Body<'tcx>,
        typeck: &'tcx TypeckResults<'tcx>,
        returns_result: bool,
        depth: u32,
    ) -> bool {
        let mut helpers = Vec::new();

        let found = for_each_expr(self.cx.tcx, body.value, |expr| {
            if is_assertion_macro(expr.span) {
                return ControlFlow::Break(());
            }
            // Code generated by attribute macros (the runtime set-up of
            // `#[tokio::test]`, for instance) is not the test's own.
            if is_from_attribute_macro(expr.span) {
                return ControlFlow::Continue(());
            }

            match expr.kind {
                ExprKind::Match(_, _, MatchSource::TryDesugar(_)) => return ControlFlow::Break(()),
                ExprKind::MethodCall(segment, receiver, ..) => {
                    let name = segment.ident.name.as_str();
                    let is_unwrap = matches!(name, "unwrap" | "unwrap_err" | "expect" | "expect_err");
                    if (is_unwrap && is_option_or_result(self.cx, typeck, receiver))
                        || (!is_unwrap && is_assertion_name(name))
                    {
                        return ControlFlow::Break(());
                    }
                },
                ExprKind::Call(callee, _) if returns_result && is_err_ctor(self.cx, callee) => {
                    return ControlFlow::Break(());
                },
                _ => {},
            }

            if matches!(expr.kind, ExprKind::Call(..) | ExprKind::MethodCall(..))
                && let Some(callee) = fn_def_id_in(self.cx, typeck, expr)
            {
                // Method names were checked above, against the written segment.
                if matches!(expr.kind, ExprKind::Call(..)) && is_assertion_name(self.cx.tcx.item_name(callee).as_str()) {
                    return ControlFlow::Break(());
                }
                if let Some(local) = callee.as_local() {
                    helpers.push(local);
                }
            }

            ControlFlow::Continue(())
        })
        .is_some();

        if found {
            return true;
        }
        if depth == 0 {
            return false;
        }

        helpers.into_iter().any(|helper| {
            if !self.visited.insert(helper) || !matches!(self.cx.tcx.def_kind(helper), DefKind::Fn | DefKind::AssocFn) {
                return false;
            }
            let Some(helper_body) = self.cx.tcx.hir_maybe_body_owned_by(helper) else {
                return false;
            };
            let helper_typeck = self.cx.tcx.typeck(helper);
            self.has_evidence(helper_body, helper_typeck, false, depth - 1)
        })
    }
}

/// `fn_def_id` resolved against `typeck`, which may belong to a helper body
/// other than the one the lint pass is currently in.
fn fn_def_id_in<'tcx>(
    cx: &LateContext<'tcx>,
    typeck: &'tcx TypeckResults<'tcx>,
    expr: &Expr<'_>,
) -> Option<rustc_hir::def_id::DefId> {
    if std::ptr::eq(typeck, cx.typeck_results()) {
        return fn_def_id(cx, expr);
    }
    match expr.kind {
        ExprKind::MethodCall(..) => typeck.type_dependent_def_id(expr.hir_id),
        ExprKind::Call(callee, _) => match callee.kind {
            ExprKind::Path(ref qpath) => match typeck.qpath_res(qpath, callee.hir_id) {
                Res::Def(DefKind::Fn | DefKind::AssocFn, def_id) => Some(def_id),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

fn is_assertion_name(name: &str) -> bool {
    CALL_PREFIXES.iter().any(|prefix| name.starts_with(prefix))
        || CALL_WORDS
            .iter()
            .any(|word| name.strip_prefix(word).is_some_and(|rest| rest.is_empty() || rest.starts_with('_')))
}

fn is_assertion_macro(span: Span) -> bool {
    let mut ctxt = span.ctxt();
    while !ctxt.is_root() {
        let data = ctxt.outer_expn_data();
        if let ExpnKind::Macro(MacroKind::Bang, name) = data.kind {
            let name = name.as_str();
            if MACRO_NAMES.contains(&name) || MACRO_PREFIXES.iter().any(|prefix| name.starts_with(prefix)) {
                return true;
            }
        }
        ctxt = data.call_site.ctxt();
    }
    false
}

fn is_from_attribute_macro(span: Span) -> bool {
    expanded_from(span, |kind| matches!(kind, MacroKind::Attr | MacroKind::Derive))
}

fn is_from_bang_macro(span: Span) -> bool {
    expanded_from(span, |kind| kind == MacroKind::Bang)
}

/// Whether any macro expansion that `span` came from has a kind accepted by `filter`.
fn expanded_from(span: Span, filter: impl Fn(MacroKind) -> bool) -> bool {
    let mut ctxt = span.ctxt();
    while !ctxt.is_root() {
        let data = ctxt.outer_expn_data();
        if let ExpnKind::Macro(kind, _) = data.kind
            && filter(kind)
        {
            return true;
        }
        ctxt = data.call_site.ctxt();
    }
    false
}

/// Whether `receiver` is an `Option` or `Result`, other than the
/// `Result<tokio::runtime::Runtime, _>` that `#[tokio::test]` builds and
/// `expect`s with the user's spans.
fn is_option_or_result(cx: &LateContext<'_>, typeck: &TypeckResults<'_>, receiver: &Expr<'_>) -> bool {
    let ty = typeck.expr_ty_adjusted(receiver).peel_refs();
    match ty.kind() {
        ty::Adt(adt, args) if cx.tcx.is_diagnostic_item(sym::Result, adt.did()) => {
            !is_adt_with_path(cx, args.type_at(0), &["tokio::runtime::Runtime"])
        },
        ty::Adt(adt, _) => cx.tcx.is_diagnostic_item(sym::Option, adt.did()),
        _ => false,
    }
}

fn is_err_ctor(cx: &LateContext<'_>, callee: &Expr<'_>) -> bool {
    if let ExprKind::Path(QPath::Resolved(_, path)) = callee.kind
        && let Res::Def(DefKind::Ctor(CtorOf::Variant, _), ctor_id) = path.res
        && let Some(variant_id) = cx.tcx.opt_parent(ctor_id)
    {
        cx.tcx.lang_items().result_err_variant() == Some(variant_id)
    } else {
        false
    }
}

#[test]
fn ui() {
    crate::ui_test_with_flags("test_without_assertion", &["--test"]);
}
