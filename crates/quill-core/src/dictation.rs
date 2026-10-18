//! Dictation: a few seconds of speech in, text to type at the cursor out.
//!
//! 1. [`transcribe`]: Whisper in short-clip mode, with the vocabulary as its
//!    prompt, then the replacement rules.
//! 2. [`polish`] (optional): an LLM removes fillers, applies self-corrections
//!    and formats lists. Skipped when there's nothing for it to fix
//!    ([`needs_polish`]), since the round trip roughly triples latency.
//!    Its output is checked before use ([`check_polish`]): a model that
//!    answers the dictated question instead of cleaning it up must never get
//!    typed into someone's message.

use quill_llm::prompts::{DictationPrompts, RenderError, render};
use quill_llm::{CompletionRequest, LlmError, LlmProvider, Message};
use quill_stt::{SttError, TranscribeOptions, Transcriber};

use crate::settings::{DictationSettings, Replacement, Style};

#[derive(Debug, thiserror::Error)]
pub enum PolishError {
    #[error(transparent)]
    Llm(#[from] LlmError),
    #[error("prompt template error: {0}")]
    Prompt(#[from] RenderError),
}

/// Shorter utterances are typed as Whisper wrote them.
pub const POLISH_MIN_WORDS: usize = 6;
/// Longer ones are always polished (paragraphs, run-ons).
pub const POLISH_ALWAYS_WORDS: usize = 60;

/// The language to transcribe in, from the setting: a code, `"auto"`
/// (`None`: detect each time), or unset for the system language.
pub fn language(setting: Option<&str>) -> Option<String> {
    match setting.map(str::trim) {
        Some("auto") => None,
        Some(code) if !code.is_empty() => Some(code.to_lowercase()),
        _ => system_language(),
    }
}

/// The OS language if Whisper knows it, e.g. "en" for en-US.
fn system_language() -> Option<String> {
    let locale = sys_locale::get_locale()?;
    let code = locale.split(['-', '_']).next()?.to_lowercase();
    quill_stt::language_supported(&code).then_some(code)
}

/// Transcribes one utterance (16 kHz mono) into text ready to type, or an
/// empty string if there was no speech.
pub fn transcribe(
    transcriber: &dyn Transcriber,
    pcm: &[f32],
    settings: &DictationSettings,
) -> Result<String, SttError> {
    let segments = transcriber.transcribe(pcm, &options(settings, None))?;
    Ok(tidy(&join(&segments), settings))
}

/// Whisper's options for dictation: short-clip mode, the language, and the
/// vocabulary as its prompt, followed by `context` (what came just before,
/// when transcribing in pieces as [`crate::live`] does).
pub(crate) fn options(settings: &DictationSettings, context: Option<&str>) -> TranscribeOptions {
    let prompt = match (vocabulary_prompt(&settings.vocabulary), context) {
        (Some(vocabulary), Some(context)) => Some(format!("{vocabulary} {context}")),
        (vocabulary, context) => vocabulary.or_else(|| context.map(str::to_string)),
    };
    TranscribeOptions {
        language: language(settings.language.as_deref()),
        initial_prompt: prompt,
        short_clip: true,
        no_fallback: false,
    }
}

/// Segments' text, trimmed and joined with spaces.
pub(crate) fn join(segments: &[quill_stt::Segment]) -> String {
    segments
        .iter()
        .map(|s| s.text.trim())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whisper's text as it should be typed: loops collapsed, replacements
/// applied.
pub fn tidy(text: &str, settings: &DictationSettings) -> String {
    apply_replacements(&collapse_repeats(text), &settings.replacements)
}

/// Whisper's prompt primes its spelling: listing the terms makes it use them.
fn vocabulary_prompt(vocabulary: &[String]) -> Option<String> {
    let terms: Vec<&str> = vocabulary
        .iter()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .collect();
    (!terms.is_empty()).then(|| format!("{}.", terms.join(", ")))
}

/// Whether the polish step is worth its ~0.7 s. Whisper's text is already
/// punctuated and usually drops "um"s itself, so on the eval clips the
/// polish model returned it unchanged 7 times out of 9. Send it only when
/// there's something to fix: fillers, a self-correction, a spoken list,
/// a stutter, or enough text to need paragraphs.
pub fn needs_polish(text: &str) -> bool {
    let words: Vec<String> = text
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'')
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect();
    if words.len() < POLISH_MIN_WORDS {
        return false;
    }
    if words.len() >= POLISH_ALWAYS_WORDS {
        return true;
    }
    const FILLERS: &[&str] = &["um", "uh", "uhm", "er", "erm", "hmm", "mm", "ah"];
    const PHRASES: &[&str] = &[
        "you know",
        "i mean",
        "no wait",
        "wait no",
        "scratch that",
        "or rather",
        "actually no",
        "no sorry",
        "sorry i mean",
        "let me rephrase",
        "new line",
        "new paragraph",
        "bullet point",
        "number one",
        "first of all",
    ];
    const ORDINALS: &[&str] = &["first", "second", "third", "firstly", "secondly", "lastly"];
    const COUNTED: &[&str] = &["things", "items", "points", "steps", "options", "reasons"];
    let joined = format!(" {} ", words.join(" "));
    let has = |w: &str| words.iter().any(|x| x == w);
    words.iter().any(|w| FILLERS.contains(&w.as_str()))
        || PHRASES.iter().any(|p| joined.contains(&format!(" {p} ")))
        // A list: two ordinals, or "three things".
        || ORDINALS.iter().filter(|o| has(o)).count() >= 2
        || words.windows(2).any(|pair| {
            COUNTED.contains(&pair[1].as_str())
                && matches!(pair[0].as_str(), "two" | "three" | "four" | "five" | "2" | "3" | "4" | "5")
        })
        // A stutter: the same word twice in a row.
        || words.windows(2).any(|pair| pair[0] == pair[1] && pair[0].len() > 1)
}

/// Result of the polish step.
#[derive(Debug, Clone, PartialEq)]
pub struct Polish {
    /// What to type: the model's version, or the input if it was rejected.
    pub text: String,
    /// Why the model's version wasn't used, if it wasn't.
    pub rejected: Option<&'static str>,
    pub model: String,
    pub latency_ms: u64,
    pub cost_usd: Option<f64>,
}

/// Where dictated text is going: the app's display name and its style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Destination<'a> {
    pub app: &'a str,
    pub style: Style,
}

/// The style for an app: the user's choice if they made one, else the
/// built-in default for well-known apps, else [`Style::Default`].
pub fn style_for(app_id: &str, settings: &DictationSettings) -> Style {
    let id = app_id.to_lowercase();
    if let Some(chosen) = settings
        .app_styles
        .iter()
        .find(|s| s.app_id.to_lowercase() == id)
    {
        return chosen.style;
    }
    built_in_style(&id)
}

/// Defaults for well-known apps, by bundle id (macOS) or executable
/// (Windows), lowercased.
pub fn built_in_style(app_id: &str) -> Style {
    const CASUAL: &[&str] = &[
        "com.tinyspeck.slackmacgap",
        "slack.exe",
        "com.apple.mobilesms",
        "com.hnc.discord",
        "discord.exe",
        "net.whatsapp.whatsapp",
        "whatsapp.exe",
        "ru.keepcoder.telegram",
        "telegram.exe",
        "com.microsoft.teams2",
        "ms-teams.exe",
        "org.whispersystems.signal-desktop",
        "signal.exe",
    ];
    const FORMAL: &[&str] = &[
        "com.apple.mail",
        "com.microsoft.outlook",
        "outlook.exe",
        "olk.exe",
        "com.readdle.smartemail-mac",
        "com.superhuman.electron",
        "com.microsoft.word",
        "winword.exe",
        "com.apple.iwork.pages",
    ];
    const TECHNICAL: &[&str] = &[
        "com.microsoft.vscode",
        "code.exe",
        "com.todesktop.230313mzl4w4u92",
        "cursor.exe",
        "dev.zed.zed",
        "com.apple.dt.xcode",
        "com.apple.terminal",
        "com.googlecode.iterm2",
        "dev.warp.warp-stable",
        "com.mitchellh.ghostty",
        "windowsterminal.exe",
    ];
    if CASUAL.contains(&app_id) {
        Style::Casual
    } else if FORMAL.contains(&app_id) {
        Style::Formal
    } else if TECHNICAL.contains(&app_id) || app_id.starts_with("com.jetbrains.") {
        Style::Technical
    } else {
        Style::Default
    }
}

/// Style rules that don't need a model: in casual apps, a one-sentence
/// message loses its final period, the way people type in chat.
pub fn apply_style(text: &str, style: Style) -> String {
    if style == Style::Casual
        && !text.contains('\n')
        && text.ends_with('.')
        && !text.ends_with("..")
        && split_sentences(text).len() == 1
    {
        return text[..text.len() - 1].to_string();
    }
    text.to_string()
}

/// The prompt's sentence about where the text is going.
fn destination_text(destination: Option<Destination<'_>>) -> String {
    let Some(Destination { app, style }) = destination else {
        return String::new();
    };
    match style {
        Style::Default => format!("The text is going into {app}."),
        Style::Casual => format!(
            "The text is going into {app}, a chat app: keep it relaxed and conversational. \
             Don't add greetings or sign-offs. Leave off the final period of a one-line message, \
             but keep question marks and exclamation marks."
        ),
        Style::Formal => format!(
            "The text is going into {app}, for email or documents: write complete, properly \
             punctuated sentences, keeping the speaker's own words."
        ),
        Style::Technical => format!(
            "The text is going into {app}, a code editor or terminal: keep code identifiers, \
             file names, commands and flags exactly as spoken, in their usual written form \
             (camelCase, snake_case, file.ext, --flag). Don't add Markdown."
        ),
    }
}

/// Cleans up `text` with an LLM. Call only when [`needs_polish`] says so.
pub async fn polish(
    provider: &dyn LlmProvider,
    model: &str,
    prompts: &DictationPrompts,
    text: &str,
    settings: &DictationSettings,
    destination: Option<Destination<'_>>,
) -> Result<Polish, PolishError> {
    let vocabulary = settings
        .vocabulary
        .iter()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    let vocabulary = if vocabulary.is_empty() {
        "(none)".to_string()
    } else {
        vocabulary
    };
    let about = about_text(&settings.about);
    let destination = destination_text(destination);
    let mut vars = vec![
        ("vocabulary", vocabulary.as_str()),
        ("destination", destination.as_str()),
    ];
    // Older prompt versions (kept for comparisons) have no {{about}}.
    if prompts.system.contains("{{about}}") {
        vars.push(("about", about.as_str()));
    }
    let system = render(prompts.system, &vars)?;
    let user = render(prompts.user, &[("transcript", text)])?;
    let words = u32::try_from(text.split_whitespace().count()).unwrap_or(u32::MAX);
    let response = provider
        .complete(CompletionRequest {
            model: model.to_string(),
            messages: vec![Message::system(system), Message::user(user)],
            // Room for the text plus list formatting, but not for an essay.
            max_tokens: Some(words.saturating_mul(3).saturating_add(64)),
            temperature: Some(0.0),
            no_reasoning: true,
        })
        .await?;
    let (text, rejected) = match check_polish(text, &response.text) {
        Ok(polished) => (polished, None),
        Err(reason) => {
            tracing::warn!(reason, "polish output rejected; typing the transcript");
            (text.to_string(), Some(reason))
        }
    };
    Ok(Polish {
        text: apply_replacements(&text, &settings.replacements),
        rejected,
        model: response.model,
        latency_ms: response.latency_ms,
        cost_usd: response.cost_usd,
    })
}

/// The prompt's "about the speaker" block, or nothing if the user wrote
/// nothing.
fn about_text(about: &str) -> String {
    let about = about.trim();
    if about.is_empty() {
        return String::new();
    }
    format!(
        "\nAbout the speaker, in their own words. Use it to get names, jargon and spelling right; never add any of it to the text:\n<about>\n{about}\n</about>\n"
    )
}

/// Validates the model's cleaned-up version of `raw`, returning it tidied,
/// or why it can't be used.
pub fn check_polish(raw: &str, output: &str) -> Result<String, &'static str> {
    let mut text = crate::summary::clean(output);
    // Models sometimes echo the wrapper or fence the answer.
    for (open, close) in [("<transcript>", "</transcript>"), ("```", "```")] {
        if let Some(inner) = text.strip_prefix(open).and_then(|t| t.strip_suffix(close)) {
            text = inner.trim().to_string();
        }
    }
    if text.len() >= 2 && text.starts_with('"') && text.ends_with('"') && !raw.starts_with('"') {
        text = text[1..text.len() - 1].trim().to_string();
    }

    // Trailing spaces (a Markdown line-break habit) would be typed too.
    let text = text
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");
    if text.is_empty() {
        return Err("empty");
    }
    let lower = text.to_lowercase();
    let raw_lower = raw.to_lowercase();
    const PREAMBLES: &[&str] = &[
        "here is",
        "here's",
        "sure",
        "certainly",
        "cleaned",
        "i can't",
        "i cannot",
        "i'm sorry",
    ];
    if PREAMBLES
        .iter()
        .any(|p| lower.starts_with(p) && !raw_lower.starts_with(p))
    {
        return Err("added a preamble");
    }
    // Cleaning only removes words (fillers, false starts) or reformats.
    // Much longer means it answered or embellished; much shorter means it
    // summarized.
    let (before, after) = (word_count(raw), word_count(&text));
    if after > before + before / 4 + 6 {
        return Err("longer than what was said");
    }
    if before >= 10 && after * 5 < before * 2 {
        return Err("dropped too much");
    }
    Ok(text)
}

pub(crate) fn word_count(text: &str) -> usize {
    // List markers aren't words.
    text.split_whitespace()
        .filter(|w| !matches!(*w, "-" | "*") && !is_list_number(w))
        .count()
}

fn is_list_number(word: &str) -> bool {
    word.strip_suffix('.')
        .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// Whether `text` repeats a run of [`LOOP_WORDS`] words: a Whisper loop
/// that [`collapse_repeats`] lets through (it repeats a passage, not one
/// sentence, or only twice). People don't repeat eight words verbatim.
pub fn has_loop(text: &str) -> bool {
    let words: Vec<String> = text
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .collect();
    let mut seen = std::collections::HashSet::new();
    words
        .windows(LOOP_WORDS)
        .any(|run| !seen.insert(run.to_vec()))
}

/// See [`has_loop`].
const LOOP_WORDS: usize = 8;

/// Collapses a sentence repeated three or more times in a row, a Whisper
/// failure mode ("Ask not. Ask not. Ask not.") when audio stops abruptly.
pub fn collapse_repeats(text: &str) -> String {
    let sentences = split_sentences(text);
    let key = |s: &str| s.trim().trim_end_matches(['.', '!', '?']).to_lowercase();
    let mut out: Vec<&str> = Vec::with_capacity(sentences.len());
    let mut i = 0;
    while i < sentences.len() {
        let mut run = 1;
        while i + run < sentences.len() && key(sentences[i + run]) == key(sentences[i]) {
            run += 1;
        }
        // Twice can be real ("No. No."); three times is the loop.
        let keep = if run >= 3 { 1 } else { run };
        out.extend(&sentences[i..i + keep]);
        i += run;
    }
    out.iter().map(|s| s.trim()).collect::<Vec<_>>().join(" ")
}

/// Splits after `.`, `!` or `?` followed by whitespace, keeping the
/// punctuation with its sentence.
fn split_sentences(text: &str) -> Vec<&str> {
    let mut sentences = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if matches!(c, '.' | '!' | '?')
            && chars.peek().is_some_and(|(_, next)| next.is_whitespace())
        {
            sentences.push(&text[start..=i]);
            start = i + 1;
        }
    }
    if start < text.len() {
        sentences.push(&text[start..]);
    }
    sentences.retain(|s| !s.trim().is_empty());
    sentences
}

/// Replaces each rule's `from` with its `to`: whole words or phrases only,
/// ignoring case.
pub fn apply_replacements(text: &str, rules: &[Replacement]) -> String {
    let mut text = text.to_string();
    for rule in rules {
        let from: Vec<char> = rule.from.trim().chars().collect();
        if from.is_empty() {
            continue;
        }
        let chars: Vec<char> = text.chars().collect();
        let mut out = String::with_capacity(text.len());
        let mut i = 0;
        while i < chars.len() {
            let candidate = chars.get(i..i + from.len());
            let boundary_before = i == 0 || !chars[i - 1].is_alphanumeric();
            let boundary_after = chars
                .get(i + from.len())
                .is_none_or(|c| !c.is_alphanumeric());
            if boundary_before
                && boundary_after
                && candidate.is_some_and(|c| {
                    c.iter()
                        .zip(&from)
                        .all(|(a, b)| a.to_lowercase().eq(b.to_lowercase()))
                })
            {
                out.push_str(&rule.to);
                i += from.len();
            } else {
                out.push(chars[i]);
                i += 1;
            }
        }
        text = out;
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(from: &str, to: &str) -> Replacement {
        Replacement {
            from: from.into(),
            to: to.into(),
        }
    }

    #[test]
    fn replacements_match_whole_words_ignoring_case() {
        let rules = [rule("open router", "OpenRouter"), rule("quill", "Quill")];
        assert_eq!(
            apply_replacements("Ask Open Router about quill, not quilled.", &rules),
            "Ask OpenRouter about Quill, not quilled."
        );
        assert_eq!(
            apply_replacements("ünïcode ok", &[rule("ünïcode", "Unicode")]),
            "Unicode ok"
        );
        assert_eq!(apply_replacements("text", &[rule("  ", "x")]), "text");
    }

    #[test]
    fn loops_are_long_repeated_runs() {
        let passage = "we agreed to ship the redesign on friday behind the flag";
        assert!(has_loop(&format!("{passage}. {passage}.")));
        assert!(!has_loop(passage));
        // A short phrase said twice is normal speech.
        assert!(!has_loop("I think so. Yes, I think so, yes."));
    }

    #[test]
    fn whisper_loops_collapse_but_real_repeats_stay() {
        assert_eq!(
            collapse_repeats("Ask not. Ask not. Ask not. Ask not."),
            "Ask not."
        );
        assert_eq!(
            collapse_repeats("Hi there. No. No. Fine!"),
            "Hi there. No. No. Fine!"
        );
        assert_eq!(
            collapse_repeats("Version 2.5 is out."),
            "Version 2.5 is out."
        );
        assert_eq!(collapse_repeats(""), "");
    }

    #[test]
    fn language_setting_resolves_codes_auto_and_system() {
        assert_eq!(language(Some("DE")), Some("de".into()));
        assert_eq!(language(Some("auto")), None);
        // Unset follows the OS; whatever it is, it's a code Whisper knows.
        if let Some(code) = language(None) {
            assert!(quill_stt::language_supported(&code), "{code}");
        }
    }

    #[test]
    fn polish_runs_only_when_there_is_something_to_fix() {
        // Whisper output from eval/dictation clips.
        assert!(!needs_polish("Sounds good. See you then."));
        assert!(!needs_polish(
            "What time does the design review start tomorrow and who is presenting?"
        ));
        assert!(!needs_polish(
            "The invoice total is $1,250, due on October 21, with a 15% discount."
        ));
        assert!(needs_polish(
            "Let's meet at 3, no wait, 4 p.m. on Thursday in the big conference room."
        ));
        assert!(needs_polish(
            "For the launch we need three things. First, the landing page. Second, the pricing table."
        ));
        assert!(needs_polish(
            "So um I was thinking we could move the standup."
        ));
        assert!(needs_polish("I think the the build is broken again today."));
        assert!(needs_polish(&"word ".repeat(POLISH_ALWAYS_WORDS)));
    }

    #[test]
    fn app_styles_come_from_the_user_then_built_ins() {
        let mut settings = DictationSettings::default();
        assert_eq!(
            style_for("com.tinyspeck.slackmacgap", &settings),
            Style::Casual
        );
        assert_eq!(style_for("OUTLOOK.EXE", &settings), Style::Formal);
        assert_eq!(
            style_for("com.jetbrains.intellij", &settings),
            Style::Technical
        );
        assert_eq!(style_for("com.example.unknown", &settings), Style::Default);
        settings.app_styles.push(crate::settings::AppStyle {
            app_id: "com.tinyspeck.slackmacgap".into(),
            name: "Slack".into(),
            style: Style::Formal,
        });
        assert_eq!(
            style_for("com.tinyspeck.slackmacgap", &settings),
            Style::Formal
        );
    }

    #[test]
    fn casual_drops_the_period_of_a_one_line_message_only() {
        assert_eq!(
            apply_style("Sounds good, see you then.", Style::Casual),
            "Sounds good, see you then"
        );
        assert_eq!(
            apply_style("Done. Shipping now.", Style::Casual),
            "Done. Shipping now."
        );
        assert_eq!(apply_style("Wait...", Style::Casual), "Wait...");
        assert_eq!(apply_style("Really?", Style::Casual), "Really?");
        assert_eq!(apply_style("List:\n- a.", Style::Casual), "List:\n- a.");
        assert_eq!(apply_style("Sounds good.", Style::Formal), "Sounds good.");
    }

    #[test]
    fn the_prompt_says_where_the_text_is_going() {
        assert_eq!(destination_text(None), "");
        let slack = destination_text(Some(Destination {
            app: "Slack",
            style: Style::Casual,
        }));
        assert!(slack.starts_with("The text is going into Slack, a chat app"));
    }

    #[test]
    fn vocabulary_becomes_a_whisper_prompt() {
        assert_eq!(vocabulary_prompt(&[]), None);
        assert_eq!(
            vocabulary_prompt(&["Quill".into(), " ".into(), "Tauri".into()]),
            Some("Quill, Tauri.".into())
        );
    }

    #[test]
    fn polish_output_that_cleans_up_is_accepted() {
        let raw = "So um I was thinking we could uh move the standup to ten thirty.";
        assert_eq!(
            check_polish(raw, "I was thinking we could move the standup to 10:30.").unwrap(),
            "I was thinking we could move the standup to 10:30."
        );
        // Wrappers and an empty think block are stripped.
        assert_eq!(
            check_polish(raw, "<think>\n</think>\n<transcript>\nWe could move the standup to 10:30.\n</transcript>")
                .unwrap(),
            "We could move the standup to 10:30."
        );
        // A list adds markers but not words.
        let list = "We need three things. First the landing page, second the pricing table, and third the onboarding emails.";
        let formatted = "We need three things:\n1. The landing page\n2. The pricing table\n3. The onboarding emails";
        assert_eq!(check_polish(list, formatted).unwrap(), formatted);
    }

    #[test]
    fn polish_output_that_answers_or_summarizes_is_rejected() {
        let question = "What time does the design review start tomorrow, and who is presenting?";
        assert_eq!(
            check_polish(
                question,
                "Here's the cleaned text: What time does the design review start?"
            ),
            Err("added a preamble")
        );
        assert_eq!(
            check_polish(
                question,
                "I don't have access to your calendar, so I can't tell you when the design review starts or who is presenting it tomorrow. You could check the invite."
            ),
            Err("longer than what was said")
        );
        let email = "Hi Sarah, thanks for sending the contract over. I've reviewed it and I have two small comments on section four. Could we jump on a call this week?";
        assert_eq!(
            check_polish(email, "Contract: two comments."),
            Err("dropped too much")
        );
        assert_eq!(check_polish(email, "  "), Err("empty"));
    }
}
