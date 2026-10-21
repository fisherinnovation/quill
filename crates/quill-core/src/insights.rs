//! Usage numbers for the Insights page: how much has been dictated and
//! recorded, when, and roughly how much time dictation saved.
//!
//! Everything is computed from the library (dictation stats hold counts, not
//! text) on demand; nothing is sent anywhere.

use jiff::civil::Date;
use jiff::{Timestamp, Zoned};
use serde::Serialize;

use crate::library::{AppUsage, DictationStat, Library, LibraryError, Meeting};
use crate::meeting::ME;

/// Typing speed that "time saved" compares against: a common average for
/// typing on a keyboard.
pub const TYPING_WPM: f64 = 40.0;
/// Days shown in the activity chart.
pub const CHART_DAYS: usize = 30;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Insights {
    pub dictation: DictationInsights,
    pub meetings: MeetingInsights,
    /// Consecutive days, up to today, with a dictation or a meeting. Today
    /// doesn't break the streak until it's over.
    pub streak_days: u32,
    /// The last [`CHART_DAYS`] days, oldest first.
    pub days: Vec<Day>,
    /// Dictations by local hour of the day, 0–23.
    pub hours: [u64; 24],
    /// Typing speed used for `minutes_saved`.
    pub typing_wpm: f64,
    /// Apps dictated into, most words first (at most [`TOP_APPS`]).
    pub apps: Vec<AppUsage>,
}

/// Apps listed on the Insights page.
pub const TOP_APPS: usize = 6;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct DictationInsights {
    pub count: u64,
    pub words: u64,
    pub words_last_7_days: u64,
    /// Words per minute of speech; `None` until there's a minute of it.
    pub speaking_wpm: Option<f64>,
    /// Typing the same words at [`TYPING_WPM`] minus the time spent
    /// speaking them. Never negative.
    pub minutes_saved: f64,
    /// Median time from releasing the shortcut to the text appearing.
    pub median_latency_ms: Option<u64>,
    /// Time to insert over the last [`CHART_DAYS`] days: from the end of
    /// speech to the text typed, the wait the user notices. `None` until a
    /// dictation has one.
    pub wait: Option<WaitInsights>,
    /// Share of dictations the clean-up step changed, 0–1.
    pub polished_share: f64,
}

/// Time to insert, over the last [`CHART_DAYS`] days.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct WaitInsights {
    /// Dictations it was measured on.
    pub count: u64,
    pub median_ms: u64,
    /// 90th percentile: the slow end of what the user lives with.
    pub p90_ms: u64,
    /// Of the median, how it splits: still holding the key after speaking,
    /// and Quill transcribing and typing after the release (medians of the
    /// same dictations, so they roughly add up).
    pub held_ms: u64,
    pub quill_ms: u64,
    /// Median over the last 7 days, and over the 7 before, for the trend.
    pub last_7_days_ms: Option<u64>,
    pub previous_7_days_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct MeetingInsights {
    pub count: u64,
    pub total_ms: u64,
    pub count_last_7_days: u64,
    pub ms_last_7_days: u64,
    pub average_ms: Option<u64>,
    /// Your share of the words spoken in transcribed meetings, 0–1.
    pub my_talk_share: Option<f64>,
    pub summaries: u64,
    pub summary_cost_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Day {
    /// Local date, `YYYY-MM-DD`.
    pub date: String,
    pub words: u64,
    pub dictations: u64,
    pub meeting_ms: u64,
    /// That day's median and 90th-percentile time to insert, if measured.
    pub wait_ms: Option<u64>,
    pub wait_p90_ms: Option<u64>,
}

/// Computes insights from the library as of `now` (which also gives the
/// time zone for "days" and "hours").
pub fn compute(library: &Library, now: &Zoned) -> Result<Insights, LibraryError> {
    let stats = library.dictation_stats()?;
    let meetings = library.list()?;
    let talk = library.words_by_speaker()?;
    let (summaries, cost) = library.summary_costs()?;
    let mut insights = from_data(&stats, &meetings, &talk, summaries, cost, now);
    insights.apps = library.dictation_apps()?;
    insights.apps.truncate(TOP_APPS);
    Ok(insights)
}

/// The value below which a share `p` (0–1) of `sorted` falls (nearest rank).
fn percentile(sorted: &[u64], p: f64) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (p * sorted.len() as f64).ceil() as usize;
    sorted.get(rank.clamp(1, sorted.len()) - 1).copied()
}

fn from_data(
    stats: &[DictationStat],
    meetings: &[Meeting],
    talk: &[(Option<String>, u64)],
    summaries: u64,
    summary_cost_usd: f64,
    now: &Zoned,
) -> Insights {
    let tz = now.time_zone().clone();
    let today = now.date();
    let local = |rfc3339: &str| -> Option<Zoned> {
        rfc3339
            .parse::<Timestamp>()
            .ok()
            .map(|t| t.to_zoned(tz.clone()))
    };
    let week_start = today
        .checked_sub(jiff::Span::new().days(6))
        .unwrap_or(today);
    let chart_start = today
        .checked_sub(jiff::Span::new().days(i64::try_from(CHART_DAYS).unwrap_or(30) - 1))
        .unwrap_or(today);

    let mut days: Vec<Day> = (0..CHART_DAYS)
        .filter_map(|i| {
            chart_start
                .checked_add(jiff::Span::new().days(i64::try_from(i).ok()?))
                .ok()
        })
        .map(|d| Day {
            date: d.to_string(),
            words: 0,
            dictations: 0,
            meeting_ms: 0,
            wait_ms: None,
            wait_p90_ms: None,
        })
        .collect();
    let day_index = |date: Date| -> Option<usize> {
        let offset = (date - chart_start).get_days();
        usize::try_from(offset).ok().filter(|&i| i < CHART_DAYS)
    };
    let mut active_days = std::collections::HashSet::new();

    // Dictation.
    let mut d = DictationInsights {
        count: u64::try_from(stats.len()).unwrap_or(u64::MAX),
        ..DictationInsights::default()
    };
    let mut hours = [0_u64; 24];
    let mut audio_ms = 0_u64;
    let mut latencies = Vec::with_capacity(stats.len());
    let mut polished = 0_u64;
    // Time to insert, by chart day, and its parts over the chart and weeks.
    let mut day_waits: Vec<Vec<u64>> = vec![Vec::new(); CHART_DAYS];
    let (mut waits, mut held, mut quill) = (Vec::new(), Vec::new(), Vec::new());
    let (mut this_week, mut last_week) = (Vec::new(), Vec::new());
    let previous_week_start = week_start
        .checked_sub(jiff::Span::new().days(7))
        .unwrap_or(week_start);
    for s in stats {
        d.words += s.words;
        audio_ms += s.audio_ms;
        latencies.push(s.latency_ms);
        polished += u64::from(s.polished);
        let Some(at) = local(&s.created_at) else {
            continue;
        };
        let date = at.date();
        active_days.insert(date);
        if date >= week_start {
            d.words_last_7_days += s.words;
        }
        if let Some(i) = day_index(date) {
            days[i].words += s.words;
            days[i].dictations += 1;
            if let Some(wait) = s.wait_ms {
                day_waits[i].push(wait);
                waits.push(wait);
                held.push(wait.saturating_sub(s.latency_ms));
                quill.push(s.latency_ms.min(wait));
            }
        }
        if let Some(wait) = s.wait_ms {
            if date >= week_start {
                this_week.push(wait);
            } else if date >= previous_week_start {
                last_week.push(wait);
            }
        }
        hours[usize::try_from(at.hour()).unwrap_or(0) % 24] += 1;
    }
    let speaking_minutes = audio_ms as f64 / 60_000.0;
    if speaking_minutes >= 1.0 {
        d.speaking_wpm = Some(d.words as f64 / speaking_minutes);
    }
    d.minutes_saved = (d.words as f64 / TYPING_WPM - speaking_minutes).max(0.0);
    latencies.sort_unstable();
    d.median_latency_ms = latencies.get(latencies.len() / 2).copied();
    for (day, mut w) in days.iter_mut().zip(day_waits) {
        w.sort_unstable();
        day.wait_ms = percentile(&w, 0.5);
        day.wait_p90_ms = percentile(&w, 0.9);
    }
    for list in [
        &mut waits,
        &mut held,
        &mut quill,
        &mut this_week,
        &mut last_week,
    ] {
        list.sort_unstable();
    }
    if let (Some(median_ms), Some(p90_ms)) = (percentile(&waits, 0.5), percentile(&waits, 0.9)) {
        d.wait = Some(WaitInsights {
            count: u64::try_from(waits.len()).unwrap_or(u64::MAX),
            median_ms,
            p90_ms,
            held_ms: percentile(&held, 0.5).unwrap_or(0),
            quill_ms: percentile(&quill, 0.5).unwrap_or(0),
            last_7_days_ms: percentile(&this_week, 0.5),
            previous_7_days_ms: percentile(&last_week, 0.5),
        });
    }
    if d.count > 0 {
        d.polished_share = polished as f64 / d.count as f64;
    }

    // Meetings.
    let mut m = MeetingInsights {
        count: u64::try_from(meetings.len()).unwrap_or(u64::MAX),
        summaries,
        summary_cost_usd,
        ..MeetingInsights::default()
    };
    for meeting in meetings {
        m.total_ms += meeting.duration_ms;
        let Some(at) = local(&meeting.started_at) else {
            continue;
        };
        let date = at.date();
        active_days.insert(date);
        if date >= week_start {
            m.count_last_7_days += 1;
            m.ms_last_7_days += meeting.duration_ms;
        }
        if let Some(i) = day_index(date) {
            days[i].meeting_ms += meeting.duration_ms;
        }
    }
    m.average_ms = m.total_ms.checked_div(m.count);
    let mine: u64 = talk
        .iter()
        .filter(|(speaker, _)| speaker.as_deref() == Some(ME))
        .map(|(_, words)| words)
        .sum();
    let all: u64 = talk.iter().map(|(_, words)| words).sum();
    if all > 0 {
        m.my_talk_share = Some(mine as f64 / all as f64);
    }

    // Streak: count back from today, or from yesterday if today is empty so
    // far (the day isn't over).
    let mut streak = 0;
    let mut day = if active_days.contains(&today) {
        Some(today)
    } else {
        today.yesterday().ok()
    };
    while let Some(date) = day
        && active_days.contains(&date)
    {
        streak += 1;
        day = date.yesterday().ok();
    }

    Insights {
        dictation: d,
        meetings: m,
        streak_days: streak,
        days,
        hours,
        typing_wpm: TYPING_WPM,
        apps: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> Zoned {
        // A fixed zone so the test doesn't depend on the machine's.
        "2026-10-11T18:00:00-04:00[America/New_York]"
            .parse()
            .unwrap()
    }

    fn stat(at: &str, words: u64, audio_ms: u64, polished: bool) -> DictationStat {
        DictationStat {
            created_at: at.into(),
            words,
            audio_ms,
            latency_ms: 300,
            polished,
            pasted: true,
            app_id: None,
            app: None,
            wait_ms: None,
        }
    }

    fn meeting(at: &str, minutes: u64) -> Meeting {
        Meeting {
            id: at.into(),
            title: "m".into(),
            started_at: at.into(),
            duration_ms: minutes * 60_000,
            has_transcript: true,
            has_summary: true,
            empty: false,
        }
    }

    #[test]
    fn totals_speed_and_time_saved() {
        let stats = [
            // 150 words in a minute of speech, today (local).
            stat("2026-10-11T14:00:00Z", 150, 60_000, true),
            // 50 words in 30 s, three days ago.
            stat("2026-10-08T14:00:00Z", 50, 30_000, false),
            // Long ago: counts in totals, not in the week or the chart.
            stat("2026-01-01T14:00:00Z", 100, 40_000, false),
        ];
        let i = from_data(&stats, &[], &[], 0, 0.0, &now());

        assert_eq!(i.dictation.count, 3);
        assert_eq!(i.dictation.words, 300);
        assert_eq!(i.dictation.words_last_7_days, 200);
        // 300 words in 130 s of speech.
        let wpm = i.dictation.speaking_wpm.unwrap();
        assert!((wpm - 300.0 / (130.0 / 60.0)).abs() < 1e-9);
        // Typing 300 words at 40 wpm = 7.5 min; speaking took 2.17 min.
        assert!((i.dictation.minutes_saved - (7.5 - 130.0 / 60.0)).abs() < 1e-9);
        assert!((i.dictation.polished_share - 1.0 / 3.0).abs() < 1e-9);
        assert_eq!(i.days.len(), CHART_DAYS);
        assert_eq!(i.days.last().unwrap().date, "2026-10-11");
        assert_eq!(i.days.last().unwrap().words, 150);
        // 14:00 UTC is 10:00 in New York in October, 9:00 in January.
        assert_eq!((i.hours[10], i.hours[9]), (2, 1));
    }

    #[test]
    fn local_days_decide_the_chart_and_streak() {
        // 01:00 UTC on the 11th is still the 10th in New York.
        let stats = [stat("2026-10-11T01:00:00Z", 10, 5_000, false)];
        let meetings = [meeting("2026-10-09T15:00:00Z", 30)];
        let i = from_data(&stats, &meetings, &[], 0, 0.0, &now());

        assert_eq!(i.days[CHART_DAYS - 2].date, "2026-10-10");
        assert_eq!(i.days[CHART_DAYS - 2].words, 10);
        assert_eq!(i.days[CHART_DAYS - 3].meeting_ms, 30 * 60_000);
        // Nothing yet today, but the 10th and 9th were active.
        assert_eq!(i.streak_days, 2);
    }

    #[test]
    fn time_to_insert_by_day_with_its_parts_and_trend() {
        let timed = |at: &str, wait_ms: u64, latency_ms: u64| DictationStat {
            wait_ms: Some(wait_ms),
            latency_ms,
            ..stat(at, 10, 3_000, false)
        };
        let stats = [
            // Today: held 0.4 s after speaking, then Quill took 0.3 s.
            timed("2026-10-11T14:00:00Z", 700, 300),
            timed("2026-10-11T15:00:00Z", 900, 300),
            timed("2026-10-11T16:00:00Z", 2_000, 400),
            // Ten days ago: slower.
            timed("2026-10-01T15:00:00Z", 1_500, 500),
            // Before the change: no end of speech known.
            stat("2026-10-11T17:00:00Z", 10, 3_000, false),
        ];
        let i = from_data(&stats, &[], &[], 0, 0.0, &now());
        let wait = i.dictation.wait.expect("measured");
        assert_eq!(wait.count, 4);
        assert_eq!(wait.median_ms, 900);
        assert_eq!(wait.p90_ms, 2_000);
        assert_eq!(wait.held_ms, 600, "median of 400, 600, 1000, 1600");
        assert_eq!(wait.quill_ms, 300);
        assert_eq!(wait.last_7_days_ms, Some(900));
        assert_eq!(wait.previous_7_days_ms, Some(1_500));
        let today = i.days.last().unwrap();
        assert_eq!((today.wait_ms, today.wait_p90_ms), (Some(900), Some(2_000)));
        assert_eq!(i.days[i.days.len() - 2].wait_ms, None);

        let none = from_data(
            &[stat("2026-10-11T17:00:00Z", 10, 3_000, false)],
            &[],
            &[],
            0,
            0.0,
            &now(),
        );
        assert_eq!(none.dictation.wait, None);
    }

    #[test]
    fn meetings_and_talk_share() {
        let meetings = [
            meeting("2026-10-11T13:00:00Z", 30),
            meeting("2026-09-01T13:00:00Z", 60),
        ];
        let talk = [
            (Some("Me".to_string()), 300),
            (Some("Others".to_string()), 700),
        ];
        let i = from_data(&[], &meetings, &talk, 2, 0.0014, &now());

        assert_eq!(i.meetings.count, 2);
        assert_eq!(i.meetings.count_last_7_days, 1);
        assert_eq!(i.meetings.average_ms, Some(45 * 60_000));
        assert_eq!(i.meetings.my_talk_share, Some(0.3));
        assert_eq!(i.streak_days, 1);
        assert_eq!(i.dictation.speaking_wpm, None);
        assert_eq!(i.dictation.median_latency_ms, None);
    }

    #[test]
    fn stats_survive_clearing_the_history() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path(), &dir.path().join("recordings")).unwrap();
        lib.record_dictation_stats(&stat("2026-10-11T14:00:00Z", 12, 4_000, false))
            .unwrap();
        lib.clear_dictations().unwrap();
        assert_eq!(compute(&lib, &now()).unwrap().dictation.words, 12);
        lib.clear_dictation_stats().unwrap();
        assert_eq!(compute(&lib, &now()).unwrap().dictation.count, 0);
    }
}
