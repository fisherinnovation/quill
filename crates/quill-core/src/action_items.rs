//! Action items: the "Action items" section of a meeting's notes as a list
//! that can be ticked off, gathered from every meeting on one page.
//!
//! The summary prompts write each item as `- **Owner** — task — due date`
//! (the due date only if one was mentioned), or a placeholder such as "None"
//! when there are none. Items are read from that Markdown when the notes are
//! saved; the notes themselves are never rewritten.

use jiff::ToSpan;
use jiff::civil::{Date, Weekday};

use crate::overview::{is_item, section};

/// One item as written in the notes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    /// "You", "Priya", "Priya and Dan"…; `None` when no one was named.
    pub owner: Option<String>,
    pub task: String,
    /// As written, e.g. "by Thursday".
    pub due: Option<String>,
}

/// The action items in a meeting's notes, in order. Placeholders ("None",
/// "No action items were assigned") aren't items.
pub fn parse(markdown: &str) -> Vec<Parsed> {
    let Some(body) = section(markdown, "action items") else {
        return Vec::new();
    };
    body.lines()
        .filter(|line| is_item(line))
        .filter_map(parse_item)
        .collect()
}

fn parse_item(line: &str) -> Option<Parsed> {
    let text = strip_marker(line.trim());
    // A task-list box, which some notes have: "[ ] …", "[x] …".
    let text = ["[ ] ", "[x] ", "[X] "]
        .iter()
        .find_map(|b| text.strip_prefix(b))
        .unwrap_or(text)
        .trim();

    let (owner, rest) = split_owner(text);
    let rest = rest.trim_start_matches(['—', '–', '-', ':', ' ']).trim();
    let (task, due) = split_due(rest);
    let task = plain(task);
    if task.is_empty() || (owner.is_none() && is_placeholder(&task)) {
        return None;
    }
    Some(Parsed {
        owner: owner.map(|o| plain(o).trim_end_matches(':').trim().to_string()),
        task,
        due: due.map(|d| plain(strip_label(d))).filter(|d| !d.is_empty()),
    })
}

/// "- x", "* x", "1. x" → "x".
fn strip_marker(line: &str) -> &str {
    if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
        return rest;
    }
    line.split_once(". ").map_or(line, |(_, rest)| rest)
}

/// "**Priya** — fix it" or "Priya: fix it" → ("Priya", "— fix it").
fn split_owner(text: &str) -> (Option<&str>, &str) {
    if let Some(rest) = text.strip_prefix("**")
        && let Some((owner, rest)) = rest.split_once("**")
    {
        return (Some(owner), rest);
    }
    if let Some((owner, rest)) = text.split_once(": ") {
        let words = owner.split_whitespace().count();
        let named = owner.chars().next().is_some_and(char::is_uppercase);
        if (1..=4).contains(&words) && named {
            return (Some(owner), rest);
        }
    }
    (None, text)
}

/// "fix the flag — by Thursday" → ("fix the flag", Some("by Thursday")). The
/// last dash-separated part is the due date only if it reads like one, so a
/// task with a dash of its own stays whole.
fn split_due(text: &str) -> (&str, Option<&str>) {
    let cut = text.rfind(" — ").or_else(|| text.rfind(" – "));
    if let Some(at) = cut {
        let sep = text[at..].chars().nth(1).map_or(0, char::len_utf8) + 2;
        let (task, due) = (&text[..at], text[at + sep..].trim());
        if looks_like_a_date(due) {
            return (task.trim(), Some(due));
        }
    }
    (text, None)
}

fn looks_like_a_date(text: &str) -> bool {
    let t = text.to_lowercase();
    const WORDS: &[&str] = &[
        "by ",
        "due",
        "before",
        "until",
        "today",
        "tomorrow",
        "tonight",
        "eod",
        "eow",
        "end of",
        "week",
        "month",
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
        "sunday",
        "jan",
        "feb",
        "mar",
        "apr",
        "may",
        "jun",
        "jul",
        "aug",
        "sep",
        "oct",
        "nov",
        "dec",
        "asap",
    ];
    WORDS.iter().any(|w| t.contains(w)) || t.chars().any(|c| c.is_ascii_digit())
}

/// "due date: Monday" → "Monday": models sometimes copy the template's
/// label ("owner — task — due date") into the text.
fn strip_label(due: &str) -> &str {
    for label in [
        "due date:",
        "due date -",
        "due date",
        "due:",
        "deadline:",
        "by date:",
    ] {
        if due.len() > label.len() && due[..label.len()].eq_ignore_ascii_case(label) {
            return due[label.len()..].trim();
        }
    }
    due
}

/// Markdown emphasis and stray punctuation off the ends.
fn plain(text: &str) -> String {
    text.replace("**", "")
        .replace("__", "")
        .trim()
        .trim_end_matches(['.', ';'])
        .trim()
        .to_string()
}

fn is_placeholder(task: &str) -> bool {
    let t = task.to_lowercase();
    [
        "none",
        "n/a",
        "nothing",
        "no action",
        "no specific",
        "no clear",
        "no explicit",
        "no follow",
    ]
    .iter()
    .any(|p| t.starts_with(p))
        || t.contains("no action items")
}

/// Whether the owner is the person recording: "You" (or "Me", in older
/// notes), alone or with others.
pub fn is_mine(owner: Option<&str>) -> bool {
    owner.is_some_and(|o| {
        o.to_lowercase()
            .split([',', '&', '/', '(', ')'])
            .flat_map(|part| part.split(" and "))
            .map(str::trim)
            .any(|who| who == "you" || who == "me")
    })
}

/// The date a due phrase means, said in a meeting on `on`: "Thursday",
/// "by end of next week", "Oct 24", "in two weeks". `None` if it isn't a
/// date (or is too vague, like "soon").
pub fn due_date(text: &str, on: Date) -> Option<Date> {
    let mut t = strip_label(text.trim())
        .to_lowercase()
        .replace(['.', ','], " ");
    t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    for prefix in ["due ", "by ", "before ", "until ", "on ", "no later than "] {
        if let Some(rest) = t.strip_prefix(prefix) {
            t = rest.to_string();
        }
    }
    for suffix in [
        " morning",
        " afternoon",
        " evening",
        " eod",
        " end of day",
        " noon",
        " at noon",
    ] {
        if let Some(rest) = t.strip_suffix(suffix) {
            t = rest.to_string();
        }
    }
    let t = t.strip_prefix("the ").unwrap_or(&t);

    match t {
        "today" | "tonight" | "eod" | "end of day" | "end of today" => return Some(on),
        "tomorrow" => return on.checked_add(1.day()).ok(),
        "end of week" | "end of the week" | "end of this week" | "this week" | "eow" => {
            return Some(friday_of(on, 0));
        }
        "next week" | "end of next week" => return Some(friday_of(on, 1)),
        "end of month" | "end of the month" | "end of this month" | "this month" | "eom" => {
            return on.last_of_month().into();
        }
        "next month" | "end of next month" => {
            return on.checked_add(1.month()).ok().map(|d| d.last_of_month());
        }
        _ => {}
    }
    if let Some(days) = in_days(t) {
        return on.checked_add(days.days()).ok();
    }
    if let Some(date) = weekday_and_day(t, on) {
        return Some(date);
    }
    if let Some((next, day)) = weekday(t) {
        // The coming one; "next Thursday" is the one in the following week.
        let mut date = on.nth_weekday(1, day).ok()?;
        if next && same_week(date, on) {
            date = date.checked_add(7.days()).ok()?;
        }
        return Some(date);
    }
    calendar_date(t, on)
}

/// Friday of `on`'s week, `weeks` later; from a weekend, the coming Friday.
fn friday_of(on: Date, weeks: i64) -> Date {
    let monday = on
        .checked_sub(i64::from(on.weekday().to_monday_zero_offset()).days())
        .unwrap_or(on);
    let mut friday = monday.checked_add((4 + 7 * weeks).days()).unwrap_or(monday);
    if friday < on {
        friday = friday.checked_add(7.days()).unwrap_or(friday);
    }
    friday
}

fn same_week(a: Date, b: Date) -> bool {
    let (a, b) = (a.iso_week_date(), b.iso_week_date());
    a.year() == b.year() && a.week() == b.week()
}

/// "friday the 20th", "friday 20th", "friday oct 24": a weekday with the
/// date spelled out. With only a day of the month, the first one still to
/// come that falls on that weekday ("Friday the 20th" said on Oct 11, 2026
/// is Nov 20, as Oct 20 is a Tuesday); if none does within a year, the next
/// such day of the month.
fn weekday_and_day(t: &str, on: Date) -> Option<Date> {
    let (first, rest) = t.split_once(' ')?;
    let (_, wanted) = weekday(first)?;
    let rest = rest.strip_prefix("the ").unwrap_or(rest);
    if let Some(date) = calendar_date(rest, on) {
        return Some(date);
    }
    let n = day(rest)?;
    let candidates: Vec<Date> = (0..13)
        .filter_map(|months| {
            let month = on.first_of_month().checked_add(months.months()).ok()?;
            Date::new(month.year(), month.month(), n).ok()
        })
        .filter(|date| *date >= on)
        .collect();
    candidates
        .iter()
        .find(|date| date.weekday() == wanted)
        .or_else(|| candidates.first())
        .copied()
}

/// "in 3 days", "in two weeks", "in a week".
fn in_days(t: &str) -> Option<i64> {
    let rest = t
        .strip_prefix("in ")
        .or_else(|| t.strip_prefix("within "))?;
    let (n, unit) = rest.split_once(' ')?;
    let n: i64 = match n {
        "a" | "an" | "one" => 1,
        "two" => 2,
        "three" => 3,
        "four" => 4,
        "five" => 5,
        n => n.parse().ok()?,
    };
    match unit.trim_end_matches('s') {
        "day" => Some(n),
        "week" => Some(7 * n),
        _ => None,
    }
}

/// "thursday", "this thu", "next friday" → (next?, day).
fn weekday(t: &str) -> Option<(bool, Weekday)> {
    let (next, name) = match t.strip_prefix("next ") {
        Some(rest) => (true, rest),
        None => (false, t.strip_prefix("this ").unwrap_or(t)),
    };
    let day = match name {
        "monday" | "mon" => Weekday::Monday,
        "tuesday" | "tue" | "tues" => Weekday::Tuesday,
        "wednesday" | "wed" => Weekday::Wednesday,
        "thursday" | "thu" | "thurs" => Weekday::Thursday,
        "friday" | "fri" => Weekday::Friday,
        "saturday" | "sat" => Weekday::Saturday,
        "sunday" | "sun" => Weekday::Sunday,
        _ => return None,
    };
    Some((next, day))
}

/// "october 24", "oct 24th", "24 october", "oct 24 2026", "2026-10-24".
/// Without a year, the one that puts it closest after the meeting.
fn calendar_date(t: &str, on: Date) -> Option<Date> {
    if let Ok(date) = t.parse::<Date>() {
        return Some(date);
    }
    let words: Vec<&str> = t.split_whitespace().collect();
    let (month, day, year) = match words.as_slice() {
        [m, d] if month(m).is_some() => (month(m)?, day(d)?, None),
        [d, m] => (month(m)?, day(d)?, None),
        [m, d, y] if month(m).is_some() => (month(m)?, day(d)?, y.parse::<i16>().ok()),
        [d, m, y] => (month(m)?, day(d)?, y.parse::<i16>().ok()),
        _ => return None,
    };
    let date = Date::new(year.unwrap_or(on.year()), month, day).ok()?;
    // Said in December about "January 5": next year. A date a little before
    // the meeting is a deadline already passed, not one a year away.
    if year.is_none() && date < on.checked_sub(60.days()).ok()? {
        return Date::new(on.year() + 1, month, day).ok();
    }
    Some(date)
}

fn month(word: &str) -> Option<i8> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    if word.len() < 3 {
        return None;
    }
    MONTHS
        .iter()
        .position(|m| word.starts_with(m))
        .and_then(|i| i8::try_from(i + 1).ok())
}

fn day(word: &str) -> Option<i8> {
    let digits = word.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    digits.parse::<i8>().ok().filter(|d| (1..=31).contains(d))
}

/// Words of a task, for matching the same item across two summaries of a
/// meeting: re-summarizing rewords things a little, and a ticked item should
/// stay ticked.
pub(crate) fn words(task: &str) -> Vec<String> {
    task.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2)
        .map(String::from)
        .collect()
}

/// How alike two tasks are, 0–1 (shared words over all words).
pub(crate) fn similarity(a: &[String], b: &[String]) -> f64 {
    let shared = a.iter().filter(|w| b.contains(w)).count();
    let all = a.len() + b.len() - shared;
    if all == 0 {
        return 0.0;
    }
    shared as f64 / all as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notes(items: &str) -> String {
        format!(
            "# Sync\n\n## Decisions\n- Ship it\n\n## Action items\n{items}\n## Discussion by topic\n- x\n"
        )
    }

    #[test]
    fn reads_owner_task_and_due_in_the_prompts_format() {
        let items = parse(&notes(
            "- **You** — send the rollout plan to support — by Wednesday\n\
             - **Priya** — fix the duplicate `signup_completed` events\n\
             - **Priya and Dan** — draft the pricing page — end of next week\n",
        ));
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].owner.as_deref(), Some("You"));
        assert_eq!(items[0].task, "send the rollout plan to support");
        assert_eq!(items[0].due.as_deref(), Some("by Wednesday"));
        let labelled = parse(&notes("- **Dan** — send the deck — due date: Monday\n"));
        assert_eq!(labelled[0].due.as_deref(), Some("Monday"));
        assert_eq!(items[1].due, None);
        assert_eq!(items[1].task, "fix the duplicate `signup_completed` events");
        assert_eq!(items[2].owner.as_deref(), Some("Priya and Dan"));
    }

    #[test]
    fn reads_other_shapes_and_skips_placeholders() {
        let items = parse(&notes(
            "- [ ] **You:** book the venue\n\
             - Dan: two pricing-page variants\n\
             - Follow up with legal — the contract’s wording\n",
        ));
        assert_eq!(items[0].owner.as_deref(), Some("You"));
        assert_eq!(items[0].task, "book the venue");
        assert_eq!(items[1].owner.as_deref(), Some("Dan"));
        // A dash in a task isn't a due date unless it reads like one.
        assert_eq!(items[2].owner, None);
        assert_eq!(
            items[2].task,
            "Follow up with legal — the contract’s wording"
        );
        assert_eq!(items[2].due, None);

        for none in ["- None.\n", "- No action items were assigned.\n", "- N/A\n"] {
            assert!(parse(&notes(none)).is_empty(), "{none}");
        }
        assert!(parse("# Sync\n\n## Decisions\n- Ship it\n").is_empty());
    }

    #[test]
    fn knows_your_items_from_others() {
        assert!(is_mine(Some("You")));
        assert!(is_mine(Some("You and Priya")));
        assert!(is_mine(Some("Priya, you")));
        assert!(is_mine(Some("Me")));
        assert!(!is_mine(Some("Priya")));
        assert!(!is_mine(Some("Yousef")));
        assert!(!is_mine(None));
    }

    #[test]
    fn works_out_due_dates_from_the_meeting_day() {
        // A Tuesday.
        let on = jiff::civil::date(2026, 10, 20);
        let due = |t: &str| due_date(t, on).map(|d| d.to_string());
        assert_eq!(due("by Thursday").as_deref(), Some("2026-10-22"));
        assert_eq!(
            due("Tuesday").as_deref(),
            Some("2026-10-27"),
            "the coming one"
        );
        assert_eq!(due("next Thursday").as_deref(), Some("2026-10-29"));
        assert_eq!(due("Friday afternoon").as_deref(), Some("2026-10-23"));
        assert_eq!(due("today").as_deref(), Some("2026-10-20"));
        assert_eq!(due("tomorrow").as_deref(), Some("2026-10-21"));
        assert_eq!(due("end of the week").as_deref(), Some("2026-10-23"));
        assert_eq!(due("by end of next week").as_deref(), Some("2026-10-30"));
        assert_eq!(due("end of month").as_deref(), Some("2026-10-31"));
        assert_eq!(due("in two weeks").as_deref(), Some("2026-11-03"));
        assert_eq!(due("Oct 24th").as_deref(), Some("2026-10-24"));
        assert_eq!(due("24 October").as_deref(), Some("2026-10-24"));
        assert_eq!(due("by 2026-11-02").as_deref(), Some("2026-11-02"));
        assert_eq!(due("soon"), None);
        // The template's label copied into the text, and a weekday with its date.
        let wednesday = jiff::civil::date(2026, 11, 18);
        let due_on = |t: &str| due_date(t, wednesday).map(|d| d.to_string());
        assert_eq!(due_on("due date: Monday").as_deref(), Some("2026-11-23"));
        assert_eq!(
            due_on("due date: Friday the 20th").as_deref(),
            Some("2026-11-20")
        );
        assert_eq!(due_on("Friday, Nov 20").as_deref(), Some("2026-11-20"));
        assert_eq!(due_on("Tuesday the 1st").as_deref(), Some("2026-12-01"));
        // The weekday settles which month: Oct 20, 2026 is a Tuesday.
        let sunday = jiff::civil::date(2026, 10, 11);
        assert_eq!(
            due_date("Friday the 20th", sunday)
                .map(|d| d.to_string())
                .as_deref(),
            Some("2026-11-20")
        );
        assert_eq!(due("after the launch"), None);
        // In December, "January 5" is next year's.
        let december = jiff::civil::date(2026, 12, 18);
        assert_eq!(
            due_date("January 5", december)
                .map(|d| d.to_string())
                .as_deref(),
            Some("2027-01-05")
        );
    }

    #[test]
    fn rewording_still_counts_as_the_same_task() {
        let a = words("Send the rollout plan to support");
        let b = words("send rollout plan to the support team");
        let c = words("Fix the duplicate signup events");
        assert!(similarity(&a, &b) > 0.6);
        assert!(similarity(&a, &c) < 0.2);
    }
}
