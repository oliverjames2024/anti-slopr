use clippy_utils::diagnostics::span_lint_and_help;
use rustc_hir::attrs::lang_items::LangItem;
use rustc_hir::{Impl, Item, ItemKind};
use rustc_lint::{LateContext, LateLintPass, Lint, LintStore};
use rustc_middle::ty::{self, AdtDef, GenericArgsRef, Ty};
use rustc_session::{declare_lint, declare_lint_pass};
use rustc_span::sym;

use crate::util::is_adt_with_path;

declare_lint! {
    /// ### What it does
    /// Flags `impl Deref` for a struct with more than one (non-zero-sized) field
    /// that stores its `Target` by value, or for an enum, when `Target` is a
    /// concrete struct or enum. That is `Deref` used to emulate inheritance:
    /// `Employee` "derefs" to the `Person` it embeds so `Person`'s methods
    /// appear on it.
    ///
    /// ### Why is this bad?
    /// `Deref` is for smart pointers (API guideline C-DEREF). Used for
    /// inheritance it is a documented anti-pattern
    /// (<https://rust-unofficial.github.io/patterns/anti_patterns/deref.html>):
    /// method resolution silently picks the inner type's methods, trait
    /// implementations of the inner type do not carry over, generic code bounded
    /// on the inner type's traits does not accept the outer type, and readers
    /// cannot tell from a call site which type a method belongs to.
    ///
    /// ### Known problems
    /// `DerefMut` requires `Deref`, so a type is reported once, at its `Deref`
    /// implementation. Guard-like types are not reported: types implementing
    /// `Drop`, types holding the target behind a reference or pointer, enums
    /// whose every variant holds just the target (`Cow`-like), and impls whose
    /// target is a type parameter, `str`, a slice, an array, a trait object,
    /// `Path`, `OsStr`, or `CStr`. Single-field newtypes are not reported.
    ///
    /// ### Example
    /// ```rust,ignore
    /// struct Employee { person: Person, salary: u32 }
    ///
    /// impl Deref for Employee {
    ///     type Target = Person;
    ///     fn deref(&self) -> &Person { &self.person }
    /// }
    /// ```
    /// Use instead:
    /// ```rust,ignore
    /// impl Employee {
    ///     fn person(&self) -> &Person { &self.person }
    ///     fn name(&self) -> &str { self.person.name() }   // forward what callers need
    /// }
    /// ```
    pub DEREF_POLYMORPHISM,
    Warn,
    "`Deref` implemented to emulate inheritance"
}

pub static LINTS: &[&Lint] = &[DEREF_POLYMORPHISM];

pub fn register(lint_store: &mut LintStore) {
    lint_store.register_late_lint_pass(Box::new(|_| Box::new(DerefPolymorphism)));
}

declare_lint_pass!(DerefPolymorphism => [DEREF_POLYMORPHISM]);

/// Unsized path-like targets whose owners are legitimately smart-pointer-shaped.
const BORROWED_TARGETS: &[&str] = &["std::path::Path", "std::ffi::OsStr", "std::ffi::CStr"];

impl<'tcx> LateLintPass<'tcx> for DerefPolymorphism {
    fn check_item(&mut self, cx: &LateContext<'tcx>, item: &'tcx Item<'tcx>) {
        let ItemKind::Impl(Impl {
            of_trait: Some(of_trait),
            self_ty: self_hir_ty,
            ..
        }) = item.kind
        else {
            return;
        };

        if item.span.from_expansion()
            || of_trait.trait_ref.trait_def_id() != cx.tcx.lang_items().get(LangItem::Deref)
        {
            return;
        }

        let self_ty = cx.tcx.type_of(item.owner_id).instantiate_identity().skip_norm_wip();
        let ty::Adt(adt, args) = *self_ty.kind() else {
            return;
        };

        let Some(target) = deref_target(cx, item) else {
            return;
        };

        let ty::Adt(target_adt, _) = target.kind() else {
            return;
        };
        if is_adt_with_path(cx, target, BORROWED_TARGETS) || implements_drop(cx, adt) {
            return;
        }

        let shape = if adt.is_struct() {
            let fields = non_zst_fields(cx, adt, args);
            if fields.len() < 2 || !fields.contains(&target) {
                return;
            }
            "a struct with several fields"
        } else if adt.is_enum() {
            if is_cow_like(cx, adt, args, target) {
                return;
            }
            "an enum"
        } else {
            return;
        };

        span_lint_and_help(
            cx,
            DEREF_POLYMORPHISM,
            item.span.with_hi(self_hir_ty.span.hi()),
            format!(
                "`{self_ty}` implements `Deref<Target = {target}>` to inherit `{}`'s methods, but it is {shape}, not a smart pointer",
                cx.tcx.item_name(target_adt.did()),
            ),
            None,
            "expose an accessor (`fn inner(&self) -> &Inner`), forward the methods callers need, or share behaviour through a trait",
        );
    }
}

fn deref_target<'tcx>(cx: &LateContext<'tcx>, item: &Item<'tcx>) -> Option<Ty<'tcx>> {
    cx.tcx
        .associated_items(item.owner_id)
        .filter_by_name_unhygienic(sym::Target)
        .next()
        .map(|assoc| cx.tcx.type_of(assoc.def_id).instantiate_identity().skip_norm_wip())
}

fn implements_drop(cx: &LateContext<'_>, adt: AdtDef<'_>) -> bool {
    cx.tcx.adt_destructor(adt.did()).is_some()
}

fn non_zst_fields<'tcx>(cx: &LateContext<'tcx>, adt: AdtDef<'tcx>, args: GenericArgsRef<'tcx>) -> Vec<Ty<'tcx>> {
    adt.all_fields()
        .map(|field| field.ty(cx.tcx, args).skip_norm_wip())
        .filter(|ty| !is_zero_sized(cx, *ty))
        .collect()
}

fn is_zero_sized<'tcx>(cx: &LateContext<'tcx>, ty: Ty<'tcx>) -> bool {
    if ty.is_unit() {
        return true;
    }
    if let ty::Adt(adt, _) = ty.kind()
        && cx.tcx.lang_items().get(LangItem::PhantomData) == Some(adt.did())
    {
        return true;
    }
    // Generic fields have no layout here; treat them as data-carrying.
    cx.tcx
        .layout_of(cx.typing_env().as_query_input(ty))
        .is_ok_and(|layout| layout.is_zst())
}

/// Every variant holds exactly one value, and it is the target itself or a
/// pointer to it (`enum MaybeOwned<'a> { Owned(Config), Borrowed(&'a Config) }`).
fn is_cow_like<'tcx>(cx: &LateContext<'tcx>, adt: AdtDef<'tcx>, args: GenericArgsRef<'tcx>, target: Ty<'tcx>) -> bool {
    adt.variants().iter().all(|variant| {
        let mut fields = variant
            .fields
            .iter()
            .map(|field| field.ty(cx.tcx, args).skip_norm_wip())
            .filter(|ty| !is_zero_sized(cx, *ty));
        match (fields.next(), fields.next()) {
            (Some(ty), None) => peel_pointers(cx, ty) == target,
            _ => false,
        }
    })
}

fn peel_pointers<'tcx>(cx: &LateContext<'tcx>, mut ty: Ty<'tcx>) -> Ty<'tcx> {
    loop {
        ty = ty.peel_refs();
        match ty.kind() {
            ty::RawPtr(pointee, _) => ty = *pointee,
            ty::Adt(adt, args)
                if ty.is_box()
                    || cx
                        .tcx
                        .get_diagnostic_name(adt.did())
                        .is_some_and(|name| matches!(name, sym::Rc | sym::Arc)) =>
            {
                ty = args.type_at(0);
            },
            _ => return ty,
        }
    }
}

#[test]
fn ui() {
    crate::ui_test("deref_polymorphism");
}
