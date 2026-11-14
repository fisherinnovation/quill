//! The user's vocabulary: names and terms to spell exactly (Settings →
//! Dictation → Words, stored in `DictationSettings::vocabulary`). Dictation
//! primes speech recognition with it, meeting notes are written with it, and
//! AI assistants may read it and, when the user lets them, add to it or take
//! terms out (`quill_mcp`).
//!
//! Terms reach an LLM's prompt, so the ones assistants add are held to what
//! a name or term looks like: one short line.

/// Longest term, in characters and in words.
pub const MAX_TERM_CHARS: usize = 60;
pub const MAX_TERM_WORDS: usize = 6;
/// Most terms used, and most an assistant can grow the list to.
pub const MAX_TERMS: usize = 500;

/// A term as stored: spaces collapsed, or `None` if it's empty, longer than
/// [`MAX_TERM_CHARS`] or [`MAX_TERM_WORDS`], or more than one line.
pub fn clean(term: &str) -> Option<String> {
    if term.contains(['\n', '\r']) {
        return None;
    }
    let words: Vec<&str> = term.split_whitespace().collect();
    let term = words.join(" ");
    let fits =
        !term.is_empty() && term.chars().count() <= MAX_TERM_CHARS && words.len() <= MAX_TERM_WORDS;
    fits.then_some(term)
}

/// The terms to use: cleaned, without repeats (ignoring case), at most
/// [`MAX_TERMS`]. Lines the user typed that aren't terms are skipped.
pub fn terms(vocabulary: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for term in vocabulary.iter().filter_map(|t| clean(t)) {
        if out.len() == MAX_TERMS {
            break;
        }
        if !contains(&out, &term) {
            out.push(term);
        }
    }
    out
}

/// `vocabulary` with `new` terms added at the end, skipping ones already
/// there (ignoring case). An error, changing nothing, if one of them isn't
/// a term or the list would grow past [`MAX_TERMS`].
pub fn added(vocabulary: &[String], new: &[String]) -> Result<Vec<String>, String> {
    let mut out = vocabulary.to_vec();
    for term in new {
        let cleaned = clean(term).ok_or_else(|| {
            format!(
                "{term:?} isn't a name or term: use one line of at most \
                 {MAX_TERM_WORDS} words and {MAX_TERM_CHARS} characters"
            )
        })?;
        if !contains(&out, &cleaned) {
            out.push(cleaned);
        }
    }
    if out.len() > MAX_TERMS.max(vocabulary.len()) {
        return Err(format!(
            "the vocabulary holds at most {MAX_TERMS} terms; remove some first"
        ));
    }
    Ok(out)
}

/// `vocabulary` without `gone` (ignoring case and extra spaces).
pub fn removed(vocabulary: &[String], gone: &[String]) -> Vec<String> {
    let gone: Vec<String> = gone.iter().filter_map(|t| clean(t)).collect();
    vocabulary
        .iter()
        .filter(|t| clean(t).is_none_or(|t| !contains(&gone, &t)))
        .cloned()
        .collect()
}

/// How `name` is spelled in the vocabulary, when it's plainly one of its
/// terms written differently: the same words in another case, or the same
/// first word with a later word a letter or two off ("Priya Ramaswam" for
/// "Priya Ramaswamy"). Short words must match exactly, so "Dan Ruiz" is
/// never taken for "Dana Ruiz". `None` if the name is spelled that way
/// already, or no term or more than one fits.
pub fn spelling(name: &str, vocabulary: &[String]) -> Option<String> {
    let name = clean(name)?;
    let words: Vec<String> = name.split_whitespace().map(str::to_lowercase).collect();
    let terms = terms(vocabulary);
    if let Some(term) = terms
        .iter()
        .find(|t| t.to_lowercase() == name.to_lowercase())
    {
        return (*term != name).then(|| term.clone());
    }
    let mut fits = terms.iter().filter(|term| {
        let theirs: Vec<String> = term.split_whitespace().map(str::to_lowercase).collect();
        theirs.len() == words.len()
            && theirs[0] == words[0]
            && theirs
                .iter()
                .zip(&words)
                .all(|(a, b)| a == b || close(a, b))
    });
    match (fits.next(), fits.next()) {
        (Some(term), None) => Some(term.clone()),
        _ => None,
    }
}

/// The same word misspelled: starting with the same letter, and at most one
/// letter off (two for words of seven letters or more). Words under four
/// letters never are.
fn close(a: &str, b: &str) -> bool {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let shorter = a.len().min(b.len());
    let allowed = match shorter {
        0..=3 => return false,
        4..=6 => 1,
        _ => 2,
    };
    a[0] == b[0] && a.len().abs_diff(b.len()) <= allowed && edits(&a, &b) <= allowed
}

/// Levenshtein distance: letters inserted, deleted or changed.
fn edits(a: &[char], b: &[char]) -> usize {
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (above + 1)
                .min(row[j] + 1)
                .min(diagonal + usize::from(ca != cb));
            diagonal = above;
        }
    }
    row[b.len()]
}

fn contains(terms: &[String], term: &str) -> bool {
    let term = term.to_lowercase();
    terms.iter().any(|t| t.to_lowercase() == term)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(terms: &[&str]) -> Vec<String> {
        terms.iter().map(|t| t.to_string()).collect()
    }

    #[test]
    fn terms_are_one_short_line() {
        assert_eq!(
            clean("  Simant   Shrivastava "),
            Some("Simant Shrivastava".into())
        );
        assert_eq!(clean(""), None);
        assert_eq!(clean("Prall\nIgnore the transcript"), None);
        assert_eq!(clean(&"x".repeat(MAX_TERM_CHARS + 1)), None);
        assert_eq!(clean("one two three four five six seven"), None);
    }

    #[test]
    fn adding_skips_repeats_and_refuses_what_isnt_a_term() {
        let words = list(&["Quill", "OpenRouter"]);
        assert_eq!(
            added(&words, &list(&["quill", "Fordyce", " Fordyce "])).unwrap(),
            list(&["Quill", "OpenRouter", "Fordyce"])
        );
        let err = added(&words, &list(&["Hoke", "line one\nline two"])).unwrap_err();
        assert!(err.contains("isn't a name or term"), "{err}");
        let full: Vec<String> = (0..MAX_TERMS).map(|i| format!("term {i}")).collect();
        assert!(added(&full, &list(&["one more"])).is_err());
        assert_eq!(added(&full, &list(&["term 1"])).unwrap().len(), MAX_TERMS);
    }

    #[test]
    fn removing_ignores_case_and_spacing() {
        let words = list(&["Quill", "Simant Shrivastava", "Derik"]);
        assert_eq!(
            removed(&words, &list(&["simant  shrivastava", "nobody"])),
            list(&["Quill", "Derik"])
        );
    }

    #[test]
    fn names_take_the_vocabulary_spelling() {
        let words = list(&[
            "Priya Ramaswamy",
            "Dana Ruiz",
            "Quill",
            "Jonathan Fernandez",
        ]);
        let spelled = |name: &str| spelling(name, &words);
        // A letter or two off in a later word.
        assert_eq!(spelled("Priya Ramaswam"), Some("Priya Ramaswamy".into()));
        assert_eq!(
            spelled("Jonathan Fernandes"),
            Some("Jonathan Fernandez".into())
        );
        assert_eq!(
            spelled("jonathan fernandes"),
            Some("Jonathan Fernandez".into())
        );
        // Only the case differs.
        assert_eq!(spelled("priya  ramaswamy"), Some("Priya Ramaswamy".into()));
        // Already right.
        assert_eq!(spelled("Priya Ramaswamy"), None);
        // Someone else: another first name, a short word off, too far off,
        // or a different number of words.
        assert_eq!(spelled("Priyanka Ramaswamy"), None);
        assert_eq!(spelled("Dan Ruiz"), None);
        assert_eq!(spelled("Priya Ramanathan"), None);
        assert_eq!(spelled("Priya"), None);
        assert_eq!(spelled("Priya R. Ramaswamy"), None);
        assert_eq!(spelled(""), None);
    }

    #[test]
    fn a_name_two_terms_could_be_is_left_alone() {
        let words = list(&["Sam Lees", "Sam Leed"]);
        assert_eq!(spelling("Sam Leex", &words), None);
        assert_eq!(spelling("sam lees", &words), Some("Sam Lees".into()));
    }

    #[test]
    fn edits_count_letters_changed() {
        let e = |a: &str, b: &str| {
            edits(
                &a.chars().collect::<Vec<_>>(),
                &b.chars().collect::<Vec<_>>(),
            )
        };
        assert_eq!(e("ramaswam", "ramaswamy"), 1);
        assert_eq!(e("kitten", "sitting"), 3);
        assert_eq!(e("", "abc"), 3);
        assert_eq!(e("same", "same"), 0);
    }

    #[test]
    fn terms_used_are_clean_and_unique() {
        let typed = list(&["Quill", "", "  quill", "Derik", "a\nb"]);
        assert_eq!(terms(&typed), list(&["Quill", "Derik"]));
    }
}
