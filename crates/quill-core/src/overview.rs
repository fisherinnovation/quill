//! What the list of meetings shows about each one: the shape of the
//! conversation, who was there, the summary's first point and how many
//! action items it has. Computed from what's stored, each time it's asked
//! for, from speaker turns (not their text) and the latest summary.

use serde::Serialize;

use crate::meeting::ME;

/// How many slices the conversation's shape is cut into.
pub const BINS: usize = 96;

/// One speaker turn: start and end (ms) and who spoke (`None`: the call
/// side, not told apart).
pub type Turn = (u64, u64, Option<String>);

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Overview {
    pub id: String,
    /// Who spoke most in each of [`BINS`] slices of the meeting: 0 nobody,
    /// 1 you, 2 and up the people on the call in order of first appearance
    /// (the order of [`Overview::people`]). Empty without a transcript.
    pub speaker: Vec<u8>,
    /// How much of each slice had someone speaking, 0–100.
    pub level: Vec<u8>,
    /// The people on the call, in order of first appearance (not you).
    pub people: Vec<String>,
    /// Your share of the speaking time, 0–1.
    pub my_share: Option<f64>,
    /// The summary's first TL;DR point, as plain text.
    pub gist: Option<String>,
    /// Action items in the summary.
    pub action_items: usize,
}

/// The overview of meeting `id` from its turns and latest summary.
pub fn overview(id: &str, turns: &[Turn], summary: Option<&str>) -> Overview {
    let others = |s: &Option<String>| {
        s.clone()
            .unwrap_or_else(|| crate::meeting::OTHERS.to_string())
    };
    let mut people: Vec<String> = Vec::new();
    for (_, _, speaker) in turns {
        let name = others(speaker);
        if name != ME && !people.contains(&name) {
            people.push(name);
        }
    }
    // 1 = you, 2.. = people by first appearance.
    let slot = |s: &Option<String>| -> usize {
        let name = others(s);
        if name == ME {
            1
        } else {
            people.iter().position(|p| *p == name).map_or(0, |i| i + 2)
        }
    };

    let span = turns.iter().map(|t| t.1).max().unwrap_or(0);
    let (mut speaker, mut level) = (Vec::new(), Vec::new());
    let (mut mine, mut total) = (0_u64, 0_u64);
    if span > 0 {
        let mut talk = vec![vec![0_u64; people.len() + 2]; BINS];
        let bin_ms = span as f64 / BINS as f64;
        for (start, end, who) in turns {
            let (start, end) = (*start, (*end).max(*start));
            let s = slot(who);
            total += end - start;
            if s == 1 {
                mine += end - start;
            }
            // Spread the turn over the slices it overlaps.
            let first = ((start as f64 / bin_ms) as usize).min(BINS - 1);
            let last = ((end as f64 / bin_ms) as usize).min(BINS - 1);
            for (i, bin) in talk.iter_mut().enumerate().take(last + 1).skip(first) {
                let lo = (i as f64 * bin_ms).max(start as f64);
                let hi = ((i + 1) as f64 * bin_ms).min(end as f64);
                if hi > lo {
                    bin[s] += (hi - lo) as u64;
                }
            }
        }
        for bin in &talk {
            let (top, ms) = bin
                .iter()
                .enumerate()
                .max_by_key(|(_, ms)| **ms)
                .map_or((0, 0), |(i, ms)| (i, *ms));
            let spoken: u64 = bin.iter().sum();
            speaker.push(if ms == 0 {
                0
            } else {
                u8::try_from(top).unwrap_or(u8::MAX)
            });
            // Overlapping speakers can add up to more than the slice.
            let fill = (spoken as f64 / bin_ms).min(1.0);
            level.push((fill * 100.0).round() as u8);
        }
    }

    let tldr = summary.and_then(|md| section(md, "tl;dr").or_else(|| section(md, "tldr")));
    Overview {
        id: id.to_string(),
        speaker,
        level,
        people,
        my_share: (total > 0).then(|| mine as f64 / total as f64),
        gist: tldr.and_then(first_point),
        // Parsed, so a "None" placeholder isn't counted as an item.
        action_items: summary.map_or(0, |md| crate::action_items::parse(md).len()),
    }
}

/// The body of the `## …` section whose heading starts with `title`
/// (ignoring case).
pub(crate) fn section<'a>(markdown: &'a str, title: &str) -> Option<&'a str> {
    let mut lines = markdown.split_inclusive('\n');
    let mut offset = 0;
    let mut start = None;
    for line in lines.by_ref() {
        let heading = line.strip_prefix("## ").map(str::trim);
        match (start, heading) {
            (None, Some(h)) if h.to_lowercase().starts_with(title) => {
                start = Some(offset + line.len())
            }
            (Some(s), Some(_)) => return Some(markdown[s..offset].trim()),
            _ => {}
        }
        offset += line.len();
    }
    start.map(|s| markdown[s..].trim())
}

/// A top-level list item: "- …", "* …" or "1. …".
pub(crate) fn is_item(line: &str) -> bool {
    let line = line.trim_end();
    line.starts_with("- ")
        || line.starts_with("* ")
        || line
            .split_once(". ")
            .is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// The first bullet (or the first paragraph) as plain text.
fn first_point(body: &str) -> Option<String> {
    let line = body.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .unwrap_or(line);
    let text = plain(line);
    (!text.is_empty()).then_some(text)
}

/// Markdown emphasis, code and links reduced to their text.
fn plain(md: &str) -> String {
    let mut out = String::with_capacity(md.len());
    let mut rest = md;
    while let Some(i) = rest.find('[') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after
            .split_once("](")
            .and_then(|(text, url)| url.split_once(')').map(|(_, tail)| (text, tail)))
        {
            Some((text, tail)) => {
                out.push_str(text);
                rest = tail;
            }
            None => {
                out.push('[');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.replace("**", "")
        .replace("__", "")
        .replace('`', "")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUMMARY: &str = "# Product sync — 2026-10-11

## TL;DR
- Ship the **onboarding** redesign Friday behind the `new-onboarding` flag ([plan](https://x.test)).
- Pause pricing tests.

## Decisions
- Keep the sync at 10:30.

## Action items
- **You** — send the rollout plan — Wednesday
- **Priya** — fix duplicate events
  - nested detail, not an item
- **Dan** — two pricing variants

## Discussion by topic
";

    fn turn(start: u64, end: u64, who: &str) -> Turn {
        (
            start * 1000,
            end * 1000,
            (who != "-").then(|| who.to_string()),
        )
    }

    #[test]
    fn reads_the_gist_and_counts_action_items() {
        let o = overview("m", &[], Some(SUMMARY));
        assert_eq!(
            o.gist.as_deref(),
            Some("Ship the onboarding redesign Friday behind the new-onboarding flag (plan).")
        );
        assert_eq!(o.action_items, 3);
        // An older summary with a paragraph TL;DR, and no summary at all.
        let para = "## TL;DR\nThe team agreed to ship on **Friday**.\n\n## Action items\n- [ ] a\n- [ ] b\n";
        let o = overview("m", &[], Some(para));
        assert_eq!(
            o.gist.as_deref(),
            Some("The team agreed to ship on Friday.")
        );
        assert_eq!(o.action_items, 2);
        // "None" isn't an action item.
        let none = "## TL;DR\n- Quick sync.\n\n## Action items\n- None.\n";
        assert_eq!(overview("m", &[], Some(none)).action_items, 0);
        assert_eq!(overview("m", &[], None).gist, None);
    }

    #[test]
    fn shapes_the_conversation_by_speaker() {
        // You for the first half, Priya for the second, a gap in the middle.
        let turns = [turn(0, 40, "Me"), turn(56, 96, "Priya"), turn(60, 70, "Me")];
        let o = overview("m", &turns, None);
        assert_eq!(o.people, ["Priya"]);
        assert_eq!(o.speaker.len(), BINS);
        assert_eq!(o.speaker[0], 1);
        assert_eq!(o.speaker[BINS - 1], 2);
        assert_eq!(o.speaker[48], 0, "nobody spoke at 48 s");
        assert_eq!(o.level[48], 0);
        assert_eq!(o.level[0], 100);
        // You spoke 50 of 90 seconds.
        assert!((o.my_share.unwrap() - 50.0 / 90.0).abs() < 1e-9);
    }

    #[test]
    fn untold_call_side_is_one_person_and_no_transcript_is_empty() {
        let o = overview("m", &[turn(0, 10, "-"), turn(10, 20, "Me")], None);
        assert_eq!(o.people, ["Others"]);
        assert_eq!(o.speaker[0], 2);
        let empty = overview("m", &[], None);
        assert!(empty.speaker.is_empty() && empty.people.is_empty() && empty.my_share.is_none());
    }
}
