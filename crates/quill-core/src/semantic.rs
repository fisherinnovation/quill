//! Search by meaning, alongside the keyword search in [`Library::search`].
//!
//! Each transcript is cut into [`chunks`] of a few segments (about half a
//! minute of conversation), and each chunk's embedding, from a
//! [`quill_embed::Embedder`], is stored in the library as a BLOB. A query is
//! embedded the same way and compared with every chunk (brute force: a
//! personal library is small; see `docs/DESIGN_NOTES.md`). Its matches are
//! then merged with the keyword matches by reciprocal rank fusion
//! ([`Library::hybrid_search`]), so "when did we talk about hiring" finds
//! "bringing on two engineers", while exact words still rank first.
//!
//! Indexing is driven from outside (the app's background worker, or
//! `quill-cli search --index`): [`Library::semantic_pending`] lists the
//! meetings to do, [`Library::index_meeting`] does one. Saving a transcript
//! forgets its chunks, so it's indexed again; vectors from another model
//! are ignored, so changing the model rebuilds the index.

use std::hash::{Hash, Hasher};
use std::path::Path;

use quill_embed::Embedder;
use quill_stt::Segment;
use quill_stt::models::{self, ModelInfo};
use rusqlite::{Connection, params};
use serde::Serialize;

use crate::library::{Library, LibraryError, SearchHit};

#[derive(Debug, thiserror::Error)]
pub enum SemanticError {
    #[error(transparent)]
    Library(#[from] LibraryError),
    #[error(transparent)]
    Embed(#[from] quill_embed::EmbedError),
}

impl From<rusqlite::Error> for SemanticError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Library(LibraryError::Db(e))
    }
}

/// How a search result was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Matched {
    /// It contains the words searched for.
    Words,
    /// It's about the same thing, in other words.
    Meaning,
    Both,
}

/// The model files search by meaning needs (weights and tokenizer).
pub fn required_models() -> &'static [ModelInfo] {
    models::SEARCH_MODELS
}

/// Whether every file search by meaning needs is in `models_dir`.
pub fn model_ready(models_dir: &Path) -> bool {
    required_models()
        .iter()
        .all(|m| m.is_downloaded(models_dir))
}

/// Loads the embedding model from `models_dir`, running on up to `threads`
/// cores. Takes about half a second and 0.5 GB of memory.
pub fn load_embedder(
    models_dir: &Path,
    threads: usize,
) -> Result<quill_embed::E5Embedder, SemanticError> {
    let [weights, tokenizer] = [0, 1].map(|i| required_models()[i].path_in(models_dir));
    Ok(quill_embed::E5Embedder::load(
        &weights, &tokenizer, threads,
    )?)
}

/// A chunk closes once it has this many words...
const CHUNK_WORDS: usize = 80;
/// ...and never grows past this many (unless one segment is longer).
const CHUNK_MAX_WORDS: usize = 140;
/// A pause this long between segments starts a new chunk.
const CHUNK_GAP_MS: u64 = 60_000;
/// A last chunk shorter than this joins the one before.
const CHUNK_MIN_WORDS: usize = 20;

/// A few consecutive segments, embedded together: single segments are
/// often a few words ("Sounds good."), too little to carry a meaning.
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    /// Index of the first and last segment in the transcript.
    pub first: usize,
    pub last: usize,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

/// Cuts a transcript into chunks of about [`CHUNK_WORDS`] words, starting a
/// new one at a long pause. Segments with no text are skipped.
pub fn chunks(segments: &[Segment]) -> Vec<Chunk> {
    let mut out: Vec<Chunk> = Vec::new();
    let mut current: Option<(Chunk, usize)> = None;
    for (i, s) in segments.iter().enumerate() {
        let text = s.text.trim();
        let words = text.split_whitespace().count();
        if words == 0 {
            continue;
        }
        if let Some((chunk, n)) = &current {
            let gap = s.start_ms.saturating_sub(chunk.end_ms) > CHUNK_GAP_MS;
            if gap || n + words > CHUNK_MAX_WORDS {
                out.extend(current.take().map(|(c, _)| c));
            }
        }
        match &mut current {
            Some((chunk, n)) => {
                chunk.last = i;
                chunk.end_ms = chunk.end_ms.max(s.end_ms);
                chunk.text.push(' ');
                chunk.text.push_str(text);
                *n += words;
            }
            None => {
                current = Some((
                    Chunk {
                        first: i,
                        last: i,
                        start_ms: s.start_ms,
                        end_ms: s.end_ms,
                        text: text.to_string(),
                    },
                    words,
                ));
            }
        }
        if current.as_ref().is_some_and(|(_, n)| *n >= CHUNK_WORDS) {
            out.extend(current.take().map(|(c, _)| c));
        }
    }
    if let Some((tail, n)) = current {
        match out.last_mut() {
            Some(prev)
                if n < CHUNK_MIN_WORDS
                    && tail.start_ms.saturating_sub(prev.end_ms) <= CHUNK_GAP_MS =>
            {
                prev.last = tail.last;
                prev.end_ms = prev.end_ms.max(tail.end_ms);
                prev.text.push(' ');
                prev.text.push_str(&tail.text);
            }
            _ => out.push(tail),
        }
    }
    out
}

/// Forgets a meeting's chunks, in the caller's transaction (when its
/// transcript is replaced).
pub(crate) fn forget(conn: &Connection, meeting_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM semantic_chunks WHERE meeting_id = ?1",
        [meeting_id],
    )?;
    conn.execute(
        "DELETE FROM semantic_index WHERE meeting_id = ?1",
        [meeting_id],
    )?;
    Ok(())
}

/// A meeting's segments, in order, with a fingerprint of their text to
/// tell whether they changed meanwhile.
fn read_segments(conn: &Connection, meeting_id: &str) -> rusqlite::Result<(Vec<Segment>, u64)> {
    let mut stmt = conn.prepare(
        "SELECT start_ms, end_ms, speaker, text FROM segments WHERE meeting_id = ?1 ORDER BY idx",
    )?;
    let segments: Vec<Segment> = stmt
        .query_map([meeting_id], |r| {
            Ok(Segment {
                start_ms: u64::try_from(r.get::<_, i64>(0)?).unwrap_or(0),
                end_ms: u64::try_from(r.get::<_, i64>(1)?).unwrap_or(0),
                speaker: r.get(2)?,
                text: r.get(3)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for s in &segments {
        (s.start_ms, &s.text).hash(&mut hasher);
    }
    Ok((segments, hasher.finish()))
}

/// Chunks embedded per call, so indexing can stop between them.
const INDEX_BATCH: usize = 16;

/// Indexing progress: meetings with a transcript, and how many of them are
/// indexed with the current model.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Coverage {
    pub indexed: u64,
    pub total: u64,
}

/// One chunk found by meaning.
#[derive(Debug, Clone, PartialEq)]
struct ChunkHit {
    meeting_id: String,
    first: usize,
    last: usize,
    start_ms: u64,
    end_ms: u64,
    score: f32,
}

/// Which chunks count as found by meaning. E5's similarities are bunched
/// together (unrelated text scores about 0.75–0.8, close matches 0.83–0.87),
/// so both cut-offs matter; see `docs/DESIGN_NOTES.md` for how they were
/// chosen (`quill-cli search --eval`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cutoffs {
    /// At least this similar to the query (cosine)...
    pub min_score: f32,
    /// ...and within this much of the best match, so a query with one clear
    /// answer doesn't trail a list of loosely related chunks.
    pub spread: f32,
}

impl Default for Cutoffs {
    fn default() -> Self {
        Self {
            min_score: 0.80,
            spread: 0.04,
        }
    }
}

/// Reciprocal rank fusion's constant: a result's score is the sum of
/// weight / (K + rank) over the lists it's in. 60 is the usual choice;
/// larger flattens the difference between first and tenth.
const RRF_K: f32 = 60.0;
/// How much a match by meaning counts against a match by words. At half,
/// a chunk found only by meaning ranks after every line with the words
/// (there are at most 50), while a line found both ways goes first.
/// Exact words are the surer sign: for a query like "Thursday", the
/// closest chunks by meaning are only loosely related.
const MEANING_WEIGHT: f32 = 0.5;
/// At most this many chunks found by meaning: past the first few, they're
/// rarely what was meant.
const MAX_BY_MEANING: usize = 10;
/// Queries shorter than this (in characters) are only searched by words:
/// while typing the first letters, meaning is noise.
const MIN_QUERY_CHARS: usize = 3;

impl Library {
    /// Meetings with a transcript that aren't indexed with `model` yet,
    /// newest first.
    pub fn semantic_pending(&self, model: &str) -> Result<Vec<String>, LibraryError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT m.id FROM meetings m
             WHERE m.transcribed = 1 AND NOT EXISTS (
                 SELECT 1 FROM semantic_index i WHERE i.meeting_id = m.id AND i.model = ?1)
             ORDER BY m.started_at DESC, m.id DESC",
        )?;
        let ids = stmt
            .query_map([model], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        Ok(ids)
    }

    /// How much of the library is indexed with `model`.
    pub fn semantic_coverage(&self, model: &str) -> Result<Coverage, LibraryError> {
        Ok(self.conn().query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(EXISTS (
                        SELECT 1 FROM semantic_index i WHERE i.meeting_id = m.id AND i.model = ?1)), 0)
             FROM meetings m WHERE m.transcribed = 1",
            [model],
            |r| {
                Ok(Coverage {
                    total: u64::try_from(r.get::<_, i64>(0)?).unwrap_or(0),
                    indexed: u64::try_from(r.get::<_, i64>(1)?).unwrap_or(0),
                })
            },
        )?)
    }

    /// Embeds one meeting's transcript and stores its chunks, replacing any
    /// from before. `keep_going` is asked between batches; if it says no,
    /// nothing is stored and this returns `Ok(None)`. Otherwise returns the
    /// number of chunks (0 for an empty transcript, which still counts as
    /// indexed). If the transcript changes meanwhile, nothing is stored
    /// either, and the meeting stays pending.
    ///
    /// The library is locked only to read and to write, not while
    /// embedding, so the app stays responsive.
    pub fn index_meeting(
        &self,
        meeting_id: &str,
        embedder: &dyn Embedder,
        keep_going: &dyn Fn() -> bool,
    ) -> Result<Option<usize>, SemanticError> {
        let (segments, fingerprint) = read_segments(&self.conn(), meeting_id)?;
        let chunks = chunks(&segments);
        let mut vectors = Vec::with_capacity(chunks.len());
        for batch in chunks.chunks(INDEX_BATCH) {
            if !keep_going() {
                return Ok(None);
            }
            let texts: Vec<&str> = batch.iter().map(|c| c.text.as_str()).collect();
            vectors.extend(embedder.embed_passages(&texts)?);
        }

        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let (_, now) = read_segments(&tx, meeting_id)?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM meetings WHERE id = ?1)",
            [meeting_id],
            |r| r.get(0),
        )?;
        if now != fingerprint || !exists {
            return Ok(None);
        }
        forget(&tx, meeting_id)?;
        let model = embedder.id();
        {
            let mut insert = tx.prepare(
                "INSERT INTO semantic_chunks
                     (meeting_id, model, first_idx, last_idx, start_ms, end_ms, vector)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for (c, v) in chunks.iter().zip(&vectors) {
                insert.execute(params![
                    meeting_id,
                    model,
                    i64::try_from(c.first).unwrap_or(i64::MAX),
                    i64::try_from(c.last).unwrap_or(i64::MAX),
                    i64::try_from(c.start_ms).unwrap_or(i64::MAX),
                    i64::try_from(c.end_ms).unwrap_or(i64::MAX),
                    quill_embed::to_bytes(v),
                ])?;
            }
        }
        tx.execute(
            "INSERT INTO semantic_index (meeting_id, model, chunks, indexed_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                meeting_id,
                model,
                i64::try_from(chunks.len()).unwrap_or(i64::MAX),
                jiff::Timestamp::now().to_string()
            ],
        )?;
        tx.commit()?;
        Ok(Some(chunks.len()))
    }

    /// Deletes the whole meaning index (every model's), e.g. to measure a
    /// rebuild. It's rebuilt as meetings are indexed again.
    pub fn clear_semantic_index(&self) -> Result<(), LibraryError> {
        self.conn()
            .execute_batch("DELETE FROM semantic_chunks; DELETE FROM semantic_index;")?;
        Ok(())
    }

    /// Every chunk from this model, scored against `query` by brute-force
    /// cosine, best first.
    fn scored_chunks(&self, query: &[f32], model: &str) -> rusqlite::Result<Vec<ChunkHit>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT meeting_id, first_idx, last_idx, start_ms, end_ms, vector
             FROM semantic_chunks WHERE model = ?1",
        )?;
        let mut rows = stmt.query([model])?;
        let mut hits: Vec<ChunkHit> = Vec::new();
        while let Some(r) = rows.next()? {
            let bytes = r.get_ref(5)?.as_blob()?;
            let Some(v) = quill_embed::from_bytes(bytes) else {
                continue;
            };
            hits.push(ChunkHit {
                meeting_id: r.get(0)?,
                first: usize::try_from(r.get::<_, i64>(1)?).unwrap_or(0),
                last: usize::try_from(r.get::<_, i64>(2)?).unwrap_or(0),
                start_ms: u64::try_from(r.get::<_, i64>(3)?).unwrap_or(0),
                end_ms: u64::try_from(r.get::<_, i64>(4)?).unwrap_or(0),
                score: quill_embed::dot(query, &v),
            });
        }
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        Ok(hits)
    }

    /// The chunks most like `query` that are close enough to count, best
    /// first.
    fn chunks_like(
        &self,
        query: &[f32],
        model: &str,
        limit: usize,
        cutoffs: Cutoffs,
    ) -> rusqlite::Result<Vec<ChunkHit>> {
        let mut hits = self.scored_chunks(query, model)?;
        let best = hits.first().map_or(0.0, |h| h.score);
        hits.retain(|h| h.score >= cutoffs.min_score && h.score >= best - cutoffs.spread);
        hits.truncate(limit.min(MAX_BY_MEANING));
        Ok(hits)
    }

    /// The `n` chunks most like `query` with their similarity, whether or
    /// not they'd count as matches: (meeting id, start ms, score). For
    /// tuning (`quill-cli search --scores`).
    pub fn meaning_scores(
        &self,
        query: &str,
        embedder: &dyn Embedder,
        n: usize,
    ) -> Result<Vec<(String, u64, f32)>, SemanticError> {
        let vector = embedder.embed_query(query.trim())?;
        let mut hits = self.scored_chunks(&vector, embedder.id())?;
        hits.truncate(n);
        Ok(hits
            .into_iter()
            .map(|h| (h.meeting_id, h.start_ms, h.score))
            .collect())
    }

    /// Search by words and, with an `embedder`, by meaning too, merged into
    /// one list: up to `limit` moments, best first. Without an embedder (or
    /// for a query of a letter or two) it's [`Library::search`].
    ///
    /// Keyword matches stay one per matching line, as before. A chunk found
    /// by meaning that holds keyword matches lifts the best of them
    /// ([`Matched::Both`]); one that holds none becomes its own result
    /// ([`Matched::Meaning`]), at the line in it that shares most words with
    /// the query, or its first.
    pub fn hybrid_search(
        &self,
        query: &str,
        embedder: Option<&dyn Embedder>,
        limit: usize,
    ) -> Result<Vec<SearchHit>, SemanticError> {
        self.hybrid_search_with(query, embedder, limit, Cutoffs::default())
    }

    /// [`Library::hybrid_search`] with other cut-offs, for tuning them.
    pub fn hybrid_search_with(
        &self,
        query: &str,
        embedder: Option<&dyn Embedder>,
        limit: usize,
        cutoffs: Cutoffs,
    ) -> Result<Vec<SearchHit>, SemanticError> {
        let keyword = self.search(query, limit)?;
        let Some(embedder) = embedder.filter(|_| query.trim().chars().count() >= MIN_QUERY_CHARS)
        else {
            return Ok(keyword);
        };
        let vector = embedder.embed_query(query.trim())?;
        let similar = self.chunks_like(&vector, embedder.id(), limit, cutoffs)?;

        let rrf = |rank: usize| 1.0 / (RRF_K + rank as f32 + 1.0);
        let mut fused: Vec<(f32, SearchHit)> = keyword
            .into_iter()
            .enumerate()
            .map(|(rank, hit)| (rrf(rank), hit))
            .collect();
        let rrf = |rank: usize| MEANING_WEIGHT * rrf(rank);
        let mut extra = Vec::new();
        for (rank, chunk) in similar.iter().enumerate() {
            let inside = fused.iter_mut().find(|(_, h)| {
                h.meeting_id == chunk.meeting_id
                    && (chunk.start_ms..=chunk.end_ms).contains(&h.start_ms)
            });
            match inside {
                Some((score, hit)) => {
                    *score += rrf(rank);
                    hit.matched = Matched::Both;
                }
                None => extra.push((rrf(rank), chunk)),
            }
        }
        for (score, chunk) in extra {
            if let Some(hit) = self.meaning_hit(chunk, query)? {
                fused.push((score, hit));
            }
        }
        // Stable, so equal scores keep keyword order.
        fused.sort_by(|a, b| b.0.total_cmp(&a.0));
        fused.truncate(limit);
        Ok(fused.into_iter().map(|(_, hit)| hit).collect())
    }

    /// A result for a chunk found by meaning: its meeting, and the line in
    /// it most like the query by shared words (or its first), with that
    /// line and the next as the snippet.
    fn meaning_hit(&self, chunk: &ChunkHit, query: &str) -> rusqlite::Result<Option<SearchHit>> {
        let conn = self.conn();
        let title: Option<String> = conn
            .query_row(
                "SELECT title FROM meetings WHERE id = ?1",
                [&chunk.meeting_id],
                |r| r.get(0),
            )
            .ok();
        let Some(title) = title else {
            return Ok(None);
        };
        let mut stmt = conn.prepare_cached(
            "SELECT start_ms, speaker, text FROM segments
             WHERE meeting_id = ?1 AND idx BETWEEN ?2 AND ?3 ORDER BY idx",
        )?;
        let lines: Vec<(u64, Option<String>, String)> = stmt
            .query_map(
                params![
                    chunk.meeting_id,
                    i64::try_from(chunk.first).unwrap_or(0),
                    i64::try_from(chunk.last).unwrap_or(0)
                ],
                |r| {
                    Ok((
                        u64::try_from(r.get::<_, i64>(0)?).unwrap_or(0),
                        r.get(1)?,
                        r.get(2)?,
                    ))
                },
            )?
            .collect::<Result<_, _>>()?;
        let lines: Vec<_> = lines
            .into_iter()
            .filter(|(_, _, t)| !t.trim().is_empty())
            .collect();
        if lines.is_empty() {
            return Ok(None);
        }
        let query_words = word_set(query);
        let best = lines
            .iter()
            .enumerate()
            .map(|(i, (_, _, text))| (i, shared_words(&query_words, &word_set(text))))
            // The first of equals.
            .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
            .map_or(0, |(i, overlap)| if overlap > 0 { i } else { 0 });
        let (start_ms, speaker, _) = &lines[best];
        let mut snippet = String::new();
        if best > 0 {
            snippet.push('…');
        }
        for (_, _, text) in &lines[best..] {
            if !snippet.is_empty() && !snippet.ends_with('…') {
                snippet.push(' ');
            }
            snippet.push_str(text.trim());
            if snippet.chars().count() >= SNIPPET_CHARS {
                break;
            }
        }
        Ok(Some(SearchHit {
            meeting_id: chunk.meeting_id.clone(),
            title,
            start_ms: *start_ms,
            speaker: speaker.clone(),
            snippet: shorten(&snippet, SNIPPET_CHARS),
            matched: Matched::Meaning,
        }))
    }
}

/// About as long as a keyword snippet (12 words either side).
const SNIPPET_CHARS: usize = 160;

/// `text` cut to about `max` characters at a word boundary, with "…".
fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    let cut = cut.rsplit_once(' ').map_or(cut.as_str(), |(head, _)| head);
    format!("{}…", cut.trim_end_matches([',', '.', ';', ':']))
}

/// How many of `query`'s words are in `line`, counting a word that starts
/// with another as the same ("crash" and "crashes", "phone" and "phones"):
/// a crude stemmer, but one that works in most languages.
fn shared_words(
    query: &std::collections::HashSet<String>,
    line: &std::collections::HashSet<String>,
) -> usize {
    query
        .iter()
        .filter(|q| {
            line.iter()
                .any(|w| w.starts_with(q.as_str()) || q.starts_with(w.as_str()))
        })
        .count()
}

/// Lowercase words of four letters or more that say something, for
/// "shares words with": "when did we talk about hiring" is about "hiring".
fn word_set(text: &str) -> std::collections::HashSet<String> {
    /// Common English words that would otherwise pick the wrong line.
    const FILLER: &[&str] = &[
        "about", "after", "again", "also", "been", "before", "could", "does", "doing", "from",
        "have", "just", "like", "said", "should", "some", "talk", "talked", "than", "that",
        "their", "them", "then", "there", "they", "this", "were", "what", "when", "where", "which",
        "while", "will", "with", "would", "your",
    ];
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 4)
        .map(str::to_lowercase)
        .filter(|w| !FILLER.contains(&w.as_str()))
        .collect()
}

/// Search quality on a set of labelled queries (`quill-cli search --eval`):
/// for each query, the rank (1-based) of the first result inside one of
/// its expected stretches (or up to [`EVAL_LEAD_MS`] before one), by words
/// only and by both. `None`: not found in the first `limit`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QueryOutcome {
    pub query: String,
    pub words_rank: Option<usize>,
    pub hybrid_rank: Option<usize>,
    /// Results found only by meaning (for a query with no answer, all of
    /// them are noise).
    pub meaning_only: usize,
    pub hybrid_ms: f64,
}

/// A result this much before the expected stretch still counts: the
/// meeting opens there with the next lines in view.
pub const EVAL_LEAD_MS: u64 = 15_000;

/// One labelled query: what to search for and where the answer is, as
/// (meeting id, start ms, end ms) stretches.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct LabelledQuery {
    pub query: String,
    pub expect: Vec<(String, u64, u64)>,
}

/// Runs labelled queries by words alone and by words and meaning.
pub fn evaluate(
    library: &Library,
    embedder: &dyn Embedder,
    queries: &[LabelledQuery],
    limit: usize,
    cutoffs: Cutoffs,
) -> Result<Vec<QueryOutcome>, SemanticError> {
    let rank_in = |hits: &[SearchHit], q: &LabelledQuery| {
        hits.iter()
            .position(|h| {
                q.expect.iter().any(|(id, from, to)| {
                    h.meeting_id == *id
                        && (from.saturating_sub(EVAL_LEAD_MS)..=*to).contains(&h.start_ms)
                })
            })
            .map(|i| i + 1)
    };
    let mut out = Vec::new();
    for q in queries {
        let words = library.search(&q.query, limit)?;
        let started = std::time::Instant::now();
        let hybrid = library.hybrid_search_with(&q.query, Some(embedder), limit, cutoffs)?;
        out.push(QueryOutcome {
            query: q.query.clone(),
            words_rank: rank_in(&words, q),
            hybrid_rank: rank_in(&hybrid, q),
            meaning_only: hybrid
                .iter()
                .filter(|h| h.matched == Matched::Meaning)
                .count(),
            hybrid_ms: started.elapsed().as_secs_f64() * 1000.0,
        });
    }
    Ok(out)
}

/// Cores for the embedding model: half of them, so indexing in the
/// background leaves room for everything else, and at most 4: more were
/// slower on a 32-thread PC (7.3 s per hour of transcript with 4, 10.3 s
/// with 16), as the model is too small to keep them busy.
pub fn default_threads() -> usize {
    std::thread::available_parallelism().map_or(2, |n| (n.get() / 2).clamp(1, 4))
}

/// For timing searches over a bigger library than the one at hand
/// (`quill-cli search --eval --scale-hours`): adds copies of the indexed
/// meetings (meeting, transcript, keyword and meaning index) until the
/// library holds about `hours` of transcript. Use on a scratch library.
pub fn pad_for_benchmark(library: &Library, hours: f64) -> Result<(), LibraryError> {
    let mut conn = library.conn();
    let tx = conn.transaction()?;
    let sources: Vec<(String, i64)> = {
        let mut stmt = tx.prepare(
            "SELECT m.id, (SELECT COALESCE(MAX(end_ms), 0) FROM segments s WHERE s.meeting_id = m.id)
             FROM meetings m JOIN semantic_index i ON i.meeting_id = m.id",
        )?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?
    };
    let mut total_ms: f64 = tx.query_row(
        "SELECT COALESCE(SUM((SELECT MAX(end_ms) FROM segments s WHERE s.meeting_id = m.id)), 0)
         FROM meetings m",
        [],
        |r| r.get::<_, i64>(0),
    )? as f64;
    let target_ms = hours * 3_600_000.0;
    let mut copy = 0;
    while total_ms < target_ms && sources.iter().any(|(_, ms)| *ms > 0) {
        for (id, ms) in &sources {
            copy += 1;
            let new = format!("{id}-copy{copy}");
            tx.execute(
                "INSERT INTO meetings (id, title, started_at, duration_ms, transcribed)
                 SELECT ?2, title, started_at, duration_ms, 1 FROM meetings WHERE id = ?1",
                params![id, new],
            )?;
            for sql in [
                "INSERT INTO segments (meeting_id, idx, start_ms, end_ms, speaker, text)
                 SELECT ?2, idx, start_ms, end_ms, speaker, text FROM segments WHERE meeting_id = ?1",
                "INSERT INTO segments_fts (text, meeting_id, start_ms, speaker)
                 SELECT text, ?2, start_ms, speaker FROM segments_fts WHERE meeting_id = ?1",
                "INSERT INTO semantic_chunks (meeting_id, model, first_idx, last_idx, start_ms, end_ms, vector)
                 SELECT ?2, model, first_idx, last_idx, start_ms, end_ms, vector
                 FROM semantic_chunks WHERE meeting_id = ?1",
                "INSERT INTO semantic_index (meeting_id, model, chunks, indexed_at)
                 SELECT ?2, model, chunks, indexed_at FROM semantic_index WHERE meeting_id = ?1",
            ] {
                tx.execute(sql, params![id, new])?;
            }
            total_ms += *ms as f64;
        }
    }
    tx.commit()?;
    Ok(())
}

/// How big the meaning index is: chunks stored, and the bytes their
/// vectors take.
pub fn index_size(library: &Library) -> Result<(u64, u64), LibraryError> {
    Ok(library.conn().query_row(
        "SELECT COUNT(*), COALESCE(SUM(LENGTH(vector)), 0) FROM semantic_chunks",
        [],
        |r| {
            Ok((
                u64::try_from(r.get::<_, i64>(0)?).unwrap_or(0),
                u64::try_from(r.get::<_, i64>(1)?).unwrap_or(0),
            ))
        },
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use quill_embed::EmbedError;

    fn seg(start_s: u64, text: &str) -> Segment {
        Segment {
            start_ms: start_s * 1000,
            end_ms: start_s * 1000 + 3000,
            text: text.into(),
            speaker: Some("Others".into()),
        }
    }

    fn words(n: usize) -> String {
        vec!["word"; n].join(" ")
    }

    #[test]
    fn chunks_group_segments_and_break_at_pauses() {
        let segments = vec![
            seg(0, &words(30)),
            seg(4, &words(30)),
            seg(8, &words(30)), // 90 words: closes
            seg(12, "  "),      // skipped
            seg(13, &words(10)),
            seg(200, &words(50)), // after a long pause: new chunk
            seg(204, &words(5)),  // short tail joins it
        ];
        let c = chunks(&segments);
        assert_eq!(c.len(), 3, "{c:?}");
        assert_eq!((c[0].first, c[0].last), (0, 2));
        assert_eq!((c[1].first, c[1].last), (4, 4));
        assert_eq!((c[2].first, c[2].last), (5, 6));
        assert_eq!(c[2].start_ms, 200_000);
        assert_eq!(c[2].end_ms, 207_000);
        // No chunk grows past the maximum by adding a segment.
        let long = vec![seg(0, &words(70)), seg(3, &words(100))];
        assert_eq!(chunks(&long).len(), 2);
        assert!(chunks(&[]).is_empty());
    }

    /// A stand-in embedder: a vector per topic, by keywords, so tests can
    /// check the plumbing without the real model.
    struct Topics;

    const TOPICS: [&[&str]; 3] = [
        &[
            "hiring",
            "engineers",
            "recruiting",
            "candidates",
            "bringing",
        ],
        &["database", "migration", "cutover"],
        &["pricing", "discount", "revenue"],
    ];

    fn topic_vector(text: &str) -> Vec<f32> {
        let text = text.to_lowercase();
        let mut v: Vec<f32> = TOPICS
            .iter()
            .map(|words| words.iter().filter(|w| text.contains(*w)).count() as f32)
            .collect();
        v.push(0.1); // never all zero
        quill_embed::normalize(&mut v);
        v
    }

    impl Embedder for Topics {
        fn id(&self) -> &str {
            "topics"
        }
        fn dims(&self) -> usize {
            4
        }
        fn embed_passages(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError> {
            Ok(texts.iter().map(|t| topic_vector(t)).collect())
        }
        fn embed_query(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
            Ok(topic_vector(text))
        }
    }

    fn library() -> (Library, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path(), &dir.path().join("recordings")).unwrap();
        (lib, dir)
    }

    #[test]
    fn finds_by_meaning_and_merges_with_keywords() {
        let (lib, _dir) = library();
        let a = lib.create_meeting().unwrap();
        lib.save_transcript(
            &a.id,
            &[
                seg(0, "The database migration finished on Tuesday."),
                seg(200, "We're bringing on two engineers next quarter."),
                seg(400, "The discount moved revenue a little."),
            ],
        )
        .unwrap();
        assert_eq!(lib.semantic_pending("topics").unwrap(), vec![a.id.clone()]);
        assert_eq!(
            lib.semantic_coverage("topics").unwrap(),
            Coverage {
                indexed: 0,
                total: 1
            }
        );
        assert_eq!(
            lib.index_meeting(&a.id, &Topics, &|| true).unwrap(),
            Some(3)
        );
        assert!(lib.semantic_pending("topics").unwrap().is_empty());
        // Another model hasn't indexed it.
        assert_eq!(lib.semantic_pending("other").unwrap().len(), 1);

        // "hiring" isn't in the transcript: only meaning finds it.
        assert!(lib.search("hiring", 10).unwrap().is_empty());
        let hits = lib.hybrid_search("hiring", Some(&Topics), 10).unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].start_ms, 200_000);
        assert_eq!(hits[0].matched, Matched::Meaning);
        assert!(hits[0].snippet.contains("two engineers"));

        // A word that's there is found by both, as one result.
        let hits = lib.hybrid_search("migration", Some(&Topics), 10).unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].matched, Matched::Both);
        assert!(hits[0].snippet.contains("[migration]"));

        // Too short to search by meaning; no embedder: words only.
        assert!(
            lib.hybrid_search("hi", Some(&Topics), 10)
                .unwrap()
                .is_empty()
        );
        assert!(lib.hybrid_search("hiring", None, 10).unwrap().is_empty());
    }

    #[test]
    fn new_transcripts_and_stale_reads_are_indexed_again() {
        let (lib, _dir) = library();
        let a = lib.create_meeting().unwrap();
        lib.save_transcript(&a.id, &[seg(0, "Pricing and the discount.")])
            .unwrap();
        lib.index_meeting(&a.id, &Topics, &|| true).unwrap();
        assert_eq!(index_size(&lib).unwrap().0, 1);

        // A new transcript forgets the old chunks.
        lib.save_transcript(&a.id, &[seg(0, "Recruiting candidates.")])
            .unwrap();
        assert_eq!(index_size(&lib).unwrap().0, 0);
        assert_eq!(lib.semantic_pending("topics").unwrap().len(), 1);
        assert!(
            lib.hybrid_search("discount", Some(&Topics), 10)
                .unwrap()
                .is_empty()
        );

        // Stopped part way: nothing stored, still pending.
        assert_eq!(lib.index_meeting(&a.id, &Topics, &|| false).unwrap(), None);
        assert_eq!(lib.semantic_pending("topics").unwrap().len(), 1);

        // An empty transcript counts as indexed, with no chunks.
        let b = lib.create_meeting().unwrap();
        lib.save_transcript(&b.id, &[]).unwrap();
        assert_eq!(
            lib.index_meeting(&b.id, &Topics, &|| true).unwrap(),
            Some(0)
        );

        // Deleting a meeting deletes its chunks.
        lib.index_meeting(&a.id, &Topics, &|| true).unwrap();
        lib.delete(&a.id).unwrap();
        assert_eq!(index_size(&lib).unwrap().0, 0);
        assert_eq!(
            lib.semantic_coverage("topics").unwrap(),
            Coverage {
                indexed: 1,
                total: 1
            }
        );
    }

    #[test]
    fn shorten_cuts_at_a_word() {
        assert_eq!(shorten("short", 10), "short");
        assert_eq!(shorten("one two three four", 9), "one two…");
    }
}
