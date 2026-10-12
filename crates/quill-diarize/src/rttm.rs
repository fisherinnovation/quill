//! Reading and writing RTTM, the standard plain-text format for diarization
//! results and references:
//!
//! ```text
//! SPEAKER <file> 1 <start s> <duration s> <NA> <NA> <speaker> <NA> <NA>
//! ```

use crate::Turn;

/// A turn with a named speaker, as in a reference file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedTurn {
    pub start_ms: u64,
    pub end_ms: u64,
    pub speaker: String,
}

/// Parses the `SPEAKER` lines of an RTTM file. Other line types and blank
/// lines are skipped.
pub fn parse(text: &str) -> Result<Vec<NamedTurn>, String> {
    let mut turns = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.first() != Some(&"SPEAKER") {
            continue;
        }
        let bad = || {
            format!(
                "line {}: expected `SPEAKER file 1 start duration … speaker …`",
                i + 1
            )
        };
        let seconds = |field: Option<&&str>| -> Result<f64, String> {
            field
                .and_then(|f| f.parse::<f64>().ok())
                .filter(|s| *s >= 0.0)
                .ok_or_else(bad)
        };
        let start = seconds(fields.get(3))?;
        let duration = seconds(fields.get(4))?;
        let speaker = fields.get(7).ok_or_else(bad)?;
        turns.push(NamedTurn {
            start_ms: ms(start),
            end_ms: ms(start + duration),
            speaker: (*speaker).to_owned(),
        });
    }
    Ok(turns)
}

/// Formats turns as RTTM, naming speakers `name(speaker)`.
pub fn format(file_id: &str, turns: &[Turn], name: impl Fn(usize) -> String) -> String {
    turns
        .iter()
        .map(|t| {
            format!(
                "SPEAKER {file_id} 1 {:.3} {:.3} <NA> <NA> {} <NA> <NA>\n",
                t.start_ms as f64 / 1000.0,
                t.duration_ms() as f64 / 1000.0,
                name(t.speaker)
            )
        })
        .collect()
}

fn ms(seconds: f64) -> u64 {
    (seconds * 1000.0).round() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let turns = [
            Turn {
                start_ms: 1500,
                end_ms: 4250,
                speaker: 0,
            },
            Turn {
                start_ms: 4000,
                end_ms: 9000,
                speaker: 1,
            },
        ];
        let text = format("meeting", &turns, |s| format!("spk{s}"));
        assert!(text.starts_with("SPEAKER meeting 1 1.500 2.750 <NA> <NA> spk0 <NA> <NA>\n"));
        let parsed = parse(&format!(";; comment\n\n{text}")).unwrap();
        assert_eq!(
            parsed,
            [
                NamedTurn {
                    start_ms: 1500,
                    end_ms: 4250,
                    speaker: "spk0".into()
                },
                NamedTurn {
                    start_ms: 4000,
                    end_ms: 9000,
                    speaker: "spk1".into()
                },
            ]
        );
    }

    #[test]
    fn rejects_malformed_lines() {
        assert!(parse("SPEAKER f 1 abc 2.0 <NA> <NA> a").is_err());
        assert!(parse("SPEAKER f 1 1.0").is_err());
    }
}
