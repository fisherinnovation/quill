//! The meeting library: every recording, its transcript and its summaries.
//!
//! Audio stays on disk in `<recordings dir>/<id>/` (mic/system .wav while
//! recording, .flac once transcribed);
//! the recordings folder defaults to `<data dir>/recordings` and can be moved
//! (see [`Library::move_recordings`]). Everything you browse or search lives in SQLite
//! (`<data dir>/quill.sqlite`), with an FTS5 full-text index over transcript
//! text. The CLI and the app share the same library.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};

use quill_stt::Segment;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::corrections::{Kind, Learned};

use crate::summary::Summary;

/// Schema migrations, applied in order; `PRAGMA user_version` records how
/// many have run. Never edit a shipped migration — add a new one.
const MIGRATIONS: &[&str] = &[
    "
CREATE TABLE meetings (
    id           TEXT PRIMARY KEY,
    title        TEXT NOT NULL,
    -- true once the user renames it, so summaries stop overwriting it
    title_locked INTEGER NOT NULL DEFAULT 0,
    started_at   TEXT NOT NULL,            -- RFC 3339
    duration_ms  INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE segments (
    meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    idx        INTEGER NOT NULL,
    start_ms   INTEGER NOT NULL,
    end_ms     INTEGER NOT NULL,
    speaker    TEXT,
    text       TEXT NOT NULL,
    PRIMARY KEY (meeting_id, idx)
);
-- Full-text index over segment text. Kept in sync by save_transcript.
CREATE VIRTUAL TABLE segments_fts USING fts5(
    text,
    meeting_id UNINDEXED,
    start_ms UNINDEXED,
    speaker UNINDEXED,
    tokenize = 'porter unicode61'
);
CREATE TABLE summaries (
    id                INTEGER PRIMARY KEY,
    meeting_id        TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    markdown          TEXT NOT NULL,
    model             TEXT NOT NULL,
    prompt_version    TEXT NOT NULL,
    prompt_tokens     INTEGER NOT NULL,
    completion_tokens INTEGER NOT NULL,
    cost_usd          REAL,
    latency_ms        INTEGER NOT NULL,
    created_at        TEXT NOT NULL
);
",
    "
-- Dictation history: what was typed, for copying again or recovering
-- text that didn't land where it should have.
CREATE TABLE dictations (
    id           INTEGER PRIMARY KEY,
    created_at   TEXT NOT NULL,            -- RFC 3339
    text         TEXT NOT NULL,            -- what was typed
    raw          TEXT,                     -- Whisper's text, if polish changed it
    polish_model TEXT,
    audio_ms     INTEGER NOT NULL,
    latency_ms   INTEGER NOT NULL,         -- key release to text typed
    pasted       INTEGER NOT NULL          -- 0: left on the clipboard
);
",
    "
-- Per-dictation numbers for the Insights page: counts only, never text, so
-- they're kept even when history is off or cleared.
CREATE TABLE dictation_stats (
    id         INTEGER PRIMARY KEY,
    created_at TEXT NOT NULL,              -- RFC 3339
    words      INTEGER NOT NULL,
    audio_ms   INTEGER NOT NULL,
    latency_ms INTEGER NOT NULL,
    polished   INTEGER NOT NULL,
    pasted     INTEGER NOT NULL
);
-- Count what's already in the history (words approximated by spaces).
INSERT INTO dictation_stats (created_at, words, audio_ms, latency_ms, polished, pasted)
SELECT created_at,
       LENGTH(TRIM(text)) - LENGTH(REPLACE(TRIM(text), ' ', '')) + 1,
       audio_ms, latency_ms, polish_model IS NOT NULL, pasted
FROM dictations;
",
    "
-- The app dictated into: id (bundle id / executable) and display name.
ALTER TABLE dictation_stats ADD COLUMN app_id TEXT;
ALTER TABLE dictation_stats ADD COLUMN app TEXT;
ALTER TABLE dictations ADD COLUMN app TEXT;
",
    "
-- Learning from corrections: the user's fixed version of a dictation, and
-- the rules suggested by fixes (pairs of words only, never the text).
ALTER TABLE dictations ADD COLUMN corrected TEXT;
CREATE TABLE suggestions (
    id         INTEGER PRIMARY KEY,
    kind       TEXT NOT NULL,              -- 'vocabulary' | 'replacement'
    from_text  TEXT NOT NULL DEFAULT '',   -- '' for vocabulary
    to_text    TEXT NOT NULL,
    app        TEXT,                       -- where it was last seen
    seen       INTEGER NOT NULL DEFAULT 1, -- corrections that suggested it
    status     TEXT NOT NULL DEFAULT 'pending', -- pending | accepted | dismissed
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (kind, from_text, to_text)
);
",
    "
-- Recordings with nothing in them: `transcribed` tells \"transcribed, and
-- nothing was said\" from \"not transcribed yet\"; `keep_empty` is set
-- when the user chooses to keep such a recording anyway.
ALTER TABLE meetings ADD COLUMN transcribed INTEGER NOT NULL DEFAULT 0;
ALTER TABLE meetings ADD COLUMN keep_empty INTEGER NOT NULL DEFAULT 0;
UPDATE meetings SET transcribed = 1
WHERE EXISTS (SELECT 1 FROM segments s WHERE s.meeting_id = meetings.id);
",
    "
-- The identifier of the app dictated into (bundle id / executable), for its
-- icon. Earlier dictations get it from the stats, matched by app name.
ALTER TABLE dictations ADD COLUMN app_id TEXT;
UPDATE dictations SET app_id = (
    SELECT s.app_id FROM dictation_stats s
    WHERE s.app = dictations.app AND s.app_id IS NOT NULL
    ORDER BY s.id DESC LIMIT 1
) WHERE app IS NOT NULL;
",
];

/// The migration that added `meetings.transcribed`; meetings from before it
/// are checked once for a transcript file (see `Library::init`).
const TRANSCRIBED_MIGRATION: i64 = 6;

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("no meeting with id {0}")]
    NotFound(String),
    #[error("{0}")]
    Move(String),
}

/// A meeting as shown in the list.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Meeting {
    pub id: String,
    pub title: String,
    pub started_at: String,
    pub duration_ms: u64,
    pub has_transcript: bool,
    pub has_summary: bool,
    /// Transcribed, and nothing was said (unless the user chose to keep it).
    pub empty: bool,
}

/// One stored summary and what produced it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StoredSummary {
    pub markdown: String,
    pub model: String,
    pub prompt_version: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cost_usd: Option<f64>,
    pub latency_ms: u64,
    pub created_at: String,
}

/// Everything about one meeting.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MeetingDetail {
    pub meeting: Meeting,
    pub segments: Vec<Segment>,
    /// Newest first.
    pub summaries: Vec<StoredSummary>,
}

/// A transcript line matching a search.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchHit {
    pub meeting_id: String,
    pub title: String,
    pub start_ms: u64,
    pub speaker: Option<String>,
    /// Matching text with hits wrapped in `[` `]`.
    pub snippet: String,
}

/// One dictation, as stored in the history.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DictationEntry {
    pub id: i64,
    pub created_at: String,
    /// What was typed.
    pub text: String,
    /// Whisper's text, when the polish step changed it.
    pub raw: Option<String>,
    pub polish_model: Option<String>,
    pub audio_ms: u64,
    /// From releasing the hotkey to the text being typed.
    pub latency_ms: u64,
    /// False if it was left on the clipboard instead (no permission to paste).
    pub pasted: bool,
    /// The app it was typed into, e.g. "Slack".
    pub app: Option<String>,
    /// That app's identifier (bundle id or executable), for its icon.
    #[serde(default)]
    pub app_id: Option<String>,
    /// The user's fixed version, if they corrected it.
    #[serde(default)]
    pub corrected: Option<String>,
}

/// A rule suggested by the user's corrections (see `corrections`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Suggestion {
    pub id: i64,
    pub kind: Kind,
    /// Empty for vocabulary.
    pub from: String,
    pub to: String,
    pub app: Option<String>,
    /// How many corrections suggested it.
    pub seen: u32,
    pub status: SuggestionStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionStatus {
    Pending,
    Accepted,
    Dismissed,
}

impl SuggestionStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Accepted => "accepted",
            Self::Dismissed => "dismissed",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "accepted" => Self::Accepted,
            "dismissed" => Self::Dismissed,
            _ => Self::Pending,
        }
    }
}

/// One dictation's numbers, kept for insights (no text).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DictationStat {
    pub created_at: String,
    pub words: u64,
    pub audio_ms: u64,
    pub latency_ms: u64,
    pub polished: bool,
    pub pasted: bool,
    /// The app dictated into: identifier and display name.
    pub app_id: Option<String>,
    pub app: Option<String>,
}

/// How much was dictated into one app.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AppUsage {
    pub app_id: String,
    pub name: String,
    pub dictations: u64,
    pub words: u64,
}

/// Dictation history entries kept; older ones are deleted as new ones arrive.
pub const DICTATION_HISTORY_LIMIT: i64 = 500;

/// The `dictations` columns [`dictation_row`] reads, in its order.
const DICTATION_COLUMNS: &str =
    "id, created_at, text, raw, polish_model, audio_ms, latency_ms, pasted, app, corrected, app_id";

fn dictation_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<DictationEntry> {
    Ok(DictationEntry {
        id: r.get(0)?,
        created_at: r.get(1)?,
        text: r.get(2)?,
        raw: r.get(3)?,
        polish_model: r.get(4)?,
        audio_ms: u64::try_from(r.get::<_, i64>(5)?).unwrap_or(0),
        latency_ms: u64::try_from(r.get::<_, i64>(6)?).unwrap_or(0),
        pasted: r.get(7)?,
        app: r.get(8)?,
        corrected: r.get(9)?,
        app_id: r.get(10)?,
    })
}

/// Handle to the library. Cheap to share behind an `Arc`; the connection is
/// behind a `Mutex` because `rusqlite::Connection` can't be used from two
/// threads at once.
pub struct Library {
    conn: Mutex<Connection>,
    recordings_dir: RwLock<PathBuf>,
}

impl Library {
    /// Opens (creating and migrating if needed) the library database in
    /// `data_dir`, with meeting audio under `recordings_dir`.
    pub fn open(data_dir: &Path, recordings_dir: &Path) -> Result<Self, LibraryError> {
        std::fs::create_dir_all(data_dir)?;
        // Not fatal: the folder may be on a drive that isn't connected right
        // now. Browsing still works; recording reports the error.
        if let Err(e) = std::fs::create_dir_all(recordings_dir) {
            tracing::warn!(dir = %recordings_dir.display(), "recordings folder unavailable: {e}");
        }
        let conn = Connection::open(data_dir.join("quill.sqlite"))?;
        Self::init(conn, recordings_dir.to_path_buf())
    }

    /// An in-memory library for tests.
    pub fn open_in_memory(recordings_dir: PathBuf) -> Result<Self, LibraryError> {
        Self::init(Connection::open_in_memory()?, recordings_dir)
    }

    fn init(conn: Connection, recordings_dir: PathBuf) -> Result<Self, LibraryError> {
        conn.pragma_update(None, "foreign_keys", true)?;
        let applied: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        for (version, migration) in (1_i64..).zip(MIGRATIONS) {
            if version > applied {
                conn.execute_batch(migration)?;
                conn.pragma_update(None, "user_version", version)?;
            }
        }
        let library = Self {
            conn: Mutex::new(conn),
            recordings_dir: RwLock::new(recordings_dir),
        };
        if applied < TRANSCRIBED_MIGRATION {
            library.mark_transcribed_from_files()?;
        }
        Ok(library)
    }

    /// Meetings transcribed before `transcribed` existed but with nothing
    /// said have no segments, so the migration can't tell them from
    /// untranscribed ones. The pipeline always writes `transcript.json`
    /// after transcribing, so its presence settles it.
    fn mark_transcribed_from_files(&self) -> Result<(), LibraryError> {
        let ids: Vec<String> = {
            let conn = self.conn();
            let mut stmt = conn.prepare("SELECT id FROM meetings WHERE transcribed = 0")?;
            let rows = stmt.query_map([], |r| r.get(0))?;
            rows.collect::<Result<_, _>>()?
        };
        for id in ids {
            if self
                .meeting_dir(&id)
                .join(crate::speakers::TRANSCRIPT_FILE)
                .is_file()
            {
                self.conn()
                    .execute("UPDATE meetings SET transcribed = 1 WHERE id = ?1", [&id])?;
            }
        }
        Ok(())
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        // A panic while holding the lock can't leave SQLite inconsistent
        // (each write is its own statement or transaction), so recover.
        self.conn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The directory holding every meeting's folder.
    pub fn recordings_dir(&self) -> PathBuf {
        self.recordings_dir
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Where a meeting's audio files live.
    pub fn meeting_dir(&self, id: &str) -> PathBuf {
        self.recordings_dir().join(id)
    }

    /// Moves every meeting's audio folder into `to` and uses it from now on.
    /// Returns how many meetings moved.
    ///
    /// All or nothing: if any folder can't be moved, the ones already moved
    /// go back. Only meeting folders move; anything else in either folder is
    /// left alone. The caller must make sure nothing is recording or being
    /// processed meanwhile.
    pub fn move_recordings(&self, to: &Path) -> Result<usize, LibraryError> {
        let from = self.recordings_dir();
        std::fs::create_dir_all(to)?;
        let to = to.canonicalize()?;
        if let Ok(from) = from.canonicalize() {
            if from == to {
                return Ok(0);
            }
            if to.starts_with(&from) {
                return Err(LibraryError::Move(
                    "choose a folder outside the current recordings folder".into(),
                ));
            }
        }

        let ids: Vec<String> = {
            let conn = self.conn();
            let mut stmt = conn.prepare("SELECT id FROM meetings")?;
            stmt.query_map([], |r| r.get(0))?
                .collect::<Result<_, _>>()?
        };
        let to_move: Vec<&String> = ids.iter().filter(|id| from.join(id).exists()).collect();
        if let Some(id) = to_move.iter().find(|id| to.join(id).exists()) {
            return Err(LibraryError::Move(format!(
                "{} already has a folder named {id}",
                to.display()
            )));
        }

        for (done, id) in to_move.iter().enumerate() {
            if let Err(e) = move_dir(&from.join(id), &to.join(id)) {
                for id in to_move.iter().take(done).rev() {
                    if let Err(undo) = move_dir(&to.join(id), &from.join(id)) {
                        tracing::error!(%id, "could not move recording back: {undo}");
                    }
                }
                return Err(LibraryError::Move(format!("could not move {id}: {e}")));
            }
        }
        *self
            .recordings_dir
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = to;
        Ok(to_move.len())
    }

    /// Creates a meeting starting now and its directory. The id is the local
    /// start time (`20261011-191503`), suffixed if two start in one second.
    pub fn create_meeting(&self) -> Result<Meeting, LibraryError> {
        let now = jiff::Zoned::now();
        let base = now.strftime("%Y%m%d-%H%M%S").to_string();
        let title = format!("Meeting {}", now.strftime("%b %-d, %-I:%M %p"));
        let conn = self.conn();
        let mut id = base.clone();
        for n in 2.. {
            let exists: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM meetings WHERE id = ?1)",
                [&id],
                |r| r.get(0),
            )?;
            if !exists && !self.meeting_dir(&id).exists() {
                break;
            }
            id = format!("{base}-{n}");
        }
        std::fs::create_dir_all(self.meeting_dir(&id))?;
        let started_at = now.timestamp().to_string();
        conn.execute(
            "INSERT INTO meetings (id, title, started_at) VALUES (?1, ?2, ?3)",
            params![id, title, started_at],
        )?;
        Ok(Meeting {
            id,
            title,
            started_at,
            duration_ms: 0,
            has_transcript: false,
            has_summary: false,
            empty: false,
        })
    }

    /// Keeps a recording with nothing in it: it's no longer flagged as
    /// empty.
    pub fn keep_empty(&self, id: &str) -> Result<(), LibraryError> {
        self.update(id, "UPDATE meetings SET keep_empty = ?2 WHERE id = ?1", 1)
    }

    pub fn set_duration(&self, id: &str, duration_ms: u64) -> Result<(), LibraryError> {
        self.update(
            id,
            "UPDATE meetings SET duration_ms = ?2 WHERE id = ?1",
            to_i64(duration_ms),
        )
    }

    /// Renames a meeting; later summaries won't overwrite this title.
    pub fn rename(&self, id: &str, title: &str) -> Result<(), LibraryError> {
        self.update(
            id,
            "UPDATE meetings SET title = ?2, title_locked = 1 WHERE id = ?1",
            title,
        )
    }

    fn update(&self, id: &str, sql: &str, value: impl rusqlite::ToSql) -> Result<(), LibraryError> {
        match self.conn().execute(sql, params![id, value])? {
            0 => Err(LibraryError::NotFound(id.to_string())),
            _ => Ok(()),
        }
    }

    /// Replaces a meeting's transcript (and its search index entries).
    pub fn save_transcript(&self, id: &str, segments: &[Segment]) -> Result<(), LibraryError> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        if !exists(&tx, id)? {
            return Err(LibraryError::NotFound(id.to_string()));
        }
        tx.execute("DELETE FROM segments WHERE meeting_id = ?1", [id])?;
        tx.execute("DELETE FROM segments_fts WHERE meeting_id = ?1", [id])?;
        for (i, s) in (0_i64..).zip(segments) {
            tx.execute(
                "INSERT INTO segments (meeting_id, idx, start_ms, end_ms, speaker, text) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, i, to_i64(s.start_ms), to_i64(s.end_ms), s.speaker, s.text],
            )?;
            tx.execute(
                "INSERT INTO segments_fts (text, meeting_id, start_ms, speaker) VALUES (?1, ?2, ?3, ?4)",
                params![s.text, id, to_i64(s.start_ms), s.speaker],
            )?;
        }
        let end = segments.iter().map(|s| s.end_ms).max().unwrap_or(0);
        tx.execute(
            "UPDATE meetings SET duration_ms = MAX(duration_ms, ?2), transcribed = 1 WHERE id = ?1",
            params![id, to_i64(end)],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Renames a speaker throughout a meeting's transcript. Renaming to a
    /// name another speaker already has merges the two. Returns how many
    /// segments changed.
    pub fn rename_speaker(&self, id: &str, from: &str, to: &str) -> Result<usize, LibraryError> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        if !exists(&tx, id)? {
            return Err(LibraryError::NotFound(id.to_string()));
        }
        let changed = tx.execute(
            "UPDATE segments SET speaker = ?3 WHERE meeting_id = ?1 AND speaker = ?2",
            params![id, from, to],
        )?;
        tx.execute(
            "UPDATE segments_fts SET speaker = ?3 WHERE meeting_id = ?1 AND speaker = ?2",
            params![id, from, to],
        )?;
        tx.commit()?;
        Ok(changed)
    }

    /// Stores a new summary. Unless the user renamed the meeting, its title
    /// becomes the summary's `# Title — date` heading.
    pub fn save_summary(&self, id: &str, summary: &Summary) -> Result<(), LibraryError> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        if !exists(&tx, id)? {
            return Err(LibraryError::NotFound(id.to_string()));
        }
        tx.execute(
            "INSERT INTO summaries (meeting_id, markdown, model, prompt_version, prompt_tokens,
                completion_tokens, cost_usd, latency_ms, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id,
                summary.markdown,
                summary.model,
                summary.prompt_version,
                to_i64(summary.prompt_tokens),
                to_i64(summary.completion_tokens),
                summary.cost_usd,
                to_i64(summary.latency_ms),
                jiff::Timestamp::now().to_string(),
            ],
        )?;
        if let Some(title) = title_from_markdown(&summary.markdown) {
            tx.execute(
                "UPDATE meetings SET title = ?2 WHERE id = ?1 AND title_locked = 0",
                params![id, title],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// An [`Overview`](crate::overview::Overview) of every meeting, for the
    /// list of meetings: two queries (speaker turns without their text, and
    /// each meeting's latest summary), however many meetings there are.
    pub fn overviews(&self) -> Result<Vec<crate::overview::Overview>, LibraryError> {
        use std::collections::HashMap;
        let conn = self.conn();
        let mut turns: HashMap<String, Vec<crate::overview::Turn>> = HashMap::new();
        let mut stmt = conn.prepare(
            "SELECT meeting_id, start_ms, end_ms, speaker FROM segments
             WHERE TRIM(text) != '' ORDER BY meeting_id, idx",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                (
                    from_i64(r.get(1)?),
                    from_i64(r.get(2)?),
                    r.get::<_, Option<String>>(3)?,
                ),
            ))
        })?;
        for row in rows {
            let (id, turn) = row?;
            turns.entry(id).or_default().push(turn);
        }
        let mut stmt = conn.prepare(
            "SELECT meeting_id, markdown FROM summaries s
             WHERE id = (SELECT MAX(id) FROM summaries WHERE meeting_id = s.meeting_id)",
        )?;
        let summaries: HashMap<String, String> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        let mut stmt = conn.prepare("SELECT id FROM meetings")?;
        let ids: Vec<String> = stmt
            .query_map([], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        Ok(ids
            .iter()
            .map(|id| {
                crate::overview::overview(
                    id,
                    turns.get(id).map_or(&[][..], Vec::as_slice),
                    summaries.get(id).map(String::as_str),
                )
            })
            .collect())
    }

    /// All meetings, newest first.
    pub fn list(&self) -> Result<Vec<Meeting>, LibraryError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "{MEETING_SELECT} ORDER BY m.started_at DESC, m.id DESC"
        ))?;
        let rows = stmt.query_map([], meeting_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn get(&self, id: &str) -> Result<MeetingDetail, LibraryError> {
        let conn = self.conn();
        let meeting = conn
            .query_row(
                &format!("{MEETING_SELECT} WHERE m.id = ?1"),
                [id],
                meeting_from_row,
            )
            .optional()?
            .ok_or_else(|| LibraryError::NotFound(id.to_string()))?;
        let segments = conn
            .prepare("SELECT start_ms, end_ms, speaker, text FROM segments WHERE meeting_id = ?1 ORDER BY idx")?
            .query_map([id], |r| {
                Ok(Segment {
                    start_ms: from_i64(r.get(0)?),
                    end_ms: from_i64(r.get(1)?),
                    speaker: r.get(2)?,
                    text: r.get(3)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        let summaries = conn
            .prepare(
                "SELECT markdown, model, prompt_version, prompt_tokens, completion_tokens, cost_usd,
                        latency_ms, created_at
                 FROM summaries WHERE meeting_id = ?1 ORDER BY id DESC",
            )?
            .query_map([id], |r| {
                Ok(StoredSummary {
                    markdown: r.get(0)?,
                    model: r.get(1)?,
                    prompt_version: r.get(2)?,
                    prompt_tokens: from_i64(r.get(3)?),
                    completion_tokens: from_i64(r.get(4)?),
                    cost_usd: r.get(5)?,
                    latency_ms: from_i64(r.get(6)?),
                    created_at: r.get(7)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(MeetingDetail {
            meeting,
            segments,
            summaries,
        })
    }

    /// Deletes a meeting, its transcript, summaries and audio files.
    pub fn delete(&self, id: &str) -> Result<(), LibraryError> {
        {
            let mut conn = self.conn();
            let tx = conn.transaction()?;
            tx.execute("DELETE FROM segments_fts WHERE meeting_id = ?1", [id])?;
            // Segments and summaries go via ON DELETE CASCADE.
            if tx.execute("DELETE FROM meetings WHERE id = ?1", [id])? == 0 {
                return Err(LibraryError::NotFound(id.to_string()));
            }
            tx.commit()?;
        }
        let dir = self.meeting_dir(id);
        if dir.exists() {
            std::fs::remove_dir_all(dir)?;
        }
        Ok(())
    }

    /// Saves a dictation (`id` and `created_at` are filled in) and trims the
    /// history to [`DICTATION_HISTORY_LIMIT`].
    pub fn add_dictation(&self, entry: &DictationEntry) -> Result<DictationEntry, LibraryError> {
        let created_at = jiff::Timestamp::now().to_string();
        let raw = entry.raw.as_deref().filter(|raw| *raw != entry.text);
        let conn = self.conn();
        conn.execute(
            "INSERT INTO dictations (created_at, text, raw, polish_model, audio_ms, latency_ms, pasted, app, app_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                created_at,
                entry.text,
                raw,
                entry.polish_model,
                i64::try_from(entry.audio_ms).unwrap_or(i64::MAX),
                i64::try_from(entry.latency_ms).unwrap_or(i64::MAX),
                entry.pasted,
                entry.app,
                entry.app_id,
            ],
        )?;
        let id = conn.last_insert_rowid();
        conn.execute(
            "DELETE FROM dictations WHERE id <= ?1",
            [id - DICTATION_HISTORY_LIMIT],
        )?;
        Ok(DictationEntry {
            id,
            created_at,
            raw: raw.map(String::from),
            ..entry.clone()
        })
    }

    /// Most recent dictations first: up to `limit` of them, older than the
    /// entry `before` if given (for loading the history a page at a time).
    pub fn dictations(
        &self,
        before: Option<i64>,
        limit: usize,
    ) -> Result<Vec<DictationEntry>, LibraryError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {DICTATION_COLUMNS} FROM dictations WHERE id < ?1 ORDER BY id DESC LIMIT ?2"
        ))?;
        let rows = stmt.query_map(
            params![
                before.unwrap_or(i64::MAX),
                i64::try_from(limit).unwrap_or(i64::MAX)
            ],
            dictation_row,
        )?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// One dictation by id.
    pub fn dictation(&self, id: i64) -> Result<DictationEntry, LibraryError> {
        let conn = self.conn();
        conn.query_row(
            &format!("SELECT {DICTATION_COLUMNS} FROM dictations WHERE id = ?1"),
            [id],
            dictation_row,
        )
        .optional()?
        .ok_or_else(|| LibraryError::NotFound(id.to_string()))
    }

    /// Saves the user's fixed version of a dictation and returns the entry.
    pub fn correct_dictation(
        &self,
        id: i64,
        corrected: &str,
    ) -> Result<DictationEntry, LibraryError> {
        let conn = self.conn();
        let changed = conn.execute(
            "UPDATE dictations SET corrected = ?2 WHERE id = ?1",
            params![id, corrected],
        )?;
        if changed == 0 {
            return Err(LibraryError::NotFound(id.to_string()));
        }
        drop(conn);
        self.dictation(id)
    }

    /// Records rules suggested by a correction. One already known counts
    /// as seen again; its status (accepted, dismissed) is kept.
    pub fn add_suggestions(
        &self,
        rules: &[Learned],
        app: Option<&str>,
    ) -> Result<(), LibraryError> {
        let now = jiff::Timestamp::now().to_string();
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for rule in rules {
            tx.execute(
                "INSERT INTO suggestions (kind, from_text, to_text, app, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5)
                 ON CONFLICT (kind, from_text, to_text) DO UPDATE SET
                     seen = seen + 1, app = COALESCE(?4, app), updated_at = ?5",
                params![rule.kind.as_str(), rule.from, rule.to, app, now],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Pending and accepted suggestions, most recent first.
    pub fn suggestions(&self) -> Result<Vec<Suggestion>, LibraryError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, kind, from_text, to_text, app, seen, status FROM suggestions
             WHERE status != 'dismissed' ORDER BY updated_at DESC, id DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, String>(6)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, kind, from, to, app, seen, status) = row?;
            let Some(kind) = Kind::parse(&kind) else {
                continue;
            };
            out.push(Suggestion {
                id,
                kind,
                from,
                to,
                app,
                seen: u32::try_from(seen).unwrap_or(u32::MAX),
                status: SuggestionStatus::parse(&status),
            });
        }
        Ok(out)
    }

    pub fn set_suggestion_status(
        &self,
        id: i64,
        status: SuggestionStatus,
    ) -> Result<(), LibraryError> {
        let changed = self.conn().execute(
            "UPDATE suggestions SET status = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, status.as_str(), jiff::Timestamp::now().to_string()],
        )?;
        if changed == 0 {
            return Err(LibraryError::NotFound(id.to_string()));
        }
        Ok(())
    }

    pub fn delete_dictation(&self, id: i64) -> Result<(), LibraryError> {
        self.conn()
            .execute("DELETE FROM dictations WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn clear_dictations(&self) -> Result<(), LibraryError> {
        self.conn().execute("DELETE FROM dictations", [])?;
        Ok(())
    }

    /// Records a dictation's numbers (not its text) for insights.
    pub fn record_dictation_stats(&self, stat: &DictationStat) -> Result<(), LibraryError> {
        self.conn().execute(
            "INSERT INTO dictation_stats
                 (created_at, words, audio_ms, latency_ms, polished, pasted, app_id, app)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                stat.created_at,
                i64::try_from(stat.words).unwrap_or(i64::MAX),
                i64::try_from(stat.audio_ms).unwrap_or(i64::MAX),
                i64::try_from(stat.latency_ms).unwrap_or(i64::MAX),
                stat.polished,
                stat.pasted,
                stat.app_id,
                stat.app,
            ],
        )?;
        Ok(())
    }

    /// Apps dictated into, most words first. The name is the latest one
    /// seen for the app.
    pub fn dictation_apps(&self) -> Result<Vec<AppUsage>, LibraryError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT app_id,
                    (SELECT app FROM dictation_stats l
                     WHERE l.app_id = s.app_id ORDER BY id DESC LIMIT 1),
                    COUNT(*), SUM(words)
             FROM dictation_stats s WHERE app_id IS NOT NULL
             GROUP BY app_id ORDER BY SUM(words) DESC, COUNT(*) DESC",
        )?;
        let to_u64 = |v: i64| u64::try_from(v).unwrap_or(0);
        let rows = stmt.query_map([], |r| {
            let app_id: String = r.get(0)?;
            let name: Option<String> = r.get(1)?;
            Ok(AppUsage {
                name: name.unwrap_or_else(|| app_id.clone()),
                app_id,
                dictations: to_u64(r.get(2)?),
                words: to_u64(r.get(3)?),
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn dictation_stats(&self) -> Result<Vec<DictationStat>, LibraryError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT created_at, words, audio_ms, latency_ms, polished, pasted, app_id, app
             FROM dictation_stats ORDER BY id",
        )?;
        let to_u64 = |v: i64| u64::try_from(v).unwrap_or(0);
        let rows = stmt.query_map([], |r| {
            Ok(DictationStat {
                created_at: r.get(0)?,
                words: to_u64(r.get(1)?),
                audio_ms: to_u64(r.get(2)?),
                latency_ms: to_u64(r.get(3)?),
                polished: r.get(4)?,
                pasted: r.get(5)?,
                app_id: r.get(6)?,
                app: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn clear_dictation_stats(&self) -> Result<(), LibraryError> {
        self.conn().execute("DELETE FROM dictation_stats", [])?;
        Ok(())
    }

    /// Approximate words spoken per speaker label across all transcripts.
    pub fn words_by_speaker(&self) -> Result<Vec<(Option<String>, u64)>, LibraryError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT speaker,
                    SUM(LENGTH(TRIM(text)) - LENGTH(REPLACE(TRIM(text), ' ', '')) + 1)
             FROM segments WHERE TRIM(text) != '' GROUP BY speaker",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get(0)?, u64::try_from(r.get::<_, i64>(1)?).unwrap_or(0)))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Summaries written and their total cost in USD (where known).
    pub fn summary_costs(&self) -> Result<(u64, f64), LibraryError> {
        Ok(self.conn().query_row(
            "SELECT COUNT(*), COALESCE(SUM(cost_usd), 0.0) FROM summaries",
            [],
            |r| Ok((u64::try_from(r.get::<_, i64>(0)?).unwrap_or(0), r.get(1)?)),
        )?)
    }

    /// Full-text search over transcripts. Every word must match; the last
    /// word also matches as a prefix, so results appear while typing.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>, LibraryError> {
        let Some(fts_query) = fts_query(query) else {
            return Ok(Vec::new());
        };
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT f.meeting_id, m.title, f.start_ms, f.speaker,
                    snippet(segments_fts, 0, '[', ']', '…', 12)
             FROM segments_fts f JOIN meetings m ON m.id = f.meeting_id
             WHERE segments_fts MATCH ?1
             ORDER BY rank LIMIT ?2",
        )?;
        let hits = stmt
            .query_map(
                params![fts_query, i64::try_from(limit).unwrap_or(i64::MAX)],
                |r| {
                    Ok(SearchHit {
                        meeting_id: r.get(0)?,
                        title: r.get(1)?,
                        start_ms: from_i64(r.get(2)?),
                        speaker: r.get(3)?,
                        snippet: r.get(4)?,
                    })
                },
            )?
            .collect::<Result<_, _>>()?;
        Ok(hits)
    }
}

const MEETING_SELECT: &str = "
    SELECT m.id, m.title, m.started_at, m.duration_ms,
           EXISTS(SELECT 1 FROM segments s WHERE s.meeting_id = m.id),
           EXISTS(SELECT 1 FROM summaries u WHERE u.meeting_id = m.id),
           m.transcribed = 1 AND m.keep_empty = 0 AND NOT EXISTS(
               SELECT 1 FROM segments s WHERE s.meeting_id = m.id AND TRIM(s.text) != '')
    FROM meetings m";

fn meeting_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Meeting> {
    Ok(Meeting {
        id: r.get(0)?,
        title: r.get(1)?,
        started_at: r.get(2)?,
        duration_ms: from_i64(r.get(3)?),
        has_transcript: r.get(4)?,
        has_summary: r.get(5)?,
        empty: r.get(6)?,
    })
}

fn exists(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM meetings WHERE id = ?1)",
        [id],
        |r| r.get(0),
    )
}

// SQLite stores integers as i64; our times and counts are u64 and never get
// near i64::MAX, so these conversions only guard against corrupt data.
fn to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn from_i64(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

/// The summary's `# Title — 2026-10-11` heading without the date. Models
/// don't always follow the template's separator, so a trailing date is
/// removed whatever precedes it: `Title (2026-10-11)`, `Title: 2026-10-11`,
/// `Title - 2026-10-11`…
fn title_from_markdown(markdown: &str) -> Option<String> {
    let heading = markdown.lines().find_map(|l| l.strip_prefix("# "))?.trim();
    let title = strip_trailing_date(heading);
    (!title.is_empty()).then(|| title.to_string())
}

fn strip_trailing_date(title: &str) -> &str {
    let t = title.trim();
    let (body, closing) = match t.strip_suffix(')') {
        Some(inner) => (inner, true),
        None => (t, false),
    };
    // An ISO date is exactly 10 characters, all ASCII (so slicing is safe).
    let Some(split) = body
        .len()
        .checked_sub(10)
        .filter(|&i| body.is_char_boundary(i))
    else {
        return t;
    };
    if body[split..].parse::<jiff::civil::Date>().is_err() {
        return t;
    }
    let rest = body[..split].trim_end();
    // "(date)" must lose its opening parenthesis; a bare date loses the
    // separator before it.
    let rest = if closing {
        match rest.strip_suffix('(') {
            Some(r) => r,
            None => return t,
        }
    } else {
        rest
    };
    rest.trim_end_matches(|c: char| {
        c.is_whitespace() || matches!(c, '—' | '–' | '-' | ':' | ',' | '|')
    })
}

/// Turns what the user typed into a safe FTS5 query: each word quoted (so
/// characters like `"` `-` `:` can't be read as query syntax), all required,
/// the last one as a prefix.
fn fts_query(input: &str) -> Option<String> {
    let words: Vec<String> = input
        .split_whitespace()
        .map(|w| format!("\"{}\"", w.replace('"', "\"\"")))
        .collect();
    let last = words.last()?;
    let mut query = words[..words.len() - 1].join(" ");
    if !query.is_empty() {
        query.push(' ');
    }
    query.push_str(last);
    query.push('*');
    Some(query)
}

/// A file name for exporting a meeting, e.g. `2026-10-11 Product sync.md`:
/// the local start date, then the title with characters that macOS or
/// Windows reject in file names replaced.
pub fn export_file_name(meeting: &Meeting) -> String {
    let date = meeting
        .started_at
        .parse::<jiff::Timestamp>()
        .map(|t| {
            t.to_zoned(jiff::tz::TimeZone::system())
                .strftime("%Y-%m-%d")
                .to_string()
        })
        .unwrap_or_default();
    let title: String = meeting
        .title
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => ' ',
            c => c,
        })
        .take(100)
        .collect();
    // A leading dot hides the file; Windows drops trailing dots and spaces.
    let title = title.trim().trim_matches('.').trim();
    let name = [date.as_str(), title]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if name.is_empty() {
        "Meeting.md".into()
    } else {
        format!("{name}.md")
    }
}

/// Renames `from` to `to`, or copies then deletes when they're on different
/// drives (where rename fails). A failed copy leaves `from` untouched.
fn move_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    if let Err(e) = copy_dir(from, to) {
        let _ = std::fs::remove_dir_all(to);
        return Err(e);
    }
    std::fs::remove_dir_all(from)
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library() -> (Library, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let lib = open(dir.path());
        (lib, dir)
    }

    fn open(dir: &Path) -> Library {
        Library::open(dir, &dir.join("recordings")).unwrap()
    }

    fn seg(start_s: u64, speaker: &str, text: &str) -> Segment {
        Segment {
            start_ms: start_s * 1000,
            end_ms: start_s * 1000 + 3000,
            text: text.into(),
            speaker: Some(speaker.into()),
        }
    }

    fn summary(markdown: &str) -> Summary {
        Summary {
            markdown: markdown.into(),
            model: "m".into(),
            prompt_version: "summary-v1",
            requests: 1,
            prompt_tokens: 100,
            completion_tokens: 20,
            cost_usd: Some(0.001),
            latency_ms: 1500,
        }
    }

    #[test]
    fn records_transcript_and_summary_and_titles_from_summary() {
        let (lib, _dir) = library();
        let m = lib.create_meeting().unwrap();
        assert!(lib.meeting_dir(&m.id).is_dir());
        assert!(m.title.starts_with("Meeting "));

        lib.save_transcript(
            &m.id,
            &[seg(0, "Me", "Hello"), seg(5, "Others", "Hi there")],
        )
        .unwrap();
        lib.save_summary(
            &m.id,
            &summary("# Weekly sync — 2026-10-11\n\n## TL;DR\n- x"),
        )
        .unwrap();

        let d = lib.get(&m.id).unwrap();
        assert_eq!(d.meeting.title, "Weekly sync");
        assert_eq!(d.meeting.duration_ms, 8000);
        assert!(d.meeting.has_transcript && d.meeting.has_summary);
        assert_eq!(d.segments[1].text, "Hi there");
        assert_eq!(d.summaries[0].prompt_version, "summary-v1");
    }

    #[test]
    fn overviews_use_the_turns_and_the_latest_summary() {
        let (lib, _dir) = library();
        let talked = lib.create_meeting().unwrap();
        let silent = lib.create_meeting().unwrap();
        lib.save_transcript(
            &talked.id,
            &[
                seg(0, "Me", "Hello"),
                seg(5, "Priya", "Hi"),
                seg(9, "Dan", " "),
            ],
        )
        .unwrap();
        lib.save_summary(&talked.id, &summary("## TL;DR\n- Old point.\n"))
            .unwrap();
        lib.save_summary(
            &talked.id,
            &summary("## TL;DR\n- New point.\n\n## Action items\n- **You** — a\n"),
        )
        .unwrap();

        let all = lib.overviews().unwrap();
        assert_eq!(all.len(), 2);
        let o = all.iter().find(|o| o.id == talked.id).unwrap();
        // Dan only said a blank, so he isn't counted.
        assert_eq!(o.people, ["Priya"]);
        assert_eq!(o.gist.as_deref(), Some("New point."));
        assert_eq!(o.action_items, 1);
        assert_eq!(o.my_share, Some(0.5));
        let o = all.iter().find(|o| o.id == silent.id).unwrap();
        assert!(o.speaker.is_empty() && o.gist.is_none());
    }

    #[test]
    fn recordings_with_nothing_said_are_flagged_until_kept() {
        let (lib, _dir) = library();
        let untranscribed = lib.create_meeting().unwrap();
        let silent = lib.create_meeting().unwrap();
        let spoken = lib.create_meeting().unwrap();
        lib.save_transcript(&silent.id, &[]).unwrap();
        lib.save_transcript(&spoken.id, &[seg(0, "Me", "Hello")])
            .unwrap();
        let empty = |id: &str| lib.get(id).unwrap().meeting.empty;
        assert!(!empty(&untranscribed.id));
        assert!(empty(&silent.id));
        assert!(!empty(&spoken.id));
        lib.keep_empty(&silent.id).unwrap();
        assert!(!empty(&silent.id));

        // A meeting transcribed before the flag existed: its transcript
        // file says it was transcribed.
        let old = lib.create_meeting().unwrap();
        std::fs::write(lib.meeting_dir(&old.id).join("transcript.json"), "[]").unwrap();
        assert!(!empty(&old.id));
        lib.mark_transcribed_from_files().unwrap();
        assert!(empty(&old.id));
        assert!(!empty(&untranscribed.id));
    }

    #[test]
    fn corrections_and_suggestions() {
        let (lib, _dir) = library();
        let entry = lib
            .add_dictation(&DictationEntry {
                id: 0,
                created_at: String::new(),
                text: "Ask open router".into(),
                raw: None,
                polish_model: None,
                audio_ms: 1000,
                latency_ms: 300,
                pasted: true,
                app: Some("Slack".into()),
                app_id: None,
                corrected: None,
            })
            .unwrap();
        let fixed = lib.correct_dictation(entry.id, "Ask OpenRouter").unwrap();
        assert_eq!(fixed.corrected.as_deref(), Some("Ask OpenRouter"));
        assert!(lib.correct_dictation(9999, "x").is_err());

        let rule = Learned {
            kind: Kind::Replacement,
            from: "open router".into(),
            to: "OpenRouter".into(),
        };
        lib.add_suggestions(std::slice::from_ref(&rule), Some("Slack"))
            .unwrap();
        lib.add_suggestions(&[rule], None).unwrap();
        let all = lib.suggestions().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!((all[0].seen, all[0].status), (2, SuggestionStatus::Pending));
        assert_eq!(all[0].app.as_deref(), Some("Slack"));

        lib.set_suggestion_status(all[0].id, SuggestionStatus::Dismissed)
            .unwrap();
        assert!(lib.suggestions().unwrap().is_empty());
    }

    #[test]
    fn renaming_a_speaker_relabels_and_can_merge() {
        let (lib, _dir) = library();
        let m = lib.create_meeting().unwrap();
        lib.save_transcript(
            &m.id,
            &[
                seg(0, "Me", "Hello"),
                seg(5, "Speaker 1", "Hi"),
                seg(9, "Speaker 2", "Hey"),
                seg(12, "Speaker 1", "Shall we start"),
            ],
        )
        .unwrap();
        assert_eq!(lib.rename_speaker(&m.id, "Speaker 1", "Priya").unwrap(), 2);
        assert_eq!(lib.rename_speaker(&m.id, "Speaker 2", "Priya").unwrap(), 1);
        let speakers: Vec<_> = lib
            .get(&m.id)
            .unwrap()
            .segments
            .into_iter()
            .map(|s| s.speaker.unwrap())
            .collect();
        assert_eq!(speakers, ["Me", "Priya", "Priya", "Priya"]);
        // Search results carry the new name too.
        assert_eq!(
            lib.search("start", 5).unwrap()[0].speaker.as_deref(),
            Some("Priya")
        );
        assert!(lib.rename_speaker("nope", "a", "b").is_err());
    }

    #[test]
    fn renamed_titles_survive_new_summaries() {
        let (lib, _dir) = library();
        let m = lib.create_meeting().unwrap();
        lib.rename(&m.id, "Board prep").unwrap();
        lib.save_summary(&m.id, &summary("# Something else — d"))
            .unwrap();
        lib.save_summary(&m.id, &summary("# Newer — d")).unwrap();

        let d = lib.get(&m.id).unwrap();
        assert_eq!(d.meeting.title, "Board prep");
        // Newest summary first.
        assert!(d.summaries[0].markdown.starts_with("# Newer"));
    }

    #[test]
    fn search_finds_words_prefixes_and_survives_odd_input() {
        let (lib, _dir) = library();
        let a = lib.create_meeting().unwrap();
        lib.save_transcript(
            &a.id,
            &[
                seg(0, "Me", "The pricing experiment converts better"),
                seg(9, "Others", "Ship on Friday"),
            ],
        )
        .unwrap();
        let b = lib.create_meeting().unwrap();
        lib.save_transcript(&b.id, &[seg(3, "Others", "Hiring plan for Q4")])
            .unwrap();

        let hits = lib.search("pricing exper", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].meeting_id, a.id);
        assert!(hits[0].snippet.contains("[pricing]"), "{}", hits[0].snippet);

        // Porter stemming: "converting" matches "converts".
        assert_eq!(lib.search("converting", 10).unwrap().len(), 1);
        assert_eq!(lib.search("hiring", 10).unwrap()[0].start_ms, 3000);
        // FTS syntax characters in user input don't cause errors.
        assert!(lib.search("\"unbalanced OR -x:", 10).unwrap().is_empty());
        assert!(lib.search("   ", 10).unwrap().is_empty());

        // Replacing a transcript replaces its index entries.
        lib.save_transcript(&b.id, &[seg(0, "Me", "Nothing relevant")])
            .unwrap();
        assert!(lib.search("hiring", 10).unwrap().is_empty());
    }

    #[test]
    fn delete_removes_rows_index_and_files() {
        let (lib, _dir) = library();
        let m = lib.create_meeting().unwrap();
        std::fs::write(lib.meeting_dir(&m.id).join("mic.wav"), b"x").unwrap();
        lib.save_transcript(&m.id, &[seg(0, "Me", "secret plans")])
            .unwrap();
        lib.save_summary(&m.id, &summary("# T — d")).unwrap();

        lib.delete(&m.id).unwrap();

        assert!(lib.list().unwrap().is_empty());
        assert!(lib.search("secret", 10).unwrap().is_empty());
        assert!(!lib.meeting_dir(&m.id).exists());
        assert!(matches!(lib.get(&m.id), Err(LibraryError::NotFound(_))));
    }

    #[test]
    fn ids_are_unique_within_a_second_and_reopening_keeps_data() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = {
            let lib = open(dir.path());
            (lib.create_meeting().unwrap(), lib.create_meeting().unwrap())
        };
        assert_ne!(a.id, b.id);

        let lib = open(dir.path());
        assert_eq!(lib.list().unwrap().len(), 2);
    }

    #[test]
    fn recordings_move_to_a_new_folder_and_stay_findable() {
        let (lib, dir) = library();
        let m = lib.create_meeting().unwrap();
        std::fs::write(lib.meeting_dir(&m.id).join("mic.wav"), b"audio").unwrap();
        let stray = lib.recordings_dir().join("notes.txt");
        std::fs::write(&stray, b"not ours").unwrap();

        let to = dir.path().join("elsewhere");
        assert_eq!(lib.move_recordings(&to).unwrap(), 1);
        assert_eq!(lib.recordings_dir(), to.canonicalize().unwrap());
        assert_eq!(
            std::fs::read(lib.meeting_dir(&m.id).join("mic.wav")).unwrap(),
            b"audio"
        );
        assert!(stray.exists(), "only meeting folders move");
        // Moving to where it already is does nothing.
        assert_eq!(lib.move_recordings(&to).unwrap(), 0);
    }

    #[test]
    fn a_move_that_would_overwrite_changes_nothing() {
        let (lib, dir) = library();
        let m = lib.create_meeting().unwrap();
        let to = dir.path().join("elsewhere");
        std::fs::create_dir_all(to.join(&m.id)).unwrap();
        let before = lib.recordings_dir();

        assert!(matches!(
            lib.move_recordings(&to),
            Err(LibraryError::Move(_))
        ));
        assert_eq!(lib.recordings_dir(), before);
        assert!(lib.meeting_dir(&m.id).exists());
        // Nor can the new folder sit inside the current one.
        assert!(lib.move_recordings(&before.join("sub")).is_err());
    }

    #[test]
    fn dictation_history_keeps_the_newest_and_raw_only_when_different() {
        let (lib, _dir) = library();
        let entry = |text: &str, raw: &str| DictationEntry {
            id: 0,
            created_at: String::new(),
            text: text.into(),
            raw: Some(raw.into()),
            polish_model: None,
            audio_ms: 3000,
            latency_ms: 300,
            pasted: true,
            app: Some("Slack".into()),
            app_id: None,
            corrected: None,
        };
        let first = lib.add_dictation(&entry("Same.", "Same.")).unwrap();
        assert_eq!(first.raw, None);
        let second = lib
            .add_dictation(&entry("Meet at 4.", "Meet at 3, no, 4."))
            .unwrap();
        assert_eq!(second.raw.as_deref(), Some("Meet at 3, no, 4."));

        let all = lib.dictations(None, 10).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0], second, "newest first, as stored");
        assert_eq!(lib.dictation(first.id).unwrap(), first);
        lib.delete_dictation(first.id).unwrap();
        assert_eq!(lib.dictations(None, 10).unwrap().len(), 1);
        lib.clear_dictations().unwrap();
        assert!(lib.dictations(None, 10).unwrap().is_empty());
    }

    #[test]
    fn dictation_history_loads_a_page_at_a_time() {
        let (lib, _dir) = library();
        let ids: Vec<i64> = (0..5)
            .map(|i| {
                lib.add_dictation(&DictationEntry {
                    id: 0,
                    created_at: String::new(),
                    text: format!("Dictation {i}."),
                    raw: None,
                    polish_model: None,
                    audio_ms: 1000,
                    latency_ms: 200,
                    pasted: true,
                    app: None,
                    app_id: None,
                    corrected: None,
                })
                .unwrap()
                .id
            })
            .collect();
        let page = |before, limit| -> Vec<i64> {
            lib.dictations(before, limit)
                .unwrap()
                .iter()
                .map(|e| e.id)
                .collect()
        };
        let first = page(None, 2);
        assert_eq!(first, vec![ids[4], ids[3]]);
        assert_eq!(page(first.last().copied(), 2), vec![ids[2], ids[1]]);
        assert_eq!(
            page(Some(ids[1]), 2),
            vec![ids[0]],
            "the last page is short"
        );
        assert!(page(Some(ids[0]), 2).is_empty());
    }

    #[test]
    fn a_dictation_keeps_the_app_id_for_its_icon() {
        let (lib, _dir) = library();
        let saved = lib
            .add_dictation(&DictationEntry {
                id: 0,
                created_at: String::new(),
                text: "Ship it.".into(),
                raw: None,
                polish_model: None,
                audio_ms: 1000,
                latency_ms: 200,
                pasted: true,
                app: Some("Slack".into()),
                app_id: Some("com.tinyspeck.slackmacgap".into()),
                corrected: None,
            })
            .unwrap();
        let read = lib.dictation(saved.id).unwrap();
        assert_eq!(read.app_id.as_deref(), Some("com.tinyspeck.slackmacgap"));
    }

    #[test]
    fn export_names_are_dated_and_safe_on_every_os() {
        let meeting = |title: &str| Meeting {
            id: "x".into(),
            title: title.into(),
            // Midday UTC is the same date in every time zone but UTC+12..14.
            started_at: "2026-10-11T12:00:00Z".into(),
            duration_ms: 0,
            has_transcript: false,
            has_summary: false,
            empty: false,
        };
        assert_eq!(
            export_file_name(&meeting("Product sync")),
            "2026-10-11 Product sync.md"
        );
        assert_eq!(
            export_file_name(&meeting("Meeting Oct 11, 7:15 PM")),
            "2026-10-11 Meeting Oct 11, 7-15 PM.md"
        );
        assert_eq!(
            export_file_name(&meeting("Q3/Q4: plan?")),
            "2026-10-11 Q3-Q4- plan-.md"
        );
        assert_eq!(export_file_name(&meeting("..")), "2026-10-11.md");
    }

    #[test]
    fn titles_drop_the_date_however_it_is_written() {
        let t = |h: &str| title_from_markdown(&format!("{h}\n\n## TL;DR"));
        assert_eq!(
            t("# Weekly sync — 2026-10-11").as_deref(),
            Some("Weekly sync")
        );
        assert_eq!(
            t("# Table leg test (2026-10-11)").as_deref(),
            Some("Table leg test")
        );
        assert_eq!(t("# Budget - 2026-10-11").as_deref(), Some("Budget"));
        assert_eq!(
            t("# Software test: 2026-10-11").as_deref(),
            Some("Software test")
        );
        assert_eq!(t("# Plain title").as_deref(), Some("Plain title"));
        // A date that is the whole title isn't removed into nothing.
        assert_eq!(t("# 2026-10-11").as_deref(), None);
        // Parentheses that aren't a date stay.
        assert_eq!(t("# Sync (EMEA)").as_deref(), Some("Sync (EMEA)"));
        assert_eq!(t("No heading"), None);
    }

    #[test]
    fn fts_query_quotes_words_and_prefixes_the_last() {
        assert_eq!(
            fts_query("ship friday").as_deref(),
            Some("\"ship\" \"friday\"*")
        );
        assert_eq!(
            fts_query("say \"hi\"").as_deref(),
            Some("\"say\" \"\"\"hi\"\"\"*")
        );
        assert_eq!(fts_query(""), None);
    }
}
