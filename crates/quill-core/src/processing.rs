//! Following a meeting through its write-up after the recording stops:
//! which steps it takes, how far each has got, how long is left, what's
//! just been heard, and what came out. The app turns this into its
//! progress events; nothing here touches the UI.
//!
//! Time left is a guess that improves as it goes: each step starts from a
//! typical speed for the recording's length, and once a transcription step
//! has made some progress, its measured pace replaces the guess for it and
//! for the transcription still to come.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::overview::{is_item, section};

/// The steps of a write-up, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    /// Loading the audio, removing the echo, loading the speech model.
    Prepare,
    /// Transcribing the microphone.
    You,
    /// Transcribing the call.
    Call,
    /// Telling the people on the call apart.
    Speakers,
    /// Writing the notes.
    Notes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    Active,
    Done,
    /// Not needed this time (no call audio, nothing said, setting off).
    Skipped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Step {
    pub kind: StepKind,
    pub status: StepStatus,
    /// How far through, 0–1, when it's known.
    pub fraction: Option<f64>,
    /// How long it took (done), or has taken so far (active).
    pub ms: Option<u64>,
}

/// A line heard during transcription, to show it's working.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Heard {
    /// From the microphone (you), or the call.
    pub you: bool,
    pub text: String,
}

/// What came out of a finished write-up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Outcome {
    pub words: usize,
    /// Everyone who spoke, you included.
    pub people: usize,
    pub action_items: usize,
    pub decisions: usize,
    /// Whether notes were written.
    pub notes: bool,
    /// How long the whole write-up took.
    pub took_ms: u64,
}

/// Where a write-up is, for the UI.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    pub steps: Vec<Step>,
    pub elapsed_ms: u64,
    /// Roughly how long is left, once there's enough to go on.
    pub eta_ms: Option<u64>,
    /// The last few lines heard, oldest first.
    pub heard: Vec<Heard>,
    pub outcome: Option<Outcome>,
}

/// How many heard lines a report carries.
const HEARD: usize = 3;
/// Time left isn't offered before this: the first seconds are all setup.
const SETTLE: Duration = Duration::from_millis(1500);

/// Follows one write-up.
#[derive(Debug, Clone)]
pub struct Tracker {
    started: Instant,
    /// The recording's length, which most steps scale with.
    audio_ms: u64,
    steps: Vec<(Step, Option<Instant>)>,
    heard: VecDeque<Heard>,
    /// Measured transcription pace: ms of work per ms of audio.
    pace: Option<f64>,
    outcome: Option<Outcome>,
}

impl Tracker {
    /// A write-up of `plan`'s steps for a recording `audio_ms` long.
    pub fn new(plan: &[StepKind], audio_ms: u64, now: Instant) -> Self {
        Self {
            started: now,
            audio_ms,
            steps: plan
                .iter()
                .map(|&kind| {
                    let step = Step {
                        kind,
                        status: StepStatus::Pending,
                        fraction: None,
                        ms: None,
                    };
                    (step, None)
                })
                .collect(),
            heard: VecDeque::new(),
            pace: None,
            outcome: None,
        }
    }

    fn step(&mut self, kind: StepKind) -> Option<&mut (Step, Option<Instant>)> {
        self.steps.iter_mut().find(|(s, _)| s.kind == kind)
    }

    /// `kind` begins, if it's still to come (a step already under way,
    /// done or skipped is left as it is). Steps before it that were still
    /// pending are skipped.
    pub fn start(&mut self, kind: StepKind, now: Instant) {
        let Some(at) = self.steps.iter().position(|(s, _)| s.kind == kind) else {
            return;
        };
        if self.steps[at].0.status != StepStatus::Pending {
            return;
        }
        for (step, _) in &mut self.steps[..at] {
            if step.status == StepStatus::Pending {
                step.status = StepStatus::Skipped;
            }
        }
        let (step, began) = &mut self.steps[at];
        step.status = StepStatus::Active;
        step.fraction = None;
        *began = Some(now);
    }

    /// `kind` is `fraction` (0–1) of the way through.
    pub fn progress(&mut self, kind: StepKind, fraction: f64, now: Instant) {
        let audio_ms = self.audio_ms;
        let Some((step, began)) = self.step(kind) else {
            return;
        };
        step.fraction = Some(fraction.clamp(0.0, 1.0));
        // A transcription's pace, once there's enough of it to trust.
        if matches!(kind, StepKind::You | StepKind::Call)
            && let Some(began) = began
            && fraction >= 0.08
            && audio_ms > 0
        {
            let spent = now.duration_since(*began).as_millis() as f64;
            if spent >= 800.0 {
                self.pace = Some(spent / fraction / audio_ms as f64);
            }
        }
    }

    pub fn finish(&mut self, kind: StepKind, now: Instant) {
        if let Some((step, began)) = self.step(kind) {
            step.status = StepStatus::Done;
            step.fraction = Some(1.0);
            step.ms = Some(began.map_or(0, |b| now.duration_since(b).as_millis() as u64));
        }
    }

    /// `kind` isn't needed after all (pending), or was given up on
    /// (active): telling speakers apart is a bonus, for one.
    pub fn skip(&mut self, kind: StepKind) {
        if let Some((step, _)) = self.step(kind)
            && matches!(step.status, StepStatus::Pending | StepStatus::Active)
        {
            step.status = StepStatus::Skipped;
            step.fraction = None;
        }
    }

    /// The step under way, if any.
    pub fn active(&self) -> Option<&Step> {
        self.steps
            .iter()
            .map(|(s, _)| s)
            .find(|s| s.status == StepStatus::Active)
    }

    /// The step under way failed.
    pub fn fail(&mut self, now: Instant) {
        if let Some((step, began)) = self
            .steps
            .iter_mut()
            .find(|(s, _)| s.status == StepStatus::Active)
        {
            step.status = StepStatus::Failed;
            step.ms = began.map(|b| now.duration_since(b).as_millis() as u64);
        }
    }

    /// A line was recognised. Bracketed non-speech ("[BLANK_AUDIO]",
    /// "(music)") is left out.
    pub fn heard(&mut self, you: bool, text: &str) {
        let text = text.trim();
        let noise = (text.starts_with('[') && text.ends_with(']'))
            || (text.starts_with('(') && text.ends_with(')'));
        if text.is_empty() || noise {
            return;
        }
        self.heard.push_back(Heard {
            you,
            text: text.to_string(),
        });
        while self.heard.len() > HEARD {
            self.heard.pop_front();
        }
    }

    /// Finished: what came out, from the transcript and the notes.
    pub fn done(&mut self, outcome: impl FnOnce(u64) -> Outcome, now: Instant) {
        // Anything never reached wasn't needed.
        for (step, _) in &mut self.steps {
            if step.status == StepStatus::Pending {
                step.status = StepStatus::Skipped;
            }
        }
        self.heard.clear();
        self.outcome = Some(outcome(now.duration_since(self.started).as_millis() as u64));
    }

    pub fn report(&self, now: Instant) -> Report {
        let elapsed = now.duration_since(self.started);
        let steps = self
            .steps
            .iter()
            .map(|(step, began)| {
                let mut step = step.clone();
                if step.status == StepStatus::Active {
                    step.ms = began.map(|b| now.duration_since(b).as_millis() as u64);
                }
                step
            })
            .collect();
        let eta_ms = (self.outcome.is_none() && elapsed >= SETTLE)
            .then(|| self.left(now))
            .flatten();
        Report {
            steps,
            elapsed_ms: elapsed.as_millis() as u64,
            eta_ms,
            heard: self.heard.iter().cloned().collect(),
            outcome: self.outcome.clone(),
        }
    }

    /// The time left: what the step under way still needs, plus the steps
    /// still to come at their expected length.
    fn left(&self, now: Instant) -> Option<u64> {
        let mut left = 0.0;
        for (step, began) in &self.steps {
            let expected = self.expected(step.kind);
            match step.status {
                StepStatus::Pending => left += expected,
                StepStatus::Active => {
                    let spent = began.map_or(0.0, |b| now.duration_since(b).as_millis() as f64);
                    let total = match step.fraction {
                        // Measured: the pace so far.
                        Some(f) if f >= 0.08 => spent / f,
                        // Not yet: the guess, but never less than it's
                        // already taken plus a little.
                        _ => expected.max(spent * 1.15),
                    };
                    left += (total - spent).max(0.0);
                }
                StepStatus::Failed => return None,
                StepStatus::Done | StepStatus::Skipped => {}
            }
        }
        Some(left.round() as u64)
    }

    /// How long a step usually takes for this recording (ms).
    fn expected(&self, kind: StepKind) -> f64 {
        let audio = self.audio_ms as f64;
        let hours = audio / 3_600_000.0;
        match kind {
            // Echo removal runs at hundreds of times real time; loading the
            // model is a second or two the first time.
            StepKind::Prepare => 1_500.0 + audio * 0.004,
            // Whisper turbo on a recent GPU, skipping silence: about 25x
            // real time for a track, until measured.
            StepKind::You | StepKind::Call => audio * self.pace.unwrap_or(0.04),
            // About 2 minutes per hour of audio, plus loading the models.
            StepKind::Speakers => 3_000.0 + audio * 0.035,
            // One model call; long meetings are summarized in parts.
            StepKind::Notes => 12_000.0 + hours * 15_000.0,
        }
    }
}

/// Counts what a write-up produced, for [`Outcome`]: words and people from
/// the transcript, action items and decisions from the notes.
pub fn outcome(segments: &[quill_stt::Segment], notes: Option<&str>, took_ms: u64) -> Outcome {
    let words = segments
        .iter()
        .map(|s| s.text.split_whitespace().count())
        .sum();
    let mut people: Vec<&str> = Vec::new();
    for s in segments {
        let who = s.speaker.as_deref().unwrap_or(crate::meeting::OTHERS);
        if !people.contains(&who) {
            people.push(who);
        }
    }
    let items = |title: &str| {
        notes
            .and_then(|md| section(md, title))
            .map_or(0, |s| s.lines().filter(|l| is_item(l)).count())
    };
    Outcome {
        words,
        people: people.len(),
        action_items: items("action items"),
        decisions: items("decisions"),
        notes: notes.is_some(),
        took_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAN: [StepKind; 5] = [
        StepKind::Prepare,
        StepKind::You,
        StepKind::Call,
        StepKind::Speakers,
        StepKind::Notes,
    ];

    fn status(t: &Tracker, now: Instant) -> Vec<StepStatus> {
        t.report(now).steps.iter().map(|s| s.status).collect()
    }

    #[test]
    fn steps_move_along_and_skip_what_is_not_needed() {
        let t0 = Instant::now();
        let mut t = Tracker::new(&PLAN, 60_000, t0);
        t.start(StepKind::Prepare, t0);
        t.finish(StepKind::Prepare, t0 + Duration::from_secs(1));
        // No call audio: straight from you to notes.
        t.start(StepKind::You, t0 + Duration::from_secs(1));
        t.finish(StepKind::You, t0 + Duration::from_secs(3));
        t.start(StepKind::Notes, t0 + Duration::from_secs(3));
        use StepStatus::*;
        assert_eq!(
            status(&t, t0 + Duration::from_secs(4)),
            [Done, Done, Skipped, Skipped, Active]
        );
        let report = t.report(t0 + Duration::from_secs(4));
        assert_eq!(report.steps[1].ms, Some(2_000));
        assert_eq!(
            report.steps[4].ms,
            Some(1_000),
            "active steps show time so far"
        );
        // Starting a step that's done, skipped or under way changes nothing.
        t.start(StepKind::You, t0 + Duration::from_secs(5));
        t.start(StepKind::Call, t0 + Duration::from_secs(5));
        assert_eq!(
            status(&t, t0 + Duration::from_secs(5)),
            [Done, Done, Skipped, Skipped, Active]
        );
    }

    #[test]
    fn time_left_follows_the_measured_pace() {
        let t0 = Instant::now();
        // A one-hour meeting.
        let mut t = Tracker::new(&PLAN, 3_600_000, t0);
        t.start(StepKind::Prepare, t0);
        t.finish(StepKind::Prepare, t0 + Duration::from_secs(2));
        t.start(StepKind::You, t0 + Duration::from_secs(2));
        // A quarter of the mic track in 30 s: 120 s for the track, so the
        // call track should take about as long again.
        let now = t0 + Duration::from_secs(32);
        t.progress(StepKind::You, 0.25, now);
        let left = t.report(now).eta_ms.unwrap() as f64;
        let expected = 90_000.0 + 120_000.0 + (3_000.0 + 3_600_000.0 * 0.035) + 27_000.0;
        assert!((left - expected).abs() < 1_000.0, "{left} vs {expected}");
    }

    #[test]
    fn no_time_left_until_settled_or_after_failure() {
        let t0 = Instant::now();
        let mut t = Tracker::new(&PLAN, 60_000, t0);
        t.start(StepKind::Prepare, t0);
        assert_eq!(t.report(t0 + Duration::from_millis(500)).eta_ms, None);
        assert!(t.report(t0 + Duration::from_secs(2)).eta_ms.is_some());
        t.fail(t0 + Duration::from_secs(2));
        assert_eq!(t.report(t0 + Duration::from_secs(3)).eta_ms, None);
        assert_eq!(
            t.report(t0 + Duration::from_secs(3)).steps[0].status,
            StepStatus::Failed
        );
    }

    #[test]
    fn keeps_the_last_few_lines_without_noise() {
        let mut t = Tracker::new(&PLAN, 60_000, Instant::now());
        for line in ["one", "[BLANK_AUDIO]", "two", "(music)", "three", "four"] {
            t.heard(true, line);
        }
        let heard: Vec<_> = t
            .report(Instant::now())
            .heard
            .into_iter()
            .map(|h| h.text)
            .collect();
        assert_eq!(heard, ["two", "three", "four"]);
    }

    #[test]
    fn counts_what_came_out() {
        let seg = |speaker: &str, text: &str| quill_stt::Segment {
            start_ms: 0,
            end_ms: 1,
            text: text.into(),
            speaker: Some(speaker.into()),
        };
        let segments = [
            seg("Me", "Let's ship it Friday."),
            seg("Speaker 1", "Agreed, behind a flag."),
            seg("Me", "Great."),
        ];
        let notes = "# Sync\n\n## Decisions\n- Ship Friday\n- Flag at 10%\n\n## Action items\n- You: plan\n- Priya: fix\n- Dan: variants\n";
        let o = outcome(&segments, Some(notes), 42_000);
        assert_eq!(
            o,
            Outcome {
                words: 9,
                people: 2,
                action_items: 3,
                decisions: 2,
                notes: true,
                took_ms: 42_000,
            }
        );
        assert_eq!(outcome(&segments, None, 1).action_items, 0);
    }
}
