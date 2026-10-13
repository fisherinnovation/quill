//! Learning from corrections: comparing what dictation typed with how the
//! user fixed it, and suggesting rules so the same mistake doesn't happen
//! again.
//!
//! Only two kinds of fix are turned into suggestions, because a rule that
//! applies everywhere has to be safe everywhere:
//!
//! - a **term** spelled differently: a name, product or jargon word ("Quil"
//!   → "Quill", "kubernetes" → "Kubernetes"). It becomes a vocabulary word,
//!   which steers both the speech model and the clean-up model.
//! - a **split-up word** joined back together ("open router" →
//!   "OpenRouter"). It becomes a replacement, applied to the text before
//!   it's typed.
//!
//! Ordinary word swaps ("there" → "their", "accept" → "except") and
//! rewrites are ignored: as global rules they'd be wrong as often as right.
//! Suggestions are shown to the user, never applied on their own.

use serde::{Deserialize, Serialize};

/// What kind of rule a correction suggests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Vocabulary,
    Replacement,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Vocabulary => "vocabulary",
            Kind::Replacement => "replacement",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "vocabulary" => Some(Kind::Vocabulary),
            "replacement" => Some(Kind::Replacement),
            _ => None,
        }
    }
}

/// A rule suggested by a correction. `from` is empty for vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Learned {
    pub kind: Kind,
    pub from: String,
    pub to: String,
}

/// Longest change (in words, either side) still treated as fixing a word
/// rather than rewriting the sentence.
const MAX_CHANGE_WORDS: usize = 4;

/// Compares what was typed with the user's corrected version and returns
/// the rules it suggests, without duplicates.
pub fn learn(typed: &str, corrected: &str) -> Vec<Learned> {
    let old: Vec<&str> = typed.split_whitespace().collect();
    let new: Vec<&str> = corrected.split_whitespace().collect();
    let mut out: Vec<Learned> = Vec::new();
    for (removed, added, at) in changes(&old, &new) {
        // A capital at the start of a sentence says nothing about a term.
        let sentence_start = at == 0 || new[at - 1].ends_with(['.', '!', '?', ':']);
        if let Some(rule) = classify(removed, added, sentence_start)
            && !out.contains(&rule)
        {
            out.push(rule);
        }
    }
    out
}

/// What a single change suggests, if anything.
fn classify(removed: &[&str], added: &[&str], sentence_start: bool) -> Option<Learned> {
    if removed.is_empty()
        || added.is_empty()
        || removed.len() > MAX_CHANGE_WORDS
        || added.len() > MAX_CHANGE_WORDS
    {
        // Pure insertions and deletions, and rewrites.
        return None;
    }
    let from = clean(&removed.join(" "));
    let to = clean(&added.join(" "));
    if from.is_empty() || to.is_empty() || from == to {
        // Punctuation-only edits are style, not recognition.
        return None;
    }
    let term = is_term(&to, sentence_start);
    if removed.len() > added.len() && (term || letters(&from) == letters(&to)) {
        // "open router" → "OpenRouter", "c plus plus" → "C++": several
        // spoken words typed as one.
        return Some(Learned {
            kind: Kind::Replacement,
            from,
            to,
        });
    }
    if term {
        // A name or term, misspelled or miscapitalized.
        return Some(Learned {
            kind: Kind::Vocabulary,
            from: String::new(),
            to,
        });
    }
    // An ordinary word swapped for another: not safe as a global rule.
    None
}

/// Looks like a name, product or jargon: a capitalized word (unless the
/// capital is just the start of a sentence), a capital inside a word, a
/// digit, or a symbol inside it ("C++", "Node.js").
fn is_term(text: &str, sentence_start: bool) -> bool {
    text.split_whitespace().enumerate().any(|(i, word)| {
        let mut chars = word.chars();
        let first_upper = chars.next().is_some_and(char::is_uppercase);
        let rest_upper = chars.any(char::is_uppercase);
        let digit = word.chars().any(|c| c.is_ascii_digit());
        let symbol = word
            .chars()
            .any(|c| !c.is_alphanumeric() && c != '\'' && c != '’');
        let capitalized = first_upper && word.chars().count() > 1 && !(i == 0 && sentence_start);
        rest_upper || digit || symbol || capitalized
    })
}

/// Trims surrounding punctuation and a possessive "'s", so "Quill's," and
/// "Quill" are the same term.
fn clean(text: &str) -> String {
    let trimmed = text.trim_matches(|c: char| !c.is_alphanumeric() && !"+#".contains(c));
    let trimmed = trimmed
        .strip_suffix("'s")
        .or_else(|| trimmed.strip_suffix("’s"))
        .unwrap_or(trimmed);
    trimmed.to_string()
}

/// Lowercase letters and digits only, to tell "open router" from
/// "OpenRouter" apart only by spacing and case.
fn letters(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Word comparison key: punctuation around a word doesn't count as a
/// change ("Quill," vs "Quill"), but case does ("quill" vs "Quill").
fn key(word: &str) -> &str {
    word.trim_matches(|c: char| !c.is_alphanumeric() && !"+#".contains(c))
}

/// The changed stretches between two word lists, as (removed, added, where
/// the added words start in `new`), from a longest-common-subsequence
/// alignment.
#[allow(clippy::type_complexity)]
fn changes<'a, 'b>(
    old: &'a [&'b str],
    new: &'a [&'b str],
) -> Vec<(&'a [&'b str], &'a [&'b str], usize)> {
    let (n, m) = (old.len(), new.len());
    // lcs[i][j]: common words between old[i..] and new[j..].
    let mut lcs = vec![vec![0_u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if key(old[i]) == key(new[j]) {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    let (mut start_i, mut start_j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && key(old[i]) == key(new[j]) {
            if start_i < i || start_j < j {
                out.push((&old[start_i..i], &new[start_j..j], start_j));
            }
            i += 1;
            j += 1;
            (start_i, start_j) = (i, j);
        } else if j < m && (i == n || lcs[i][j + 1] >= lcs[i + 1][j]) {
            j += 1;
        } else {
            i += 1;
        }
    }
    if start_i < n || start_j < m {
        out.push((&old[start_i..], &new[start_j..], start_j));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vocab(to: &str) -> Learned {
        Learned {
            kind: Kind::Vocabulary,
            from: String::new(),
            to: to.into(),
        }
    }

    fn replace(from: &str, to: &str) -> Learned {
        Learned {
            kind: Kind::Replacement,
            from: from.into(),
            to: to.into(),
        }
    }

    #[test]
    fn misspelled_names_become_vocabulary() {
        assert_eq!(
            learn("Send it to quil by Friday.", "Send it to Quill by Friday."),
            [vocab("Quill")]
        );
        // Punctuation and a possessive around the term don't matter.
        assert_eq!(
            learn("That's Kwil's job.", "That's Quill's job."),
            [vocab("Quill")]
        );
        assert_eq!(
            learn("we use kubernetes now", "we use Kubernetes now"),
            [vocab("Kubernetes")]
        );
        assert_eq!(
            learn("deploy to node one", "deploy to Node1"),
            [replace("node one", "Node1")]
        );
    }

    #[test]
    fn split_words_become_replacements() {
        assert_eq!(
            learn("Ask open router for a key.", "Ask OpenRouter for a key."),
            [replace("open router", "OpenRouter")]
        );
        assert_eq!(
            learn("I asked chat gpt.", "I asked ChatGPT."),
            [replace("chat gpt", "ChatGPT")]
        );
        assert_eq!(
            learn("written in c plus plus", "written in C++"),
            [replace("c plus plus", "C++")]
        );
    }

    #[test]
    fn ordinary_edits_are_ignored() {
        // Homophones, grammar, punctuation, additions, rewrites.
        assert!(learn("Put it over their.", "Put it over there.").is_empty());
        // A capital that only starts a sentence isn't a name.
        assert!(learn("Done. their turn now", "Done. They're turn now").is_empty());
        assert!(learn("the plan works", "The plan works").is_empty());
        assert!(learn("Thanks, see you soon", "Thanks! See you soon.").is_empty());
        assert!(learn("Let's meet tomorrow.", "Let's meet tomorrow at noon.").is_empty());
        assert!(learn("Let's meet tomorrow at noon.", "Let's meet tomorrow.").is_empty());
        assert!(
            learn(
                "I think we should probably push the launch back a bit",
                "Launch moves to next Tuesday, pending legal review today"
            )
            .is_empty()
        );
    }

    #[test]
    fn several_fixes_in_one_correction() {
        assert_eq!(
            learn(
                "Tell Priya the open router key is in one password, and quil reads it.",
                "Tell Priya the OpenRouter key is in 1Password, and Quill reads it."
            ),
            [
                replace("open router", "OpenRouter"),
                replace("one password", "1Password"),
                vocab("Quill")
            ]
        );
    }

    #[test]
    fn duplicates_are_suggested_once() {
        assert_eq!(learn("quil and quil", "Quill and Quill"), [vocab("Quill")]);
    }
}
