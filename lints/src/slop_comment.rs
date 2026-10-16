use clippy_utils::diagnostics::span_lint_and_help;
use rustc_ast::{CRATE_NODE_ID, Crate, NodeId};
use rustc_lint::{EarlyContext, EarlyLintPass, Lint, LintStore};
use rustc_session::{declare_lint, impl_lint_pass};
use rustc_span::Span;
use serde::Deserialize;

use crate::comment_scan::{CommentGroup, CommentLine, Deferred, LintScopes, comment_groups, flush_deferred_findings};

declare_lint! {
    /// ### What it does
    /// Flags comments that are artifacts of code generation rather than
    /// documentation. Four categories are recognised, matched case-insensitively:
    ///
    /// - `elision`: placeholders standing in for code that is not there, such as
    ///   `// ...`, `// ... rest of the code`, `// existing code unchanged`,
    ///   `// your code here`, `// add more variants as needed`, `// etc.`
    /// - `narration`: comments that describe an edit instead of the code, such as
    ///   `// Added error handling`, `// Fixed: off-by-one`, `// Now uses a HashMap`,
    ///   `// Previously, this cloned the vector`, `// We used to panic here`.
    /// - `hedging`: disclaimers that the code is not the real thing, such as
    ///   `// In a real application ...`, `// For simplicity ...`,
    ///   `// This is a placeholder`, `// Replace this with a real ...`,
    ///   `// You might want to ...`, `// For demonstration purposes`.
    /// - `unfinished`: markers admitting that behaviour is missing, such as
    ///   `// TODO: implement`, `// TODO: add error handling`,
    ///   `// TODO: handle errors`, `// implement me`, `// not yet implemented`.
    ///
    /// ### Why is this bad?
    /// These comments carry no information about the code in front of the reader.
    /// Elisions and placeholders mean code was dropped or never written; narration
    /// belongs in the commit message and goes stale on the next edit; hedges admit
    /// the code is not fit for purpose without saying how.
    ///
    /// ### Known problems
    /// The lint is textual. To keep false positives low it only matches narrow
    /// phrasings: narration only at the start of a sentence and only in plain
    /// (non-doc) comments, because doc comments legitimately record API history;
    /// the more conversational hedges (`for simplicity`, `you might want to`) are
    /// likewise only checked in plain comments; fenced code blocks in doc comments
    /// are skipped, so `// ...` in a doc example is fine. `TODO`s that say what is
    /// missing (`TODO(#123): retry on 503`) are not flagged; only bare admissions
    /// such as `TODO: implement` are. `placeholder value` is not flagged, since
    /// writing a placeholder that is patched later is a real technique.
    ///
    /// An `#[allow]`/`#[expect]` applies to comments inside the item, statement,
    /// expression, match arm, or enum variant that carries it (including its own
    /// doc comments). Comments outside any attributed node (for example between
    /// two items) can only be allowed at crate level.
    ///
    /// ### Example
    /// ```rust,ignore
    /// fn load(path: &Path) -> Config {
    ///     // In a real application, this would read the file.
    ///     // Added a default for now.
    ///     Config::default()
    ///     // ... rest of the code
    /// }
    /// ```
    /// Use instead:
    /// ```rust,ignore
    /// fn load(path: &Path) -> io::Result<Config> {
    ///     let text = fs::read_to_string(path)?;
    ///     // Unknown keys are ignored so older binaries accept newer files.
    ///     Ok(toml::from_str(&text)?)
    /// }
    /// ```
    ///
    /// ### Configuration
    /// ```toml
    /// [slop_comment]
    /// extra_patterns = ["as an ai"]          # extra case-insensitive substrings
    /// disabled_categories = ["narration"]    # any of: elision, narration, hedging, unfinished
    /// ```
    pub SLOP_COMMENT,
    Warn,
    "comment that is an artifact of code generation rather than documentation"
}

pub static LINTS: &[&Lint] = &[SLOP_COMMENT];

pub fn register(lint_store: &mut LintStore) {
    lint_store.register_early_lint_pass(Box::new(|| Box::new(SlopComment::new())));
}

#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Category {
    Elision,
    Narration,
    Hedging,
    Unfinished,
}

#[derive(Default, Deserialize)]
struct Config {
    #[serde(default)]
    extra_patterns: Vec<String>,
    #[serde(default)]
    disabled_categories: Vec<Category>,
}

struct Finding {
    span: Span,
    message: String,
    help: &'static str,
}

struct SlopComment {
    config: Config,
    deferred: Deferred<Finding>,
}

impl SlopComment {
    fn new() -> Self {
        let mut config: Config = dylint_linting::config_or_default(SLOP_COMMENT.name_lower().as_str());
        for pattern in &mut config.extra_patterns {
            *pattern = normalize(pattern);
        }
        config.extra_patterns.retain(|pattern| !pattern.is_empty());

        Self {
            config,
            deferred: Deferred::default(),
        }
    }

    fn enabled(&self, category: Category) -> bool {
        !self.config.disabled_categories.contains(&category)
    }

    fn findings(&self, group: &CommentGroup) -> Vec<Finding> {
        let lines: Vec<&CommentLine> = group.lines.iter().filter(|line| !line.in_code_block).collect();
        let words = words(&lines);
        let mut findings = Vec::new();

        if self.enabled(Category::Elision)
            && let Some(line) = lines.iter().find(|line| is_elision(&line.text))
        {
            findings.push(Finding {
                span: line.span,
                message: format!("placeholder comment `{}` stands in for code that is not there", line.text),
                help: "write the code the placeholder stands for, or delete the comment",
            });
        }

        if self.enabled(Category::Unfinished)
            && let Some(line) = lines.iter().find(|line| is_unfinished(&line.text))
        {
            findings.push(Finding {
                span: line.span,
                message: format!("comment `{}` admits that behaviour is missing", line.text),
                help: "implement it, or fail loudly with `todo!()`/`unimplemented!()` or an error, and reference a tracking issue",
            });
        }

        if self.enabled(Category::Hedging)
            && let Some((line, phrase)) = find_phrase(&words, HEDGES, group.doc)
        {
            findings.push(Finding {
                span: lines[line].span,
                message: format!("comment hedges that this is not the real implementation (`{phrase}`)"),
                help: "implement the real behaviour, or state the concrete limitation and link an issue that tracks it",
            });
        }

        if self.enabled(Category::Narration)
            && !group.doc
            && words.len() <= MAX_NARRATION_WORDS
            && let Some((line, phrase)) = find_narration(&words)
        {
            findings.push(Finding {
                span: lines[line].span,
                message: format!("comment narrates an edit (`{phrase}`) instead of describing the code"),
                help: "describe what the code does and why; the history of a change belongs in the commit message",
            });
        }

        if !self.config.extra_patterns.is_empty() {
            let joined: Vec<String> = lines.iter().map(|line| normalize(&line.text)).collect();
            let text = joined.join(" ");
            for pattern in &self.config.extra_patterns {
                if let Some(at) = text.find(pattern.as_str()) {
                    findings.push(Finding {
                        span: lines[line_at(&joined, at)].span,
                        message: format!("comment matches the configured slop pattern `{pattern}`"),
                        help: "rewrite the comment to say what the code does and why",
                    });
                    break;
                }
            }
        }

        findings
    }

    fn flush(&mut self, cx: &EarlyContext<'_>, id: NodeId) {
        for finding in self.deferred.take(id) {
            span_lint_and_help(cx, SLOP_COMMENT, finding.span, finding.message, None, finding.help);
        }
    }
}

impl_lint_pass!(SlopComment => [SLOP_COMMENT]);

impl EarlyLintPass for SlopComment {
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

/// Lowercase, typographic apostrophes folded, whitespace collapsed.
fn normalize(text: &str) -> String {
    text.to_lowercase()
        .replace('\u{2019}', "'")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The index of the line that byte `at` of `lines.join(" ")` falls in.
fn line_at(lines: &[String], at: usize) -> usize {
    let mut end = 0;
    for (index, line) in lines.iter().enumerate() {
        end += line.len() + 1;
        if at < end {
            return index;
        }
    }
    lines.len().saturating_sub(1)
}

struct Word {
    text: String,
    line: usize,
    /// Whether the word opens a sentence, list item, or comment.
    sentence_start: bool,
    /// The punctuation character directly after the word, if any.
    followed_by: Option<char>,
}

fn words(lines: &[&CommentLine]) -> Vec<Word> {
    let mut words: Vec<Word> = Vec::new();

    for (index, line) in lines.iter().enumerate() {
        let text = normalize(&line.text);
        let bullet = ["- ", "* ", "+ "].iter().any(|bullet| text.starts_with(bullet))
            || text.split_once(". ").is_some_and(|(number, _)| number.chars().all(|c| c.is_ascii_digit()));
        let mut line_start = true;
        let mut chars = text.char_indices().peekable();

        while let Some((start, character)) = chars.next() {
            if !is_word_char(character) {
                continue;
            }
            let mut end = start + character.len_utf8();
            while let Some(&(next, character)) = chars.peek()
                && is_word_char(character)
            {
                end = next + character.len_utf8();
                chars.next();
            }

            let previous = words.last();
            let sentence_start = previous.is_none()
                || (line_start && (bullet || previous.is_some_and(|word| word.line + 1 < index || index == 0)))
                || previous.is_some_and(|word| matches!(word.followed_by, Some('.' | '!' | '?' | ':')));

            words.push(Word {
                text: text[start..end].trim_matches('\'').to_owned(),
                line: index,
                sentence_start,
                followed_by: text[end..].chars().next().filter(char::is_ascii_punctuation),
            });
            line_start = false;
        }
    }

    words
}

fn is_word_char(character: char) -> bool {
    character.is_alphanumeric() || character == '\''
}

/// A phrase is a sequence of slots; each slot lists the words accepted at that
/// position. An empty string makes the slot optional.
struct Phrase {
    slots: &'static [&'static [&'static str]],
    /// Whether the phrase is also slop inside a doc comment.
    in_docs: bool,
    /// Whether the phrase only counts at the start of a sentence.
    opens_sentence: bool,
}

const HEDGES: &[Phrase] = &[
    Phrase {
        slots: &[&["in"], &["a", "an"], &["real", "actual", "proper"], &["world", "life", ""], &[
            "implementation",
            "application",
            "app",
            "world",
            "scenario",
            "system",
            "project",
            "program",
            "codebase",
            "environment",
            "setting",
            "version",
            "life",
            "service",
        ]],
        in_docs: true,
        opens_sentence: false,
    },
    Phrase {
        slots: &[&["in"], &["production", "prod"], &["you", "we"], &["would", "should", "might", "could", "may"]],
        in_docs: false,
        opens_sentence: false,
    },
    Phrase {
        slots: &[&["in"], &["a"], &["production"], &["environment", "setting", "system", "app", "application", "codebase"], &[
            "you", "we",
        ]],
        in_docs: false,
        opens_sentence: false,
    },
    Phrase {
        slots: &[&["for"], &["simplicity", "simplicity's"]],
        in_docs: false,
        opens_sentence: true,
    },
    Phrase {
        slots: &[&["for"], &["the"], &["sake"], &["of"], &["simplicity"]],
        in_docs: false,
        opens_sentence: true,
    },
    Phrase {
        slots: &[&["simplified"], &["version", "implementation", "example", "approach", "logic", "placeholder"]],
        in_docs: false,
        opens_sentence: false,
    },
    Phrase {
        slots: &[&["this", "it", "that", "here"], &["is"], &["just", "only", "merely", ""], &["a", "an"], &[
            "placeholder",
            "simplified",
        ]],
        in_docs: true,
        opens_sentence: false,
    },
    Phrase {
        slots: &[&["it's", "that's", "here's"], &["just", "only", "merely", ""], &["a", "an"], &["placeholder", "simplified"]],
        in_docs: true,
        opens_sentence: false,
    },
    Phrase {
        slots: &[&["this", "it", "that", "here"], &["is"], &["just", "only", "merely", ""], &["a", "an"], &["stub"]],
        in_docs: false,
        opens_sentence: false,
    },
    Phrase {
        slots: &[&["this", "it", "that", "here"], &["is"], &["just", "only", "merely", ""], &["a"], &["mock", "dummy", "fake"], &[
            "implementation",
            "version",
        ]],
        in_docs: false,
        opens_sentence: false,
    },
    Phrase {
        slots: &[&["placeholder"], &["implementation", "logic", "code", "body"]],
        in_docs: true,
        opens_sentence: false,
    },
    Phrase {
        slots: &[&["you"], &["may", "might", "would", "could"], &["want"], &["to"]],
        in_docs: false,
        opens_sentence: false,
    },
    Phrase {
        slots: &[&["you'd", "you'll"], &["want"], &["to"]],
        in_docs: false,
        opens_sentence: false,
    },
    Phrase {
        slots: &[&["you", "one"], &["would", "should"], &["normally", "typically", "usually"]],
        in_docs: false,
        opens_sentence: false,
    },
    Phrase {
        slots: &[&["for"], &["demonstration", "demo", "illustration", "illustrative", "example", "educational"], &[
            "purposes", "purpose",
        ]],
        in_docs: false,
        opens_sentence: false,
    },
    Phrase {
        slots: &[&["replace"], &["this", "it", ""], &["with"], &["a", "an", "the", "your", ""], &[
            "real",
            "actual",
            "proper",
            "production",
            "correct",
        ]],
        in_docs: false,
        opens_sentence: false,
    },
    Phrase {
        slots: &[&["should", "must"], &["be"], &["replaced"], &["with", "by"], &["a", "an", "the", ""], &[
            "real", "proper",
        ]],
        in_docs: false,
        opens_sentence: false,
    },
];

/// Matches `slots` against `words` starting at `at`; returns the end index.
fn match_at(words: &[Word], at: usize, slots: &[&[&str]]) -> Option<usize> {
    let Some((slot, rest)) = slots.split_first() else {
        return Some(at);
    };

    slot.iter().find_map(|alternative| {
        if alternative.is_empty() {
            match_at(words, at, rest)
        } else if words.get(at).is_some_and(|word| word.text == *alternative) {
            match_at(words, at + 1, rest)
        } else {
            None
        }
    })
}

fn phrase_text(words: &[Word]) -> String {
    words.iter().map(|word| word.text.as_str()).collect::<Vec<_>>().join(" ")
}

/// The first phrase occurrence: its line index and matched text.
fn find_phrase(words: &[Word], phrases: &[Phrase], doc: bool) -> Option<(usize, String)> {
    (0..words.len()).find_map(|at| {
        phrases
            .iter()
            .filter(|phrase| (phrase.in_docs || !doc) && (!phrase.opens_sentence || words[at].sentence_start))
            .find_map(|phrase| match_at(words, at, phrase.slots))
            .map(|end| (words[at].line, phrase_text(&words[at..end])))
    })
}

/// Longer comments that mention history usually explain a pitfall with its
/// reasoning ("we used to do X, but that is wrong because ..."), which is
/// documentation; generated narration is a terse one-liner.
const MAX_NARRATION_WORDS: usize = 20;

/// Past-tense verbs that open a change description.
const EDIT_VERBS: &[&str] = &[
    "added",
    "changed",
    "updated",
    "fixed",
    "removed",
    "modified",
    "replaced",
    "moved",
    "renamed",
    "refactored",
    "switched",
];

/// Words that, after an edit verb, mark the verb as narration rather than an
/// adjective (`Removed entries are ...`, `Fixed point iteration`).
const EDIT_OBJECTS: &[&str] = &[
    "a",
    "an",
    "the",
    "new",
    "support",
    "proper",
    "missing",
    "more",
    "extra",
    "error",
    "check",
    "checks",
    "validation",
    "logging",
    "logic",
    "code",
    "handling",
    "import",
    "imports",
    "field",
    "fields",
    "method",
    "methods",
    "function",
    "functions",
    "helper",
    "helpers",
    "test",
    "tests",
    "type",
    "types",
    "comment",
    "comments",
    "this",
    "these",
    "unused",
    "old",
    "debug",
    "explicit",
    "caching",
    "docs",
    "documentation",
    "bounds",
    "bug",
];

const NOW_VERBS: &[&str] = &[
    "uses",
    "returns",
    "handles",
    "supports",
    "accepts",
    "takes",
    "calls",
    "checks",
    "validates",
    "includes",
    "works",
    "correctly",
    "properly",
    "also",
];

const PREVIOUSLY_FOLLOWERS: &[&str] = &[
    "this", "it", "we", "the", "used", "was", "were", "returned", "had", "did", "called", "would",
];

fn find_narration(words: &[Word]) -> Option<(usize, String)> {
    let text_at = |at: usize| words.get(at).map(|word| word.text.as_str());

    (0..words.len()).filter(|&at| words[at].sentence_start).find_map(|at| {
        let word = &words[at];
        let next = text_at(at + 1);

        let end = match word.text.as_str() {
            verb if EDIT_VERBS.contains(&verb) => {
                if word.followed_by == Some(':') {
                    return Some((word.line, format!("{verb}:")));
                }

                let narrates = verb == "refactored"
                    || next.is_some_and(|next| {
                        EDIT_OBJECTS.contains(&next)
                            || (matches!(verb, "changed" | "moved" | "renamed" | "switched")
                                && matches!(next, "to" | "from" | "into" | "back"))
                            || (verb == "replaced" && next == "with")
                            || (verb == "moved" && matches!(next, "here" | "out"))
                    });
                narrates.then_some((at + 2).min(words.len()))
            },
            "new" if word.followed_by == Some(':') => return Some((word.line, "new:".to_owned())),
            "now" if next.is_some_and(|next| NOW_VERBS.contains(&next)) => Some(at + 2),
            "this" | "it" if next == Some("now") && text_at(at + 2).is_some_and(|verb| NOW_VERBS.contains(&verb)) => {
                Some(at + 3)
            },
            "previously" | "formerly"
                if matches!(word.followed_by, Some(',' | ':'))
                    || next.is_some_and(|next| PREVIOUSLY_FOLLOWERS.contains(&next)) =>
            {
                Some(at + 1)
            },
            "this" | "it" | "we" if next == Some("used") && text_at(at + 2) == Some("to") => Some(at + 3),
            _ => None,
        }?;

        Some((word.line, phrase_text(&words[at..end])))
    })
}

/// Lowercased words of `text`, ignoring punctuation.
fn plain_words(text: &str) -> Vec<String> {
    normalize(text)
        .split(|character: char| !is_word_char(character))
        .map(|word| word.trim_matches('\''))
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

/// `words` as matchable words without position information.
fn bare_words(words: &[String]) -> Vec<Word> {
    words
        .iter()
        .map(|text| Word {
            text: text.clone(),
            line: 0,
            sentence_start: false,
            followed_by: None,
        })
        .collect()
}

/// Whether the whole of `words` matches one of `phrases`.
fn matches_whole(words: &[String], phrases: &[&[&[&str]]]) -> bool {
    let words = bare_words(words);
    phrases
        .iter()
        .any(|slots| match_at(&words, 0, slots) == Some(words.len()))
}

fn is_elision(text: &str) -> bool {
    let text = normalize(text);
    let has_ellipsis = text.contains("...") || text.contains('\u{2026}');
    let core = text.trim_matches(|character: char| {
        character.is_whitespace() || matches!(character, '.' | '\u{2026}' | '(' | ')' | '[' | ']' | '-' | '\u{2014}' | '*')
    });

    if core.is_empty() {
        return has_ellipsis;
    }

    if core == "etc" {
        return true;
    }

    let words = plain_words(core);

    // Completed placeholders: the whole comment is the elision.
    const TAILS: &[&str] = &[
        "",
        "here",
        "unchanged",
        "remains",
        "remain",
        "stays",
        "stay",
        "omitted",
        "elided",
        "goes",
        "follows",
        "continues",
        "as",
    ];
    const WHOLE: &[&[&[&str]]] = &[
        // rest of the code / remainder of the function [remains unchanged]
        &[
            &["the", ""],
            &["rest", "remainder"],
            &["of"],
            &["the", "your", "this", ""],
            &["code", "implementation", "impl", "function", "method", "file", "logic", "body", "module", "program"],
        ],
        // [keep] existing code [here]
        &[
            &["keep", "keeping", "the", "your", "rest", ""],
            &["of", ""],
            &["the", ""],
            &["existing"],
            &["code", "implementation", "logic", "methods", "fields", "functions", "imports", "tests", "variants", "impls"],
        ],
        &[&["your", "the", "actual", "real"], &["code", "logic", "implementation"], &["goes", ""], &["here"]],
        &[&["add", "put", "insert", "write"], &["your"], &["code", "logic", "implementation"], &["here"]],
        &[&["implementation", "code", "logic"], &["goes", ""], &["here"]],
    ];

    let matchable = bare_words(&words);
    let matches_prefix = |slots: &[&[&str]]| {
        match_at(&matchable, 0, slots).is_some_and(|end| {
            words.get(end).is_none_or(|tail| TAILS.contains(&tail.as_str()))
                // `is the same`, `is unchanged`, `is omitted`
                || (words[end] == "is"
                    && words
                        .get(end + 1)
                        .is_some_and(|next| matches!(next.as_str(), "the" | "same" | "unchanged" | "omitted" | "elided")))
        })
    };

    if WHOLE.iter().any(|slots| matches_prefix(slots)) {
        return true;
    }

    // `// ... rest of the sync logic`: a leading ellipsis marks elided code
    // whatever the trailing description is.
    let opens_with_ellipsis = text.trim_start().starts_with("...") || text.trim_start().starts_with('\u{2026}');
    if opens_with_ellipsis
        && words.first().is_some_and(|word| matches!(word.as_str(), "rest" | "remainder"))
        && words.get(1).is_some_and(|word| word == "of")
    {
        return true;
    }

    // add more variants here / add more checks as needed
    if words.first().is_some_and(|word| word == "add")
        && words.get(1).is_some_and(|word| matches!(word.as_str(), "more" | "other"))
        && (words.last().is_some_and(|word| word == "here") || core.ends_with("as needed") || core.ends_with("if needed"))
    {
        return true;
    }

    // `// ... other fields`, `// ... more code ...`
    has_ellipsis
        && matches_whole(&words, &[&[
            &["other", "more", "remaining", "additional"],
            &[
                "code", "fields", "methods", "variants", "functions", "cases", "arms", "logic", "impls", "items", "tests",
            ],
            &["here", ""],
        ]])
}

fn is_unfinished(text: &str) -> bool {
    let text = normalize(text);

    // `TODO: implement`, but not `TODO(#123): implement ...`.
    let rest = ["todo", "fixme"].iter().find_map(|marker| text.strip_prefix(marker));
    let words = match rest {
        Some(rest) if rest.starts_with('(') => return false,
        Some(rest) if rest.starts_with([':', ' ', '-', '!']) => plain_words(rest),
        Some(_) => return false,
        None => plain_words(&text),
    };

    const WHOLE: &[&[&[&str]]] = &[
        &[&["implement"], &["this", "me", "it", ""], &["properly", "later", "here", "function", "method", ""]],
        &[&["add"], &["proper", "real", ""], &["error"], &["handling"], &["here", "later", ""]],
        &[&["handle"], &["the", ""], &["error", "errors"], &["properly", "here", "later", "case", "cases", ""]],
        &[&["this", "it", "it's", ""], &["is", ""], &["not"], &["yet", ""], &["implemented"], &["yet", ""]],
    ];

    // A bare `TODO: implement` is too short to be a statement of intent, but
    // `implement` alone outside a TODO is just a word.
    if rest.is_none() && words.len() == 1 {
        return false;
    }

    matches_whole(&words, WHOLE)
}

#[test]
fn ui() {
    crate::ui_test("slop_comment");
}
