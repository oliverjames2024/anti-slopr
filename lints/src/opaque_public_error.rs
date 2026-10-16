use clippy_utils::diagnostics::span_lint_and_help;
use clippy_utils::sym;
use rustc_hir::def::DefKind;
use rustc_hir::def_id::LocalDefId;
use rustc_hir::intravisit::FnKind;
use rustc_hir::{
    Body, ClosureKind, CoroutineDesugaring, CoroutineKind, CoroutineSource, ExprKind, FnDecl, FnRetTy, TraitFn,
    TraitItem, TraitItemKind,
};
use rustc_lint::{LateContext, LateLintPass, Lint, LintStore};
use rustc_middle::ty::{self, Ty};
use rustc_session::config::CrateType;
use rustc_session::{declare_lint, declare_lint_pass};
use rustc_span::Span;

use crate::stringly_typed_error::result_err_ty;
use crate::util::is_adt_with_path;

declare_lint! {
    /// ### What it does
    /// In a library crate, flags functions and methods reachable from outside
    /// the crate whose return type is `Result<_, E>` with an opaque error type
    /// `E`: `Box<dyn Error>` (with or without `+ Send + Sync`), `anyhow::Error`,
    /// or `eyre::Report` (also re-exported as `color_eyre::Report`).
    ///
    /// ### Why is this bad?
    /// Callers of a library need to react to individual failure modes: retry on
    /// a timeout, report a missing file, give up on a parse error. An opaque
    /// error forces them to downcast or to match on message text. The Rust API
    /// Guidelines (C-GOOD-ERR) and Effective Rust (Item 4) recommend a concrete,
    /// matchable error type in public APIs. `anyhow` and `eyre` are fine in
    /// binaries and in crate-private code, which this lint ignores.
    ///
    /// ### Known problems
    /// Only `rlib`/`dylib` crates are checked. Trait implementations are
    /// skipped, as the trait fixes the signature. Required trait methods without
    /// a body are checked, but `async fn` trait declarations are not.
    ///
    /// ### Example
    /// ```rust,ignore
    /// pub fn load_config(path: &Path) -> anyhow::Result<Config> { .. }
    /// ```
    /// Use instead:
    /// ```rust,ignore
    /// #[derive(Debug, thiserror::Error)]
    /// pub enum ConfigError {
    ///     #[error("cannot read {path}")]
    ///     Read { path: PathBuf, #[source] source: io::Error },
    ///     #[error("invalid config")]
    ///     Parse(#[from] toml::de::Error),
    /// }
    ///
    /// pub fn load_config(path: &Path) -> Result<Config, ConfigError> { .. }
    /// ```
    pub OPAQUE_PUBLIC_ERROR,
    Warn,
    "public library function returns an opaque error type"
}

pub static LINTS: &[&Lint] = &[OPAQUE_PUBLIC_ERROR];

pub fn register(lint_store: &mut LintStore) {
    lint_store.register_late_lint_pass(Box::new(|_| Box::new(OpaquePublicError)));
}

declare_lint_pass!(OpaquePublicError => [OPAQUE_PUBLIC_ERROR]);

impl<'tcx> LateLintPass<'tcx> for OpaquePublicError {
    fn check_fn(
        &mut self,
        cx: &LateContext<'tcx>,
        kind: FnKind<'tcx>,
        decl: &'tcx FnDecl<'tcx>,
        body: &'tcx Body<'tcx>,
        span: Span,
        def_id: LocalDefId,
    ) {
        if matches!(kind, FnKind::Closure)
            || span.from_expansion()
            || !is_library(cx)
            || !cx.effective_visibilities.is_exported(def_id)
            || is_trait_impl_item(cx, def_id)
        {
            return;
        }

        let FnRetTy::Return(ret_hir_ty) = decl.output else {
            return;
        };

        let output = async_fn_output(cx, body)
            .unwrap_or_else(|| cx.tcx.fn_sig(def_id).instantiate_identity().skip_binder().output());

        check_output(cx, output, ret_hir_ty.span);
    }

    fn check_trait_item(&mut self, cx: &LateContext<'tcx>, item: &'tcx TraitItem<'tcx>) {
        // Provided methods are also visited by `check_fn`; only report required ones here.
        let TraitItemKind::Fn(sig, TraitFn::Required(_)) = item.kind else {
            return;
        };

        if item.span.from_expansion() || !is_library(cx) || !cx.effective_visibilities.is_exported(item.owner_id.def_id) {
            return;
        }

        let FnRetTy::Return(ret_hir_ty) = sig.decl.output else {
            return;
        };

        let output = cx.tcx.fn_sig(item.owner_id).instantiate_identity().skip_binder().output();
        check_output(cx, output, ret_hir_ty.span);
    }
}

fn check_output<'tcx>(cx: &LateContext<'tcx>, output: Ty<'tcx>, span: Span) {
    let Some(err_ty) = result_err_ty(cx, output) else {
        return;
    };

    let Some(name) = opaque_error_name(cx, err_ty) else {
        return;
    };

    span_lint_and_help(
        cx,
        OPAQUE_PUBLIC_ERROR,
        span,
        format!("public function returns the opaque error type `{name}`"),
        None,
        "return a concrete error type (an enum of failure modes implementing `std::error::Error`) so callers can match on it",
    );
}

/// `Some(display name)` for `Box<dyn Error ..>`, `anyhow::Error`, and `eyre::Report`.
fn opaque_error_name<'tcx>(cx: &LateContext<'tcx>, ty: Ty<'tcx>) -> Option<String> {
    if let Some(inner) = ty.boxed_ty()
        && let ty::Dynamic(predicates, ..) = inner.kind()
        && predicates
            .principal_def_id()
            .is_some_and(|principal| cx.tcx.is_diagnostic_item(sym::Error, principal))
    {
        let auto_traits: String = predicates
            .auto_traits()
            .map(|auto_trait| format!(" + {}", cx.tcx.item_name(auto_trait)))
            .collect();
        return Some(format!("Box<dyn Error{auto_traits}>"));
    }

    if is_adt_with_path(cx, ty, &["anyhow::Error"]) {
        return Some("anyhow::Error".to_owned());
    }

    is_adt_with_path(cx, ty, &["eyre::Report"]).then(|| "eyre::Report".to_owned())
}

/// Whether the crate being compiled is a Rust library other crates link against.
fn is_library(cx: &LateContext<'_>) -> bool {
    cx.tcx
        .crate_types()
        .iter()
        .any(|crate_type| matches!(crate_type, CrateType::Rlib | CrateType::Dylib))
}

fn is_trait_impl_item(cx: &LateContext<'_>, def_id: LocalDefId) -> bool {
    cx.tcx.def_kind(def_id) == DefKind::AssocFn
        && cx
            .tcx
            .opt_parent(def_id.to_def_id())
            .is_some_and(|parent| matches!(cx.tcx.def_kind(parent), DefKind::Impl { of_trait: true }))
}

/// The value an `async fn` body evaluates to, rather than its `impl Future` type.
fn async_fn_output<'tcx>(cx: &LateContext<'tcx>, body: &'tcx Body<'tcx>) -> Option<Ty<'tcx>> {
    let ExprKind::Closure(closure) = body.value.kind else {
        return None;
    };

    if !matches!(
        closure.kind,
        ClosureKind::Coroutine(CoroutineKind::Desugared(CoroutineDesugaring::Async, CoroutineSource::Fn))
    ) {
        return None;
    }

    match cx.typeck_results().expr_ty(body.value).kind() {
        ty::Coroutine(_, args) => Some(args.as_coroutine().return_ty()),
        _ => None,
    }
}

#[test]
fn ui() {
    crate::ui_test("opaque_public_error");
}
