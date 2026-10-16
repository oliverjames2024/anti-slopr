use clippy_utils::diagnostics::span_lint_and_help;
use clippy_utils::sym;
use clippy_utils::ty::get_iterator_item_ty;
use clippy_utils::visitors::is_local_used;
use clippy_utils::{fn_def_id, peel_blocks};
use rustc_hir::def::Res;
use rustc_hir::{Expr, ExprKind, PatKind, QPath};
use rustc_lint::{LateContext, LateLintPass, Lint, LintStore};
use rustc_middle::ty::{self, Ty};
use rustc_session::{declare_lint, impl_lint_pass};
use rustc_span::Symbol;
use serde::Deserialize;

use crate::stringly_typed_error::result_err_ty;
use crate::util::def_path;

declare_lint! {
    /// ### What it does
    /// Flags code that swallows the error of a `Result` by substituting a value
    /// or dropping the item:
    /// - `Result::unwrap_or_default()`, `Result::unwrap_or(..)`, and
    ///   `Result::unwrap_or_else(|_| ..)` whose closure ignores the error;
    /// - `.ok()` on a `Result` followed by `unwrap_or`, `unwrap_or_default`,
    ///   `unwrap_or_else`, or `flatten`;
    /// - `iter.filter_map(Result::ok)`, `.filter_map(|r| r.ok())` (and the same
    ///   with `flat_map`), and `.flatten()` on an iterator of `Result`s.
    ///
    /// ### Why is this bad?
    /// The failure becomes indistinguishable from a legitimate value or an
    /// empty input, and its cause is gone: nothing is logged, nothing is
    /// propagated. A config file that fails to parse silently becomes the
    /// default config; an unreadable directory entry silently disappears.
    ///
    /// ### Known problems
    /// Errors that carry no information beyond "absent" or "out of range" are
    /// skipped by default: `std::env::VarError` and `std::num::TryFromIntError`
    /// (so `env::var("X").unwrap_or_default()` and saturating `try_into()`
    /// conversions are allowed). `Option` methods are never flagged.
    /// `unwrap_or_else(|err| ..)` that uses `err` (to log or inspect it) is
    /// not flagged, and neither is `map_while(Result::ok)`, which at least stops
    /// at the first error.
    ///
    /// ### Example
    /// ```rust,ignore
    /// let config = fs::read_to_string(path).ok().and_then(parse).unwrap_or_default();
    /// let port = input.parse().unwrap_or(8080);
    /// let names: Vec<_> = fs::read_dir(dir)?.filter_map(Result::ok).collect();
    /// ```
    /// Use instead:
    /// ```rust,ignore
    /// let port = match input.parse() {
    ///     Ok(port) => port,
    ///     Err(err) => {
    ///         tracing::warn!(%err, input, "invalid port, using 8080");
    ///         8080
    ///     },
    /// };
    /// let names = fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;
    /// ```
    ///
    /// ### Configuration
    /// ```toml
    /// [silent_error_default]
    /// allowed_error_types = ["my_crate::NotFound"]   # error types that may be defaulted
    /// ```
    pub SILENT_ERROR_DEFAULT,
    Warn,
    "`Result` error silently replaced by a default value or dropped"
}

pub static LINTS: &[&Lint] = &[SILENT_ERROR_DEFAULT];

pub fn register(lint_store: &mut LintStore) {
    lint_store.register_late_lint_pass(Box::new(|_| Box::new(SilentErrorDefault::new())));
}

/// Error types whose only failure mode is "absent" or "does not fit".
const ALLOWED_ERROR_TYPES: &[&str] = &["std::env::VarError", "std::num::TryFromIntError", "std::convert::Infallible"];

const HELP: &str = "propagate the error with `?`, or `match` on it and handle or log the failure explicitly";

#[derive(Default, Deserialize)]
struct Config {
    #[serde(default)]
    allowed_error_types: Vec<String>,
}

struct SilentErrorDefault {
    config: Config,
}

impl SilentErrorDefault {
    fn new() -> Self {
        Self {
            config: dylint_linting::config_or_default(SILENT_ERROR_DEFAULT.name_lower().as_str()),
        }
    }

    /// Whether `result_ty` is a `Result` whose error type may be swallowed.
    fn is_reportable<'tcx>(&self, cx: &LateContext<'tcx>, result_ty: Ty<'tcx>) -> bool {
        let Some(err_ty) = result_err_ty(cx, result_ty) else {
            return false;
        };

        let ty::Adt(adt, _) = err_ty.kind() else {
            return true;
        };

        let path = def_path(cx, adt.did());
        !ALLOWED_ERROR_TYPES.contains(&path.as_str()) && !self.config.allowed_error_types.contains(&path)
    }
}

impl_lint_pass!(SilentErrorDefault => [SILENT_ERROR_DEFAULT]);

impl<'tcx> LateLintPass<'tcx> for SilentErrorDefault {
    fn check_expr(&mut self, cx: &LateContext<'tcx>, expr: &'tcx Expr<'tcx>) {
        let ExprKind::MethodCall(segment, receiver, args, call_span) = expr.kind else {
            return;
        };

        if expr.span.from_expansion() {
            return;
        }

        let method = segment.ident.name;
        let typeck = cx.typeck_results();
        let receiver_ty = typeck.expr_ty(receiver);

        let mut span = call_span;

        let message = if is_result(cx, receiver_ty) {
            if !self.is_reportable(cx, receiver_ty) {
                return;
            }

            match (method, args) {
                (sym::unwrap_or_default | sym::unwrap_or, _) => {
                    format!("`{method}` replaces the error of this `Result` with a fallback value")
                },
                (sym::unwrap_or_else, [handler]) if closure_ignores_error(cx, handler) => {
                    "`unwrap_or_else` ignores the error of this `Result` and returns a fallback value".to_owned()
                },
                _ => return,
            }
        } else if is_option(cx, receiver_ty)
            && matches!(
                method,
                sym::unwrap_or | sym::unwrap_or_default | sym::unwrap_or_else | sym::flatten
            )
            && let ExprKind::MethodCall(ok_segment, result, [], _) = receiver.kind
            && ok_segment.ident.name == sym::ok
            && is_result(cx, typeck.expr_ty(result))
        {
            if !self.is_reportable(cx, typeck.expr_ty(result)) {
                return;
            }

            span = ok_segment.ident.span.to(call_span);
            let arguments = if args.is_empty() { "()" } else { "(..)" };
            format!("`.ok().{method}{arguments}` discards the error of this `Result`")
        } else if is_iterator_method(cx, expr) {
            let Some(item_ty) = get_iterator_item_ty(cx, receiver_ty) else {
                return;
            };

            if !is_result(cx, item_ty) || !self.is_reportable(cx, item_ty) {
                return;
            }

            match (method, args) {
                (sym::filter_map | sym::flat_map, [mapper]) if is_result_ok_fn(cx, mapper) => {
                    format!("`{method}` with `Result::ok` silently drops every `Err` item")
                },
                (sym::flatten, []) => "`flatten` on an iterator of `Result`s silently drops every `Err` item".to_owned(),
                _ => return,
            }
        } else {
            return;
        };

        span_lint_and_help(cx, SILENT_ERROR_DEFAULT, span, message, None, HELP);
    }
}

fn is_result(cx: &LateContext<'_>, ty: Ty<'_>) -> bool {
    has_diagnostic_name(cx, ty, sym::Result)
}

fn is_option(cx: &LateContext<'_>, ty: Ty<'_>) -> bool {
    has_diagnostic_name(cx, ty, sym::Option)
}

fn has_diagnostic_name(cx: &LateContext<'_>, ty: Ty<'_>, name: Symbol) -> bool {
    matches!(ty.kind(), ty::Adt(adt, _) if cx.tcx.is_diagnostic_item(name, adt.did()))
}

/// Whether the method called by `expr` is a provided method of `Iterator`.
fn is_iterator_method(cx: &LateContext<'_>, expr: &Expr<'_>) -> bool {
    fn_def_id(cx, expr)
        .and_then(|callee| cx.tcx.trait_of_assoc(callee))
        .is_some_and(|trait_id| cx.tcx.is_diagnostic_item(sym::Iterator, trait_id))
}

/// `|_| ..` or `|err| ..` that never reads `err`, and whose body does not diverge.
fn closure_ignores_error(cx: &LateContext<'_>, handler: &Expr<'_>) -> bool {
    let ExprKind::Closure(closure) = handler.kind else {
        return false;
    };

    let body = cx.tcx.hir_body(closure.body);

    let [param] = body.params else {
        return false;
    };

    // `|_| panic!(..)` or `|_| process::exit(1)` does not swallow anything.
    if cx.tcx.typeck_body(closure.body).expr_ty(body.value).is_never() {
        return false;
    }

    match param.pat.kind {
        PatKind::Wild => true,
        PatKind::Binding(_, binding, _, None) => !is_local_used(cx, body.value, binding),
        _ => false,
    }
}

/// `Result::ok` as a path, or a closure `|r| r.ok()`.
fn is_result_ok_fn(cx: &LateContext<'_>, mapper: &Expr<'_>) -> bool {
    match mapper.kind {
        ExprKind::Path(ref qpath) => {
            if let Res::Def(_, def_id) = cx.qpath_res(qpath, mapper.hir_id)
                && cx.tcx.item_name(def_id) == sym::ok
                && let Some(impl_id) = cx.tcx.impl_of_assoc(def_id)
            {
                is_result(cx, cx.tcx.type_of(impl_id).instantiate_identity().skip_norm_wip())
            } else {
                false
            }
        },
        ExprKind::Closure(closure) => {
            let body = cx.tcx.hir_body(closure.body);
            let typeck = cx.tcx.typeck_body(closure.body);

            if let [param] = body.params
                && let PatKind::Binding(_, binding, _, None) = param.pat.kind
                && let ExprKind::MethodCall(segment, receiver, [], _) = peel_blocks(body.value).kind
                && segment.ident.name == sym::ok
                && let ExprKind::Path(QPath::Resolved(None, path)) = receiver.kind
                && path.res == Res::Local(binding)
            {
                is_result(cx, typeck.expr_ty(receiver))
            } else {
                false
            }
        },
        _ => false,
    }
}

#[test]
fn ui() {
    crate::ui_test("silent_error_default");
}
