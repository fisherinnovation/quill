//! Human-readable outputs: the leaderboard (which model is best for the
//! money) and a side-by-side review document for spot checks.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::RunResult;

/// One model + prompt version, aggregated over all cases.
#[derive(Debug, Clone, PartialEq)]
pub struct LeaderboardRow {
    pub model: String,
    pub prompt_version: &'static str,
    pub cases: usize,
    pub failures: usize,
    /// Mean judge scores over scored cases (`None` if nothing was scored).
    pub overall: Option<f64>,
    pub accuracy: Option<f64>,
    pub faithfulness: Option<f64>,
    pub action_items: Option<f64>,
    /// Total cost ÷ total meeting hours, over successful cases. `None` if any
    /// cost was unknown.
    pub cost_per_hour: Option<f64>,
    pub mean_latency_secs: Option<f64>,
}

/// Aggregates results per (model, prompt version), best overall score first;
/// ties (and unscored rows) are ordered by cost, cheapest first.
pub fn leaderboard(results: &[RunResult]) -> Vec<LeaderboardRow> {
    let mut groups: BTreeMap<(&str, &'static str), Vec<&RunResult>> = BTreeMap::new();
    for r in results {
        groups
            .entry((&r.model, r.prompt_version))
            .or_default()
            .push(r);
    }

    let mut rows: Vec<LeaderboardRow> = groups
        .into_iter()
        .map(|((model, prompt_version), rs)| {
            let ok: Vec<_> = rs
                .iter()
                .filter_map(|r| r.outcome.as_ref().ok().map(|s| (r, s)))
                .collect();
            let scored: Vec<_> = rs.iter().filter_map(|r| r.scores.as_ref()).collect();
            let mean_score = |f: fn(&crate::Scores) -> f64| mean(scored.iter().map(|s| f(s)));
            let hours: f64 = ok.iter().map(|(r, _)| r.case_hours).sum();
            let cost: Option<f64> = ok.iter().map(|(_, s)| s.cost_usd).sum();
            LeaderboardRow {
                model: model.to_string(),
                prompt_version,
                cases: rs.len(),
                failures: rs.len() - ok.len(),
                overall: mean_score(|s| s.overall),
                accuracy: mean_score(|s| s.accuracy),
                faithfulness: mean_score(|s| s.faithfulness),
                action_items: mean_score(|s| s.action_items),
                cost_per_hour: cost.filter(|_| hours > 0.0).map(|c| c / hours),
                mean_latency_secs: mean(ok.iter().map(|(_, s)| s.latency_ms as f64 / 1000.0)),
            }
        })
        .collect();

    rows.sort_by(|a, b| {
        let score = |r: &LeaderboardRow| r.overall.unwrap_or(f64::NEG_INFINITY);
        let cost = |r: &LeaderboardRow| r.cost_per_hour.unwrap_or(f64::INFINITY);
        score(b)
            .total_cmp(&score(a))
            .then(cost(a).total_cmp(&cost(b)))
    });
    rows
}

fn mean(values: impl Iterator<Item = f64>) -> Option<f64> {
    let (sum, n) = values.fold((0.0, 0_usize), |(s, n), v| (s + v, n + 1));
    (n > 0).then(|| sum / n as f64)
}

/// The leaderboard as a Markdown table.
pub fn leaderboard_markdown(rows: &[LeaderboardRow]) -> String {
    let mut out = String::from(
        "| # | Model | Prompt | Overall | Accuracy | Faithfulness | Action items | $ / meeting-hour | Avg latency | Failed |\n\
         |---:|---|---|---:|---:|---:|---:|---:|---:|---:|\n",
    );
    let score = |v: Option<f64>| v.map_or("–".into(), |v| format!("{v:.1}"));
    for (i, r) in rows.iter().enumerate() {
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {}/{} |",
            i + 1,
            r.model,
            r.prompt_version,
            score(r.overall),
            score(r.accuracy),
            score(r.faithfulness),
            score(r.action_items),
            r.cost_per_hour.map_or("?".into(), |c| format!("${c:.4}")),
            r.mean_latency_secs
                .map_or("–".into(), |s| format!("{s:.1}s")),
            r.failures,
            r.cases,
        );
    }
    out
}

/// Every summary, grouped by case, with its scores and the judge's notes,
/// for reading side by side.
pub fn review_markdown(results: &[RunResult]) -> String {
    let mut by_case: BTreeMap<&str, Vec<&RunResult>> = BTreeMap::new();
    for r in results {
        by_case.entry(&r.case).or_default().push(r);
    }
    let mut out = String::from("# Eval review\n");
    for (case, mut rs) in by_case {
        rs.sort_by(|a, b| {
            let o = |r: &RunResult| r.scores.as_ref().map_or(f64::NEG_INFINITY, |s| s.overall);
            o(b).total_cmp(&o(a))
        });
        let _ = write!(out, "\n## {case}\n");
        for r in rs {
            let _ = write!(out, "\n### {} ({})\n\n", r.model, r.prompt_version);
            match (&r.outcome, &r.scores) {
                (Err(e), _) => {
                    let _ = writeln!(out, "**Failed:** {e}");
                }
                (Ok(summary), scores) => {
                    if let Some(s) = scores {
                        let _ = writeln!(
                            out,
                            "**Overall {:.1}** · completeness {:.0} · accuracy {:.0} · action items {:.0} · \
                             faithfulness {:.0} · formatting {:.0}\n\n> {}\n",
                            s.overall,
                            s.completeness,
                            s.accuracy,
                            s.action_items,
                            s.faithfulness,
                            s.formatting,
                            s.notes
                        );
                    }
                    let cost = summary.cost_usd.map_or("?".into(), |c| format!("${c:.4}"));
                    let _ = writeln!(
                        out,
                        "_{} · {} in / {} out tokens · {cost} · {:.1}s_\n",
                        summary.model,
                        summary.prompt_tokens,
                        summary.completion_tokens,
                        summary.latency_ms as f64 / 1000.0
                    );
                    out.push_str(&demote_headings(&summary.markdown, 3));
                    out.push('\n');
                }
            }
        }
    }
    out
}

/// Adds `levels` `#`s to every Markdown heading so a summary's own `#`/`##`
/// headings nest under the review's `###` model heading.
fn demote_headings(markdown: &str, levels: usize) -> String {
    let prefix = "#".repeat(levels);
    markdown
        .lines()
        .map(|line| {
            if line.starts_with('#') {
                format!("{prefix}{line}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Scores;
    use quill_core::summary::Summary;

    fn result(case: &str, hours: f64, model: &str, overall: Option<f64>, cost: f64) -> RunResult {
        RunResult {
            case: case.into(),
            case_hours: hours,
            model: model.into(),
            prompt_version: "summary-v1",
            outcome: Ok(Summary {
                markdown: "# Title\n## TL;DR\n- x".into(),
                model: model.into(),
                prompt_version: "summary-v1",
                requests: 1,
                prompt_tokens: 10,
                completion_tokens: 5,
                cost_usd: Some(cost),
                latency_ms: 2000,
            }),
            scores: overall.map(|o| Scores {
                completeness: o,
                accuracy: o,
                action_items: o,
                faithfulness: o,
                formatting: o,
                overall: o,
                notes: "n".into(),
            }),
        }
    }

    #[test]
    fn ranks_by_quality_then_cost_and_normalizes_cost_per_hour() {
        let results = vec![
            result("a", 0.5, "cheap", Some(7.0), 0.001),
            result("b", 1.5, "cheap", Some(8.0), 0.003),
            result("a", 0.5, "pricey", Some(9.0), 0.05),
            result("b", 1.5, "pricey", Some(9.0), 0.15),
            result("a", 0.5, "free", Some(7.5), 0.0),
            RunResult {
                outcome: Err("down".into()),
                scores: None,
                ..result("b", 1.5, "free", None, 0.0)
            },
        ];

        let rows = leaderboard(&results);

        let order: Vec<_> = rows.iter().map(|r| r.model.as_str()).collect();
        assert_eq!(order, ["pricey", "free", "cheap"]);
        // $0.20 over 2 hours.
        assert!((rows[0].cost_per_hour.unwrap() - 0.10).abs() < 1e-12);
        // Only the successful 0.5 h case counts toward "free"'s hours.
        assert_eq!(rows[1].cost_per_hour, Some(0.0));
        assert_eq!((rows[1].failures, rows[1].cases), (1, 2));
        assert_eq!(rows[2].overall, Some(7.5));

        let table = leaderboard_markdown(&rows);
        assert!(
            table.contains("| 1 | pricey | summary-v1 | 9.0 |"),
            "{table}"
        );
    }

    #[test]
    fn review_nests_summary_headings_and_shows_failures() {
        let results = vec![
            result("a", 1.0, "m1", Some(8.0), 0.01),
            RunResult {
                outcome: Err("timeout".into()),
                ..result("a", 1.0, "m2", None, 0.0)
            },
        ];

        let review = review_markdown(&results);

        assert!(review.contains("\n## a\n"));
        assert!(review.contains("### m1 (summary-v1)"));
        assert!(review.contains("\n#### Title\n##### TL;DR\n"));
        assert!(review.contains("**Failed:** timeout"));
    }
}
