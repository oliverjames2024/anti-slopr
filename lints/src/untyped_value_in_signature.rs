use clippy_utils::diagnostics::span_lint_and_help;
use std::collections::HashSet;

use rustc_hir::def::DefKind;
use rustc_hir::def_id::{DefId, LocalDefId};
use rustc_hir::intravisit::FnKind;
use rustc_hir::{
    Attribute, Body, FieldDef, FnDecl, FnRetTy, Item, ItemKind, TraitFn, TraitItem, TraitItemKind, VariantData,
};
use rustc_lint::{LateContext, LateLintPass, Lint, LintStore};
use rustc_middle::ty::{self, GenericArgKind, Ty, Unnormalized};
use rustc_session::{declare_lint, impl_lint_pass};
use clippy_utils::sym;
use rustc_span::{ExpnKind, Ident, Span};
use serde::Deserialize;

use crate::util::def_path;

declare_lint! {
    /// ### What it does
    /// Flags function parameters, return types, and struct or enum fields whose
    /// type contains a dynamically typed value: `serde_json::Value`,
    /// `serde_json::Map`, `toml::Value`, `toml::Table`, `serde_yaml::Value`,
    /// `serde_yaml::Mapping`, `serde_yml::Value`, `rmpv::Value`, or a `dyn Any`
    /// trait object. The type is searched through wrappers such as `Option`,
    /// `Vec`, `HashMap`, `Result`, references, `Box`, and `impl Trait` outputs.
    ///
    /// ### Why is this bad?
    /// An untyped value moves parsing and validation to every consumer, each of
    /// which must re-check the shape and may disagree about it. Parse at the
    /// boundary into a typed struct or enum so invalid states cannot be
    /// represented (Effective Rust, Item 1: "use the type system to express your
    /// data structures").
    ///
    /// ### Known problems
    /// Functions whose job *is* the boundary (a generic JSON passthrough, a
    /// plugin host) are legitimate; exempt them with `allowed_paths`.
    /// Implementations of a trait are skipped; the trait declaration is flagged
    /// instead. Fields marked `#[serde(flatten)]` (the idiomatic catch-all for
    /// unknown keys) are skipped. The downcasting hooks `as_any`, `as_any_mut`,
    /// and `into_any`, and `Box<dyn Any + Send>` panic payloads in the error
    /// position of a `Result` (`std::thread::Result`), are not reported.
    ///
    /// ### Example
    /// ```rust,ignore
    /// fn create_user(payload: serde_json::Value) -> Result<UserId, Error> {
    ///     let name = payload["name"].as_str().ok_or(Error::MissingName)?;
    ///     ..
    /// }
    /// ```
    /// Use instead:
    /// ```rust,ignore
    /// #[derive(Deserialize)]
    /// struct NewUser { name: String }
    ///
    /// fn create_user(payload: NewUser) -> Result<UserId, Error> { .. }
    /// ```
    ///
    /// ### Configuration
    /// ```toml
    /// [untyped_value_in_signature]
    /// allowed_paths = ["my_crate::proxy::forward"]   # exact fn/type paths or `prefix::` prefixes
    /// extra_types = ["my_crate::dynamic::Blob"]     # more types to treat as untyped
    /// ```
    pub UNTYPED_VALUE_IN_SIGNATURE,
    Warn,
    "signature or field carries an untyped dynamic value instead of a parsed type"
}

pub static LINTS: &[&Lint] = &[UNTYPED_VALUE_IN_SIGNATURE];

pub fn register(lint_store: &mut LintStore) {
    lint_store.register_late_lint_pass(Box::new(|_| Box::new(UntypedValueInSignature::new())));
}

/// `(crate, item name)` pairs; matching on the defining crate and the item name
/// is robust against the crates' internal module layout (`serde_json::value::Value`
/// is re-exported as `serde_json::Value`, `toml::Table` is an alias of `toml::map::Map`).
const UNTYPED_ADTS: &[(&str, &str)] = &[
    ("serde_json", "Value"),
    ("serde_json", "Map"),
    ("toml", "Value"),
    ("toml", "Map"),
    ("toml", "Table"),
    ("serde_yaml", "Value"),
    ("serde_yaml", "Mapping"),
    ("serde_yml", "Value"),
    ("serde_yml", "Mapping"),
    ("rmpv", "Value"),
];

/// Methods whose purpose is handing out `dyn Any` for downcasting.
const ANY_HOOKS: &[&str] = &["as_any", "as_any_mut", "into_any"];

const HELP: &str = "parse it at the boundary into a struct or enum that states the expected shape";

#[derive(Default, Deserialize)]
struct Config {
    #[serde(default)]
    allowed_paths: Vec<String>,
    #[serde(default)]
    extra_types: Vec<String>,
}

struct UntypedValueInSignature {
    config: Config,
}

impl_lint_pass!(UntypedValueInSignature => [UNTYPED_VALUE_IN_SIGNATURE]);

impl UntypedValueInSignature {
    fn new() -> Self {
        Self {
            config: dylint_linting::config_or_default(UNTYPED_VALUE_IN_SIGNATURE.name_lower().as_str()),
        }
    }

    fn is_allowed(&self, cx: &LateContext<'_>, def_id: DefId) -> bool {
        let path = def_path(cx, def_id);
        self.config.allowed_paths.iter().any(|candidate| path_matches(&path, candidate))
    }

    fn is_untyped_adt(&self, cx: &LateContext<'_>, did: DefId) -> bool {
        let krate = cx.tcx.crate_name(did.krate);
        let name = cx.tcx.item_name(did);
        if UNTYPED_ADTS
            .iter()
            .any(|(candidate_krate, candidate_name)| krate.as_str() == *candidate_krate && name.as_str() == *candidate_name)
        {
            return true;
        }

        if self.config.extra_types.is_empty() {
            return false;
        }
        let path = def_path(cx, did);
        self.config.extra_types.iter().any(|candidate| path_matches(&path, candidate))
    }

    /// The first untyped type found inside `root`, searching generic arguments,
    /// trait-object arguments, and the bounds of `impl Trait` types.
    fn find_untyped<'tcx>(&self, cx: &LateContext<'tcx>, root: Ty<'tcx>, allow_any: bool) -> Option<Ty<'tcx>> {
        let mut pending = vec![root];
        let mut seen_opaques = HashSet::new();

        while let Some(ty) = pending.pop() {
            let mut walker = ty.walk();
            while let Some(arg) = walker.next() {
                let GenericArgKind::Type(inner) = arg.kind() else {
                    continue;
                };

                match inner.kind() {
                    ty::Adt(adt, args) => {
                        if self.is_untyped_adt(cx, adt.did()) {
                            return Some(inner);
                        }
                        // `Result<T, Box<dyn Any + Send>>` is the std panic payload; only look at `T`.
                        if cx.tcx.is_diagnostic_item(sym::Result, adt.did())
                            && args.type_at(1).boxed_ty().is_some_and(|boxed| is_dyn_any(cx, boxed))
                        {
                            pending.push(args.type_at(0));
                            walker.skip_current_subtree();
                        }
                    },
                    ty::Dynamic(..) if !allow_any && is_dyn_any(cx, inner) => return Some(inner),
                    ty::Alias(_, alias) => {
                        if let ty::Opaque { def_id } = alias.kind
                            && seen_opaques.insert(def_id)
                        {
                            for (clause, _) in cx
                                .tcx
                                .explicit_item_bounds(def_id)
                                .iter_identity_copied()
                                .map(Unnormalized::skip_norm_wip)
                            {
                                if let ty::ClauseKind::Projection(projection) = clause.kind().skip_binder()
                                    && let Some(term) = projection.term.as_type()
                                {
                                    pending.push(term);
                                }
                            }
                        }
                    },
                    _ => {},
                }
            }
        }

        None
    }

    fn check_signature<'tcx>(
        &self,
        cx: &LateContext<'tcx>,
        def_id: LocalDefId,
        decl: &FnDecl<'tcx>,
        param_names: &[Option<Ident>],
    ) {
        if self.is_allowed(cx, def_id.to_def_id()) {
            return;
        }

        let allow_any = ANY_HOOKS.contains(&cx.tcx.item_name(def_id.to_def_id()).as_str());
        let sig = cx.tcx.fn_sig(def_id).instantiate_identity().skip_binder();

        for (index, (hir_ty, ty)) in decl.inputs.iter().zip(sig.inputs().iter()).enumerate() {
            if is_from_macro(hir_ty.span) {
                continue;
            }
            if let Some(found) = self.find_untyped(cx, *ty, allow_any) {
                let subject = match param_names.get(index).copied().flatten() {
                    Some(ident) if ident.name != rustc_span::kw::Underscore => format!("parameter `{ident}`"),
                    _ => format!("parameter {}", index + 1),
                };
                emit(cx, hir_ty.span, &subject, *ty, found);
            }
        }

        if let FnRetTy::Return(hir_ty) = decl.output
            && !is_from_macro(hir_ty.span)
            && let Some(found) = self.find_untyped(cx, sig.output(), allow_any)
        {
            // For an `async fn` the signature's output is the opaque future; name
            // the written type rather than `impl Future<Output = ..>`.
            let shown = if cx.tcx.asyncness(def_id).is_async() { found } else { sig.output() };
            emit(cx, hir_ty.span, "return type", shown, found);
        }
    }

    fn check_fields(&self, cx: &LateContext<'_>, fields: &[FieldDef<'_>], variant: Option<Ident>) {
        for field in fields {
            if field.span.from_expansion() || is_serde_flatten(cx.tcx.hir_attrs(field.hir_id)) {
                continue;
            }

            let ty = cx.tcx.type_of(field.def_id).instantiate_identity().skip_norm_wip();
            if let Some(found) = self.find_untyped(cx, ty, false) {
                let subject = match variant {
                    Some(variant) => format!("field `{}` of variant `{variant}`", field.ident),
                    None => format!("field `{}`", field.ident),
                };
                emit(cx, field.ty.span, &subject, ty, found);
            }
        }
    }
}

impl<'tcx> LateLintPass<'tcx> for UntypedValueInSignature {
    fn check_fn(
        &mut self,
        cx: &LateContext<'tcx>,
        kind: FnKind<'tcx>,
        decl: &'tcx FnDecl<'tcx>,
        body: &'tcx Body<'tcx>,
        span: Span,
        def_id: LocalDefId,
    ) {
        if matches!(kind, FnKind::Closure) || span.from_expansion() || is_trait_impl_item(cx, def_id) {
            return;
        }

        let names: Vec<Option<Ident>> = body.params.iter().map(|param| param.pat.simple_ident()).collect();
        self.check_signature(cx, def_id, decl, &names);
    }

    fn check_trait_item(&mut self, cx: &LateContext<'tcx>, item: &'tcx TraitItem<'tcx>) {
        // Provided methods are visited by `check_fn`; only required ones are handled here.
        if let TraitItemKind::Fn(sig, TraitFn::Required(idents)) = item.kind
            && !item.span.from_expansion()
        {
            self.check_signature(cx, item.owner_id.def_id, sig.decl, idents);
        }
    }

    fn check_item(&mut self, cx: &LateContext<'tcx>, item: &'tcx Item<'tcx>) {
        if item.span.from_expansion() {
            return;
        }

        match item.kind {
            ItemKind::Struct(_, _, ref data) | ItemKind::Union(_, _, ref data)
                if !self.is_allowed(cx, item.owner_id.to_def_id()) =>
            {
                self.check_fields(cx, data.fields(), None);
            },
            ItemKind::Enum(_, _, ref def) if !self.is_allowed(cx, item.owner_id.to_def_id()) => {
                for variant in def.variants {
                    if !matches!(variant.data, VariantData::Unit(..)) {
                        self.check_fields(cx, variant.data.fields(), Some(variant.ident));
                    }
                }
            },
            _ => {},
        }
    }
}

fn emit<'tcx>(cx: &LateContext<'tcx>, span: Span, subject: &str, ty: Ty<'tcx>, found: Ty<'tcx>) {
    let message = match peel_ref_prefix(ty) {
        (prefix, peeled) if peeled == found => {
            format!("{subject} has untyped type `{prefix}{}`", display_name(cx, found))
        },
        _ => format!("{subject} contains untyped `{}`", display_name(cx, found)),
    };
    span_lint_and_help(cx, UNTYPED_VALUE_IN_SIGNATURE, span, message, None, HELP);
}

/// Splits `&'a mut &T` into `"&mut &"` and `T`.
fn peel_ref_prefix(mut ty: Ty<'_>) -> (String, Ty<'_>) {
    let mut prefix = String::new();
    while let ty::Ref(_, inner, mutability) = ty.kind() {
        prefix.push('&');
        prefix.push_str(mutability.prefix_str());
        ty = *inner;
    }
    (prefix, ty)
}

/// Whether `span` comes from a macro expansion (as opposed to compiler
/// desugaring such as `async fn`).
fn is_from_macro(span: Span) -> bool {
    span.from_expansion() && matches!(span.ctxt().outer_expn_data().kind, ExpnKind::Macro(..))
}

/// The public name of an untyped type: `serde_json::Value` rather than
/// `serde_json::value::Value`, `serde_json::Map` without its fixed arguments.
fn display_name(cx: &LateContext<'_>, ty: Ty<'_>) -> String {
    match ty.kind() {
        ty::Adt(adt, _) => {
            let did = adt.did();
            let krate = cx.tcx.crate_name(did.krate);
            if UNTYPED_ADTS.iter().any(|(candidate, _)| krate.as_str() == *candidate) {
                format!("{krate}::{}", cx.tcx.item_name(did))
            } else {
                def_path(cx, did)
            }
        },
        ty::Dynamic(..) => "dyn std::any::Any".to_owned(),
        _ => ty.to_string(),
    }
}

fn is_dyn_any(cx: &LateContext<'_>, ty: Ty<'_>) -> bool {
    if let ty::Dynamic(predicates, ..) = ty.kind()
        && let Some(principal) = predicates.principal_def_id()
    {
        cx.tcx.is_diagnostic_item(sym::Any, principal)
    } else {
        false
    }
}

/// `#[serde(flatten)]`, the catch-all for unknown keys alongside typed fields.
fn is_serde_flatten(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path_matches(&[rustc_span::Symbol::intern("serde")])
            && attr
                .meta_item_list()
                .is_some_and(|items| items.iter().any(|item| item.has_name(rustc_span::Symbol::intern("flatten"))))
    })
}

fn path_matches(path: &str, candidate: &str) -> bool {
    match candidate.strip_suffix("::") {
        Some(prefix) => path.strip_prefix(prefix).is_some_and(|rest| rest.starts_with("::")),
        None => path == candidate,
    }
}

fn is_trait_impl_item(cx: &LateContext<'_>, def_id: LocalDefId) -> bool {
    cx.tcx.def_kind(def_id) == DefKind::AssocFn
        && cx
            .tcx
            .opt_parent(def_id.to_def_id())
            .is_some_and(|parent| matches!(cx.tcx.def_kind(parent), DefKind::Impl { of_trait: true }))
}

#[test]
fn ui() {
    crate::ui_test("untyped_value_in_signature");
}
