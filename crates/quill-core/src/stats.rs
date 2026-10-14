//! Stats for one meeting or one dictation, worked out from what's already
//! stored: the transcript's timings, speakers and words, or a dictation's
//! audio length and texts. Nothing is saved; they're computed when asked for.
//!
//! Word lists (common words, fillers) are English, like the summary
//! prompts. Other languages still get the timings and counts, but their
//! "most used words" can include everyday words the list doesn't know.

use std::collections::HashMap;

use quill_stt::Segment;
use serde::Serialize;

use crate::insights::TYPING_WPM;
use crate::library::DictationEntry;
use crate::meeting::{ME, OTHERS, TURN_MAX_GAP_MS};

const TOP_WORDS: usize = 12;
const TOP_PHRASES: usize = 6;
const SIGNATURE_WORDS: usize = 3;
/// A quiet stretch shorter than this isn't worth pointing out.
const MIN_SILENCE_MS: u64 = 10_000;
/// Below this much talking, words per minute is noise.
const MIN_PACE_MS: u64 = 30_000;
/// Fewer words than this and nobody has "signature" words.
const MIN_SIGNATURE_SPEAKER_WORDS: usize = 40;
/// Words in a stretch's preview.
const PREVIEW_WORDS: usize = 14;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MeetingStats {
    /// From the start of the recording to the last word.
    pub span_ms: u64,
    /// Time when at least one person was talking.
    pub speech_ms: u64,
    pub words: usize,
    /// Words per minute of talking, everyone together.
    pub wpm: Option<f64>,
    /// Stretches of talking; each change of speaker starts a new one.
    pub turns: usize,
    pub questions: usize,
    /// "Me" first, then everyone else in order of first appearance (the
    /// order the transcript gives them colours in).
    pub speakers: Vec<SpeakerStats>,
    /// Each stretch of someone talking, in time order.
    pub stretches: Vec<Stretch>,
    pub longest_stretch: Option<Stretch>,
    /// The longest time nobody spoke, if at least [`MIN_SILENCE_MS`].
    pub longest_silence: Option<Silence>,
    /// The two people who handed the floor to each other most often.
    pub busiest_pair: Option<Pair>,
    /// Most used words, leaving out common ones ("the", "really").
    pub top_words: Vec<WordCount>,
    /// Two-word phrases said more than once ("pricing page").
    pub top_phrases: Vec<WordCount>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SpeakerStats {
    pub name: String,
    pub talk_ms: u64,
    /// Of everyone's talking time, 0–1.
    pub share: f64,
    pub words: usize,
    pub wpm: Option<f64>,
    pub turns: usize,
    pub questions: usize,
    pub longest_stretch_ms: u64,
    /// "um", "you know", "basically" and the like.
    pub fillers: usize,
    /// Words this person said far more than anyone else.
    pub signature_words: Vec<String>,
}

/// One person talking, without a pause of more than [`TURN_MAX_GAP_MS`] or
/// anyone else cutting in.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Stretch {
    pub speaker: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub words: usize,
    /// The first few words.
    pub preview: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Silence {
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Pair {
    pub a: String,
    pub b: String,
    pub handovers: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WordCount {
    pub text: String,
    pub count: usize,
}

pub fn meeting(segments: &[Segment]) -> MeetingStats {
    let mut segs: Vec<&Segment> = segments
        .iter()
        .filter(|s| !s.text.trim().is_empty())
        .collect();
    segs.sort_by_key(|s| s.start_ms);
    let speaker = |s: &Segment| s.speaker.clone().unwrap_or_else(|| OTHERS.to_string());

    // Speakers: "Me" first, then by first appearance.
    let mut names: Vec<String> = Vec::new();
    for s in &segs {
        let name = speaker(s);
        if !names.contains(&name) {
            names.push(name);
        }
    }
    if let Some(i) = names.iter().position(|n| n == ME) {
        let me = names.remove(i);
        names.insert(0, me);
    }

    let stretches = stretches(&segs, speaker);
    let span_ms = segs.iter().map(|s| s.end_ms).max().unwrap_or(0);
    let all: Vec<(u64, u64)> = segs.iter().map(|s| (s.start_ms, s.end_ms)).collect();
    let speech = merged(all);
    let speech_ms = speech.iter().map(|(a, b)| b - a).sum();

    // Words, overall and per speaker.
    let mut overall: HashMap<String, usize> = HashMap::new();
    let mut by_speaker: HashMap<String, HashMap<String, usize>> = HashMap::new();
    let mut phrases: HashMap<String, usize> = HashMap::new();
    for s in &segs {
        let tokens = tokens(&s.text);
        let counts = by_speaker.entry(speaker(s)).or_default();
        for t in tokens.iter().filter(|t| is_content(t)) {
            *overall.entry(t.clone()).or_default() += 1;
            *counts.entry(t.clone()).or_default() += 1;
        }
        for pair in tokens.windows(2) {
            if is_content(&pair[0]) && is_content(&pair[1]) && pair[0] != pair[1] {
                *phrases
                    .entry(format!("{} {}", pair[0], pair[1]))
                    .or_default() += 1;
            }
        }
    }

    let talking = names
        .iter()
        .filter(|n| by_speaker.get(*n).is_some_and(|c| !c.is_empty()))
        .count();
    let mut speakers: Vec<SpeakerStats> = names
        .iter()
        .map(|name| {
            let mine: Vec<&&Segment> = segs.iter().filter(|s| speaker(s) == *name).collect();
            let talk_ms: u64 = merged(mine.iter().map(|s| (s.start_ms, s.end_ms)).collect())
                .iter()
                .map(|(a, b)| b - a)
                .sum();
            let words = mine.iter().map(|s| s.text.split_whitespace().count()).sum();
            let own = stretches.iter().filter(|st| st.speaker == *name);
            SpeakerStats {
                name: name.clone(),
                talk_ms,
                share: 0.0,
                words,
                wpm: pace(words, talk_ms),
                turns: own.clone().count(),
                questions: mine.iter().map(|s| questions(&s.text)).sum(),
                longest_stretch_ms: own.map(|st| st.end_ms - st.start_ms).max().unwrap_or(0),
                fillers: mine.iter().map(|s| fillers(&tokens(&s.text))).sum(),
                signature_words: if talking >= 2 && words >= MIN_SIGNATURE_SPEAKER_WORDS {
                    signature(&by_speaker[name], &overall)
                } else {
                    Vec::new()
                },
            }
        })
        .collect();
    let total_talk: u64 = speakers.iter().map(|s| s.talk_ms).sum();
    for s in &mut speakers {
        s.share = if total_talk > 0 {
            s.talk_ms as f64 / total_talk as f64
        } else {
            0.0
        };
    }

    let words = speakers.iter().map(|s| s.words).sum();
    MeetingStats {
        span_ms,
        speech_ms,
        words,
        wpm: pace(words, total_talk),
        turns: stretches.len(),
        questions: speakers.iter().map(|s| s.questions).sum(),
        longest_stretch: stretches
            .iter()
            .max_by_key(|s| s.end_ms - s.start_ms)
            .cloned(),
        longest_silence: speech
            .windows(2)
            .map(|w| Silence {
                start_ms: w[0].1,
                end_ms: w[1].0,
            })
            .filter(|s| s.end_ms - s.start_ms >= MIN_SILENCE_MS)
            .max_by_key(|s| s.end_ms - s.start_ms),
        busiest_pair: busiest_pair(&stretches),
        top_words: top(overall, TOP_WORDS),
        top_phrases: top(phrases, TOP_PHRASES),
        speakers,
        stretches,
    }
}

/// Consecutive segments by the same person, a pause of at most
/// [`TURN_MAX_GAP_MS`] apart, joined into stretches.
fn stretches(segs: &[&Segment], speaker: impl Fn(&Segment) -> String) -> Vec<Stretch> {
    let mut out: Vec<Stretch> = Vec::new();
    for s in segs {
        let name = speaker(s);
        let words = s.text.split_whitespace().count();
        if let Some(last) = out.last_mut()
            && last.speaker == name
            && s.start_ms <= last.end_ms + TURN_MAX_GAP_MS
        {
            last.end_ms = last.end_ms.max(s.end_ms);
            last.words += words;
            let have = last.preview.split_whitespace().count();
            if have < PREVIEW_WORDS {
                let more: Vec<&str> = s
                    .text
                    .split_whitespace()
                    .take(PREVIEW_WORDS - have)
                    .collect();
                last.preview = format!("{} {}", last.preview, more.join(" "))
                    .trim()
                    .to_string();
            }
            continue;
        }
        out.push(Stretch {
            speaker: name,
            start_ms: s.start_ms,
            end_ms: s.end_ms,
            words,
            preview: s
                .text
                .split_whitespace()
                .take(PREVIEW_WORDS)
                .collect::<Vec<_>>()
                .join(" "),
        });
    }
    out
}

/// Intervals merged where they overlap or touch, in order.
fn merged(mut intervals: Vec<(u64, u64)>) -> Vec<(u64, u64)> {
    intervals.sort_unstable();
    let mut out: Vec<(u64, u64)> = Vec::new();
    for (start, end) in intervals {
        match out.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => out.push((start, end.max(start))),
        }
    }
    out
}

fn pace(words: usize, ms: u64) -> Option<f64> {
    (ms >= MIN_PACE_MS).then(|| words as f64 / (ms as f64 / 60_000.0))
}

/// Question marks, counting "??" once.
fn questions(text: &str) -> usize {
    let mut count = 0;
    let mut previous = ' ';
    for c in text.chars() {
        if c == '?' && previous != '?' {
            count += 1;
        }
        previous = c;
    }
    count
}

fn busiest_pair(stretches: &[Stretch]) -> Option<Pair> {
    let mut counts: HashMap<(&str, &str), usize> = HashMap::new();
    for w in stretches.windows(2) {
        let (a, b) = (w[0].speaker.as_str(), w[1].speaker.as_str());
        if a != b {
            *counts
                .entry(if a < b { (a, b) } else { (b, a) })
                .or_default() += 1;
        }
    }
    counts
        .into_iter()
        .filter(|(_, n)| *n >= 2)
        // Ties go to the alphabetically first pair, so the answer is stable.
        .max_by(|x, y| x.1.cmp(&y.1).then_with(|| y.0.cmp(&x.0)))
        .map(|((a, b), handovers)| {
            // "Me" reads better first.
            let (a, b) = if b == ME { (b, a) } else { (a, b) };
            Pair {
                a: a.into(),
                b: b.into(),
                handovers,
            }
        })
}

/// The most frequent entries seen at least twice, most first, then A–Z.
fn top(counts: HashMap<String, usize>, n: usize) -> Vec<WordCount> {
    let mut list: Vec<WordCount> = counts
        .into_iter()
        .filter(|(_, count)| *count >= 2)
        .map(|(text, count)| WordCount { text, count })
        .collect();
    list.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.text.cmp(&b.text)));
    list.truncate(n);
    list
}

/// Words one person said at least twice, and at least 70% of the times
/// anyone said them.
fn signature(mine: &HashMap<String, usize>, overall: &HashMap<String, usize>) -> Vec<String> {
    let mut words: Vec<(&String, usize)> = mine
        .iter()
        .filter(|(w, n)| **n >= 2 && **n * 10 >= overall.get(*w).copied().unwrap_or(0) * 7)
        .map(|(w, n)| (w, *n))
        .collect();
    words.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    words
        .into_iter()
        .take(SIGNATURE_WORDS)
        .map(|(w, _)| w.clone())
        .collect()
}

/// Lowercased words, punctuation trimmed, possessives dropped ("Priya's" →
/// "priya").
fn tokens(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|w| {
            let w = w.replace('’', "'").to_lowercase();
            let w = w.trim_matches(|c: char| !c.is_alphanumeric());
            w.strip_suffix("'s").unwrap_or(w).to_string()
        })
        .filter(|w| w.chars().any(char::is_alphabetic))
        .collect()
}

fn is_content(word: &str) -> bool {
    word.chars().count() >= 3 && !STOPWORDS.contains(&word) && !FILLERS.contains(&word)
}

fn fillers(tokens: &[String]) -> usize {
    let single = tokens
        .iter()
        .filter(|t| FILLERS.contains(&t.as_str()))
        .count();
    let pairs = tokens
        .windows(2)
        .filter(|w| FILLER_PAIRS.contains(&(w[0].as_str(), w[1].as_str())))
        .count();
    single + pairs
}

/// Fillers on their own. "like" and "so" are left out: too often real words.
const FILLERS: &[&str] = &[
    "um",
    "umm",
    "uh",
    "uhh",
    "erm",
    "er",
    "hmm",
    "mm",
    "basically",
    "literally",
    "actually",
];
const FILLER_PAIRS: &[(&str, &str)] = &[
    ("you", "know"),
    ("i", "mean"),
    ("kind", "of"),
    ("sort", "of"),
];

/// Everyday English words that say nothing about what a meeting was about.
#[rustfmt::skip]
const STOPWORDS: &[&str] = &[
    "about", "above", "after", "again", "against", "all", "also", "always", "and", "another", "any",
    "anyone", "anything", "are", "aren't", "around", "because", "been", "before", "being", "below",
    "best", "better", "between", "big", "bit", "both", "but", "can", "can't", "cannot", "could",
    "couldn't", "did", "didn't", "does", "doesn't", "doing", "don't", "done", "down", "during",
    "each", "else", "even", "ever", "every", "few", "for", "from", "further", "get", "gets",
    "getting", "give", "going", "gonna", "good", "got", "gotta", "great", "had", "hadn't", "has",
    "hasn't", "have", "haven't", "having", "he'd", "he'll", "her", "here", "hers", "herself", "him",
    "himself", "his", "how", "i'd", "i'll", "i'm", "i've", "into", "isn't", "it'll", "it's", "its",
    "itself", "just", "keep", "kind", "know", "last", "let", "let's", "like", "little", "look",
    "lot", "lots", "made", "make", "many", "may", "maybe", "mean", "might", "more", "most", "much",
    "must", "need", "never", "new", "next", "nice", "nor", "not", "now", "off", "okay", "once",
    "one", "only", "other", "others", "our", "ours", "ourselves", "out", "over", "own", "pretty",
    "probably", "put", "quite", "really", "right", "said", "same", "say", "see", "she", "she'd",
    "she'll", "should", "shouldn't", "since", "some", "something", "sort", "still", "such", "sure",
    "take", "talk", "tell", "than", "thank", "thanks", "that", "that's", "the", "their", "theirs",
    "them", "themselves", "then", "there", "there's", "these", "they", "they'd", "they'll",
    "they're", "they've", "thing", "things", "think", "this", "those", "though", "through", "too",
    "two", "under", "until", "very", "want", "was", "wasn't", "way", "we'd", "we'll", "we're",
    "we've", "well", "went", "were", "weren't", "what", "what's", "when", "where", "which", "while",
    "who", "whom", "why", "will", "with", "won't", "would", "wouldn't", "yeah", "yep", "yes", "yet",
    "you", "you'd", "you'll", "you're", "you've", "your", "yours", "yourself", "yourselves",
];

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DictationStats {
    pub words: usize,
    /// Words per minute of audio, if there were at least two seconds.
    pub speaking_wpm: Option<f64>,
    /// Typing the same words at [`TYPING_WPM`], minus the time spent speaking.
    pub saved_ms: u64,
    pub typing_wpm: f64,
    /// Words in what Whisper heard, when clean-up changed it.
    pub raw_words: Option<usize>,
    /// Fillers in what Whisper heard that clean-up took out.
    pub fillers_removed: usize,
    /// The user's average speaking speed across all dictations, to compare.
    pub usual_wpm: Option<f64>,
}

/// Stats for one dictation. `usual_wpm` is the user's average (from
/// [`crate::insights`]), passed through for comparison.
pub fn dictation(entry: &DictationEntry, usual_wpm: Option<f64>) -> DictationStats {
    let words = crate::dictation::word_count(&entry.text);
    let speaking_minutes = entry.audio_ms as f64 / 60_000.0;
    let typing_ms = words as f64 / TYPING_WPM * 60_000.0;
    let raw = entry.raw.as_deref();
    DictationStats {
        words,
        speaking_wpm: (entry.audio_ms >= 2_000).then(|| words as f64 / speaking_minutes),
        saved_ms: (typing_ms - entry.audio_ms as f64).max(0.0) as u64,
        typing_wpm: TYPING_WPM,
        raw_words: raw.map(crate::dictation::word_count),
        fillers_removed: raw.map_or(0, |raw| {
            fillers(&tokens(raw)).saturating_sub(fillers(&tokens(&entry.text)))
        }),
        usual_wpm,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start_s: u64, end_s: u64, speaker: &str, text: &str) -> Segment {
        Segment {
            start_ms: start_s * 1000,
            end_ms: end_s * 1000,
            text: text.into(),
            speaker: Some(speaker.into()),
        }
    }

    fn sample() -> Vec<Segment> {
        vec![
            seg(
                0,
                6,
                "Me",
                "Okay, what's the plan for the onboarding launch?",
            ),
            seg(
                7,
                20,
                "Speaker 1",
                "The onboarding launch is Friday. The pricing page needs work, um, basically.",
            ),
            seg(
                20,
                40,
                "Speaker 1",
                "Pricing page copy, pricing page design, and the onboarding emails.",
            ),
            seg(41, 45, "Me", "Sounds good. Who owns the pricing page??"),
            // 25 seconds of silence.
            seg(
                70,
                80,
                "Speaker 2",
                "I can take the pricing page. Analytics first, though.",
            ),
            seg(81, 90, "Speaker 1", "Analytics, you know, is broken."),
            seg(91, 95, "Me", "Then analytics first?"),
        ]
    }

    #[test]
    fn speakers_come_me_first_with_talk_time_and_share() {
        let stats = meeting(&sample());
        let names: Vec<&str> = stats.speakers.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Me", "Speaker 1", "Speaker 2"]);
        let s1 = &stats.speakers[1];
        assert_eq!(
            s1.talk_ms,
            (13 + 20 + 9) * 1000,
            "touching segments aren't double counted"
        );
        assert_eq!(
            s1.turns, 2,
            "one stretch until someone else spoke, then another"
        );
        assert_eq!(s1.longest_stretch_ms, 33_000);
        let total: f64 = stats.speakers.iter().map(|s| s.share).sum();
        assert!((total - 1.0).abs() < 1e-9);
        assert_eq!(stats.speakers[0].questions, 3, "?? counts once");
        assert_eq!(stats.questions, 3);
        assert_eq!(s1.fillers, 3, "um, basically, you know");
    }

    #[test]
    fn moments_find_the_longest_stretch_silence_and_busiest_pair() {
        let stats = meeting(&sample());
        assert_eq!(stats.turns, 6);
        let longest = stats.longest_stretch.unwrap();
        assert_eq!(
            (longest.speaker.as_str(), longest.start_ms, longest.end_ms),
            ("Speaker 1", 7000, 40_000)
        );
        assert!(
            longest
                .preview
                .starts_with("The onboarding launch is Friday.")
        );
        assert_eq!(longest.preview.split_whitespace().count(), PREVIEW_WORDS);
        assert_eq!(
            stats.longest_silence,
            Some(Silence {
                start_ms: 45_000,
                end_ms: 70_000
            })
        );
        let pair = stats.busiest_pair.unwrap();
        assert_eq!(
            (pair.a.as_str(), pair.b.as_str(), pair.handovers),
            ("Me", "Speaker 1", 3)
        );
        assert_eq!(stats.span_ms, 95_000);
        assert_eq!(stats.speech_ms, (6 + 33 + 4 + 10 + 9 + 4) * 1000);
    }

    #[test]
    fn top_words_and_phrases_skip_common_words() {
        let stats = meeting(&sample());
        let words: Vec<(&str, usize)> = stats
            .top_words
            .iter()
            .map(|w| (w.text.as_str(), w.count))
            .collect();
        assert_eq!(&words[..3], [("page", 5), ("pricing", 5), ("analytics", 3)]);
        assert!(
            !words
                .iter()
                .any(|(w, _)| ["the", "basically", "though"].contains(w))
        );
        assert_eq!(
            stats.top_phrases[0],
            WordCount {
                text: "pricing page".into(),
                count: 5
            }
        );
        assert!(
            stats
                .top_phrases
                .iter()
                .any(|p| p.text == "onboarding launch")
        );
    }

    #[test]
    fn signature_words_need_enough_words_and_another_speaker() {
        let stats = meeting(&sample());
        assert!(
            stats.speakers.iter().all(|s| s.signature_words.is_empty()),
            "nobody said 40 words"
        );

        let mut long = sample();
        long.push(seg(
            100,
            160,
            "Speaker 1",
            &"retention cohort retention cohort dashboard ".repeat(10),
        ));
        let stats = meeting(&long);
        assert_eq!(
            stats.speakers[1].signature_words,
            ["cohort", "retention", "dashboard"]
        );
    }

    #[test]
    fn an_empty_transcript_has_no_stats_to_speak_of() {
        let stats = meeting(&[]);
        assert_eq!((stats.words, stats.turns, stats.speech_ms), (0, 0, 0));
        assert!(
            stats.speakers.is_empty() && stats.wpm.is_none() && stats.longest_stretch.is_none()
        );
    }

    #[test]
    fn dictation_stats_count_pace_savings_and_clean_up() {
        let entry = DictationEntry {
            id: 1,
            created_at: String::new(),
            text: "Let's meet at 4 p.m. on Thursday.".into(),
            raw: Some("Um, let's meet at 3, no, uh, 4 p.m. on Thursday, you know.".into()),
            polish_model: Some("model".into()),
            audio_ms: 6000,
            latency_ms: 900,
            pasted: true,
            app: None,
            corrected: None,
        };
        let stats = dictation(&entry, Some(150.0));
        assert_eq!(stats.words, 7);
        assert_eq!(stats.speaking_wpm, Some(70.0));
        assert_eq!(stats.raw_words, Some(13));
        assert_eq!(stats.fillers_removed, 3);
        // 7 words at 40 wpm is 10.5 s of typing; 6 s spent speaking.
        assert_eq!(stats.saved_ms, 4500);
        assert_eq!(stats.usual_wpm, Some(150.0));
    }
}
