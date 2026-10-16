use clippy_utils::diagnostics::span_lint_and_help;
use rustc_ast::{Attribute, LitKind, MetaItemKind};
use rustc_lint::{EarlyContext, EarlyLintPass, Lint, LintStore};
use rustc_session::{declare_lint, impl_lint_pass};
use rustc_span::sym;
use serde::Deserialize;

declare_lint! {
    /// ### What it does
    /// Flags `#[allow(.., reason = "..")]` and `#[expect(.., reason = "..")]`
    /// whose reason says nothing: fewer than `min_words` words, or only
    /// placeholder phrases such as "needed", "intentional", "false positive",
    /// "temporary" or "fix later".
    ///
    /// ### Why is this bad?
    /// A reason exists to record the invariant or the evidence that makes the
    /// suppression correct, so the next reader can check it still holds. A
    /// placeholder reason is just an `allow` that slips past
    /// `clippy::allow_attributes_without_reason`.
    ///
    /// ### Known problems
    /// The check is textual: a long reason can still be empty of content. Short
    /// reasons that contain a URL or an issue reference (`#1234`) are accepted,
    /// because the reference is the evidence. Attributes produced by macros are
    /// ignored.
    ///
    /// ### Example
    /// ```rust,ignore
    /// #[expect(clippy::cast_possible_truncation, reason = "intentional")]
    /// let byte = value as u8;
    /// ```
    /// Use instead:
    /// ```rust,ignore
    /// #[expect(clippy::cast_possible_truncation, reason = "value is masked to 0..=255 above")]
    /// let byte = value as u8;
    /// ```
    ///
    /// ### Configuration
    /// ```toml
    /// [low_signal_lint_reason]
    /// min_words = 3                       # reasons with fewer words are flagged
    /// extra_phrases = ["as discussed"]    # more placeholder phrases
    /// ```
    pub LOW_SIGNAL_LINT_REASON,
    Warn,
    "lint suppression reason that does not explain anything"
}

pub static LINTS: &[&Lint] = &[LOW_SIGNAL_LINT_REASON];

pub fn register(lint_store: &mut LintStore) {
    lint_store.register_early_lint_pass(Box::new(|| Box::new(LowSignalLintReason::new())));
}

/// Phrases that carry no information on their own. A reason made only of these
/// (and filler words) is flagged.
const PLACEHOLDER_PHRASES: &[&str] = &[
    "needed",
    "required",
    "necessary",
    "intentional",
    "intended",
    "on purpose",
    "false positive",
    "fp",
    "ok",
    "okay",
    "fine",
    "it's fine",
    "safe",
    "clippy is wrong",
    "lint is wrong",
    "temporary",
    "temp",
    "todo",
    "fix later",
    "fix this later",
    "legacy",
    "for now",
    "not needed",
    "unused",
    "works",
    "expected",
    "allowed",
    "ignore",
    "suppress",
    "suppressed",
    "silence",
    "warning",
];

/// Words that are dropped before matching placeholder phrases, so that
/// "this is needed here" reduces to "needed".
const FILLER_WORDS: &[&str] = &[
    "a", "an", "the", "this", "that", "it", "its", "is", "was", "be", "are", "here", "just", "so", "we", "i", "for",
    "now", "and", "or", "but", "also", "still", "currently", "really", "very",
];

fn default_min_words() -> usize {
    3
}

#[derive(Deserialize)]
struct Config {
    #[serde(default = "default_min_words")]
    min_words: usize,
    #[serde(default)]
    extra_phrases: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            min_words: default_min_words(),
            extra_phrases: Vec::new(),
        }
    }
}

struct LowSignalLintReason {
    min_words: usize,
    /// Every placeholder phrase, normalized into filler-free word sequences.
    phrases: Vec<Vec<String>>,
}

impl LowSignalLintReason {
    fn new() -> Self {
        let config: Config = dylint_linting::config_or_default(LOW_SIGNAL_LINT_REASON.name_lower().as_str());
        let phrases = PLACEHOLDER_PHRASES
            .iter()
            .copied()
            .chain(config.extra_phrases.iter().map(String::as_str))
            .map(|phrase| without_filler(&words(phrase)))
            .filter(|phrase| !phrase.is_empty())
            .collect();

        Self {
            min_words: config.min_words,
            phrases,
        }
    }

    fn is_low_signal(&self, reason: &str) -> bool {
        if cites_evidence(reason) {
            return false;
        }

        let words = words(reason);
        words.len() < self.min_words || self.is_only_placeholders(&without_filler(&words))
    }

    /// Whether `words` can be split entirely into placeholder phrases, as in
    /// "intentional false positive" or "temporary fix later".
    fn is_only_placeholders(&self, words: &[String]) -> bool {
        // covered[i]: the first `i` words are a sequence of placeholder phrases.
        let mut covered = vec![false; words.len() + 1];
        covered[0] = true;

        for start in 0..words.len() {
            if !covered[start] {
                continue;
            }
            for phrase in &self.phrases {
                if words[start..].starts_with(phrase) {
                    covered[start + phrase.len()] = true;
                }
            }
        }

        covered[words.len()]
    }
}

impl_lint_pass!(LowSignalLintReason => [LOW_SIGNAL_LINT_REASON]);

impl EarlyLintPass for LowSignalLintReason {
    fn check_attribute(&mut self, cx: &EarlyContext<'_>, attr: &Attribute) {
        let Some(level) = attr.name() else {
            return;
        };

        if !matches!(level, sym::allow | sym::expect) || attr.span.from_expansion() {
            return;
        }

        let Some(items) = attr.meta_item_list() else {
            return;
        };

        for item in &items {
            if let Some(meta) = item.meta_item()
                && meta.has_name(sym::reason)
                && let MetaItemKind::NameValue(lit) = &meta.kind
                && let LitKind::Str(reason, _) = lit.kind
                && self.is_low_signal(reason.as_str())
            {
                span_lint_and_help(
                    cx,
                    LOW_SIGNAL_LINT_REASON,
                    lit.span,
                    format!("`{level}` reason {:?} does not say why the suppression is correct", reason.as_str()),
                    None,
                    "state the invariant or evidence the lint cannot see, e.g. \
                     `reason = \"index is bounded by the length check above\"`",
                );
            }
        }
    }
}

/// Lowercased words of `text`; punctuation separates words and apostrophes are dropped.
fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .replace(['\'', '\u{2019}'], "")
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

fn without_filler(words: &[String]) -> Vec<String> {
    words
        .iter()
        .filter(|word| !FILLER_WORDS.contains(&word.as_str()))
        .cloned()
        .collect()
}

/// Whether `reason` points at a URL or an issue (`#1234`, `rust-clippy#7797`).
fn cites_evidence(reason: &str) -> bool {
    reason.contains("://")
        || reason
            .match_indices('#')
            .any(|(index, _)| reason[index + 1..].starts_with(|character: char| character.is_ascii_digit()))
}

#[test]
fn ui() {
    crate::ui_test("low_signal_lint_reason");
}
