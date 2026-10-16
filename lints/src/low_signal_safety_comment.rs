use clippy_utils::diagnostics::span_lint_and_help;
use rustc_ast::{CRATE_NODE_ID, Crate, NodeId};
use rustc_lint::{EarlyContext, EarlyLintPass, Lint, LintStore};
use rustc_session::{declare_lint, impl_lint_pass};
use rustc_span::Span;
use serde::Deserialize;

use crate::comment_scan::{CommentGroup, Deferred, LintScopes, comment_groups, flush_deferred_findings};

declare_lint! {
    /// ### What it does
    /// Flags `// SAFETY:` comments (line or block, any case) whose justification
    /// is too short to say anything (fewer than `min_words` words, 4 by default)
    /// or is a generic reassurance such as "this is safe", "trust me",
    /// "should be fine", "no UB", or "the pointer is valid".
    ///
    /// ### Why is this bad?
    /// A `SAFETY:` comment exists to show a reviewer why every precondition of
    /// the unsafe operation holds at this call site. `clippy::undocumented_unsafe_blocks`
    /// only checks that the comment exists, so a placeholder satisfies it while
    /// documenting nothing. The justification should name the invariants
    /// (non-null, aligned, initialized, in bounds, not aliased, ...) and point at
    /// what guarantees them.
    ///
    /// ### Known problems
    /// Word counting is a heuristic: a terse but precise reference such as
    /// `// SAFETY: see invariant (2) on Arena` passes, and `// SAFETY: checked above`
    /// is flagged even if the check really is directly above. Continuation lines
    /// of the same comment count toward the justification. `# Safety` sections in
    /// doc comments are not checked.
    ///
    /// An `#[allow]`/`#[expect]` applies to comments inside the item, statement,
    /// or expression that carries it; comments outside any attributed node can
    /// only be allowed at crate level.
    ///
    /// ### Example
    /// ```rust,ignore
    /// // SAFETY: this is safe.
    /// let first = unsafe { *slice.get_unchecked(0) };
    /// ```
    /// Use instead:
    /// ```rust,ignore
    /// // SAFETY: `slice` is non-empty: the early return above handles `len == 0`.
    /// let first = unsafe { *slice.get_unchecked(0) };
    /// ```
    ///
    /// ### Configuration
    /// ```toml
    /// [low_signal_safety_comment]
    /// min_words = 4   # minimum number of words after `SAFETY:`
    /// ```
    pub LOW_SIGNAL_SAFETY_COMMENT,
    Warn,
    "`SAFETY:` comment whose justification is empty or generic"
}

pub static LINTS: &[&Lint] = &[LOW_SIGNAL_SAFETY_COMMENT];

pub fn register(lint_store: &mut LintStore) {
    lint_store.register_early_lint_pass(Box::new(|| Box::new(LowSignalSafetyComment::new())));
}

#[derive(Deserialize)]
struct Config {
    #[serde(default = "default_min_words")]
    min_words: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            min_words: default_min_words(),
        }
    }
}

fn default_min_words() -> usize {
    4
}

struct Finding {
    span: Span,
    message: String,
}

struct LowSignalSafetyComment {
    config: Config,
    deferred: Deferred<Finding>,
}

impl LowSignalSafetyComment {
    fn new() -> Self {
        Self {
            config: dylint_linting::config_or_default(LOW_SIGNAL_SAFETY_COMMENT.name_lower().as_str()),
            deferred: Deferred::default(),
        }
    }

    fn findings(&self, group: &CommentGroup) -> Vec<Finding> {
        let mut findings = Vec::new();

        if group.doc {
            return findings;
        }

        let mut index = 0;
        while index < group.lines.len() {
            let Some(first) = safety_justification(&group.lines[index].text) else {
                index += 1;
                continue;
            };

            // The justification continues to the end of the comment or the next `SAFETY:`.
            let mut text = first.to_owned();
            let mut last = index;
            while let Some(line) = group.lines.get(last + 1)
                && safety_justification(&line.text).is_none()
            {
                text.push(' ');
                text.push_str(&line.text);
                last += 1;
            }

            let words = words(&text);
            // Trailing empty comment lines are not part of the justification.
            while last > index && group.lines[last].text.is_empty() {
                last -= 1;
            }
            let span = group.lines[index].span.to(group.lines[last].span);
            let shown = text.split_whitespace().collect::<Vec<_>>().join(" ");

            if words.is_empty() {
                findings.push(Finding {
                    span,
                    message: "`SAFETY:` comment gives no justification".to_owned(),
                });
            } else if words.len() < self.config.min_words {
                findings.push(Finding {
                    span,
                    message: format!("`SAFETY:` justification `{shown}` is too short to show why the unsafe code is sound"),
                });
            } else if is_generic(&words) {
                findings.push(Finding {
                    span,
                    message: format!("`SAFETY:` justification `{shown}` is a generic reassurance"),
                });
            }

            index = last + 1;
        }

        findings
    }

    fn flush(&mut self, cx: &EarlyContext<'_>, id: NodeId) {
        for finding in self.deferred.take(id) {
            span_lint_and_help(
                cx,
                LOW_SIGNAL_SAFETY_COMMENT,
                finding.span,
                finding.message,
                None,
                "name each precondition of the unsafe operation and why it holds here, \
                 e.g. why the pointer is non-null, aligned, and initialized",
            );
        }
    }
}

impl_lint_pass!(LowSignalSafetyComment => [LOW_SIGNAL_SAFETY_COMMENT]);

impl EarlyLintPass for LowSignalSafetyComment {
    fn check_crate(&mut self, cx: &EarlyContext<'_>, krate: &Crate) {
        let scopes = LintScopes::new(krate);

        for group in comment_groups(cx, krate) {
            for finding in self.findings(&group) {
                self.deferred
                    .push(scopes.innermost(finding.span).unwrap_or(CRATE_NODE_ID), finding);
            }
        }

        self.flush(cx, CRATE_NODE_ID);
    }

    flush_deferred_findings!();
}

/// The text after a leading `SAFETY:` (any case).
fn safety_justification(line: &str) -> Option<&str> {
    let prefix = line.get(..7)?;
    prefix.eq_ignore_ascii_case("safety:").then(|| &line[7..])
}

fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .replace('\u{2019}', "'")
        .split(|character: char| !(character.is_alphanumeric() || character == '\''))
        .map(|word| word.trim_matches('\''))
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Words that add emphasis but no information.
const FILLER: &[&str] = &[
    "here", "obviously", "clearly", "definitely", "totally", "just", "really", "perfectly", "completely", "actually",
    "trust", "me", "promise", "100", "always", "surely", "certainly",
];

/// Words that join clauses; a justification made only of generic clauses is generic.
const CONJUNCTIONS: &[&str] = &["and", "because", "since", "so", "as", "but", "also"];

const GENERIC: &[&str] = &[
    "",
    "safe",
    "this is safe",
    "it is safe",
    "it's safe",
    "its safe",
    "should be safe",
    "this should be safe",
    "it should be safe",
    "we know this is safe",
    "we know it is safe",
    "we know it's safe",
    "i know this is safe",
    "fine",
    "it's fine",
    "its fine",
    "it is fine",
    "this is fine",
    "should be fine",
    "this should be fine",
    "it should be fine",
    "ok",
    "okay",
    "this is ok",
    "this is okay",
    "it's ok",
    "it is ok",
    "sound",
    "this is sound",
    "it is sound",
    "valid",
    "this is valid",
    "it is valid",
    "it's valid",
    "no ub",
    "there is no ub",
    "there's no ub",
    "this is not ub",
    "not ub",
    "no undefined behavior",
    "no undefined behaviour",
    "there is no undefined behavior",
    "there is no undefined behaviour",
    "the pointer is valid",
    "pointer is valid",
    "ptr is valid",
    "the ptr is valid",
    "the pointer is fine",
    "the pointer is safe",
    "we know what we're doing",
    "we know what we are doing",
    "i know what i'm doing",
    "i know what i am doing",
    "nothing can go wrong",
    "this can't fail",
    "this cannot fail",
    "it can't fail",
    "it cannot fail",
    "can't fail",
    "cannot fail",
    "this never fails",
    "this is never ub",
    "this won't cause ub",
    "this does not cause ub",
    "this doesn't cause ub",
    "the invariants are upheld",
    "invariants are upheld",
    "all invariants are upheld",
    "the safety requirements are met",
    "safety requirements are met",
    "all safety requirements are met",
];

fn is_generic(words: &[String]) -> bool {
    let kept: Vec<&str> = words
        .iter()
        .map(String::as_str)
        .filter(|word| !FILLER.contains(word))
        .collect();

    kept.split(|word| CONJUNCTIONS.contains(word))
        .all(|clause| GENERIC.contains(&clause.join(" ").as_str()))
}

#[test]
fn ui() {
    crate::ui_test("low_signal_safety_comment");
}
