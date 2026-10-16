use clippy_utils::diagnostics::span_lint_and_help;
use clippy_utils::fn_def_id;
use rustc_hir::{Expr, ExprKind};
use rustc_lint::{LateContext, LateLintPass, Lint, LintStore};
use rustc_session::{declare_lint, impl_lint_pass};
use serde::Deserialize;

use crate::util::{def_path, is_in_async_body};

declare_lint! {
    /// ### What it does
    /// Flags calls to blocking standard-library (and well-known third-party) APIs
    /// made directly inside an `async fn`, `async` block, or `async` closure.
    ///
    /// ### Why is this bad?
    /// Async executors multiplex many tasks onto a few threads. A blocking call
    /// parks the whole worker thread, stalling every other task scheduled on it.
    /// A good rule of thumb is that an async task should not run more than
    /// 10–100 µs between `.await` points (<https://ryhl.io/blog/async-what-is-blocking/>).
    ///
    /// ### Known problems
    /// The lint matches callee paths, so blocking calls hidden behind a helper
    /// function are not found. Very short blocking calls (a `metadata` on a local
    /// file) can be acceptable; suppress those with a reasoned `#[expect]`.
    ///
    /// ### Example
    /// ```rust,ignore
    /// async fn load(path: &Path) -> io::Result<String> {
    ///     std::thread::sleep(Duration::from_millis(10));
    ///     std::fs::read_to_string(path)
    /// }
    /// ```
    /// Use instead:
    /// ```rust,ignore
    /// async fn load(path: &Path) -> io::Result<String> {
    ///     tokio::time::sleep(Duration::from_millis(10)).await;
    ///     tokio::fs::read_to_string(path).await
    /// }
    /// ```
    ///
    /// ### Configuration
    /// ```toml
    /// [blocking_call_in_async]
    /// extra_paths = ["my_crate::db::query_sync"]   # exact paths or `prefix::` prefixes
    /// ```
    pub BLOCKING_CALL_IN_ASYNC,
    Warn,
    "blocking call inside an async body stalls the executor thread"
}

pub static LINTS: &[&Lint] = &[BLOCKING_CALL_IN_ASYNC];

pub fn register(lint_store: &mut LintStore) {
    lint_store.register_late_lint_pass(Box::new(|_| Box::new(BlockingCallInAsync::new())));
}

/// Paths ending in `::` match every item below that module or type.
const BLOCKING_PATHS: &[(&str, &str)] = &[
    ("std::thread::sleep", "use the runtime's timer, e.g. `tokio::time::sleep(..).await`"),
    ("std::thread::JoinHandle::join", "await an async handle or move the join into `spawn_blocking`"),
    ("std::fs::", "use `tokio::fs` (or another async fs API), or move the work into `spawn_blocking`"),
    ("std::io::Stdin::", "use the runtime's async stdin, or read it on a dedicated thread"),
    ("std::io::stdin", "use the runtime's async stdin, or read it on a dedicated thread"),
    ("std::net::", "use the runtime's async networking types, e.g. `tokio::net`"),
    ("std::process::Command::output", "use `tokio::process::Command`, or `spawn_blocking`"),
    ("std::process::Command::status", "use `tokio::process::Command`, or `spawn_blocking`"),
    ("std::process::Child::wait", "use `tokio::process::Child::wait`, or `spawn_blocking`"),
    ("std::process::Child::wait_with_output", "use `tokio::process::Child::wait_with_output`, or `spawn_blocking`"),
    ("std::sync::mpsc::Receiver::recv", "use an async channel, e.g. `tokio::sync::mpsc`"),
    ("std::sync::mpsc::Receiver::recv_timeout", "use an async channel, e.g. `tokio::sync::mpsc`"),
    ("std::sync::Barrier::wait", "use an async barrier, e.g. `tokio::sync::Barrier`"),
    ("std::sync::Condvar::", "use an async notification primitive, e.g. `tokio::sync::Notify`"),
    ("reqwest::blocking::", "use the async `reqwest::Client`"),
];

#[derive(Default, Deserialize)]
struct Config {
    #[serde(default)]
    extra_paths: Vec<String>,
}

struct BlockingCallInAsync {
    config: Config,
}

impl BlockingCallInAsync {
    fn new() -> Self {
        Self {
            config: dylint_linting::config_or_default(BLOCKING_CALL_IN_ASYNC.name_lower().as_str()),
        }
    }

    fn advice(&self, path: &str) -> Option<&'static str> {
        let matches = |candidate: &str| match candidate.strip_suffix("::") {
            Some(prefix) => path.strip_prefix(prefix).is_some_and(|rest| rest.starts_with("::")),
            None => path == candidate,
        };

        BLOCKING_PATHS
            .iter()
            .find(|(candidate, _)| matches(candidate))
            .map(|(_, advice)| *advice)
            .or_else(|| {
                self.config
                    .extra_paths
                    .iter()
                    .any(|candidate| matches(candidate))
                    .then_some("use a non-blocking equivalent, or move the work into `spawn_blocking`")
            })
    }
}

impl_lint_pass!(BlockingCallInAsync => [BLOCKING_CALL_IN_ASYNC]);

impl<'tcx> LateLintPass<'tcx> for BlockingCallInAsync {
    fn check_expr(&mut self, cx: &LateContext<'tcx>, expr: &'tcx Expr<'tcx>) {
        if !matches!(expr.kind, ExprKind::Call(..) | ExprKind::MethodCall(..)) || expr.span.from_expansion() {
            return;
        }

        let Some(callee) = fn_def_id(cx, expr) else {
            return;
        };

        let path = def_path(cx, callee);

        let Some(advice) = self.advice(&path) else {
            return;
        };

        if !is_in_async_body(cx, expr.hir_id) {
            return;
        }

        span_lint_and_help(
            cx,
            BLOCKING_CALL_IN_ASYNC,
            expr.span,
            format!("blocking call to `{path}` inside an async body"),
            None,
            advice,
        );
    }
}

#[test]
fn ui() {
    crate::ui_test("blocking_call_in_async");
}
