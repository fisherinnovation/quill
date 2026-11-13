//! Masking swear words and slurs in dictated text, when the user turns it
//! on (Settings → Dictation → Words): each is typed as its first letter and
//! a star for every other letter ("fuck" → "f***").
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
    let mut word = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() {
            word.push(c);
        } else {
            push_word(&mut out, &word);
            word.clear();
            out.push(c);
        }
    }
    push_word(&mut out, &word);
    out
}

/// Appends `word`, masked if it's on the list.
fn push_word(out: &mut String, word: &str) {
    if !WORDS.contains(&word.to_lowercase().as_str()) {
        out.push_str(word);
        return;
    }
    let mut chars = word.chars();
    out.extend(chars.next());
    out.extend(chars.map(|_| '*'));
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
        }
    }

    #[test]
    fn the_list_is_lowercase_single_words() {
        for word in WORDS {
            assert!(word.chars().all(|c| c.is_ascii_lowercase()), "{word}");
        }
    }
}
