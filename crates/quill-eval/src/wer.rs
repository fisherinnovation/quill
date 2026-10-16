//! Word error rate: how far dictated text is from what should have been
//! typed. Substitutions, deletions and insertions over the reference's word
//! count, from a word-level edit-distance alignment.
//!
//! Words are compared lowercased, without the punctuation around them, and
//! without list markers, so "Sounds good." and "sounds good" match, and a
//! list formatted as lines matches the same items in a sentence. Punctuation
//! inside a word stays: "10:30" and "ten thirty" differ, as they would on
//! screen.

/// One step of an alignment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    Same(String),
    /// Reference word, word typed instead.
    Sub(String, String),
    /// A reference word that's missing.
    Del(String),
    /// A word that shouldn't be there.
    Ins(String),
}

/// How `hypothesis` lines up against `reference`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Alignment {
    pub edits: Vec<Edit>,
}

impl Alignment {
    pub fn tally(&self) -> Tally {
        let mut tally = Tally::default();
        for edit in &self.edits {
            match edit {
                Edit::Same(_) => tally.reference_words += 1,
                Edit::Sub(..) => {
                    tally.reference_words += 1;
                    tally.substitutions += 1;
                }
                Edit::Del(_) => {
                    tally.reference_words += 1;
                    tally.deletions += 1;
                }
                Edit::Ins(_) => tally.insertions += 1,
            }
        }
        tally
    }

    /// The mistakes, briefly: `quil→quill`, `-the`, `+um`.
    pub fn errors(&self) -> Vec<String> {
        self.edits
            .iter()
            .filter_map(|edit| match edit {
                Edit::Same(_) => None,
                Edit::Sub(r, h) => Some(format!("{h}→{r}")),
                Edit::Del(r) => Some(format!("-{r}")),
                Edit::Ins(h) => Some(format!("+{h}")),
            })
            .collect()
    }
}

/// Error counts, summed over one or more alignments.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    pub reference_words: usize,
    pub substitutions: usize,
    pub deletions: usize,
    pub insertions: usize,
}

impl Tally {
    pub fn errors(&self) -> usize {
        self.substitutions + self.deletions + self.insertions
    }

    /// Errors per reference word (0 is perfect; can exceed 1 with many
    /// insertions). 0 for an empty reference with nothing typed.
    pub fn wer(&self) -> f64 {
        if self.reference_words == 0 {
            return if self.errors() == 0 { 0.0 } else { 1.0 };
        }
        self.errors() as f64 / self.reference_words as f64
    }
}

impl std::ops::AddAssign for Tally {
    fn add_assign(&mut self, other: Self) {
        self.reference_words += other.reference_words;
        self.substitutions += other.substitutions;
        self.deletions += other.deletions;
        self.insertions += other.insertions;
    }
}

/// The words compared: lowercased, outer punctuation trimmed, list markers
/// at the start of a line dropped.
pub fn words(text: &str) -> Vec<String> {
    text.lines()
        .flat_map(|line| {
            let mut tokens = line.split_whitespace().peekable();
            if tokens.peek().is_some_and(|t| is_list_marker(t)) {
                tokens.next();
            }
            tokens
        })
        .map(|t| {
            t.replace(['\u{2019}', '\u{2018}'], "'")
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

fn is_list_marker(token: &str) -> bool {
    matches!(token, "-" | "*" | "•")
        || token
            .strip_suffix(['.', ')'])
            .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// Aligns `hypothesis` (what was typed) against `reference` (what should
/// have been) with the fewest edits.
pub fn align(reference: &str, hypothesis: &str) -> Alignment {
    let r = words(reference);
    let h = words(hypothesis);
    // cost[i][j]: edits to turn h[..j] into r[..i].
    let mut cost = vec![vec![0_usize; h.len() + 1]; r.len() + 1];
    for (i, row) in cost.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, c) in cost[0].iter_mut().enumerate() {
        *c = j;
    }
    for i in 1..=r.len() {
        for j in 1..=h.len() {
            let diagonal = cost[i - 1][j - 1] + usize::from(r[i - 1] != h[j - 1]);
            cost[i][j] = diagonal.min(cost[i - 1][j] + 1).min(cost[i][j - 1] + 1);
        }
    }

    let mut edits = Vec::with_capacity(r.len().max(h.len()));
    let (mut i, mut j) = (r.len(), h.len());
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && cost[i][j] == cost[i - 1][j - 1] + usize::from(r[i - 1] != h[j - 1]) {
            edits.push(if r[i - 1] == h[j - 1] {
                Edit::Same(r[i - 1].clone())
            } else {
                Edit::Sub(r[i - 1].clone(), h[j - 1].clone())
            });
            i -= 1;
            j -= 1;
        } else if i > 0 && cost[i][j] == cost[i - 1][j] + 1 {
            edits.push(Edit::Del(r[i - 1].clone()));
            i -= 1;
        } else {
            edits.push(Edit::Ins(h[j - 1].clone()));
            j -= 1;
        }
    }
    edits.reverse();
    Alignment { edits }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_punctuation_and_list_markers_dont_count() {
        assert_eq!(
            words("Sounds good, see you then."),
            ["sounds", "good", "see", "you", "then"]
        );
        assert_eq!(
            words("We need:\n1. The page\n- Pricing\n2) Emails"),
            ["we", "need", "the", "page", "pricing", "emails"]
        );
        // Inside a word, punctuation is part of it.
        assert_eq!(
            words("Meet at 10:30, I’m in."),
            ["meet", "at", "10:30", "i'm", "in"]
        );
        assert_eq!(words(" — "), Vec::<String>::new());
        let same = align(
            "For the launch: the page, the emails.",
            "For the launch:\n- The page\n- The emails",
        );
        assert_eq!(same.tally().errors(), 0);
    }

    #[test]
    fn counts_each_kind_of_error() {
        let a = align(
            "please ship the quill release today",
            "please um uh ship the quil release",
        );
        assert_eq!(
            a.tally(),
            Tally {
                reference_words: 6,
                substitutions: 1,
                deletions: 1,
                insertions: 2,
            }
        );
        assert_eq!(a.errors(), ["+um", "+uh", "quil→quill", "-today"]);
        assert!((a.tally().wer() - 4.0 / 6.0).abs() < 1e-9);
    }

    #[test]
    fn empty_texts() {
        assert_eq!(align("", "").tally().wer(), 0.0);
        assert_eq!(align("", "hello").tally().wer(), 1.0);
        assert_eq!(align("hello there", "").tally().wer(), 1.0);
    }

    #[test]
    fn tallies_add_up() {
        let mut total = align("a b c", "a b").tally();
        total += align("d e", "d x").tally();
        assert_eq!(total.reference_words, 5);
        assert_eq!(total.errors(), 2);
    }
}
