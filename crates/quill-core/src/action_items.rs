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
    /// Who asked for it or other commentary the notes added ("per Sam",
    /// "Sam didn't respond"), kept out of the task.
    pub note: Option<String>,
    /// As written, e.g. "by Thursday".
    pub due: Option<String>,
    /// Someone took it on; false when the notes mark it "(suggested)".
    pub committed: bool,
    /// Where in the recording it was said, from the notes' `[mm:ss]`.
    pub source_ms: Option<u64>,
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

    let (text, source_ms) = split_timestamp(text);
    let (owner, rest) = split_owner(text);
    // "**Alex** and **Sam** — …": the others who share it.
    let (owner, rest) = match owner {
        Some(first) => match more_owners(rest) {
            Some((more, rest)) => (Some(format!("{first}{more}")), rest),
            None => (Some(first.to_string()), rest),
        },
        None => (None, rest),
    };
    let mut notes: Vec<String> = Vec::new();
    // "**Priya** (suggested) — …", or with no owner "(suggested) — …".
    let (rest, suggested_first) = without_suggested(rest.trim_start_matches(SEPARATORS));
    let mut rest = rest.trim_start_matches(SEPARATORS).trim();
    // "(per Sam) — finish the list": who asked, before the task.
    if let Some(inside) = rest.strip_prefix('(')
        && let Some((note, after)) = inside.split_once(')')
        && !note.eq_ignore_ascii_case("suggested")
    {
        notes.push(note.trim().to_string());
        rest = after.trim_start_matches(SEPARATORS).trim();
    }
    let (task, due) = split_due(rest);
    let (task, suggested_last) = without_suggested(task);
    let task = without_commentary(task, &mut notes);
    let task = plain(task.trim_matches(SEPARATORS));
    if task.is_empty() || (owner.is_none() && is_placeholder(&task)) {
        return None;
    }
    let owner = owner.map(|o| plain(&o).trim_end_matches(':').trim().to_string());
    // "Priya (suggested): …" puts it in the owner, and "Sam (per Lee)" who
    // asked.
    let (owner, suggested_owner) = match owner {
        Some(o) => {
            let (o, suggested) = without_suggested(&o);
            (
                Some(without_commentary(o, &mut notes).to_string()),
                suggested,
            )
        }
        None => (None, false),
    };
    let note = notes
        .iter()
        .map(|n| plain(n))
        .filter(|n| !n.is_empty())
        .collect::<Vec<_>>()
        .join("; ");
    Some(Parsed {
        owner: owner.filter(|o| !o.is_empty() && !is_no_owner(o)),
        task,
        note: (!note.is_empty()).then_some(note),
        due: due.map(|d| plain(strip_label(d))).filter(|d| !d.is_empty()),
        committed: !(suggested_first || suggested_last || suggested_owner),
        source_ms,
    })
}

/// What separates an item's owner, task and due date.
const SEPARATORS: [char; 6] = ['—', '–', '-', ':', ',', ' '];

/// `text` without commentary at its end, which goes to `notes`: a bracket
/// ("(Lee said it; Sam did not respond)") or a dash-separated part ("— per
/// Sam") that reads like who asked or what happened, rather than part of
/// the task. "Book the room (Friday)" keeps its brackets.
fn without_commentary<'a>(text: &'a str, notes: &mut Vec<String>) -> &'a str {
    let t = text.trim().trim_end_matches(SEPARATORS);
    if let Some(inner) = t.strip_suffix(')')
        && let Some(at) = inner.rfind('(')
        && is_commentary(&inner[at + 1..])
        && !inner[..at].trim().is_empty()
    {
        notes.insert(0, inner[at + 1..].trim().to_string());
        return without_commentary(&inner[..at], notes);
    }
    let cut = t.rfind(" — ").or_else(|| t.rfind(" – "));
    if let Some(at) = cut {
        let sep = t[at..].chars().nth(1).map_or(0, char::len_utf8) + 2;
        let part = t[at + sep..].trim();
        if is_commentary(part) && !t[..at].trim().is_empty() {
            notes.insert(0, part.to_string());
            return without_commentary(&t[..at], notes);
        }
    }
    t
}

/// Reads like who asked for an item or what happened to it, not like the
/// task: "per Sam", "Lee said it", "Sam didn't respond", "unconfirmed".
fn is_commentary(text: &str) -> bool {
    let t = format!(" {} ", text.trim().to_lowercase());
    const STARTS: &[&str] = &[
        " per ",
        " as per ",
        " according to ",
        " via ",
        " on behalf of ",
        " asked by ",
        " raised by ",
        " suggested by ",
        " requested by ",
        " said by ",
    ];
    const ANYWHERE: &[&str] = &[
        " said ",
        " says ",
        " asked ",
        " did not ",
        " didn't ",
        " has not ",
        " hasn't ",
        " not confirmed",
        " unconfirmed",
        " respond",
        " volunteered",
        " offered",
        " tentative",
        " no response",
    ];
    STARTS.iter().any(|s| t.starts_with(s)) || ANYWHERE.iter().any(|w| t.contains(w))
}

/// "and **Sam** — link up…" after a first owner: the other owners, as
/// they follow it (" and Sam", ", Sam and Dana"), and the rest. Only when
/// what comes before the dash reads like names.
fn more_owners(rest: &str) -> Option<(String, &str)> {
    let t = rest.trim_start();
    let connector = ["and ", "& ", ", "].iter().find(|c| t.starts_with(**c))?;
    let (names, after) = t.split_once(" — ").or_else(|| t.split_once(" – "))?;
    let names = names.replace("**", "");
    if !looks_like_names(names[connector.len()..].trim()) {
        return None;
    }
    let joined = if names.starts_with(',') {
        names.trim_end().to_string()
    } else {
        format!(" {}", names.trim())
    };
    Some((joined, after))
}

/// "Sam", "Priya Ramaswamy", "Sam and you", "Alex, Sam and Dana", "Priya
/// (suggested)": one to six words, each a capitalized name, "you", or a
/// word joining names.
fn looks_like_names(text: &str) -> bool {
    let (text, _) = without_suggested(text);
    let words: Vec<&str> = text
        .split_whitespace()
        .map(|w| w.trim_end_matches(','))
        .collect();
    let person = |w: &&str| {
        w.starts_with(char::is_uppercase) || ["you", "me"].contains(&w.to_lowercase().as_str())
    };
    (1..=6).contains(&words.len())
        && words.iter().any(person)
        && words
            .iter()
            .all(|w| person(w) || ["and", "&"].contains(&w.to_lowercase().as_str()))
}

/// "send the deck [12:34]" → ("send the deck", 754 s in ms): the
/// timestamp the notes end an item with, `[mm:ss]` or `[h:mm:ss]` (or in
/// round brackets). Only the first of several counts.
fn split_timestamp(text: &str) -> (&str, Option<u64>) {
    let t = text.trim_end().trim_end_matches('.').trim_end();
    let open = match t.chars().last() {
        Some(']') => '[',
        Some(')') => '(',
        _ => return (text, None),
    };
    let Some(at) = t.rfind(open) else {
        return (text, None);
    };
    let inside = &t[at + 1..t.len() - 1];
    let first = inside.split([',', ';', '–']).next().unwrap_or("").trim();
    match clock_ms(first) {
        Some(ms) => (t[..at].trim_end(), Some(ms)),
        None => (text, None),
    }
}

/// "12:34" or "1:02:03" in milliseconds.
fn clock_ms(text: &str) -> Option<u64> {
    let parts: Vec<&str> = text.split(':').collect();
    let numbers = parts
        .iter()
        .map(|p| (!p.is_empty() && p.chars().all(|c| c.is_ascii_digit())).then(|| p.parse()))
        .collect::<Option<Result<Vec<u64>, _>>>()?
        .ok()?;
    let seconds_ok = parts.last().is_some_and(|s| s.len() == 2);
    let secs = match numbers.as_slice() {
        [m, s] if *s < 60 => m * 60 + s,
        [h, m, s] if *m < 60 && *s < 60 => (h * 60 + m) * 60 + s,
        _ => return None,
    };
    seconds_ok.then_some(secs * 1000)
}

/// `text` without a "(suggested)" mark at its start or end (ignoring
/// case), and whether it had one.
fn without_suggested(text: &str) -> (&str, bool) {
    const MARK: &str = "(suggested)";
    let t = text.trim();
    let mark = |s: Option<&str>| s.is_some_and(|s| s.eq_ignore_ascii_case(MARK));
    if mark(t.get(..MARK.len())) {
        return (t[MARK.len()..].trim(), true);
    }
    let cut = t.len().saturating_sub(MARK.len());
    if t.len() >= MARK.len() && mark(t.get(cut..)) {
        return (t[..cut].trim(), true);
    }
    (text, false)
}

/// Owners that name no one: "Unassigned", or "Another participant" (the
/// prompts' word for someone the transcript doesn't name).
fn is_no_owner(owner: &str) -> bool {
    let o = owner.to_lowercase();
    let o = o.trim_start_matches("an ").trim_start_matches("a ");
    [
        "unassigned",
        "not assigned",
        "no owner",
        "unknown",
        "unclear",
        "tbd",
        "n/a",
        "none",
        "someone",
        "other participant",
        "another participant",
        "participant",
        "other participants",
        "the other participant",
    ]
    .contains(&o)
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
    // "Sam — send the deck": names, then the dash the prompts put after an
    // owner. "Follow up with legal — …" isn't names, so it stays a task.
    if let Some((owner, rest)) = text.split_once(" — ").or_else(|| text.split_once(" – "))
        && looks_like_names(owner)
    {
        return (Some(owner), rest);
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
        if says_no_date(due) {
            return (task.trim(), None);
        }
        if looks_like_a_date(due) {
            return (task.trim(), Some(due));
        }
    }
    (text, None)
}

/// "no due date given", "Due date: none", "not specified": the model
/// saying nothing was said, which isn't a due date.
fn says_no_date(text: &str) -> bool {
    let t = plain(strip_label(text))
        .to_lowercase()
        .trim_matches(['(', ')'])
        .to_string();
    [
        "no due",
        "no deadline",
        "no date",
        "no specific",
        "no timeline",
        "none",
        "n/a",
        "tbd",
        "not specified",
        "not given",
        "not mentioned",
        "not stated",
        "unspecified",
    ]
    .iter()
    .any(|p| t.starts_with(p))
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
        "hour",
        "day",
        "later",
        "soon",
        "next ",
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

/// Whether the owner is the person recording, alone or with others:
/// "You" (or "Me", in older notes), or `you_as`, the name the notes call
/// them by. A full name also counts by its first name alone ("Sam" for
/// "Sam Lee"): the notes write anyone else's full name.
pub fn is_mine(owner: Option<&str>, you_as: Option<&str>) -> bool {
    owner.is_some_and(|o| owners(o).iter().any(|who| is_you(who, you_as)))
}

/// "You" or "Me", or `you_as` or its first name, ignoring case.
fn is_you(who: &str, you_as: Option<&str>) -> bool {
    let who = who.to_lowercase();
    let name = you_as.map(str::to_lowercase);
    let first = name
        .as_deref()
        .filter(|n| n.split_whitespace().count() > 1)
        .and_then(|n| n.split_whitespace().next());
    who == "you" || who == "me" || name.as_deref() == Some(&*who) || first == Some(&*who)
}

/// "Priya, Dan and you" → ["Priya", "Dan", "you"].
fn owners(owner: &str) -> Vec<&str> {
    owner
        .split([',', '&', '/', '(', ')'])
        .flat_map(|part| part.split(" and "))
        .map(str::trim)
        .filter(|who| !who.is_empty())
        .collect()
}

/// `owner` with the person recording first, so every meeting writes a
/// shared item the same way: "Sam and you" → "You and Sam", "Dan, Sam
/// Lee and Priya" → "Sam Lee, Dan and Priya" (with `you_as` "Sam Lee").
/// Unchanged when they aren't among its owners.
pub fn you_first(owner: &str, you_as: Option<&str>) -> String {
    let mut names = owners(owner);
    let Some(at) = names.iter().position(|who| is_you(who, you_as)) else {
        return owner.to_string();
    };
    let you = names.remove(at);
    let you = if ["you", "me"].contains(&you.to_lowercase().as_str()) {
        "You"
    } else {
        you
    };
    names.insert(0, you);
    match names.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        _ => names.join(""),
    }
}

/// The date a due phrase means, said in a meeting on `on`: "Thursday",
/// "by end of next week", "Oct 24", "in two weeks". `None` if it isn't a
/// date (or is too vague, like "soon").
pub fn due_date(text: &str, on: Date) -> Option<Date> {
    // "end of day today (Friday)": the words around the brackets, else the
    // ones in them.
    if let Some((before, rest)) = text.split_once('(')
        && let Some((inside, after)) = rest.split_once(')')
    {
        return due_date(&format!("{before} {after}"), on).or_else(|| due_date(inside, on));
    }
    let mut t = strip_label(text.trim())
        .to_lowercase()
        .replace(['.', ',', '"', '\u{201C}', '\u{201D}'], " ");
    t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    // Until none is left: "by end of day on Monday", "hopefully by the end
    // of this week".
    loop {
        let before = t.len();
        for prefix in [
            "hopefully ",
            "ideally ",
            "probably ",
            "maybe ",
            "around ",
            "roughly ",
            "sometime ",
            "due ",
            "by ",
            "before ",
            "until ",
            "on ",
            "no later than ",
            "end of day ",
            "end of the day ",
            "eod ",
            "cob ",
            "close of business ",
            "end of business ",
            "eob ",
        ] {
            if let Some(rest) = t.strip_prefix(prefix) {
                t = rest.to_string();
            }
        }
        if t.len() == before {
            break;
        }
    }
    for suffix in [
        " morning",
        " afternoon",
        " evening",
        " eod",
        " end of day",
        " cob",
        " close of business",
        " noon",
        " at noon",
    ] {
        if let Some(rest) = t.strip_suffix(suffix) {
            t = rest.to_string();
        }
    }
    let t = t.strip_prefix("the ").unwrap_or(&t);

    match t {
        "today" | "tonight" | "eod" | "end of day" | "end of the day" | "end of today" | "cob"
        | "close of business" | "eob" | "end of business" => return Some(on),
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
    fn saying_no_date_or_no_owner_is_none() {
        let items = parse(&notes(
            "- **You** — send the deck — no due date given\n\
             - **Dan** — book the room — Due date: none\n\
             - **Priya** — review the plan — (not specified)\n\
             - **Other participant** — share the recording\n\
             - **Unassigned** — find a venue — by Friday\n\
             - **Another participant** — draft the brief\n",
        ));
        assert_eq!(items.len(), 6);
        for item in &items[..3] {
            assert_eq!(item.due, None, "{item:?}");
        }
        assert_eq!(
            items[0].task, "send the deck",
            "the phrase leaves the task too"
        );
        assert_eq!(items[3].owner, None);
        assert_eq!(items[4].owner, None);
        assert_eq!(items[4].due.as_deref(), Some("by Friday"));
        assert_eq!(items[5].owner, None);
        // Real names, and you, stay.
        assert_eq!(items[0].owner.as_deref(), Some("You"));
        assert_eq!(items[1].owner.as_deref(), Some("Dan"));
    }

    #[test]
    fn reads_v5_timestamps_and_suggested_items() {
        let items = parse(&notes(
            "- **You** — send the deck — by Friday [12:34]\n\
             - **Priya** (suggested) — look at the churn numbers [1:02:03]\n\
             - (suggested) — find a venue [05:00].\n\
             - **Dan** — fix the build (suggested)\n\
             - **Dan** — book the room (Friday)\n",
        ));
        assert_eq!(items.len(), 5);
        assert_eq!(items[0].task, "send the deck");
        assert_eq!(items[0].due.as_deref(), Some("by Friday"));
        assert_eq!(items[0].source_ms, Some(754_000));
        assert!(items[0].committed);
        assert_eq!(items[1].owner.as_deref(), Some("Priya"));
        assert_eq!(items[1].task, "look at the churn numbers");
        assert_eq!(items[1].source_ms, Some(3_723_000));
        assert!(!items[1].committed);
        assert_eq!(
            (items[2].owner.as_deref(), items[2].task.as_str()),
            (None, "find a venue")
        );
        assert_eq!(items[2].source_ms, Some(300_000));
        assert!(!items[2].committed);
        assert_eq!(items[3].task, "fix the build");
        assert!(!items[3].committed && items[3].source_ms.is_none());
        // Brackets that aren't a time stay part of the item.
        assert_eq!(items[4].task, "book the room (Friday)");
        assert!(items[4].committed && items[4].source_ms.is_none());
        // An owner with the mark after it, the other way models write it.
        let other = parse(&notes("- Priya (suggested): draft the brief [00:42]\n"));
        assert_eq!(other[0].owner.as_deref(), Some("Priya"));
        assert!(!other[0].committed);
    }

    #[test]
    fn end_of_day_and_bracketed_days_resolve() {
        // A Friday.
        let on = jiff::civil::date(2026, 10, 30);
        let due = |t: &str| due_date(t, on).map(|d| d.to_string());
        assert_eq!(
            due("end of day today (Friday)").as_deref(),
            Some("2026-10-30")
        );
        assert_eq!(due("end of day Monday").as_deref(), Some("2026-11-02"));
        assert_eq!(due("by EOD Monday").as_deref(), Some("2026-11-02"));
        assert_eq!(
            due("by end of day on Tuesday").as_deref(),
            Some("2026-11-03")
        );
        assert_eq!(due("COB Wednesday").as_deref(), Some("2026-11-04"));
        assert_eq!(due("Monday COB").as_deref(), Some("2026-11-02"));
        assert_eq!(due("close of business").as_deref(), Some("2026-10-30"));
        // Only the brackets say when.
        assert_eq!(
            due("before the review (Nov 5)").as_deref(),
            Some("2026-11-05")
        );
        assert_eq!(due("after the launch (soon)"), None);
    }

    #[test]
    fn hedged_and_quoted_due_dates_resolve() {
        // A Tuesday.
        let on = jiff::civil::date(2026, 11, 3);
        let due = |t: &str| due_date(t, on).map(|d| d.to_string());
        assert_eq!(
            due("\"hopefully by the end of this week\"").as_deref(),
            Some("2026-11-06")
        );
        assert_eq!(
            due("\u{201C}Ideally Thursday\u{201D}").as_deref(),
            Some("2026-11-05")
        );
        assert_eq!(due("sometime next week").as_deref(), Some("2026-11-13"));
        assert_eq!(due("hopefully soon"), None);
    }

    #[test]
    fn owners_tasks_and_notes_come_apart() {
        let items = parse(&notes(
            "- **Alex** and **Sam** — link up on the rollout plan — [12:00]\n\
             - Sam — Send you the deck — in another couple of hours [13:10]\n\
             - **Lee** — (per Sam) — Finish the list of open roles [14:00]\n\
             - **Sam** — be in touch about next steps (Lee said it; Sam did not respond in the meeting) [15:00]\n\
             - **Priya** — write up the decision — per Dan — by Friday [16:00]\n\
             - **Dana**, **Sam** and you — review the budget — [17:00]\n\
             - **Jordan** — share the recording —\n",
        ));
        // Owner, task, note, due.
        type Fields<'a> = (Option<&'a str>, &'a str, Option<&'a str>, Option<&'a str>);
        let got: Vec<Fields> = items
            .iter()
            .map(|i| {
                (
                    i.owner.as_deref(),
                    i.task.as_str(),
                    i.note.as_deref(),
                    i.due.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            got,
            [
                (
                    Some("Alex and Sam"),
                    "link up on the rollout plan",
                    None,
                    None
                ),
                (
                    Some("Sam"),
                    "Send you the deck",
                    None,
                    Some("in another couple of hours")
                ),
                (
                    Some("Lee"),
                    "Finish the list of open roles",
                    Some("per Sam"),
                    None
                ),
                (
                    Some("Sam"),
                    "be in touch about next steps",
                    Some("Lee said it; Sam did not respond in the meeting"),
                    None
                ),
                (
                    Some("Priya"),
                    "write up the decision",
                    Some("per Dan"),
                    Some("by Friday")
                ),
                (Some("Dana, Sam and you"), "review the budget", None, None),
                (Some("Jordan"), "share the recording", None, None),
            ]
        );
        assert_eq!(items[0].source_ms, Some(720_000));
        // The person recording among the owners.
        assert!(is_mine(items[5].owner.as_deref(), None));
    }

    #[test]
    fn you_come_first_among_owners() {
        assert_eq!(you_first("Sam and you", None), "You and Sam");
        assert_eq!(you_first("Dana, Sam and you", None), "You, Dana and Sam");
        assert_eq!(you_first("You and Sam", None), "You and Sam");
        assert_eq!(
            you_first("Dan, Sam Lee and Priya", Some("Sam Lee")),
            "Sam Lee, Dan and Priya"
        );
        assert_eq!(
            you_first("Jordan and Sam", Some("Sam Lee")),
            "Sam and Jordan"
        );
        // Not theirs: as written.
        assert_eq!(you_first("Priya & Dan", None), "Priya & Dan");
        assert_eq!(you_first("You", None), "You");
    }

    #[test]
    fn knows_your_items_from_others() {
        assert!(is_mine(Some("You"), None));
        assert!(is_mine(Some("You and Priya"), None));
        assert!(is_mine(Some("Priya, you"), None));
        assert!(is_mine(Some("Me"), None));
        assert!(!is_mine(Some("Priya"), None));
        assert!(!is_mine(Some("Yousef"), None));
        assert!(!is_mine(None, None));
        // Notes that call the user by name.
        assert!(is_mine(Some("Sam Lee"), Some("Sam Lee")));
        assert!(is_mine(Some("Priya and sam lee"), Some("Sam Lee")));
        // A first name alone is yours: the notes write others' full names.
        assert!(is_mine(Some("Sam and Jordan"), Some("Sam Lee")));
        assert!(!is_mine(Some("Samuel"), Some("Sam Lee")));
        assert!(!is_mine(Some("Priya"), Some("Sam Lee")));
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
