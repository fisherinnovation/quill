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
    fn terms_used_are_clean_and_unique() {
        let typed = list(&["Quill", "", "  quill", "Derik", "a\nb"]);
        assert_eq!(terms(&typed), list(&["Quill", "Derik"]));
    }
}
