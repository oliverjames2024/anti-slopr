//! Helpers shared by several lints.

use rustc_hir::def_id::DefId;
use rustc_hir::{ClosureKind, CoroutineDesugaring, CoroutineKind, ExprKind, HirId, Node};
use rustc_lint::LateContext;
use rustc_middle::ty::print::with_no_trimmed_paths;
use rustc_middle::ty::{self, Ty};

/// The fully qualified, user-visible path of `def_id` with generic arguments
/// removed, such as `std::thread::sleep` or `std::sync::mpsc::Receiver::recv`.
///
/// `#![no_std]` crates see `core::` and `alloc::` paths for items that `std`
/// re-exports; those are reported under `std::` so one path list covers both.
pub fn def_path(cx: &LateContext<'_>, def_id: DefId) -> String {
    let path = strip_generic_args(&with_no_trimmed_paths!(cx.tcx.def_path_str(def_id)));

    match path.strip_prefix("core::").or_else(|| path.strip_prefix("alloc::")) {
        Some(rest) => format!("std::{rest}"),
        None => path,
    }
}

/// Removes `::<..>` and `<..>` groups: `Receiver::<T>::recv` becomes `Receiver::recv`.
fn strip_generic_args(path: &str) -> String {
    let mut stripped = String::with_capacity(path.len());
    let mut depth = 0_usize;

    for character in path.chars() {
        match character {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            _ if depth == 0 => stripped.push(character),
            _ => {},
        }
    }

    stripped.replace("::::", "::")
}

/// Whether `ty` is an ADT whose definition path is one of `paths`.
pub fn is_adt_with_path(cx: &LateContext<'_>, ty: Ty<'_>, paths: &[&str]) -> bool {
    if let ty::Adt(adt, _) = ty.kind() {
        let path = def_path(cx, adt.did());
        paths.iter().any(|candidate| *candidate == path)
    } else {
        false
    }
}

/// Whether the expression `hir_id` executes inside an `async` body (an `async fn`,
/// `async` block, or `async` closure) without an intervening synchronous closure
/// or item. Code inside a synchronous closure, such as the one handed to
/// `spawn_blocking`, does not run on the async executor.
pub fn is_in_async_body(cx: &LateContext<'_>, hir_id: HirId) -> bool {
    for (_, node) in cx.tcx.hir_parent_iter(hir_id) {
        match node {
            Node::Expr(expr) => {
                if let ExprKind::Closure(closure) = expr.kind {
                    return matches!(
                        closure.kind,
                        ClosureKind::Coroutine(CoroutineKind::Desugared(CoroutineDesugaring::Async, _))
                            | ClosureKind::CoroutineClosure(CoroutineDesugaring::Async)
                    );
                }
            },
            Node::Item(_) | Node::ImplItem(_) | Node::TraitItem(_) | Node::ForeignItem(_) => return false,
            _ => {},
        }
    }

    false
}
