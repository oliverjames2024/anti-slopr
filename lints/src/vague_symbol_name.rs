use clippy_utils::diagnostics::span_lint_and_help;
use clippy_utils::{is_cfg_test, is_entrypoint_fn, is_in_test};
use rustc_hir::{HirId, ImplItem, ImplItemImplKind, ImplItemKind, Item, ItemKind, TraitItem, find_attr};
use rustc_lint::{LateContext, LateLintPass, Lint, LintStore};
use rustc_session::{declare_lint, impl_lint_pass};
use rustc_span::Ident;
use serde::Deserialize;

declare_lint! {
    /// ### What it does
    /// Flags locally defined types, traits, type aliases, functions, methods,
    /// modules, consts and statics whose name contains a vague word such as
    /// `util`, `helper`, `misc`, `manager`, `thing`, `stuff`, `tmp`, `foo` or `my`.
    /// Names are split into words at `_` and at CamelCase humps, and whole words
    /// are compared case-insensitively, so `UserManager`, `string_utils` and
    /// `TMP_DIR` are flagged but `Bark`, `Mystery` and `Shapefile` are not.
    /// `impl` is only flagged in type names (`ServiceImpl`).
    ///
    /// ### Why is this bad?
    /// A vague word names the *kind* of code rather than what it does. `utils`
    /// modules and `Manager` types grow without bound because anything fits;
    /// placeholder names like `foo` or `tmp` record no intent at all.
    ///
    /// ### Known problems
    /// `temp` can abbreviate "temperature" and `shape` is a real domain concept
    /// in geometry code; remove them with `forbidden_segments` if so. Names that
    /// cannot be changed are skipped: trait impl items, `extern` items, the entry
    /// point, `#[no_mangle]`/`#[export_name]` items, macro-generated items,
    /// `#[test]` functions and anything inside `#[cfg(test)]`.
    ///
    /// ### Example
    /// ```rust,ignore
    /// mod utils { pub fn format_thing(..) { .. } }
    /// struct UserManager { .. }
    /// ```
    /// Use instead:
    /// ```rust,ignore
    /// mod duration_format { pub fn format_elapsed(..) { .. } }
    /// struct UserDirectory { .. }
    /// ```
    ///
    /// ### Configuration
    /// ```toml
    /// [vague_symbol_name]
    /// forbidden_segments = ["util", "utils"]   # replaces the default list
    /// extra_segments = ["processor"]           # added to the (default or replaced) list
    /// ```
    pub VAGUE_SYMBOL_NAME,
    Warn,
    "symbol name contains a vague word that names no responsibility"
}

pub static LINTS: &[&Lint] = &[VAGUE_SYMBOL_NAME];

pub fn register(lint_store: &mut LintStore) {
    lint_store.register_late_lint_pass(Box::new(|_| Box::new(VagueSymbolName::new())));
}

const DEFAULT_SEGMENTS: &[&str] = &[
    "util", "utils", "helper", "helpers", "misc", "stuff", "thing", "things", "manager", "shape", "impl", "obj", "foo",
    "bar", "baz", "temp", "tmp", "dummy", "my",
];

/// Segments that are only vague in type names: `ServiceImpl` is vague, but a
/// function `impl_display` usually describes generated or delegated impls.
const TYPE_ONLY_SEGMENTS: &[&str] = &["impl"];

#[derive(Default, Deserialize)]
struct Config {
    forbidden_segments: Option<Vec<String>>,
    #[serde(default)]
    extra_segments: Vec<String>,
}

struct VagueSymbolName {
    segments: Vec<String>,
}

impl VagueSymbolName {
    fn new() -> Self {
        let config: Config = dylint_linting::config_or_default(VAGUE_SYMBOL_NAME.name_lower().as_str());
        let base = config
            .forbidden_segments
            .unwrap_or_else(|| DEFAULT_SEGMENTS.iter().map(|segment| (*segment).to_owned()).collect());

        Self {
            segments: base
                .into_iter()
                .chain(config.extra_segments)
                .map(|segment| segment.to_lowercase())
                .collect(),
        }
    }

    fn check(&self, cx: &LateContext<'_>, hir_id: HirId, ident: Ident, is_type: bool) {
        if ident.span.from_expansion() || is_cfg_test(cx.tcx, hir_id) || is_in_test(cx.tcx, hir_id) {
            return;
        }

        let name = ident.as_str();
        let Some(segment) = name_segments(name).into_iter().find(|segment| {
            let segment = segment.to_lowercase();
            self.segments.contains(&segment) && (is_type || !TYPE_ONLY_SEGMENTS.contains(&segment.as_str()))
        }) else {
            return;
        };

        span_lint_and_help(
            cx,
            VAGUE_SYMBOL_NAME,
            ident.span,
            format!("`{name}` contains the vague word `{}`", segment.to_lowercase()),
            None,
            "name the responsibility instead (what does it do, own, or represent?)",
        );
    }
}

impl_lint_pass!(VagueSymbolName => [VAGUE_SYMBOL_NAME]);

impl<'tcx> LateLintPass<'tcx> for VagueSymbolName {
    fn check_item(&mut self, cx: &LateContext<'tcx>, item: &'tcx Item<'tcx>) {
        let (ident, is_type) = match item.kind {
            ItemKind::Struct(ident, ..)
            | ItemKind::Enum(ident, ..)
            | ItemKind::Union(ident, ..)
            | ItemKind::Trait { ident, .. }
            | ItemKind::TraitAlias(_, ident, ..)
            | ItemKind::TyAlias(ident, ..) => (ident, true),
            ItemKind::Fn { ident, .. } => {
                if is_entrypoint_fn(cx, item.owner_id.to_def_id()) || has_fixed_symbol(cx, item.hir_id()) {
                    return;
                }
                (ident, false)
            },
            ItemKind::Static(_, ident, ..) => {
                if has_fixed_symbol(cx, item.hir_id()) {
                    return;
                }
                (ident, false)
            },
            ItemKind::Const(ident, ..) | ItemKind::Mod(ident, _) => (ident, false),
            _ => return,
        };

        self.check(cx, item.hir_id(), ident, is_type);
    }

    fn check_impl_item(&mut self, cx: &LateContext<'tcx>, item: &'tcx ImplItem<'tcx>) {
        // Names of trait impl items are dictated by the trait.
        if matches!(item.impl_kind, ImplItemImplKind::Trait { .. }) {
            return;
        }

        if matches!(item.kind, ImplItemKind::Fn(..)) && has_fixed_symbol(cx, item.hir_id()) {
            return;
        }

        let is_type = matches!(item.kind, ImplItemKind::Type(..));
        self.check(cx, item.hir_id(), item.ident, is_type);
    }

    fn check_trait_item(&mut self, cx: &LateContext<'tcx>, item: &'tcx TraitItem<'tcx>) {
        let is_type = matches!(item.kind, rustc_hir::TraitItemKind::Type(..));
        self.check(cx, item.hir_id(), item.ident, is_type);
    }
}

/// Whether the item's symbol name is fixed for linking, so renaming it breaks callers outside Rust.
fn has_fixed_symbol(cx: &LateContext<'_>, hir_id: HirId) -> bool {
    find_attr!(cx.tcx, hir_id, NoMangle(..) | ExportName { .. })
}

/// Splits an identifier into its words: `HTTPServerUtils_v2` becomes
/// `["HTTP", "Server", "Utils", "v", "2"]`.
fn name_segments(name: &str) -> Vec<&str> {
    let mut segments = Vec::new();

    for part in name.split('_').filter(|part| !part.is_empty()) {
        let chars: Vec<(usize, char)> = part.char_indices().collect();
        let mut start = 0;

        for window in 1..chars.len() {
            let (index, current) = chars[window];
            let (_, previous) = chars[window - 1];
            let next = chars.get(window + 1).map(|(_, next)| *next);

            let boundary = (previous.is_lowercase() && current.is_uppercase())
                || (previous.is_alphabetic() && current.is_ascii_digit())
                || (previous.is_ascii_digit() && current.is_alphabetic())
                // The last capital of an acronym starts the next word: `HTTPServer`.
                || (previous.is_uppercase() && current.is_uppercase() && next.is_some_and(char::is_lowercase));

            if boundary {
                segments.push(&part[start..index]);
                start = index;
            }
        }

        segments.push(&part[start..]);
    }

    segments
}

#[test]
fn ui() {
    crate::ui_test_with_flags("vague_symbol_name", &["--test"]);
}
