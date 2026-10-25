//! `quill-cli voices`: recognising the same people across recordings
//! ("known voices"). `match` compares two recordings; `eval` measures how
//! often people are recognised, and wrongly named, on audio with reference
//! RTTMs such as AMI (eval/diarization).
//!
//! Neither writes anything: a recording folder's cached analysis is read if
//! it's there, and otherwise computed in memory.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use quill_core::speakers;
use quill_diarize::cluster::ClusterOptions;
use quill_diarize::rttm::{self, NamedTurn};
use quill_diarize::voices::{self, MatchOptions, Voice};
use quill_diarize::{Analysis, Diarizer, DiarizerConfig, Turn};

#[derive(Debug, clap::Subcommand)]
pub enum Command {
    /// Compare the people in two recordings by voice, and show who in B
    /// would be recognised as someone from A
    Match(MatchArgs),
    /// Measure recognition across recordings with reference RTTMs (e.g.
    /// AMI): how many people are recognised, and how many wrongly named,
    /// at a range of thresholds
    Eval(EvalArgs),
}

#[derive(Debug, clap::Args)]
pub struct MatchArgs {
    /// The recording whose people are "known": a recording folder or an
    /// audio file
    a: PathBuf,
    /// The recording to recognise them in
    b: PathBuf,
    /// Least similarity for a match
    #[arg(long, default_value_t = voices::DEFAULT_MIN_SIMILARITY)]
    min_similarity: f32,
    /// Least speech, in seconds, behind a voice for it to be matched
    #[arg(long, default_value_t = voices::DEFAULT_MIN_SPEECH_MS as f64 / 1000.0)]
    min_speech: f64,
}

#[derive(Debug, clap::Args)]
pub struct EvalArgs {
    /// Audio files to learn voices from, each with a reference RTTM beside
    /// it (same name, .rttm). RTTM speaker names must identify people
    /// across files, as AMI's do
    #[arg(required = true)]
    files: Vec<PathBuf>,
    /// Audio files to recognise people in [default: the same files]. A file
    /// is never matched against voices learned from the same meeting (the
    /// RTTM's file id), so a re-encoded copy can be tested here
    #[arg(long, num_args = 1..)]
    test: Vec<PathBuf>,
    /// Folder to cache each file's analysis in, so later runs are instant
    #[arg(long)]
    cache: Option<PathBuf>,
    /// Least speech behind a voice, in seconds, to try (comma-separated)
    #[arg(long, value_delimiter = ',', default_value = "15,30,45")]
    min_speech: Vec<f64>,
    /// Voice-embedding model (see `quill-cli models list`)
    #[arg(long, default_value = quill_diarize::DEFAULT_VOICE_MODEL)]
    voice: String,
}

pub fn run(command: Command, models_dir: &Path) -> anyhow::Result<()> {
    match command {
        Command::Match(args) => run_match(args, models_dir),
        Command::Eval(args) => run_eval(args, models_dir),
    }
}

/// Loads the models only when a recording needs analysing.
struct Models<'a> {
    dir: &'a Path,
    voice: String,
    diarizer: Option<Diarizer>,
}

impl Models<'_> {
    fn diarizer(&mut self) -> anyhow::Result<&Diarizer> {
        if self.diarizer.is_none() {
            let segmentation = crate::models::download_with_progress(
                &quill_stt::models::SEGMENTATION_MODEL,
                self.dir,
            )?;
            if !self.voice.starts_with("voice-") {
                bail!("`{}` isn't a voice model", self.voice);
            }
            let voice = crate::models::download_with_progress(
                crate::models::lookup(&self.voice)?,
                self.dir,
            )?;
            self.diarizer = Some(Diarizer::load(&DiarizerConfig::new(segmentation, voice))?);
        }
        self.diarizer.as_ref().context("models not loaded")
    }

    /// The analysis of an audio file or a recording folder's system track.
    fn analysis(&mut self, path: &Path, cache: Option<&Path>) -> anyhow::Result<Analysis> {
        if path.is_dir()
            && let Some(analysis) = speakers::cached_analysis(path, &self.voice)
        {
            return Ok(analysis);
        }
        let cached = cache.map(|dir| {
            let stem = path
                .file_stem()
                .map_or_else(|| "audio".into(), |s| s.to_string_lossy());
            dir.join(format!("{stem}.{}.json", self.voice))
        });
        if let Some(file) = &cached
            && let Ok(json) = std::fs::read_to_string(file)
            && let Ok(analysis) = serde_json::from_str(&json)
        {
            return Ok(analysis);
        }
        let pcm = if path.is_dir() {
            quill_core::meeting::load_system_track(path)?
                .with_context(|| format!("no system-audio track in {}", path.display()))?
        } else {
            quill_audio::load_for_stt(path)
                .with_context(|| format!("reading {}", path.display()))?
        };
        eprintln!("analysing {}…", path.display());
        let analysis = self.diarizer()?.analyze(&pcm)?;
        if let Some(file) = &cached {
            if let Some(dir) = file.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(file, serde_json::to_string(&analysis)?)?;
        }
        Ok(analysis)
    }
}

/// A recording's people on the call, labelled as the app would (from
/// `transcript.json` in a recording folder) or by order of first turn.
fn people(path: &Path, analysis: &Analysis) -> Vec<(String, Voice)> {
    if path.is_dir()
        && let Some(track) = speakers::load_track_transcript(path)
    {
        let labelled =
            speakers::label_and_recognize(&track, analysis, None, &[], &MatchOptions::default());
        return labelled
            .voices
            .into_iter()
            .map(|v| (v.label, v.voice))
            .collect();
    }
    let (_, voices) = analysis.speakers_with_voices(&ClusterOptions::new(None));
    voices
        .into_iter()
        .enumerate()
        .map(|(i, v)| (speakers::speaker_label(i), v))
        .collect()
}

fn run_match(args: MatchArgs, models_dir: &Path) -> anyhow::Result<()> {
    let mut models = Models {
        dir: models_dir,
        voice: speakers::VOICE_MODEL.into(),
        diarizer: None,
    };
    let a = people(&args.a, &models.analysis(&args.a, None)?);
    let b = people(&args.b, &models.analysis(&args.b, None)?);
    let options = MatchOptions {
        min_similarity: args.min_similarity,
        min_speech_ms: (args.min_speech * 1000.0) as u64,
    };
    let known: Vec<Vec<f32>> = a
        .iter()
        .filter(|(_, v)| v.speech_ms >= options.min_speech_ms)
        .map(|(_, v)| v.embedding.clone())
        .collect();
    let known_names: Vec<&str> = a
        .iter()
        .filter(|(_, v)| v.speech_ms >= options.min_speech_ms)
        .map(|(n, _)| n.as_str())
        .collect();
    let b_voices: Vec<Voice> = b.iter().map(|(_, v)| v.clone()).collect();
    let matches = voices::match_voices(&b_voices, &known, &options);

    print!("{:<16}", "B \\ A");
    for (name, voice) in &a {
        print!(
            " {:>14}",
            format!("{name} {:.0}s", voice.speech_ms as f64 / 1000.0)
        );
    }
    println!("   recognised as");
    for ((name, voice), m) in b.iter().zip(&matches) {
        print!(
            "{:<16}",
            format!("{name} {:.0}s", voice.speech_ms as f64 / 1000.0)
        );
        for (_, known) in &a {
            print!(" {:>14.3}", voice.similarity(&known.embedding));
        }
        match m {
            Some(m) => println!("   {}", known_names[m.known]),
            None if voice.speech_ms < options.min_speech_ms => println!("   (too little speech)"),
            None => println!("   -"),
        }
    }
    Ok(())
}

/// One person on the call in an eval recording.
struct Heard {
    /// The reference speaker they mostly are.
    who: Option<String>,
    /// How much of their speech is `who`'s (0–1): diarization sometimes
    /// merges two people into one speaker.
    purity: f64,
    voice: Voice,
}

/// A speaker at least this pure is one person, as far as naming them goes.
const PURE: f64 = 0.8;

struct Recording {
    /// The meeting (RTTM file id): copies of one meeting share it.
    meeting: String,
    people: BTreeSet<String>,
    heard: Vec<Heard>,
}

fn load_recording(
    models: &mut Models,
    path: &Path,
    cache: Option<&Path>,
) -> anyhow::Result<Recording> {
    let rttm_path = path.with_extension("rttm");
    let text = std::fs::read_to_string(&rttm_path)
        .with_context(|| format!("reading {}", rttm_path.display()))?;
    let reference =
        rttm::parse(&text).map_err(|e| anyhow::anyhow!("{}: {e}", rttm_path.display()))?;
    let meeting = text
        .lines()
        .find_map(|l| l.split_whitespace().nth(1).map(str::to_owned))
        .unwrap_or_else(|| path.display().to_string());
    let analysis = models.analysis(path, cache)?;
    let (turns, voices) = analysis.speakers_with_voices(&ClusterOptions::new(None));
    let heard = voices
        .into_iter()
        .enumerate()
        .map(|(i, voice)| {
            let (who, purity) = mostly(&turns, i, &reference);
            Heard { who, purity, voice }
        })
        .collect();
    Ok(Recording {
        meeting,
        people: reference.iter().map(|t| t.speaker.clone()).collect(),
        heard,
    })
}

/// The reference speaker who talks most during `speaker`'s turns, and
/// their share of the reference speech there.
fn mostly(turns: &[Turn], speaker: usize, reference: &[NamedTurn]) -> (Option<String>, f64) {
    let mut overlap: BTreeMap<&str, u64> = BTreeMap::new();
    for t in turns.iter().filter(|t| t.speaker == speaker) {
        for r in reference {
            let o = t
                .end_ms
                .min(r.end_ms)
                .saturating_sub(t.start_ms.max(r.start_ms));
            if o > 0 {
                *overlap.entry(&r.speaker).or_default() += o;
            }
        }
    }
    let total: u64 = overlap.values().sum();
    overlap
        .into_iter()
        .max_by_key(|&(_, ms)| ms)
        .map_or((None, 0.0), |(who, ms)| {
            (Some(who.to_owned()), ms as f64 / total.max(1) as f64)
        })
}

/// Known voices learned from `from`, as if the user had named everyone with
/// enough speech: per person, each meeting's voice (speakers who are the
/// same person merged, as typing the same name does), averaged over
/// meetings.
fn learn(from: &[&Recording], min_speech_ms: u64) -> Vec<(String, Vec<f32>)> {
    let mut samples: BTreeMap<String, Vec<Voice>> = BTreeMap::new();
    for rec in from {
        let mut per_person: BTreeMap<&str, Vec<&Voice>> = BTreeMap::new();
        for h in &rec.heard {
            if let Some(who) = &h.who {
                per_person.entry(who).or_default().push(&h.voice);
            }
        }
        for (who, vs) in per_person {
            if let Some(v) = voices::average(vs)
                && v.speech_ms >= min_speech_ms
            {
                samples.entry(who.to_owned()).or_default().push(v);
            }
        }
    }
    samples
        .into_iter()
        .filter_map(|(who, vs)| voices::average(&vs).map(|v| (who, v.embedding)))
        .collect()
}

#[derive(Default, Clone, Copy)]
struct Tally {
    /// Speakers whose person was known: named right, named wrong, unnamed.
    known: u64,
    right: u64,
    wrong: u64,
    /// Speakers whose person wasn't known, and of those, how many got a name.
    strangers: u64,
    strangers_named: u64,
    /// Wrong names given to speakers who are mostly one person (not two
    /// people merged by diarization).
    wrong_pure: u64,
}

impl Tally {
    fn add(&mut self, other: &Tally) {
        self.known += other.known;
        self.right += other.right;
        self.wrong += other.wrong;
        self.strangers += other.strangers;
        self.strangers_named += other.strangers_named;
        self.wrong_pure += other.wrong_pure;
    }

    fn names_given(&self) -> u64 {
        self.right + self.wrong + self.strangers_named
    }
}

fn pct(n: u64, of: u64) -> String {
    if of == 0 {
        "    -".into()
    } else {
        format!("{:>5.1}", n as f64 * 100.0 / of as f64)
    }
}

/// Matches `test`'s speakers against `known` and scores the names given.
fn score(test: &Recording, known: &[(String, Vec<f32>)], options: &MatchOptions) -> Tally {
    let embeddings: Vec<Vec<f32>> = known.iter().map(|(_, e)| e.clone()).collect();
    let heard: Vec<Voice> = test.heard.iter().map(|h| h.voice.clone()).collect();
    let matches = voices::match_voices(&heard, &embeddings, options);
    let mut t = Tally::default();
    for (h, m) in test.heard.iter().zip(matches) {
        if h.voice.speech_ms < options.min_speech_ms {
            continue;
        }
        let is_known = h
            .who
            .as_ref()
            .is_some_and(|w| known.iter().any(|(k, _)| k == w));
        let named = m.map(|m| &known[m.known].0);
        if named.is_some_and(|n| Some(n) != h.who.as_ref()) && h.purity >= PURE {
            t.wrong_pure += 1;
        }
        if is_known {
            t.known += 1;
            match named {
                Some(n) if Some(n) == h.who.as_ref() => t.right += 1,
                Some(_) => t.wrong += 1,
                None => {}
            }
        } else {
            t.strangers += 1;
            if named.is_some() {
                t.strangers_named += 1;
            }
        }
    }
    t
}

fn run_eval(args: EvalArgs, models_dir: &Path) -> anyhow::Result<()> {
    let mut models = Models {
        dir: models_dir,
        voice: args.voice.clone(),
        diarizer: None,
    };
    let cache = args.cache.as_deref();
    let enroll: Vec<Recording> = args
        .files
        .iter()
        .map(|f| load_recording(&mut models, f, cache))
        .collect::<anyhow::Result<_>>()?;
    let test: Vec<Recording> = if args.test.is_empty() {
        Vec::new()
    } else {
        args.test
            .iter()
            .map(|f| load_recording(&mut models, f, cache))
            .collect::<anyhow::Result<_>>()?
    };
    let test: &[Recording] = if test.is_empty() { &enroll } else { &test };
    let people: BTreeSet<&String> = enroll.iter().flat_map(|r| &r.people).collect();
    println!(
        "{} recordings to learn from ({} people), {} to recognise in\n",
        enroll.len(),
        people.len(),
        test.len()
    );

    // Similarity of each speaker to the voice learned for them from another
    // meeting, and to the closest other person's, at the default speech
    // minimum: what the threshold has to separate.
    let min = voices::DEFAULT_MIN_SPEECH_MS;
    let mut same = Vec::new();
    let mut other = Vec::new();
    for t in test {
        let others: Vec<&Recording> = enroll.iter().filter(|e| e.meeting != t.meeting).collect();
        let known = learn(&others, min);
        for h in t.heard.iter().filter(|h| h.voice.speech_ms >= min) {
            let mut closest_other = f32::MIN;
            for (who, e) in &known {
                let sim = h.voice.similarity(e);
                if Some(who) == h.who.as_ref() {
                    same.push(sim);
                } else {
                    closest_other = closest_other.max(sim);
                }
            }
            if closest_other > f32::MIN {
                other.push(closest_other);
            }
        }
    }
    let quantiles = |v: &mut Vec<f32>| {
        v.sort_by(f32::total_cmp);
        let q = |p: f64| {
            v.get(((v.len() - 1) as f64 * p).round() as usize)
                .copied()
                .unwrap_or(0.0)
        };
        format!(
            "n={:<4} min {:.3}  5% {:.3}  median {:.3}  95% {:.3}  max {:.3}",
            v.len(),
            q(0.0),
            q(0.05),
            q(0.5),
            q(0.95),
            q(1.0)
        )
    };
    if !same.is_empty() && !other.is_empty() {
        println!(
            "Similarity to the person's own voice:  {}",
            quantiles(&mut same)
        );
        println!(
            "To the closest other person's voice:   {}\n",
            quantiles(&mut other)
        );
    }

    // Scenarios: who Quill knows when it meets each test recording.
    type Scenario = fn(&Recording, &Recording) -> bool;
    let scenarios: [(&str, Scenario, bool); 4] = [
        // Each other meeting on its own, with the same people…
        (
            "one meeting, same people",
            |e, t| e.meeting != t.meeting && !e.people.is_disjoint(&t.people),
            true,
        ),
        // …or with strangers only.
        (
            "one meeting, strangers",
            |e, t| e.people.is_disjoint(&t.people),
            true,
        ),
        // Everyone from every other meeting.
        ("everyone else", |e, t| e.meeting != t.meeting, false),
        // Everyone from meetings without any of these people.
        (
            "strangers only",
            |e, t| e.people.is_disjoint(&t.people),
            false,
        ),
    ];
    let thresholds: Vec<f32> = (6..=17).map(|i| i as f32 * 0.05).collect();
    for &min_speech in &args.min_speech {
        let min_speech_ms = (min_speech * 1000.0) as u64;
        println!(
            "Least speech {min_speech} s. Per scenario: % of known people named right / wrong; % of strangers named"
        );
        print!("thresh");
        for (name, _, _) in &scenarios {
            print!(" | {name:^23}");
        }
        println!(
            " | wrong names: share of all given (count; of those, on speakers at least 80% one person)"
        );
        let mut rows: Vec<(f32, Vec<Tally>)> = thresholds
            .iter()
            .map(|&t| (t, vec![Tally::default(); scenarios.len()]))
            .collect();
        for t in test {
            for (s, (_, include, pairwise)) in scenarios.iter().enumerate() {
                let sources: Vec<&Recording> = enroll.iter().filter(|e| include(e, t)).collect();
                let known_sets: Vec<Vec<(String, Vec<f32>)>> = if *pairwise {
                    sources.iter().map(|e| learn(&[e], min_speech_ms)).collect()
                } else {
                    vec![learn(&sources, min_speech_ms)]
                };
                for known in &known_sets {
                    for (threshold, tallies) in &mut rows {
                        let options = MatchOptions {
                            min_similarity: *threshold,
                            min_speech_ms,
                        };
                        tallies[s].add(&score(t, known, &options));
                    }
                }
            }
        }
        for (threshold, tallies) in &rows {
            print!("{threshold:>6.2}");
            let mut all = Tally::default();
            for t in tallies {
                all.add(t);
                print!(
                    " | {}/{} {}  (n={:<4})",
                    pct(t.right, t.known),
                    pct(t.wrong, t.known).trim(),
                    pct(t.strangers_named, t.strangers),
                    t.known + t.strangers
                );
            }
            println!(
                " | {} ({} of {}; {} on one-person speakers)",
                pct(all.wrong + all.strangers_named, all.names_given()),
                all.wrong + all.strangers_named,
                all.names_given(),
                all.wrong_pure
            );
        }
        println!();
    }
    Ok(())
}
