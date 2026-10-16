#![feature(rustc_private)]
#![warn(unused_extern_crates)]

//! anti-slopr: opinionated lints that reject low-evidence and low-signal Rust.
//!
//! Each lint lives in its own module. A module exposes `LINTS` (every lint it
//! declares) and `register`, which installs its pass. Adding a lint means adding
//! a module, listing it in `modules!`, and adding a UI example in `ui/`.

extern crate rustc_ast;
extern crate rustc_errors;
extern crate rustc_hir;
extern crate rustc_lexer;
extern crate rustc_lint;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;

dylint_linting::dylint_library!();

mod comment_scan;
mod util;

macro_rules! modules {
    ($($module:ident),* $(,)?) => {
        $(mod $module;)*

        fn all_lints() -> Vec<&'static rustc_lint::Lint> {
            let mut lints = Vec::new();
            $(lints.extend_from_slice($module::LINTS);)*
            lints
        }

        fn register_passes(lint_store: &mut rustc_lint::LintStore) {
            $($module::register(lint_store);)*
        }
    };
}

modules! {
    blanket_lint_suppression,
    blocking_call_in_async,
    deny_warnings_in_source,
    deref_polymorphism,
    discarded_join_handle,
    error_to_string,
    low_signal_lint_reason,
    low_signal_panic_message,
    low_signal_safety_comment,
    missing_item_spacing,
    opaque_public_error,
    silent_error_default,
    slop_comment,
    stringly_typed_error,
    test_without_assertion,
    untyped_value_in_signature,
    vague_symbol_name,
}

#[expect(clippy::no_mangle_with_rust_abi, reason = "dylint loads this symbol by name with the Rust ABI")]
#[unsafe(no_mangle)]
pub fn register_lints(sess: &rustc_session::Session, lint_store: &mut rustc_lint::LintStore) {
    dylint_linting::init_config(sess);
    lint_store.register_lints(&all_lints());
    register_passes(lint_store);
}

/// Runs the UI example named after `lint` with every other anti-slopr lint
/// allowed, so each example's `.stderr` only records the lint under test.
#[cfg(test)]
fn ui_test(lint: &str) {
    ui_test_with_flags(lint, &[]);
}

/// Like [`ui_test`], additionally passing `extra_flags` to rustc (for example
/// `--test`, so that `#[test]` functions and `#[cfg(test)]` modules are compiled).
#[cfg(test)]
fn ui_test_with_flags(lint: &str, extra_flags: &[&str]) {
    let flags = all_lints()
        .into_iter()
        .map(|other| other.name_lower())
        .filter(|other| other != lint)
        .flat_map(|other| ["-A".to_owned(), other])
        .chain(extra_flags.iter().map(|flag| (*flag).to_owned()));

    dylint_testing::ui::Test::example(env!("CARGO_PKG_NAME"), lint)
        .rustc_flags(flags)
        .run();
}
