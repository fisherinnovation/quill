//! Search across the whole library: moments in transcripts (as on the
//! Meetings page, [`Library::hybrid_search`]), and meeting titles, each
//! meeting's latest notes, action items and dictation history.
//!
//! Everything but transcripts is a row in `search_items`, with an FTS5 index
//! over its text. The rows follow what they're from: SQLite triggers for
//! titles, action items and dictations (migration 12), [`sync_notes`] for
//! notes, and deletes cascade, so a deleted dictation or meeting leaves
//! nothing behind. Notes are cut into their points ([`note_points`]);
//! their "Action items" section is left out, as each item is its own row.
//!
//! For search by meaning, each row except titles gets an embedding, like a
//! transcript chunk: [`Library::index_search_items`] embeds the ones still
//! without one, a change of text clears it, and matches are merged with the
//! keyword ones by the same reciprocal rank fusion as transcripts.

use quill_embed::Embedder;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::library::{Library, LibraryError, SearchHit, fts_query};
use crate::semantic::{
    self, Coverage, Cutoffs, MAX_BY_MEANING, MEANING_WEIGHT, Matched, RRF_K, SNIPPET_CHARS,
    SemanticError, shorten,
};

/// What a search covers: everything, or one kind of thing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    #[default]
    All,
    /// Meeting titles and moments in transcripts.
    Meetings,
    Notes,
    Actions,
    Dictations,
}

impl Scope {
    fn covers(self, kind: Scope) -> bool {
        self == Scope::All || self == kind
    }
}

/// A row's kind in `search_items`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Title,
    Notes,
    Action,
    Dictation,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Title => "title",
            Kind::Notes => "notes",
            Kind::Action => "action",
            Kind::Dictation => "dictation",
        }
    }
}

/// Up to the limit asked for, best first, and whether there were more.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Group<T> {
    pub hits: Vec<T>,
    pub more: bool,
}

impl<T> Default for Group<T> {
    fn default() -> Self {
        Self {
            hits: Vec::new(),
            more: false,
        }
    }
}

impl<T> Group<T> {
    /// `hits` fetched with one more than `limit`, to tell if there are more.
    fn of(mut hits: Vec<T>, limit: usize) -> Self {
        let more = hits.len() > limit;
        hits.truncate(limit);
        Self { hits, more }
    }
}

/// What a search found, by kind.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Results {
    /// Meetings whose title matches.
    pub meetings: Group<MeetingHit>,
    /// Moments in transcripts.
    pub moments: Group<SearchHit>,
    pub notes: Group<NoteHit>,
    pub actions: Group<ActionHit>,
    pub dictations: Group<DictationHit>,
}

impl Results {
    /// How many results there are in all (shown, not counting "more").
    pub fn len(&self) -> usize {
        self.meetings.hits.len()
            + self.moments.hits.len()
            + self.notes.hits.len()
            + self.actions.hits.len()
            + self.dictations.hits.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A meeting found by its title.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MeetingHit {
    pub meeting_id: String,
    pub title: String,
    pub started_at: String,
    /// The title with matches wrapped in `[` `]`.
    pub snippet: String,
}

/// A section of a meeting's notes.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NoteHit {
    pub meeting_id: String,
    pub title: String,
    pub started_at: String,
    /// The section or topic, e.g. "Decisions".
    pub heading: Option<String>,
    pub snippet: String,
    pub matched: Matched,
}

/// An action item.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ActionHit {
    pub action_id: i64,
    pub meeting_id: String,
    /// Its meeting's title.
    pub title: String,
    pub owner: Option<String>,
    pub task: String,
    pub done: bool,
    pub snippet: String,
    pub matched: Matched,
}

/// A dictation from the history.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DictationHit {
    pub dictation_id: i64,
    pub created_at: String,
    /// The app it was typed into, and its identifier (for its icon).
    pub app: Option<String>,
    pub app_id: Option<String>,
    pub snippet: String,
    pub matched: Matched,
}

/// A `search_items` row that matched, before its details are read.
#[derive(Debug, Clone, PartialEq)]
struct ItemHit {
    id: i64,
    snippet: String,
    matched: Matched,
}

/// Rows embedded per call, so indexing can stop between them.
const ITEM_BATCH: usize = 16;

impl Library {
    /// Searches everything `scope` covers, by words and, with an
    /// `embedder`, by meaning too: up to `limit` results of each kind.
    pub fn search_everything(
        &self,
        query: &str,
        scope: Scope,
        embedder: Option<&dyn Embedder>,
        limit: usize,
    ) -> Result<Results, SemanticError> {
        self.search_everything_with(query, scope, embedder, limit, Cutoffs::default())
    }

    /// [`Library::search_everything`] with other cut-offs, for tuning them.
    pub fn search_everything_with(
        &self,
        query: &str,
        scope: Scope,
        embedder: Option<&dyn Embedder>,
        limit: usize,
        cutoffs: Cutoffs,
    ) -> Result<Results, SemanticError> {
        let mut results = Results::default();
        if query.trim().is_empty() || limit == 0 {
            return Ok(results);
        }
        let fetch = limit + 1;
        let kinds: Vec<Kind> = [
            (Scope::Notes, Kind::Notes),
            (Scope::Actions, Kind::Action),
            (Scope::Dictations, Kind::Dictation),
        ]
        .into_iter()
        .filter(|(s, _)| scope.covers(*s))
        .map(|(_, k)| k)
        .collect();

        // By meaning: the query embedded once, every vector in scope scored
        // once, and all of them held to the best match among them.
        let mut chunks = None;
        let mut items: Vec<(i64, Kind, f32)> = Vec::new();
        if let (Some(vector), Some(e)) = (semantic::embed_query(query, embedder)?, embedder) {
            let mut scored = if scope.covers(Scope::Meetings) {
                self.scored_chunks(&vector, e.id())?
            } else {
                Vec::new()
            };
            items = self.scored_items(&vector, e.id(), &kinds)?;
            let best = scored
                .first()
                .map(|c| c.score)
                .into_iter()
                .chain(items.first().map(|i| i.2))
                .fold(f32::MIN, f32::max);
            semantic::cut(&mut scored, |c| c.score, best, cutoffs, fetch);
            chunks = Some(scored);
            items.retain(|i| i.2 >= cutoffs.min_score && i.2 >= best - cutoffs.spread);
        }
        let similar = |kind: Kind| -> Option<Vec<i64>> {
            chunks.as_ref().map(|_| {
                let mut ids: Vec<i64> = items.iter().filter(|i| i.1 == kind).map(|i| i.0).collect();
                ids.truncate(fetch.min(MAX_BY_MEANING));
                ids
            })
        };

        if scope.covers(Scope::Meetings) {
            let hits = self.item_hits(query, Kind::Title, None, fetch)?;
            results.meetings = Group::of(self.meeting_hits(&hits)?, limit);
            let moments = self.hybrid_search_by(query, chunks.clone(), fetch)?;
            results.moments = Group::of(moments, limit);
        }
        if scope.covers(Scope::Notes) {
            let hits = self.item_hits(query, Kind::Notes, similar(Kind::Notes), fetch)?;
            results.notes = Group::of(self.note_hits(&hits)?, limit);
        }
        if scope.covers(Scope::Actions) {
            let hits = self.item_hits(query, Kind::Action, similar(Kind::Action), fetch)?;
            results.actions = Group::of(self.action_hits(&hits)?, limit);
        }
        if scope.covers(Scope::Dictations) {
            let hits = self.item_hits(query, Kind::Dictation, similar(Kind::Dictation), fetch)?;
            results.dictations = Group::of(self.dictation_hits(&hits)?, limit);
        }
        Ok(results)
    }

    /// Every row of `kinds` embedded with `model`, scored against `query` by
    /// brute-force cosine, best first.
    fn scored_items(
        &self,
        query: &[f32],
        model: &str,
        kinds: &[Kind],
    ) -> rusqlite::Result<Vec<(i64, Kind, f32)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT id, kind, vector FROM search_items WHERE model = ?1 AND vector IS NOT NULL",
        )?;
        let mut rows = stmt.query([model])?;
        let mut scored = Vec::new();
        while let Some(r) = rows.next()? {
            let kind = match r.get_ref(1)?.as_str()? {
                "notes" => Kind::Notes,
                "action" => Kind::Action,
                "dictation" => Kind::Dictation,
                _ => continue,
            };
            if !kinds.contains(&kind) {
                continue;
            }
            if let Some(v) = quill_embed::from_bytes(r.get_ref(2)?.as_blob()?) {
                scored.push((r.get(0)?, kind, quill_embed::dot(query, &v)));
            }
        }
        scored.sort_by(|a, b| b.2.total_cmp(&a.2));
        Ok(scored)
    }

    /// The `n` notes points, action items and dictations most like
    /// `query`, with their similarity, whether or not they'd count as
    /// matches: (kind, text, score). For tuning the cut-offs.
    pub fn item_scores(
        &self,
        query: &str,
        embedder: &dyn Embedder,
        n: usize,
    ) -> Result<Vec<(&'static str, String, f32)>, SemanticError> {
        let vector = embedder.embed_query(query.trim())?;
        let all = [Kind::Notes, Kind::Action, Kind::Dictation];
        let scored = self.scored_items(&vector, embedder.id(), &all)?;
        let conn = self.conn();
        let mut text = conn.prepare_cached("SELECT text FROM search_items WHERE id = ?1")?;
        let mut out = Vec::new();
        for (id, kind, score) in scored.into_iter().take(n) {
            out.push((kind.as_str(), text.query_row([id], |r| r.get(0))?, score));
        }
        Ok(out)
    }

    /// Rows of one kind matching `query`: by words (FTS5, best first), and
    /// by meaning when `similar` lists the rows found that way (best first,
    /// already held to the cut-offs), merged by reciprocal rank fusion as
    /// in [`Library::hybrid_search`].
    fn item_hits(
        &self,
        query: &str,
        kind: Kind,
        similar: Option<Vec<i64>>,
        limit: usize,
    ) -> rusqlite::Result<Vec<ItemHit>> {
        let conn = self.conn();
        let rrf = |rank: usize| 1.0 / (RRF_K + rank as f32 + 1.0);
        let mut fused: Vec<(f32, ItemHit)> = Vec::new();
        if let Some(fts) = fts_query(query) {
            let mut stmt = conn.prepare_cached(
                "SELECT i.id, snippet(search_fts, 0, '[', ']', '…', 12)
                 FROM search_fts f JOIN search_items i ON i.id = f.rowid
                 WHERE search_fts MATCH ?1 AND i.kind = ?2
                 ORDER BY rank LIMIT ?3",
            )?;
            let rows = stmt.query_map(
                params![fts, kind.as_str(), i64::try_from(limit).unwrap_or(i64::MAX)],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
            )?;
            for (rank, row) in rows.enumerate() {
                let (id, snippet) = row?;
                fused.push((
                    rrf(rank),
                    ItemHit {
                        id,
                        snippet,
                        matched: Matched::Words,
                    },
                ));
            }
        }
        if let Some(similar) = similar {
            let mut text = conn.prepare_cached("SELECT text FROM search_items WHERE id = ?1")?;
            for (rank, id) in similar.into_iter().enumerate() {
                let score = MEANING_WEIGHT * rrf(rank);
                match fused.iter_mut().find(|(_, h)| h.id == id) {
                    Some((total, hit)) => {
                        *total += score;
                        hit.matched = Matched::Both;
                    }
                    None => {
                        let whole: String = text.query_row([id], |r| r.get(0))?;
                        fused.push((
                            score,
                            ItemHit {
                                id,
                                snippet: shorten(&whole, SNIPPET_CHARS),
                                matched: Matched::Meaning,
                            },
                        ));
                    }
                }
            }
        }
        // Stable, so equal scores keep keyword order.
        fused.sort_by(|a, b| b.0.total_cmp(&a.0));
        fused.truncate(limit);
        Ok(fused.into_iter().map(|(_, hit)| hit).collect())
    }

    fn meeting_hits(&self, hits: &[ItemHit]) -> rusqlite::Result<Vec<MeetingHit>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT m.id, m.title, m.started_at
             FROM search_items i JOIN meetings m ON m.id = i.meeting_id WHERE i.id = ?1",
        )?;
        let mut out = Vec::new();
        for hit in hits {
            if let Some((meeting_id, title, started_at)) = stmt
                .query_row([hit.id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .optional()?
            {
                out.push(MeetingHit {
                    meeting_id,
                    title,
                    started_at,
                    snippet: hit.snippet.clone(),
                });
            }
        }
        Ok(out)
    }

    fn note_hits(&self, hits: &[ItemHit]) -> rusqlite::Result<Vec<NoteHit>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT m.id, m.title, m.started_at, i.heading
             FROM search_items i JOIN meetings m ON m.id = i.meeting_id WHERE i.id = ?1",
        )?;
        let mut out = Vec::new();
        for hit in hits {
            if let Some((meeting_id, title, started_at, heading)) = stmt
                .query_row([hit.id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })
                .optional()?
            {
                out.push(NoteHit {
                    meeting_id,
                    title,
                    started_at,
                    heading,
                    snippet: hit.snippet.clone(),
                    matched: hit.matched,
                });
            }
        }
        Ok(out)
    }

    fn action_hits(&self, hits: &[ItemHit]) -> rusqlite::Result<Vec<ActionHit>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT a.id, a.meeting_id, m.title, a.owner, a.task, a.done_at IS NOT NULL
             FROM search_items i
             JOIN action_items a ON a.id = i.action_id
             JOIN meetings m ON m.id = a.meeting_id
             WHERE i.id = ?1",
        )?;
        let mut out = Vec::new();
        for hit in hits {
            if let Some(found) = stmt
                .query_row([hit.id], |r| {
                    Ok(ActionHit {
                        action_id: r.get(0)?,
                        meeting_id: r.get(1)?,
                        title: r.get(2)?,
                        owner: r.get(3)?,
                        task: r.get(4)?,
                        done: r.get(5)?,
                        snippet: hit.snippet.clone(),
                        matched: hit.matched,
                    })
                })
                .optional()?
            {
                out.push(found);
            }
        }
        Ok(out)
    }

    fn dictation_hits(&self, hits: &[ItemHit]) -> rusqlite::Result<Vec<DictationHit>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT d.id, d.created_at, d.app, d.app_id
             FROM search_items i JOIN dictations d ON d.id = i.dictation_id WHERE i.id = ?1",
        )?;
        let mut out = Vec::new();
        for hit in hits {
            if let Some(found) = stmt
                .query_row([hit.id], |r| {
                    Ok(DictationHit {
                        dictation_id: r.get(0)?,
                        created_at: r.get(1)?,
                        app: r.get(2)?,
                        app_id: r.get(3)?,
                        snippet: hit.snippet.clone(),
                        matched: hit.matched,
                    })
                })
                .optional()?
            {
                out.push(found);
            }
        }
        Ok(out)
    }

    /// How many notes points, action items and dictations there are, and
    /// how many are embedded with `model` (titles are searched by words
    /// only).
    pub fn search_items_coverage(&self, model: &str) -> Result<Coverage, LibraryError> {
        Ok(self.conn().query_row(
            "SELECT COUNT(*), COALESCE(SUM(model IS ?1 AND vector IS NOT NULL), 0)
             FROM search_items WHERE kind != 'title'",
            [model],
            |r| {
                Ok(Coverage {
                    total: u64::try_from(r.get::<_, i64>(0)?).unwrap_or(0),
                    indexed: u64::try_from(r.get::<_, i64>(1)?).unwrap_or(0),
                })
            },
        )?)
    }

    /// Embeds a batch of rows not yet embedded with this model, newest
    /// first. Returns how many were stored (0 once there are none left), or
    /// `None` if `keep_going` said to stop. A row whose text changed
    /// meanwhile isn't stored, so it stays pending.
    ///
    /// The library is locked only to read and to write, not while
    /// embedding.
    pub fn index_search_items(
        &self,
        embedder: &dyn Embedder,
        keep_going: &dyn Fn() -> bool,
    ) -> Result<Option<usize>, SemanticError> {
        let model = embedder.id();
        let pending: Vec<(i64, Option<String>, String)> = {
            let conn = self.conn();
            let mut stmt = conn.prepare_cached(
                "SELECT id, heading, text FROM search_items
                 WHERE kind != 'title' AND (model IS NOT ?1 OR vector IS NULL)
                 ORDER BY id DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(
                params![model, i64::try_from(ITEM_BATCH).unwrap_or(i64::MAX)],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            rows.collect::<Result<_, _>>()?
        };
        if pending.is_empty() {
            return Ok(Some(0));
        }
        if !keep_going() {
            return Ok(None);
        }
        // A section's heading says what it's about ("Decisions: …").
        let passages: Vec<String> = pending
            .iter()
            .map(|(_, heading, text)| match heading {
                Some(h) => format!("{h}: {text}"),
                None => text.clone(),
            })
            .collect();
        let texts: Vec<&str> = passages.iter().map(String::as_str).collect();
        let vectors = embedder.embed_passages(&texts)?;

        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut stored = 0;
        {
            let mut update = tx.prepare_cached(
                "UPDATE search_items SET model = ?2, vector = ?3 WHERE id = ?1 AND text = ?4",
            )?;
            for ((id, _, text), v) in pending.iter().zip(&vectors) {
                stored += update.execute(params![id, model, quill_embed::to_bytes(v), text])?;
            }
        }
        tx.commit()?;
        // Every row in the batch was deleted or changed meanwhile: they're
        // gone or pending again, so report progress rather than "done".
        Ok(Some(stored.max(1)))
    }

    /// Reads each meeting's latest notes into `search_items`, once, after
    /// the migration that added it.
    pub(crate) fn index_notes_for_search(&self) -> Result<(), LibraryError> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let latest: Vec<(String, String)> = {
            let mut stmt = tx.prepare(
                "SELECT s.meeting_id, s.markdown FROM summaries s
                 WHERE s.id = (SELECT MAX(id) FROM summaries t WHERE t.meeting_id = s.meeting_id)",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<Result<_, _>>()?
        };
        for (meeting_id, markdown) in latest {
            sync_notes(&tx, &meeting_id, &markdown)?;
        }
        tx.commit()?;
        Ok(())
    }
}

/// Adds `n` dictations to a scratch library, all with the same embedding
/// (embedded once), to time searching a long dictation history
/// (`quill-cli search --eval … --scale-dictations`). Never for a real
/// library.
pub fn pad_dictations_for_benchmark(
    library: &Library,
    n: usize,
    embedder: &dyn Embedder,
) -> Result<(), SemanticError> {
    const TEXT: &str = "Thanks, I'll send the updated figures over this afternoon.";
    let vector = embedder.embed_passages(&[TEXT])?.remove(0);
    let mut conn = library.conn();
    let tx = conn.transaction()?;
    {
        let mut insert = tx.prepare(
            "INSERT INTO dictations (created_at, text, audio_ms, latency_ms, pasted)
             VALUES (?1, ?2, 3000, 300, 1)",
        )?;
        let now = jiff::Timestamp::now().to_string();
        for i in 0..n {
            insert.execute(params![now, format!("{TEXT} ({i})")])?;
        }
    }
    tx.execute(
        "UPDATE search_items SET model = ?1, vector = ?2 WHERE kind = 'dictation'",
        params![embedder.id(), quill_embed::to_bytes(&vector)],
    )?;
    tx.commit()?;
    Ok(())
}

/// Replaces a meeting's notes rows with the points of `markdown`, its
/// latest notes. A point that's still there word for word keeps its row,
/// and so its embedding.
pub(crate) fn sync_notes(
    conn: &Connection,
    meeting_id: &str,
    markdown: &str,
) -> rusqlite::Result<()> {
    let mut existing: Vec<(i64, Option<String>, String)> = {
        let mut stmt = conn.prepare_cached(
            "SELECT id, heading, text FROM search_items WHERE kind = 'notes' AND meeting_id = ?1",
        )?;
        let rows = stmt.query_map([meeting_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    let mut insert = conn.prepare_cached(
        "INSERT INTO search_items (kind, meeting_id, heading, text) VALUES ('notes', ?1, ?2, ?3)",
    )?;
    for point in note_points(markdown) {
        let same = existing
            .iter()
            .position(|(_, heading, text)| *heading == point.heading && *text == point.text);
        match same {
            Some(i) => {
                existing.swap_remove(i);
            }
            None => {
                insert.execute(params![meeting_id, point.heading, point.text])?;
            }
        }
    }
    let mut delete = conn.prepare_cached("DELETE FROM search_items WHERE id = ?1")?;
    for (id, _, _) in existing {
        delete.execute([id])?;
    }
    Ok(())
}

/// One point in a meeting's notes, searched on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotePoint {
    /// The heading it's under ("Decisions", or a topic), if any.
    pub heading: Option<String>,
    /// The line, as plain text.
    pub text: String,
}

/// Cuts notes into their points: each line (a bullet, or a paragraph such
/// as the TL;DR) under its heading. Lines, not whole sections: a section
/// holds unrelated points ("Hire an engineer", "Move the pricing redesign"),
/// and embedded together they come out close to everything and to nothing
/// (see `docs/DESIGN_NOTES.md`). The `#` title line is left out (the
/// meeting's title is searched on its own), and so is the "Action items"
/// section (each item is). Markdown marks are dropped.
pub fn note_points(markdown: &str) -> Vec<NotePoint> {
    let mut out = Vec::new();
    let mut heading: Option<String> = None;
    let mut skip = false;
    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            let level = trimmed.chars().take_while(|c| *c == '#').count();
            let title = plain(trimmed.trim_start_matches('#'));
            match level {
                1 => {
                    heading = None;
                    skip = false;
                }
                2 => {
                    skip = title.to_lowercase().starts_with("action item");
                    heading = Some(title).filter(|t| !t.is_empty());
                }
                // A topic keeps its section's skip.
                _ => heading = Some(title).filter(|t| !t.is_empty()),
            }
            continue;
        }
        if skip {
            continue;
        }
        let text = plain(trimmed);
        if !text.is_empty() {
            out.push(NotePoint {
                heading: heading.clone(),
                text,
            });
        }
    }
    out
}

/// A line of markdown as plain text: no list marker, quote mark, emphasis
/// or code marks.
fn plain(line: &str) -> String {
    let mut s = line.trim();
    for marker in ["- ", "* ", "+ ", "> "] {
        if let Some(rest) = s.strip_prefix(marker) {
            s = rest.trim_start();
            break;
        }
    }
    // "1. " or "1) ".
    let digits = s.chars().take_while(char::is_ascii_digit).count();
    if digits > 0
        && let Some(rest) = s[digits..]
            .strip_prefix(". ")
            .or_else(|| s[digits..].strip_prefix(") "))
    {
        s = rest.trim_start();
    }
    let cleaned = s.replace("**", "").replace("__", "").replace('`', "");
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::DictationEntry;
    use crate::semantic::testing::Topics;
    use crate::summary::Summary;

    fn dictation(text: &str) -> DictationEntry {
        DictationEntry {
            id: 0,
            created_at: String::new(),
            text: text.into(),
            raw: None,
            polish_model: None,
            audio_ms: 3000,
            latency_ms: 300,
            pasted: true,
            app: Some("Slack".into()),
            app_id: None,
            wait_ms: None,
            corrected: None,
        }
    }

    fn library() -> (Library, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path(), &dir.path().join("recordings")).unwrap();
        (lib, dir)
    }

    fn summary(markdown: &str) -> Summary {
        Summary {
            markdown: markdown.into(),
            model: "test".into(),
            prompt_version: "test",
            requests: 1,
            prompt_tokens: 0,
            completion_tokens: 0,
            cost_usd: None,
            latency_ms: 0,
            complete: true,
        }
    }

    const NOTES: &str = "# Hiring plan — 2026-10-20

## TL;DR
- We agreed to start **hiring two engineers**.

## Decisions
- Post the job ads on Monday.

## Action items
- **Priya** — draft the database cutover plan — by Friday

## Discussion by topic
### Pricing
- The discount moved revenue a little.
";

    #[test]
    fn notes_are_cut_into_points_without_action_items() {
        let points = note_points(NOTES);
        let headings: Vec<_> = points.iter().map(|s| s.heading.as_deref()).collect();
        assert_eq!(
            headings,
            vec![Some("TL;DR"), Some("Decisions"), Some("Pricing")]
        );
        assert_eq!(points[0].text, "We agreed to start hiring two engineers.");
        assert!(!points.iter().any(|s| s.text.contains("Priya")));
    }

    #[test]
    fn each_line_is_a_point_under_its_heading() {
        let points =
            note_points("## Decisions\n- Hire an engineer.\n\n- Move the **pricing** redesign.\n");
        assert_eq!(points.len(), 2, "{points:?}");
        assert_eq!(points[1].heading.as_deref(), Some("Decisions"));
        assert_eq!(points[1].text, "Move the pricing redesign.");
        assert_eq!(plain("1. `quill` **now**"), "quill now");
    }

    #[test]
    fn finds_titles_notes_action_items_and_dictations_by_words() {
        let (lib, _dir) = library();
        let m = lib.create_meeting().unwrap();
        lib.save_summary(&m.id, &summary(NOTES)).unwrap();
        lib.add_dictation(&dictation("Can we post the job ads today?"))
            .unwrap();

        let all = lib
            .search_everything("job ads", Scope::All, None, 5)
            .unwrap();
        assert_eq!(all.notes.hits.len(), 1, "{all:?}");
        assert_eq!(all.notes.hits[0].heading.as_deref(), Some("Decisions"));
        assert!(all.notes.hits[0].snippet.contains("[job]"));
        assert_eq!(all.dictations.hits.len(), 1);
        assert!(all.meetings.hits.is_empty());

        // The title comes from the notes' heading.
        let titles = lib
            .search_everything("hiring", Scope::All, None, 5)
            .unwrap();
        assert_eq!(titles.meetings.hits.len(), 1, "{titles:?}");
        assert_eq!(titles.meetings.hits[0].title, "Hiring plan");

        // Action items, by owner or task; the notes' section isn't found twice.
        let actions = lib.search_everything("priya", Scope::All, None, 5).unwrap();
        assert_eq!(actions.actions.hits.len(), 1, "{actions:?}");
        assert_eq!(
            actions.actions.hits[0].task,
            "draft the database cutover plan"
        );
        assert!(actions.notes.hits.is_empty());

        // A scope leaves the rest out.
        let only = lib
            .search_everything("job ads", Scope::Dictations, None, 5)
            .unwrap();
        assert!(only.notes.hits.is_empty());
        assert_eq!(only.dictations.hits.len(), 1);
    }

    #[test]
    fn rows_follow_what_theyre_from() {
        let (lib, _dir) = library();
        let m = lib.create_meeting().unwrap();
        lib.save_summary(&m.id, &summary(NOTES)).unwrap();
        let d = lib
            .add_dictation(&dictation("Pricing for the renewal"))
            .unwrap();

        // A rename is found by its new title.
        lib.rename(&m.id, "Quarterly roadmap").unwrap();
        let found = lib
            .search_everything("roadmap", Scope::Meetings, None, 5)
            .unwrap();
        assert_eq!(found.meetings.hits.len(), 1);
        assert!(
            lib.search_everything("hiring plan", Scope::Meetings, None, 5)
                .unwrap()
                .meetings
                .hits
                .is_empty()
        );

        // A correction is what's searched.
        lib.correct_dictation(d.id, "Pricing for the enterprise renewal")
            .unwrap();
        assert_eq!(
            lib.search_everything("enterprise", Scope::Dictations, None, 5)
                .unwrap()
                .dictations
                .hits
                .len(),
            1
        );

        // New notes replace the old points.
        lib.save_summary(&m.id, &summary("## TL;DR\n- Nothing about jobs.\n"))
            .unwrap();
        assert!(
            lib.search_everything("ads", Scope::Notes, None, 5)
                .unwrap()
                .notes
                .hits
                .is_empty()
        );

        // Deleting a dictation, clearing the history or deleting a meeting
        // leaves nothing behind.
        lib.delete_dictation(d.id).unwrap();
        lib.delete(&m.id).unwrap();
        let left: i64 = lib
            .conn()
            .query_row("SELECT COUNT(*) FROM search_items", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0);
        let fts: i64 = lib
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM search_fts WHERE search_fts MATCH 'pricing'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(fts, 0);
        lib.add_dictation(&dictation("Pricing again")).unwrap();
        lib.clear_dictations().unwrap();
        assert!(
            lib.search_everything("pricing", Scope::All, None, 5)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn finds_by_meaning_once_embedded() {
        let (lib, _dir) = library();
        let m = lib.create_meeting().unwrap();
        lib.save_summary(&m.id, &summary(NOTES)).unwrap();
        lib.add_dictation(&dictation("We should interview more candidates."))
            .unwrap();
        let before = lib.search_items_coverage("topics").unwrap();
        assert_eq!(before.indexed, 0);
        // Three notes points, one action item, one dictation.
        assert_eq!(before.total, 5);

        // Not embedded yet: meaning finds nothing.
        let none = lib
            .search_everything("recruiting", Scope::All, Some(&Topics), 5)
            .unwrap();
        assert!(none.is_empty(), "{none:?}");

        assert_eq!(lib.index_search_items(&Topics, &|| false).unwrap(), None);
        assert_eq!(lib.index_search_items(&Topics, &|| true).unwrap(), Some(5));
        assert_eq!(lib.index_search_items(&Topics, &|| true).unwrap(), Some(0));
        assert_eq!(lib.search_items_coverage("topics").unwrap().indexed, 5);

        // "recruiting" isn't written anywhere: found by meaning.
        let found = lib
            .search_everything("recruiting", Scope::All, Some(&Topics), 5)
            .unwrap();
        assert_eq!(found.dictations.hits.len(), 1, "{found:?}");
        assert_eq!(found.dictations.hits[0].matched, Matched::Meaning);
        assert!(
            found
                .notes
                .hits
                .iter()
                .any(|n| n.heading.as_deref() == Some("TL;DR"))
        );

        // A changed dictation is embedded again.
        let id = found.dictations.hits[0].dictation_id;
        lib.correct_dictation(id, "The discount and revenue.")
            .unwrap();
        assert_eq!(lib.search_items_coverage("topics").unwrap().indexed, 4);
    }
}
