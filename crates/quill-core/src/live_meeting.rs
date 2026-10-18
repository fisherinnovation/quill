//! Live notes during a meeting: a rough transcript built as the recording
//! goes, and notes an LLM keeps up to date from it every couple of minutes
//! (what to ask before the end, follow-ups, decisions).
//!
//! The transcript is a working draft for the notes. The meeting's real
//! transcript and summary are still made after the recording, as before,
//! with echo cancelled over the whole recording and speakers told apart.

use quill_llm::prompts::{LivePrompts, RenderError, render};
use quill_llm::{CompletionRequest, LlmError, LlmProvider, Message};
use quill_stt::{SttError, TranscribeOptions, Transcriber};
use serde::{Deserialize, Serialize};

use crate::live::{pause, rms};
use crate::meeting::{ME, OTHERS};

const SAMPLES_PER_MS: usize = 16;
/// Transcribe once this much audio has built up...
const CHUNK_MS: usize = 20_000;
/// ...cutting where both tracks are quiet, found in the last stretch of it...
const CUT_SEARCH_MS: usize = 8_000;
/// ...or, if nobody pauses, here anyway.
const MAX_CHUNK_MS: usize = 45_000;
const PAUSE_RATIO: f32 = 0.2;
/// The notes are updated from this much of the latest transcript; the notes
/// themselves carry what came before, so each update costs about the same.
pub const WINDOW_MS: u64 = 15 * 60_000;

/// One transcribed line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Line {
    /// From the start of the recording.
    pub start_ms: u64,
    /// "Me" or "Others".
    pub speaker: &'static str,
    pub text: String,
}

/// The transcript so far, from the two tracks as they're recorded.
#[derive(Debug, Default)]
pub struct LiveTranscript {
    mic: Vec<f32>,
    system: Vec<f32>,
    /// Where the audio not yet transcribed starts, in samples.
    from: usize,
    lines: Vec<Line>,
}

impl LiveTranscript {
    pub fn new() -> Self {
        Self::default()
    }

    /// New audio (16 kHz mono) from the microphone and the call. The two
    /// tracks are aligned from the start, so only what both have reached is
    /// transcribed.
    pub fn push(&mut self, mic: &[f32], system: &[f32]) {
        self.mic.extend_from_slice(mic);
        self.system.extend_from_slice(system);
    }

    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    /// Transcribes the next chunk if enough audio has built up (or all of it
    /// with `flush`). Returns how many lines were added.
    pub fn step(
        &mut self,
        transcriber: &dyn Transcriber,
        opts: &TranscribeOptions,
        flush: bool,
    ) -> Result<usize, SttError> {
        let ready = self.mic.len().min(self.system.len());
        let Some(cut) = self.cut(ready, flush) else {
            return Ok(0);
        };
        let system: Vec<f32> = self.system.drain(..cut).collect();
        let mic: Vec<f32> = self.mic.drain(..cut).collect();
        // Take out what the speakers played, so the call isn't heard as you.
        let mic = quill_audio::aec::cancel_echo(&mic, &system, quill_audio::STT_SAMPLE_RATE)
            .unwrap_or(mic);
        let offset_ms = (self.from / SAMPLES_PER_MS) as u64;
        let before = self.lines.len();
        for (pcm, speaker) in [(&mic, ME), (&system, OTHERS)] {
            if rms(pcm) < 1e-4 {
                continue;
            }
            for segment in transcriber.transcribe(pcm, opts)? {
                let text = segment.text.trim();
                if !text.is_empty() {
                    self.lines.push(Line {
                        start_ms: offset_ms + segment.start_ms,
                        speaker,
                        text: text.to_string(),
                    });
                }
            }
        }
        self.lines[before..].sort_by_key(|l| l.start_ms);
        self.from += cut;
        Ok(self.lines.len() - before)
    }

    /// Where to cut the audio that's ready: in a pause on both tracks near
    /// its end, once there's enough; at the end when flushing or when nobody
    /// has paused for a long time.
    fn cut(&self, ready: usize, flush: bool) -> Option<usize> {
        let ms = |ms: usize| ms * SAMPLES_PER_MS;
        if flush {
            return (ready > 0).then_some(ready);
        }
        if ready < ms(CHUNK_MS) {
            return None;
        }
        let both: Vec<f32> = self.mic[..ready]
            .iter()
            .zip(&self.system[..ready])
            .map(|(a, b)| a.abs().max(b.abs()))
            .collect();
        let quiet = rms(&both) * PAUSE_RATIO;
        pause(&both, ready - ms(CUT_SEARCH_MS), ready - ms(500), quiet)
            .or_else(|| (ready >= ms(MAX_CHUNK_MS)).then_some(ready))
    }

    /// The latest [`WINDOW_MS`] as "[mm:ss] Me: …" lines, for the notes.
    pub fn recent_text(&self) -> String {
        let end = self.lines.last().map_or(0, |l| l.start_ms);
        self.lines
            .iter()
            .filter(|l| l.start_ms + WINDOW_MS >= end)
            .map(|l| {
                let s = l.start_ms / 1000;
                format!("[{:02}:{:02}] {}: {}", s / 60, s % 60, l.speaker, l.text)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// What kind of note an item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Worth raising before the meeting ends.
    Ask,
    FollowUp,
    Decision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub kind: Kind,
    pub text: String,
    #[serde(default)]
    pub who: Option<String>,
    /// Dealt with: asked, answered, given an owner (by the conversation or
    /// ticked off by the user).
    #[serde(default)]
    pub done: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notes {
    pub gist: String,
    pub items: Vec<Item>,
}

/// An item as the model returns it: no id for new ones, lenient fields.
#[derive(Debug, Deserialize)]
struct Reply {
    #[serde(default)]
    gist: String,
    #[serde(default)]
    items: Vec<ReplyItem>,
}

#[derive(Debug, Deserialize)]
struct ReplyItem {
    #[serde(default)]
    id: Option<String>,
    kind: Kind,
    text: String,
    #[serde(default)]
    who: Option<String>,
    #[serde(default)]
    done: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum NotesError {
    #[error(transparent)]
    Llm(#[from] LlmError),
    #[error("prompt template error: {0}")]
    Prompt(#[from] RenderError),
    #[error("the model's notes weren't valid JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// One update: the notes so far, what the user dismissed or ticked off,
/// and the latest transcript in; the notes brought up to date out.
pub struct Update<'a> {
    pub previous: &'a Notes,
    /// Items the user dismissed: never shown again.
    pub dismissed: &'a [Item],
    /// Ids of items the user ticked off.
    pub ticked: &'a [String],
    pub transcript: &'a str,
    pub minutes: u64,
}

/// Asks the model for updated notes, and what the call cost.
pub async fn update(
    provider: &dyn LlmProvider,
    model: &str,
    prompts: &LivePrompts,
    update: Update<'_>,
) -> Result<(Notes, Option<f64>), NotesError> {
    let notes = serde_json::to_string(update.previous)?;
    let dismissed = if update.dismissed.is_empty() {
        "(none)".to_string()
    } else {
        update
            .dismissed
            .iter()
            .map(|i| format!("{} (\"{}\")", i.id, i.text))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let minutes = update.minutes.to_string();
    let user = render(
        prompts.user,
        &[
            ("minutes", minutes.as_str()),
            ("notes", notes.as_str()),
            ("dismissed", dismissed.as_str()),
            ("transcript", update.transcript),
        ],
    )?;
    let response = provider
        .complete(CompletionRequest {
            model: model.to_string(),
            messages: vec![Message::system(prompts.system), Message::user(user)],
            max_tokens: Some(1_500),
            temperature: Some(0.2),
            no_reasoning: true,
        })
        .await?;
    let reply = parse(&response.text)?;
    Ok((
        merge(update.previous, reply, update.dismissed, update.ticked),
        response.cost_usd,
    ))
}

/// The JSON object in the model's reply, fences or chatter around it
/// ignored.
fn parse(text: &str) -> Result<Reply, serde_json::Error> {
    let start = text.find('{').unwrap_or(0);
    let end = text.rfind('}').map_or(text.len(), |i| i + 1);
    serde_json::from_str(&text[start..end.max(start)])
}

/// The new notes: ids kept where the model kept them, new ones numbered
/// after the highest so far, dismissed items left out (by id or text, so
/// they can't come back renumbered), ticked ones done.
fn merge(previous: &Notes, reply: Reply, dismissed: &[Item], ticked: &[String]) -> Notes {
    let key = |text: &str| {
        text.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    let gone = |id: Option<&str>, text: &str| {
        dismissed
            .iter()
            .any(|d| Some(d.id.as_str()) == id || key(&d.text) == key(text))
    };
    let mut next = previous
        .items
        .iter()
        .chain(dismissed)
        .filter_map(|i| i.id.strip_prefix('n')?.parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    let known: Vec<&str> = previous.items.iter().map(|i| i.id.as_str()).collect();
    let mut items: Vec<Item> = Vec::new();
    for item in reply.items {
        if gone(item.id.as_deref(), &item.text) || item.text.trim().is_empty() {
            continue;
        }
        let id = match item.id {
            Some(id) if known.contains(&id.as_str()) && !items.iter().any(|i| i.id == id) => id,
            _ => {
                next += 1;
                format!("n{next}")
            }
        };
        let who = item
            .who
            .map(|w| w.trim().to_string())
            .filter(|w| !w.is_empty());
        // "Priya: fix…" with who "Priya" would show the name twice.
        let text = item.text.trim();
        let text = who
            .as_deref()
            .and_then(|w| text.strip_prefix(w))
            .and_then(|rest| rest.strip_prefix(':').or_else(|| rest.strip_prefix(" -")))
            .map_or(text, str::trim);
        items.push(Item {
            // Decisions are facts, not to-dos.
            done: item.kind != Kind::Decision && (item.done || ticked.contains(&id)),
            id,
            kind: item.kind,
            text: text.to_string(),
            who,
        });
    }
    Notes {
        gist: reply.gist.trim().to_string(),
        items,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quill_stt::Segment;

    /// Says what it heard, by loudness: loud audio is "hello".
    struct Echo;

    impl Transcriber for Echo {
        fn transcribe(&self, pcm: &[f32], _: &TranscribeOptions) -> Result<Vec<Segment>, SttError> {
            Ok((rms(pcm) > 0.05)
                .then(|| Segment {
                    start_ms: 0,
                    end_ms: (pcm.len() / SAMPLES_PER_MS) as u64,
                    text: format!("{} ms", pcm.len() / SAMPLES_PER_MS),
                    speaker: None,
                })
                .into_iter()
                .collect())
        }
    }

    fn speech(ms: usize) -> Vec<f32> {
        (0..ms * SAMPLES_PER_MS)
            .map(|i| if i % 2 == 0 { 0.3 } else { -0.3 })
            .collect()
    }

    #[test]
    fn transcribes_in_chunks_cut_at_a_shared_pause() {
        let mut live = LiveTranscript::new();
        // The call talks for 18 s, pauses 1 s, talks 6 s; you're silent.
        let mut system = speech(18_000);
        system.extend(vec![0.0; 16_000]);
        system.extend(speech(6_000));
        let mic = vec![0.0; system.len()];
        // Not enough yet, then only what both tracks have.
        live.push(&mic[..10 * 16_000], &system[..10 * 16_000]);
        assert_eq!(
            live.step(&Echo, &TranscribeOptions::default(), false)
                .unwrap(),
            0
        );
        live.push(&mic[10 * 16_000..], &system[10 * 16_000..]);
        assert_eq!(
            live.step(&Echo, &TranscribeOptions::default(), false)
                .unwrap(),
            1
        );
        // Cut in the pause (18–19 s), not mid-speech.
        let line = &live.lines()[0];
        assert_eq!(line.speaker, OTHERS);
        let cut_ms: u64 = line.text.trim_end_matches(" ms").parse().unwrap();
        assert!((18_000..19_000).contains(&cut_ms), "{cut_ms}");
        // The rest comes on flush, timed from the start of the recording.
        assert_eq!(
            live.step(&Echo, &TranscribeOptions::default(), true)
                .unwrap(),
            1
        );
        assert_eq!(live.lines()[1].start_ms, cut_ms);
        assert!(live.recent_text().starts_with("[00:00] Others: "));
    }

    #[test]
    fn merging_keeps_ids_numbers_new_items_and_respects_the_user() {
        let previous = Notes {
            gist: "Launch.".into(),
            items: vec![
                Item {
                    id: "n1".into(),
                    kind: Kind::Ask,
                    text: "Ask who owns rollout".into(),
                    who: None,
                    done: false,
                },
                Item {
                    id: "n2".into(),
                    kind: Kind::FollowUp,
                    text: "Fix events".into(),
                    who: None,
                    done: false,
                },
            ],
        };
        let reply = parse(
            r#"Here you go:
```json
{"gist": "Rollout owner agreed.", "items": [
  {"id": "n1", "kind": "ask", "text": "Ask who owns the rollout plan", "done": true},
  {"id": "n2", "kind": "follow_up", "text": "Priya: Fix duplicate events", "who": "Priya"},
  {"id": null, "kind": "decision", "text": "Ship Friday behind a flag", "done": true},
  {"id": "n9", "kind": "ask", "text": "Made-up id"},
  {"id": "n3", "kind": "ask", "text": "Dismissed earlier"},
  {"id": null, "kind": "ask", "text": "dismissed earlier!"}
]}
```"#,
        )
        .unwrap();
        let dismissed = [Item {
            id: "n3".into(),
            kind: Kind::Ask,
            text: "Dismissed earlier".into(),
            who: None,
            done: false,
        }];
        let notes = merge(&previous, reply, &dismissed, &["n2".into()]);
        assert_eq!(notes.gist, "Rollout owner agreed.");
        let ids: Vec<&str> = notes.items.iter().map(|i| i.id.as_str()).collect();
        // New items are numbered after the highest id, dismissed ones
        // included; n9 wasn't ours, so it's renumbered too. The dismissed
        // item stays gone, by id or by its text without one.
        assert_eq!(ids, ["n1", "n2", "n4", "n5"]);
        assert!(notes.items[0].done, "the model saw it dealt with");
        assert!(notes.items[1].done, "the user ticked it");
        assert_eq!(notes.items[1].who.as_deref(), Some("Priya"));
        assert_eq!(
            notes.items[1].text, "Fix duplicate events",
            "the name isn't repeated"
        );
        assert_eq!(notes.items[2].kind, Kind::Decision);
        assert!(!notes.items[2].done, "decisions are never done");
    }

    #[test]
    fn a_reply_that_isnt_json_is_an_error() {
        assert!(parse("Sorry, I can't help with that.").is_err());
    }
}
