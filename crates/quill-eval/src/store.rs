//! Persisting eval runs: SQLite for history across runs, CSV for
//! spreadsheets.

use std::path::Path;

use rusqlite::{Connection, params};
use serde::Serialize;

use crate::{EvalError, RunResult};

/// Facts about the whole run (one row in `runs`).
#[derive(Debug, Clone)]
pub struct RunMeta {
    pub created_at: String,
    pub judge_model: Option<String>,
    pub judge_prompt: Option<&'static str>,
    pub judge_cost_usd: Option<f64>,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS runs (
    id             INTEGER PRIMARY KEY,
    created_at     TEXT NOT NULL,
    judge_model    TEXT,
    judge_prompt   TEXT,
    judge_cost_usd REAL
);
CREATE TABLE IF NOT EXISTS results (
    id                INTEGER PRIMARY KEY,
    run_id            INTEGER NOT NULL REFERENCES runs(id),
    case_name         TEXT NOT NULL,
    case_hours        REAL NOT NULL,
    model             TEXT NOT NULL,
    model_reported    TEXT,
    prompt_version    TEXT NOT NULL,
    summary           TEXT,
    error             TEXT,
    requests          INTEGER,
    prompt_tokens     INTEGER,
    completion_tokens INTEGER,
    cost_usd          REAL,
    latency_ms        INTEGER,
    completeness      REAL,
    accuracy          REAL,
    action_items      REAL,
    faithfulness    REAL,
    formatting        REAL,
    overall           REAL,
    judge_notes       TEXT
);
";

/// Appends a run and its results to the database at `path` (created if
/// missing). Returns the new run's id.
pub fn save_sqlite(path: &Path, meta: &RunMeta, results: &[RunResult]) -> Result<i64, EvalError> {
    let mut conn = Connection::open(path)?;
    conn.execute_batch(SCHEMA)?;
    // One transaction: either the whole run is saved or none of it.
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO runs (created_at, judge_model, judge_prompt, judge_cost_usd) VALUES (?1, ?2, ?3, ?4)",
        params![meta.created_at, meta.judge_model, meta.judge_prompt, meta.judge_cost_usd],
    )?;
    let run_id = tx.last_insert_rowid();
    {
        let mut insert = tx.prepare(
            "INSERT INTO results (run_id, case_name, case_hours, model, model_reported, prompt_version,
                summary, error, requests, prompt_tokens, completion_tokens, cost_usd, latency_ms,
                completeness, accuracy, action_items, faithfulness, formatting, overall, judge_notes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
        )?;
        for row in results.iter().map(Row::from) {
            insert.execute(params![
                run_id,
                row.case_name,
                row.case_hours,
                row.model,
                row.model_reported,
                row.prompt_version,
                row.summary,
                row.error,
                row.requests,
                row.prompt_tokens,
                row.completion_tokens,
                row.cost_usd,
                row.latency_ms,
                row.completeness,
                row.accuracy,
                row.action_items,
                row.faithfulness,
                row.formatting,
                row.overall,
                row.judge_notes,
            ])?;
        }
    }
    tx.commit()?;
    Ok(run_id)
}

/// Writes one CSV row per result (summary text included).
pub fn save_csv(path: &Path, results: &[RunResult]) -> Result<(), EvalError> {
    let mut writer = csv::Writer::from_path(path)?;
    for row in results.iter().map(Row::from) {
        writer.serialize(row)?;
    }
    writer.flush()?;
    Ok(())
}

/// A result flattened into columns, shared by SQLite and CSV.
#[derive(Debug, Serialize)]
struct Row<'a> {
    case_name: &'a str,
    case_hours: f64,
    model: &'a str,
    model_reported: Option<&'a str>,
    prompt_version: &'a str,
    requests: Option<i64>,
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
    cost_usd: Option<f64>,
    latency_ms: Option<i64>,
    completeness: Option<f64>,
    accuracy: Option<f64>,
    action_items: Option<f64>,
    faithfulness: Option<f64>,
    formatting: Option<f64>,
    overall: Option<f64>,
    judge_notes: Option<&'a str>,
    error: Option<&'a str>,
    // Last, so the CSV stays readable in a spreadsheet.
    summary: Option<&'a str>,
}

impl<'a> From<&'a RunResult> for Row<'a> {
    fn from(r: &'a RunResult) -> Self {
        let ok = r.outcome.as_ref().ok();
        let s = r.scores.as_ref();
        // SQLite integers are i64; token counts and times always fit.
        let int = |v: u64| i64::try_from(v).ok();
        Row {
            case_name: &r.case,
            case_hours: r.case_hours,
            model: &r.model,
            model_reported: ok.map(|o| o.model.as_str()),
            prompt_version: r.prompt_version,
            requests: ok.and_then(|o| int(o.requests as u64)),
            prompt_tokens: ok.and_then(|o| int(o.prompt_tokens)),
            completion_tokens: ok.and_then(|o| int(o.completion_tokens)),
            cost_usd: ok.and_then(|o| o.cost_usd),
            latency_ms: ok.and_then(|o| int(o.latency_ms)),
            completeness: s.map(|s| s.completeness),
            accuracy: s.map(|s| s.accuracy),
            action_items: s.map(|s| s.action_items),
            faithfulness: s.map(|s| s.faithfulness),
            formatting: s.map(|s| s.formatting),
            overall: s.map(|s| s.overall),
            judge_notes: s.map(|s| s.notes.as_str()),
            error: r.outcome.as_ref().err().map(String::as_str),
            summary: ok.map(|o| o.markdown.as_str()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Scores;
    use quill_core::summary::Summary;

    fn results() -> Vec<RunResult> {
        let ok = RunResult {
            case: "c".into(),
            case_hours: 0.5,
            model: "m".into(),
            prompt_version: "summary-v1",
            outcome: Ok(Summary {
                markdown: "# Notes, with \"quotes\"\nline 2".into(),
                model: "m-2026".into(),
                prompt_version: "summary-v1",
                requests: 1,
                prompt_tokens: 1000,
                completion_tokens: 200,
                cost_usd: Some(0.002),
                latency_ms: 1500,
            }),
            scores: Some(Scores {
                completeness: 8.0,
                accuracy: 9.0,
                action_items: 7.0,
                faithfulness: 10.0,
                formatting: 9.0,
                overall: 8.5,
                notes: "Good.".into(),
            }),
        };
        let failed = RunResult {
            model: "x".into(),
            outcome: Err("no API key".into()),
            scores: None,
            ..ok.clone()
        };
        vec![ok, failed]
    }

    #[test]
    fn saves_runs_to_sqlite_and_appends() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("eval.sqlite");
        let meta = RunMeta {
            created_at: "2026-10-11T19:00:00Z".into(),
            judge_model: Some("judge".into()),
            judge_prompt: Some("judge-v1"),
            judge_cost_usd: Some(0.05),
        };

        let first = save_sqlite(&db, &meta, &results()).unwrap();
        let second = save_sqlite(&db, &meta, &results()).unwrap();

        assert_eq!(second, first + 1);
        let conn = Connection::open(&db).unwrap();
        let (count, overall, error): (i64, Option<f64>, Option<String>) = conn
            .query_row(
                "SELECT COUNT(*), MAX(overall), MAX(error) FROM results WHERE run_id = ?1",
                [second],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            (count, overall, error.as_deref()),
            (2, Some(8.5), Some("no API key"))
        );
    }

    #[test]
    fn writes_csv_with_quoted_multiline_summaries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("results.csv");

        save_csv(&path, &results()).unwrap();

        let mut reader = csv::Reader::from_path(&path).unwrap();
        let headers = reader.headers().unwrap().clone();
        assert_eq!(&headers[0], "case_name");
        let rows: Vec<csv::StringRecord> = reader.records().map(Result::unwrap).collect();
        assert_eq!(rows.len(), 2);
        let summary_col = headers.iter().position(|h| h == "summary").unwrap();
        assert_eq!(&rows[0][summary_col], "# Notes, with \"quotes\"\nline 2");
    }
}
