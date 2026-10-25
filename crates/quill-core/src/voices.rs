//! Known voices: remembering the people the user names on calls, so later
//! meetings can label them by name instead of "Speaker 2".
//!
//! Three tables (see the migration in `library`):
//!
//! - `meeting_voices`: each meeting's people on the call, by their label in
//!   the transcript, with their average voice in that meeting. Written when
//!   speakers are told apart, and kept in step with renames.
//! - `known_voices`: a name and the voice model its embeddings came from.
//! - `voice_samples`: one sample per known voice per meeting, taken from
//!   `meeting_voices` when the user names (or confirms) someone. A known
//!   voice is matched by the average of its samples.
//!
//! Only what the user says teaches Quill a voice: naming a speaker, merging
//! two, or confirming an automatic name. An automatic match never adds a
//! sample by itself, so a wrong match can't reinforce itself; renaming it
//! takes the meeting's sample away from the wrong person. The mic track
//! ("Me") is never a call voice, so it's never remembered.
//!
//! Embeddings are biometric-like data: they stay in the local library, are
//! never logged, and are deleted with Settings → Known voices.

use quill_diarize::voices::{self as matching, DEFAULT_MIN_SPEECH_MS, Voice};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::library::{Library, LibraryError};
use crate::meeting::{ME, OTHERS};
use crate::speakers::{CallVoice, KnownVoice, rememberable};

/// The least speech behind a meeting's voice for it to be remembered: the
/// same as for matching, since a sample shorter than that is too noisy.
pub const MIN_SPEECH_TO_REMEMBER_MS: u64 = DEFAULT_MIN_SPEECH_MS;

/// A known voice, as Settings lists it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KnownVoiceInfo {
    pub id: i64,
    pub name: String,
    pub voice_model: String,
    /// Meetings it was learned from.
    pub meetings: u64,
    /// Speech behind it, over all those meetings.
    pub speech_ms: u64,
    pub created_at: String,
    pub updated_at: String,
}

/// A person on a meeting's call, as the transcript shows them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MeetingVoice {
    pub label: String,
    /// Named automatically by voice; the user hasn't confirmed or changed it.
    pub recognized: bool,
    /// How close the voices were, when recognised (cosine, up to 1).
    pub similarity: Option<f32>,
    /// Quill remembers this voice under this name from this meeting.
    pub remembered: bool,
    /// Enough speech in this meeting to remember the voice from.
    pub enough_speech: bool,
}

impl Library {
    /// Every known voice made with `voice_model`, each the average of its
    /// samples, for [`crate::speakers::label_and_recognize`].
    pub fn known_voices_to_match(
        &self,
        voice_model: &str,
    ) -> Result<Vec<KnownVoice>, LibraryError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT v.id, v.name, s.embedding, s.speech_ms
             FROM known_voices v JOIN voice_samples s ON s.voice_id = v.id
             WHERE v.voice_model = ?1 ORDER BY v.id",
        )?;
        let rows = stmt.query_map([voice_model], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                voice_from_row(&r.get::<_, Vec<u8>>(2)?, r.get(3)?),
            ))
        })?;
        let mut grouped: Vec<(i64, String, Vec<Voice>)> = Vec::new();
        for row in rows {
            let (id, name, voice) = row?;
            match grouped.last_mut() {
                Some((last, _, samples)) if *last == id => samples.push(voice),
                _ => grouped.push((id, name, vec![voice])),
            }
        }
        Ok(grouped
            .into_iter()
            .filter_map(|(id, name, samples)| {
                matching::average(&samples).map(|voice| KnownVoice { id, name, voice })
            })
            .collect())
    }

    /// Every known voice, by name.
    pub fn known_voices(&self) -> Result<Vec<KnownVoiceInfo>, LibraryError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT v.id, v.name, v.voice_model, COUNT(s.meeting_id),
                    COALESCE(SUM(s.speech_ms), 0), v.created_at, v.updated_at
             FROM known_voices v LEFT JOIN voice_samples s ON s.voice_id = v.id
             GROUP BY v.id ORDER BY v.name",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(KnownVoiceInfo {
                id: r.get(0)?,
                name: r.get(1)?,
                voice_model: r.get(2)?,
                meetings: u64::try_from(r.get::<_, i64>(3)?).unwrap_or(0),
                speech_ms: u64::try_from(r.get::<_, i64>(4)?).unwrap_or(0),
                created_at: r.get(5)?,
                updated_at: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Forgets one known voice and all its samples. Transcripts keep the
    /// name; it's just no longer recognised.
    pub fn delete_known_voice(&self, id: i64) -> Result<(), LibraryError> {
        self.conn()
            .execute("DELETE FROM known_voices WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Forgets every known voice. Returns how many there were.
    pub fn forget_known_voices(&self) -> Result<usize, LibraryError> {
        Ok(self.conn().execute("DELETE FROM known_voices", [])?)
    }

    /// Records the people on a meeting's call after speakers were told
    /// apart, replacing what was there. Call after `save_transcript`, which
    /// clears them.
    pub fn save_meeting_voices(
        &self,
        id: &str,
        voice_model: &str,
        voices: &[CallVoice],
    ) -> Result<(), LibraryError> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM meeting_voices WHERE meeting_id = ?1", [id])?;
        for v in voices.iter().filter(|v| !v.voice.embedding.is_empty()) {
            tx.execute(
                "INSERT OR REPLACE INTO meeting_voices
                 (meeting_id, label, voice_model, embedding, speech_ms, recognized, similarity)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    id,
                    v.label,
                    voice_model,
                    to_blob(&v.voice.embedding),
                    to_i64(v.voice.speech_ms),
                    v.recognized.map(|r| r.voice_id),
                    v.recognized.map(|r| f64::from(r.similarity)),
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The people on a meeting's call that Quill has voices for.
    pub fn meeting_voices(&self, id: &str) -> Result<Vec<MeetingVoice>, LibraryError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT m.label, m.recognized IS NOT NULL, m.similarity, m.speech_ms,
                    EXISTS (SELECT 1 FROM voice_samples s JOIN known_voices v ON v.id = s.voice_id
                            WHERE s.meeting_id = m.meeting_id AND v.name = m.label
                              AND v.voice_model = m.voice_model)
             FROM meeting_voices m WHERE m.meeting_id = ?1",
        )?;
        let rows = stmt.query_map([id], |r| {
            let speech_ms = u64::try_from(r.get::<_, i64>(3)?).unwrap_or(0);
            Ok(MeetingVoice {
                label: r.get(0)?,
                recognized: r.get(1)?,
                similarity: r.get::<_, Option<f64>>(2)?.map(|s| s as f32),
                remembered: r.get(4)?,
                enough_speech: speech_ms >= MIN_SPEECH_TO_REMEMBER_MS,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The user says an automatic name is right. With `remember_voice`, this
    /// meeting's voice is added to what Quill knows of them.
    pub fn confirm_speaker(
        &self,
        id: &str,
        label: &str,
        remember_voice: bool,
    ) -> Result<(), LibraryError> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE meeting_voices SET recognized = NULL, similarity = NULL
             WHERE meeting_id = ?1 AND label = ?2",
            params![id, label],
        )?;
        if remember_voice {
            remember(&tx, id, label)?;
        }
        tx.commit()?;
        Ok(())
    }
}

/// Keeps `meeting_voices` in step with a speaker renamed from `from` to `to`
/// in meeting `id`, inside the rename's transaction, and learns from it:
///
/// - the old name, if it was a known voice, loses this meeting's sample (the
///   user says it wasn't them), and is forgotten if that was its only one;
/// - renaming to a name another speaker has merges their voices;
/// - with `remember`, the new name is remembered with this meeting's voice.
pub(crate) fn renamed(
    conn: &Connection,
    id: &str,
    from: &str,
    to: &str,
    remember_voice: bool,
) -> rusqlite::Result<()> {
    let Some(old) = row(conn, id, from)? else {
        // Not a call voice ("Me"), or told apart before voices were kept.
        return Ok(());
    };
    forget_sample(conn, id, from, &old.voice_model)?;
    conn.execute(
        "DELETE FROM meeting_voices WHERE meeting_id = ?1 AND label = ?2",
        params![id, from],
    )?;
    // Merged into the mic track or back into "Others": no longer a voice of
    // its own on the call.
    if [ME, OTHERS, "You"]
        .iter()
        .any(|r| to.eq_ignore_ascii_case(r))
    {
        return Ok(());
    }
    let voice = match row(conn, id, to)? {
        Some(other) if other.voice_model == old.voice_model => {
            matching::average([&old.voice, &other.voice]).unwrap_or(old.voice)
        }
        _ => old.voice,
    };
    conn.execute(
        "INSERT OR REPLACE INTO meeting_voices
         (meeting_id, label, voice_model, embedding, speech_ms, recognized, similarity)
         VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL)",
        params![
            id,
            to,
            old.voice_model,
            to_blob(&voice.embedding),
            to_i64(voice.speech_ms)
        ],
    )?;
    if remember_voice {
        remember(conn, id, to)?;
    }
    Ok(())
}

struct Row {
    voice_model: String,
    voice: Voice,
}

fn row(conn: &Connection, id: &str, label: &str) -> rusqlite::Result<Option<Row>> {
    conn.query_row(
        "SELECT voice_model, embedding, speech_ms FROM meeting_voices
         WHERE meeting_id = ?1 AND label = ?2",
        params![id, label],
        |r| {
            Ok(Row {
                voice_model: r.get(0)?,
                voice: voice_from_row(&r.get::<_, Vec<u8>>(1)?, r.get(2)?),
            })
        },
    )
    .optional()
}

/// Saves meeting `id`'s voice for `label` as a sample of the known voice of
/// that name, creating it if needed. Does nothing for names that can't be
/// remembered or voices with too little speech. Returns whether it saved.
fn remember(conn: &Connection, id: &str, label: &str) -> rusqlite::Result<bool> {
    let Some(row) = row(conn, id, label)? else {
        return Ok(false);
    };
    if !rememberable(label)
        || row.voice.speech_ms < MIN_SPEECH_TO_REMEMBER_MS
        || row.voice.embedding.is_empty()
    {
        return Ok(false);
    }
    let now = jiff::Timestamp::now().to_string();
    conn.execute(
        "INSERT INTO known_voices (name, voice_model, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?3)
         ON CONFLICT (name, voice_model) DO UPDATE SET updated_at = excluded.updated_at",
        params![label.trim(), row.voice_model, now],
    )?;
    let voice_id: i64 = conn.query_row(
        "SELECT id FROM known_voices WHERE name = ?1 AND voice_model = ?2",
        params![label.trim(), row.voice_model],
        |r| r.get(0),
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO voice_samples (voice_id, meeting_id, embedding, speech_ms, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            voice_id,
            id,
            to_blob(&row.voice.embedding),
            to_i64(row.voice.speech_ms),
            now
        ],
    )?;
    Ok(true)
}

/// Removes meeting `id`'s sample from the known voice called `name`, and the
/// voice itself if that was all there was of it.
fn forget_sample(
    conn: &Connection,
    id: &str,
    name: &str,
    voice_model: &str,
) -> rusqlite::Result<()> {
    let voice_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM known_voices WHERE name = ?1 AND voice_model = ?2",
            params![name.trim(), voice_model],
            |r| r.get(0),
        )
        .optional()?;
    let Some(voice_id) = voice_id else {
        return Ok(());
    };
    conn.execute(
        "DELETE FROM voice_samples WHERE voice_id = ?1 AND meeting_id = ?2",
        params![voice_id, id],
    )?;
    conn.execute(
        "DELETE FROM known_voices WHERE id = ?1
         AND NOT EXISTS (SELECT 1 FROM voice_samples WHERE voice_id = ?1)",
        [voice_id],
    )?;
    Ok(())
}

fn to_blob(embedding: &[f32]) -> Vec<u8> {
    embedding.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn from_blob(blob: &[u8]) -> Vec<f32> {
    let (chunks, _) = blob.as_chunks::<4>();
    chunks.iter().map(|&b| f32::from_le_bytes(b)).collect()
}

fn voice_from_row(blob: &[u8], speech_ms: i64) -> Voice {
    Voice {
        embedding: from_blob(blob),
        speech_ms: u64::try_from(speech_ms).unwrap_or(0),
    }
}

fn to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::speakers::Recognized;
    use quill_stt::Segment;

    const MODEL: &str = "voice-test";

    fn library() -> (Library, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path(), &dir.path().join("recordings")).unwrap();
        (lib, dir)
    }

    fn seg(start_s: u64, speaker: &str) -> Segment {
        Segment {
            start_ms: start_s * 1000,
            end_ms: start_s * 1000 + 3000,
            text: "words".into(),
            speaker: Some(speaker.into()),
        }
    }

    fn voice(axis: usize, speech_ms: u64) -> Voice {
        let mut embedding = vec![0.0; 4];
        embedding[axis] = 1.0;
        Voice {
            embedding,
            speech_ms,
        }
    }

    fn call(label: &str, voice: Voice) -> CallVoice {
        CallVoice {
            label: label.into(),
            voice,
            recognized: None,
        }
    }

    /// A meeting with "Me" and two people on the call.
    fn meeting(lib: &Library, voices: &[CallVoice]) -> String {
        let m = lib.create_meeting().unwrap();
        let mut segments = vec![seg(0, ME)];
        segments.extend(
            voices
                .iter()
                .enumerate()
                .map(|(i, v)| seg(5 + i as u64 * 5, &v.label)),
        );
        lib.save_transcript(&m.id, &segments).unwrap();
        lib.save_meeting_voices(&m.id, MODEL, voices).unwrap();
        m.id
    }

    fn names(lib: &Library) -> Vec<(String, u64)> {
        lib.known_voices()
            .unwrap()
            .into_iter()
            .map(|v| (v.name, v.meetings))
            .collect()
    }

    #[test]
    fn naming_a_speaker_remembers_their_voice() {
        let (lib, _dir) = library();
        let id = meeting(
            &lib,
            &[
                call("Speaker 1", voice(0, 60_000)),
                call("Speaker 2", voice(1, 5000)),
            ],
        );
        lib.rename_speaker(&id, "Speaker 1", "Priya", true).unwrap();
        // Too little speech to remember Speaker 2 by.
        lib.rename_speaker(&id, "Speaker 2", "Dan", true).unwrap();
        // The mic track isn't a call voice.
        lib.rename_speaker(&id, ME, "Matt", true).unwrap();
        assert_eq!(names(&lib), [("Priya".to_string(), 1)]);

        let known = lib.known_voices_to_match(MODEL).unwrap();
        assert_eq!(known.len(), 1);
        assert_eq!(known[0].name, "Priya");
        assert_eq!(known[0].voice.embedding, voice(0, 0).embedding);
        // Embeddings from another model are never offered for matching.
        assert!(lib.known_voices_to_match("voice-other").unwrap().is_empty());

        let voices = lib.meeting_voices(&id).unwrap();
        let priya = voices.iter().find(|v| v.label == "Priya").unwrap();
        assert!(priya.remembered && priya.enough_speech && !priya.recognized);
        let dan = voices.iter().find(|v| v.label == "Dan").unwrap();
        assert!(!dan.remembered && !dan.enough_speech);
    }

    #[test]
    fn nothing_is_remembered_when_the_feature_is_off() {
        let (lib, _dir) = library();
        let id = meeting(&lib, &[call("Speaker 1", voice(0, 60_000))]);
        lib.rename_speaker(&id, "Speaker 1", "Priya", false)
            .unwrap();
        assert!(names(&lib).is_empty());
        // The label still follows the rename, for naming them later.
        assert_eq!(lib.meeting_voices(&id).unwrap()[0].label, "Priya");
    }

    #[test]
    fn a_voice_learns_from_each_meeting_and_survives_the_meeting() {
        let (lib, _dir) = library();
        let first = meeting(&lib, &[call("Speaker 1", voice(0, 60_000))]);
        let second = meeting(&lib, &[call("Speaker 1", voice(1, 40_000))]);
        lib.rename_speaker(&first, "Speaker 1", "Priya", true)
            .unwrap();
        // Typed in lower case: the same person.
        lib.rename_speaker(&second, "Speaker 1", "priya", true)
            .unwrap();
        assert_eq!(names(&lib), [("Priya".to_string(), 2)]);
        let known = &lib.known_voices_to_match(MODEL).unwrap()[0];
        // Weighted by speech: mostly the first meeting's voice.
        assert!(known.voice.similarity(&voice(0, 0).embedding) > 0.8);
        assert_eq!(known.voice.speech_ms, 100_000);

        lib.delete(&first).unwrap();
        assert_eq!(names(&lib), [("Priya".to_string(), 2)]);
    }

    #[test]
    fn merging_two_speakers_remembers_their_combined_voice() {
        let (lib, _dir) = library();
        let id = meeting(
            &lib,
            &[
                call("Speaker 1", voice(0, 30_000)),
                call("Speaker 2", voice(1, 30_000)),
            ],
        );
        lib.rename_speaker(&id, "Speaker 1", "Priya", true).unwrap();
        lib.rename_speaker(&id, "Speaker 2", "Priya", true).unwrap();
        let known = &lib.known_voices_to_match(MODEL).unwrap()[0];
        assert_eq!(known.voice.speech_ms, 60_000);
        assert!((known.voice.similarity(&voice(0, 0).embedding) - 0.707).abs() < 0.01);
        assert_eq!(lib.meeting_voices(&id).unwrap().len(), 1);
    }

    #[test]
    fn correcting_an_automatic_name_takes_the_sample_back() {
        let (lib, _dir) = library();
        let first = meeting(&lib, &[call("Speaker 1", voice(0, 60_000))]);
        lib.rename_speaker(&first, "Speaker 1", "Priya", true)
            .unwrap();
        let priya = lib.known_voices_to_match(MODEL).unwrap()[0].id;

        // Recognised automatically in a second meeting: nothing learned yet.
        let mut auto = call("Priya", voice(0, 60_000));
        auto.recognized = Some(Recognized {
            voice_id: priya,
            similarity: 0.8,
        });
        let second = meeting(&lib, &[auto]);
        let v = &lib.meeting_voices(&second).unwrap()[0];
        assert!(v.recognized && !v.remembered);
        assert_eq!(names(&lib), [("Priya".to_string(), 1)]);

        // Confirming it adds the second meeting's voice.
        lib.confirm_speaker(&second, "Priya", true).unwrap();
        let v = &lib.meeting_voices(&second).unwrap()[0];
        assert!(!v.recognized && v.remembered);
        assert_eq!(names(&lib), [("Priya".to_string(), 2)]);

        // It was Sam after all: Priya loses this meeting, Sam gains it.
        lib.rename_speaker(&second, "Priya", "Sam", true).unwrap();
        assert_eq!(
            names(&lib),
            [("Priya".to_string(), 1), ("Sam".to_string(), 1)]
        );

        // Un-naming the only meeting Sam was learned from forgets Sam.
        lib.rename_speaker(&second, "Sam", "Speaker 1", true)
            .unwrap();
        assert_eq!(names(&lib), [("Priya".to_string(), 1)]);
    }

    #[test]
    fn forgetting_voices_keeps_the_transcripts() {
        let (lib, _dir) = library();
        let id = meeting(&lib, &[call("Speaker 1", voice(0, 60_000))]);
        lib.rename_speaker(&id, "Speaker 1", "Priya", true).unwrap();
        let priya = lib.known_voices().unwrap()[0].id;
        lib.delete_known_voice(priya).unwrap();
        assert!(names(&lib).is_empty());
        let detail = lib.get(&id).unwrap();
        assert!(
            detail
                .segments
                .iter()
                .any(|s| s.speaker.as_deref() == Some("Priya"))
        );

        lib.rename_speaker(&id, "Priya", "Dan", true).unwrap();
        assert_eq!(lib.forget_known_voices().unwrap(), 1);
        assert!(names(&lib).is_empty());
    }

    #[test]
    fn transcribing_again_clears_the_meetings_voices() {
        let (lib, _dir) = library();
        let id = meeting(&lib, &[call("Speaker 1", voice(0, 60_000))]);
        lib.save_transcript(&id, &[seg(0, OTHERS)]).unwrap();
        assert!(lib.meeting_voices(&id).unwrap().is_empty());
    }
}
