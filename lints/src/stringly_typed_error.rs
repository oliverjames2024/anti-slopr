use clippy_utils::diagnostics::span_lint_and_help;
use rustc_hir::def::DefKind;
use rustc_hir::def_id::LocalDefId;
use rustc_hir::intravisit::FnKind;
use rustc_hir::{Body, FnDecl, FnRetTy};
use rustc_hir::attrs::lang_items::LangItem;
use rustc_lint::{LateContext, LateLintPass, Lint, LintStore};
use rustc_middle::ty::{self, Ty};
use rustc_session::{declare_lint, declare_lint_pass};
use rustc_span::{Span, sym};

declare_lint! {
    /// ### What it does
    /// Flags functions whose return type is `Result<_, E>` where `E` is a string:
    /// `String`, `&str`, `Box<str>`, `Cow<str>`, or `Arc<str>`/`Rc<str>`.
    ///
    /// ### Why is this bad?
    /// A string is not an error type. It does not implement `std::error::Error`,
    /// cannot carry a `source()` chain, and forces callers to parse prose to
    /// decide how to react. Effective Rust Item 4 and the Rust API Guidelines
    /// (C-GOOD-ERR) both recommend a dedicated error type.
    ///
    /// ### Known problems
    /// Implementations of a trait are skipped; the trait declaration is flagged
    /// instead, because that is where the signature can be changed.
    ///
    /// ### Example
    /// ```rust,ignore
    /// fn parse_port(input: &str) -> Result<u16, String> {
    ///     input.parse().map_err(|err| format!("invalid port: {err}"))
    /// }
    /// ```
    /// Use instead:
    /// ```rust,ignore
    /// #[derive(Debug, thiserror::Error)]
    /// #[error("invalid port {input:?}")]
    /// struct InvalidPort { input: String, #[source] cause: ParseIntError }
    ///
    /// fn parse_port(input: &str) -> Result<u16, InvalidPort> { .. }
    /// ```
    pub STRINGLY_TYPED_ERROR,
    Warn,
    "`Result` whose error type is a string"
}

pub static LINTS: &[&Lint] = &[STRINGLY_TYPED_ERROR];

pub fn register(lint_store: &mut LintStore) {
    lint_store.register_late_lint_pass(Box::new(|_| Box::new(StringlyTypedError)));
}

declare_lint_pass!(StringlyTypedError => [STRINGLY_TYPED_ERROR]);

impl<'tcx> LateLintPass<'tcx> for StringlyTypedError {
    fn check_fn(
        &mut self,
        cx: &LateContext<'tcx>,
        kind: FnKind<'tcx>,
        decl: &'tcx FnDecl<'tcx>,
        _: &'tcx Body<'tcx>,
        span: Span,
        def_id: LocalDefId,
    ) {
        if matches!(kind, FnKind::Closure) || span.from_expansion() || is_trait_impl_item(cx, def_id) {
            return;
        }

        let FnRetTy::Return(ret_hir_ty) = decl.output else {
            return;
        };

        let output = cx.tcx.fn_sig(def_id).instantiate_identity().skip_binder().output();

        if let Some(err_ty) = result_err_ty(cx, output)
            && is_string_like(cx, err_ty)
        {
            span_lint_and_help(
                cx,
                STRINGLY_TYPED_ERROR,
                ret_hir_ty.span,
                format!("error type `{err_ty}` is a string"),
                None,
                "define an error type (an enum or struct implementing `std::error::Error`) that keeps the cause",
            );
        }
    }

    fn check_trait_item(&mut self, cx: &LateContext<'tcx>, item: &'tcx rustc_hir::TraitItem<'tcx>) {
        let rustc_hir::TraitItemKind::Fn(sig, _) = item.kind else {
            return;
        };

        let FnRetTy::Return(ret_hir_ty) = sig.decl.output else {
            return;
        };

        // Provided methods are also visited by `check_fn`; only report required ones here.
        if matches!(item.kind, rustc_hir::TraitItemKind::Fn(_, rustc_hir::TraitFn::Provided(_))) {
            return;
        }

        let output = cx.tcx.fn_sig(item.owner_id).instantiate_identity().skip_binder().output();

        if let Some(err_ty) = result_err_ty(cx, output)
            && is_string_like(cx, err_ty)
        {
            span_lint_and_help(
                cx,
                STRINGLY_TYPED_ERROR,
                ret_hir_ty.span,
                format!("error type `{err_ty}` is a string"),
                None,
                "define an error type (an enum or struct implementing `std::error::Error`) that keeps the cause",
            );
        }
    }
}

fn is_lang_adt(cx: &LateContext<'_>, ty: Ty<'_>, item: LangItem) -> bool {
    matches!(ty.kind(), ty::Adt(adt, _) if cx.tcx.lang_items().get(item) == Some(adt.did()))
}

fn is_trait_impl_item(cx: &LateContext<'_>, def_id: LocalDefId) -> bool {
    cx.tcx.def_kind(def_id) == DefKind::AssocFn
        && cx
            .tcx
            .opt_parent(def_id.to_def_id())
            .is_some_and(|parent| matches!(cx.tcx.def_kind(parent), DefKind::Impl { of_trait: true }))
}

pub(crate) fn result_err_ty<'tcx>(cx: &LateContext<'tcx>, ty: Ty<'tcx>) -> Option<Ty<'tcx>> {
    let ty = ty.peel_refs();

    if let ty::Adt(adt, args) = ty.kind()
        && cx.tcx.is_diagnostic_item(sym::Result, adt.did())
    {
        Some(args.type_at(1))
    } else {
        None
    }
}

pub(crate) fn is_string_like<'tcx>(cx: &LateContext<'tcx>, ty: Ty<'tcx>) -> bool {
    let ty = ty.peel_refs();

    if ty.is_str() || is_lang_adt(cx, ty, LangItem::String) {
        return true;
    }

    // `Box<str>`, `Cow<'_, str>`, `Arc<str>`, `Rc<str>`.
    if let ty::Adt(adt, args) = ty.kind()
        && let Some(name) = cx.tcx.get_diagnostic_name(adt.did())
        && matches!(name, sym::Cow | sym::Arc | sym::Rc)
        && let Some(inner) = args.types().next()
    {
        return inner.is_str();
    }

    ty.boxed_ty().is_some_and(|inner| inner.is_str())
}

#[test]
fn ui() {
    crate::ui_test("stringly_typed_error");
}
