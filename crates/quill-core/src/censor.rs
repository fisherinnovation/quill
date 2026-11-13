//! Hiding swear words and slurs in dictated text, when the user turns it on
//! (Settings → Dictation → Words): each is typed as its first letter and a
//! star for every other letter ("fuck" → "f***", [`mask`]), or left out
//! altogether ([`remove`]).
//!
//! Whole words from a fixed English list, never parts of words: "class",
//! "Scunthorpe" and "shiitake" stay as they are. The list spells out each
//! form ("fucking", "bullshit") rather than matching stems, since a stem
//! like "ass" or "hell" starts too many ordinary words. Mild words (damn,
//! hell, crap) and ones that are also names or everyday words (dick, cock,
//! prick) aren't on it.

/// Lowercase, one word each (no spaces or apostrophes: "fuckin'" is
/// "fuckin" followed by an apostrophe).
const WORDS: &[&str] = &[
    // fuck
    "fuck",
    "fucks",
    "fucked",
    "fucker",
    "fuckers",
    "fucking",
    "fuckin",
    "fuckery",
    "fuckface",
    "fuckhead",
    "fuckheads",
    "fuckup",
    "fuckups",
    "fuckwit",
    "fuckwits",
    "clusterfuck",
    "motherfucker",
    "motherfuckers",
    "motherfucking",
    "motherfuckin",
    // shit
    "shit",
    "shits",
    "shitty",
    "shitting",
    "shitted",
    "shithead",
    "shitheads",
    "shithole",
    "shitholes",
    "shitshow",
    "apeshit",
    "batshit",
    "bullshit",
    "bullshitting",
    "bullshitter",
    "chickenshit",
    "dipshit",
    "horseshit",
    // others
    "arse",
    "arsehole",
    "arseholes",
    "ass",
    "asshole",
    "assholes",
    "bastard",
    "bastards",
    "bitch",
    "bitches",
    "bitched",
    "bitching",
    "bitchy",
    "bollocks",
    "cunt",
    "cunts",
    "dickhead",
    "dickheads",
    "dumbass",
    "jackass",
    "jackasses",
    "piss",
    "pissed",
    "pissing",
    "tosser",
    "tossers",
    "twat",
    "twats",
    "wank",
    "wanker",
    "wankers",
    // slurs
    "chink",
    "chinks",
    "dyke",
    "dykes",
    "fag",
    "fags",
    "faggot",
    "faggots",
    "gook",
    "gooks",
    "kike",
    "kikes",
    "nigga",
    "niggas",
    "nigger",
    "niggers",
    "raghead",
    "ragheads",
    "retard",
    "retarded",
    "retards",
    "spic",
    "spics",
    "tranny",
    "trannies",
    "wetback",
    "wetbacks",
];

/// `text` with every word on the list masked. Everything else, spacing and
/// punctuation included, is left as it was.
pub fn mask(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for piece in pieces(text) {
        match piece {
            Piece::Word(word) if listed(word) => {
                let mut chars = word.chars();
                out.extend(chars.next());
                out.extend(chars.map(|_| '*'));
            }
            Piece::Word(word) => out.push_str(word),
            Piece::Other(c) => out.push(c),
        }
    }
    out
}

/// `text` with every word on the list left out, and what's around it
/// tidied: no doubled spaces or space before punctuation, no comma or full
/// stop left at the start of a sentence, and a sentence that began with a
/// removed word still begins with a capital ("Shit, it broke." → "It
/// broke."). What's attached to a removed word goes with it: "'s", a
/// trailing apostrophe ("fuckin'"), a hyphen ("shit-show"). Text left with
/// no words at all becomes empty, so nothing is typed.
pub fn remove(text: &str) -> String {
    let pieces = pieces(text);
    let mut out = String::with_capacity(text.len());
    // A word was just left out: the spacing after it, and whether stray
    // punctuation stays, depend on what comes next.
    let mut gap = false;
    // The next word starts a sentence that a removed capitalized word began.
    let mut capitalize = false;
    let mut i = 0;
    while i < pieces.len() {
        match pieces[i] {
            Piece::Word(word) if listed(word) => {
                if !gap {
                    let joined = out.strip_suffix('-').is_some_and(|before| {
                        before
                            .chars()
                            .next_back()
                            .is_some_and(char::is_alphanumeric)
                    });
                    if joined {
                        out.pop();
                    }
                    out.truncate(out.trim_end_matches([' ', '\t']).len());
                    capitalize = word.starts_with(char::is_uppercase) && sentence_start(&out);
                }
                gap = true;
                i += match (pieces.get(i + 1), pieces.get(i + 2)) {
                    (Some(Piece::Other('\'' | '’')), Some(Piece::Word(s)))
                        if s.eq_ignore_ascii_case("s") =>
                    {
                        3
                    }
                    (Some(Piece::Other('\'' | '’')), next)
                        if !matches!(next, Some(Piece::Word(_))) =>
                    {
                        2
                    }
                    (Some(Piece::Other('-')), Some(Piece::Word(_))) => 2,
                    _ => 1,
                };
                continue;
            }
            // Spaces after a removed word: one goes back before the next word.
            Piece::Other(' ' | '\t') if gap => {}
            // Nothing left before it in its sentence.
            Piece::Other(',' | ';' | ':' | '.' | '!' | '?') if gap && sentence_start(&out) => {}
            piece => {
                if gap {
                    gap = false;
                    let attaches = match piece {
                        Piece::Other(c) => c.is_whitespace() || ",;:.!?)]}\"'”’…".contains(c),
                        Piece::Word(_) => false,
                    };
                    let opened = out.is_empty()
                        || out.ends_with(char::is_whitespace)
                        || out.ends_with(['(', '[', '{', '"', '“', '‘']);
                    if !attaches && !opened {
                        out.push(' ');
                    }
                }
                match piece {
                    Piece::Word(word) if capitalize => {
                        capitalize = false;
                        let mut chars = word.chars();
                        out.extend(chars.next().into_iter().flat_map(char::to_uppercase));
                        out.extend(chars);
                    }
                    Piece::Word(word) => out.push_str(word),
                    Piece::Other(c) => out.push(c),
                }
            }
        }
        i += 1;
    }
    if !out.chars().any(char::is_alphanumeric) {
        return String::new();
    }
    // Whitespace the removal left at either end, where the text had none.
    if !text.starts_with(char::is_whitespace) {
        out = out.trim_start().to_string();
    }
    if !text.ends_with(char::is_whitespace) {
        out.truncate(out.trim_end().len());
    }
    out
}

/// A word (letters and digits) or a single other character.
#[derive(Clone, Copy)]
enum Piece<'a> {
    Word(&'a str),
    Other(char),
}

fn pieces(text: &str) -> Vec<Piece<'_>> {
    let mut pieces = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices() {
        if c.is_alphanumeric() {
            start.get_or_insert(i);
        } else {
            if let Some(s) = start.take() {
                pieces.push(Piece::Word(&text[s..i]));
            }
            pieces.push(Piece::Other(c));
        }
    }
    if let Some(s) = start {
        pieces.push(Piece::Word(&text[s..]));
    }
    pieces
}

fn listed(word: &str) -> bool {
    WORDS.contains(&word.to_lowercase().as_str())
}

/// Whether what's typed so far ends a sentence, or there's nothing yet.
fn sentence_start(out: &str) -> bool {
    let out = out.trim_end_matches([' ', '\t']);
    out.is_empty() || out.ends_with(['.', '!', '?', '…', '\n'])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listed_words_keep_their_first_letter() {
        assert_eq!(mask("What the fuck."), "What the f***.");
        assert_eq!(mask("Shit, that's BULLSHIT!"), "S***, that's B*******!");
        assert_eq!(mask("fuckin' motherfucker"), "f*****' m***********");
        // Possessives and hyphenated words: the listed part only.
        assert_eq!(
            mask("the bitch's car, a shit-show"),
            "the b****'s car, a s***-show"
        );
    }

    #[test]
    fn other_words_and_parts_of_words_stay() {
        for text in [
            "A class in Scunthorpe about shiitake and cocktails.",
            "Hello, assume the assessment passed. Damn, hell, crap.",
            "Dick Cheney pricked his finger.",
            "",
            "  spaces\tand\nnew lines  ",
            "日本語のテキスト。",
        ] {
            assert_eq!(mask(text), text);
            assert_eq!(remove(text), text);
        }
    }

    #[test]
    fn removed_words_leave_tidy_sentences() {
        for (said, typed) in [
            ("What the fuck is this?", "What the is this?"),
            ("Oh shit, the build broke.", "Oh, the build broke."),
            ("You fucking idiot!", "You idiot!"),
            ("That's bullshit.", "That's."),
            // A sentence that started with one still starts with a capital.
            ("Shit, that broke.", "That broke."),
            ("It's done. Fuck, finally.", "It's done. Finally."),
            ("Fucking bullshit, it broke again.", "It broke again."),
            // What's attached goes with it.
            ("the bitch's car", "the car"),
            ("a shit-show and bat-shit crazy", "a show and bat crazy"),
            ("fuckin' great", "great"),
            // Lists and new lines keep their shape.
            (
                "Notes:\n- fuck the deadline\n- ship it",
                "Notes:\n- the deadline\n- ship it",
            ),
            ("Fuck\nNext line", "Next line"),
        ] {
            assert_eq!(remove(said), typed, "{said}");
        }
    }

    #[test]
    fn nothing_is_typed_when_nothing_else_was_said() {
        for said in ["Fuck.", "Shit!", "Fucking bullshit.", "fuck, fuck"] {
            assert_eq!(remove(said), "", "{said}");
        }
    }

    #[test]
    fn the_list_is_lowercase_single_words() {
        for word in WORDS {
            assert!(word.chars().all(|c| c.is_ascii_lowercase()), "{word}");
        }
    }
}
