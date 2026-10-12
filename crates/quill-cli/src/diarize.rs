//! `quill-cli diarize`: who spoke when in one audio file, optionally scored
//! against a reference RTTM.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, bail};
use clap::ValueEnum;
use quill_diarize::metrics::{self, Report};
use quill_diarize::{Diarizer, DiarizerConfig, Turn, rttm};

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Audio file (WAV or FLAC, any rate), or a recording directory to
    /// diarize its system-audio track
    file: PathBuf,
    /// Number of speakers, if known [default: estimate it]
    #[arg(long, short)]
    speakers: Option<usize>,
    /// Voice-embedding model (see `quill-cli models list`)
    #[arg(long, default_value = quill_diarize::DEFAULT_VOICE_MODEL)]
    voice: String,
    /// Clustering threshold when --speakers isn't given; lower finds more
    /// speakers
    #[arg(long, default_value_t = quill_diarize::DEFAULT_THRESHOLD)]
    threshold: f32,
    /// Who clusters the voice embeddings
    #[arg(long, value_enum, default_value_t = ClusteringArg::Quill)]
    clustering: ClusteringArg,
    /// Quill clustering: least speech, in seconds, for a group to count as a
    /// person rather than an outlier
    #[arg(long, default_value_t = quill_diarize::DEFAULT_MIN_SPEAKER_MS as f64 / 1000.0)]
    min_speaker: f64,
    /// Segmentation window step as a fraction of its 10 s; larger is faster
    #[arg(long, default_value_t = quill_diarize::DEFAULT_WINDOW_SHIFT)]
    window_shift: f32,
    /// CPU threads per model [default: up to 8]
    #[arg(long)]
    threads: Option<usize>,
    /// Reference RTTM to score against
    #[arg(long)]
    reference: Option<PathBuf>,
    /// With --reference: run the models once, then score Quill clustering
    /// at a range of thresholds and with the true speaker count
    #[arg(long, requires = "reference")]
    sweep: bool,
    /// Output format
    #[arg(long, short, value_enum, default_value_t = Format::Text)]
    format: Format,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ClusteringArg {
    Sherpa,
    Quill,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Format {
    /// Speaker summary (and the score, with --reference)
    Text,
    /// RTTM turns
    Rttm,
    /// One JSON object with the turns and the score
    Json,
}

pub fn run(args: Args, models_dir: &Path) -> anyhow::Result<()> {
    let audio_path = if args.file.is_dir() {
        [
            quill_core::meeting::SYSTEM_FLAC,
            quill_core::meeting::SYSTEM_FILE,
        ]
        .into_iter()
        .map(|name| args.file.join(name))
        .find(|p| p.is_file())
        .with_context(|| format!("no system-audio track in {}", args.file.display()))?
    } else {
        args.file.clone()
    };
    let reference = args
        .reference
        .as_deref()
        .map(|path| -> anyhow::Result<_> {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?;
            rttm::parse(&text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))
        })
        .transpose()?;

    let segmentation =
        crate::models::download_with_progress(&quill_stt::models::SEGMENTATION_MODEL, models_dir)?;
    let voice_model = crate::models::lookup(&args.voice)?;
    if !args.voice.starts_with("voice-") {
        bail!(
            "`{}` isn't a voice model; pick one starting with `voice-`",
            args.voice
        );
    }
    let voice = crate::models::download_with_progress(voice_model, models_dir)?;

    let pcm = quill_audio::load_for_stt(&audio_path)
        .with_context(|| format!("reading {}", audio_path.display()))?;
    let audio_secs = pcm.len() as f64 / f64::from(quill_diarize::SAMPLE_RATE);

    let started = Instant::now();
    let mut config = DiarizerConfig::new(segmentation, voice);
    config.threshold = args.threshold;
    config.clustering = match args.clustering {
        ClusteringArg::Sherpa => quill_diarize::Clustering::Sherpa,
        ClusteringArg::Quill => quill_diarize::Clustering::Quill,
    };
    config.min_speaker_ms = (args.min_speaker * 1000.0) as u64;
    config.window_shift = args.window_shift;
    if let Some(threads) = args.threads {
        config.threads = threads;
    }
    let diarizer = Diarizer::load(&config)?;
    let load_secs = started.elapsed().as_secs_f64();
    let started = Instant::now();
    if args.sweep
        && let Some(reference) = &reference
    {
        let analysis = diarizer.analyze(&pcm)?;
        let secs = started.elapsed().as_secs_f64();
        let truth = metrics::score(reference, &[], 0).reference_speakers;
        println!("setting      found    DER   conf  collar");
        let mut settings: Vec<(String, f32, Option<usize>)> = (3..=16)
            .map(|i| {
                let t = i as f32 * 0.05;
                (format!("t={t:.2}"), t, None)
            })
            .collect();
        settings.push(("known".into(), args.threshold, Some(truth)));
        for (name, threshold, speakers) in settings {
            let turns = analysis.speakers(&quill_diarize::cluster::ClusterOptions {
                threshold,
                speakers,
                min_speaker_ms: config.min_speaker_ms,
            });
            let (strict, collar) = (
                metrics::score(reference, &turns, 0),
                metrics::score(reference, &turns, 250),
            );
            println!(
                "{name:<10} {:>3}/{truth:<3} {:>5.1}% {:>5.1}% {:>6.1}%",
                quill_diarize::speaker_count(&turns),
                strict.der() * 100.0,
                strict.confusion() * 100.0,
                collar.der() * 100.0,
            );
        }
        eprintln!("\n{audio_secs:.1}s audio · models {secs:.2}s");
        return Ok(());
    }
    let turns = diarizer.diarize(&pcm, args.speakers)?;
    let secs = started.elapsed().as_secs_f64();

    let scores = reference.as_ref().map(|reference| {
        (
            metrics::score(reference, &turns, 0),
            metrics::score(reference, &turns, 250),
        )
    });

    match args.format {
        Format::Rttm => {
            let id = audio_path
                .file_stem()
                .map_or_else(|| "audio".into(), |s| s.to_string_lossy());
            print!("{}", rttm::format(&id, &turns, speaker_name));
        }
        Format::Json => {
            let json = serde_json::json!({
                "file": audio_path,
                "voice": args.voice,
                "threshold": args.threshold,
                "speakers_given": args.speakers,
                "speakers_found": quill_diarize::speaker_count(&turns),
                "audio_secs": audio_secs,
                "diarize_secs": secs,
                "turns": turns.iter().map(|t| serde_json::json!([t.start_ms, t.end_ms, t.speaker])).collect::<Vec<_>>(),
                "score": scores.as_ref().map(|(strict, collar)| serde_json::json!({
                    "reference_speakers": strict.reference_speakers,
                    "der": strict.der(),
                    "missed": strict.missed(),
                    "false_alarm": strict.false_alarm(),
                    "confusion": strict.confusion(),
                    "der_collar_250ms": collar.der(),
                    "speakers": strict.speakers.iter().map(|s| serde_json::json!({
                        "name": s.name,
                        "talk_ms": s.talk_ms,
                        "matched": s.matched,
                        "coverage": s.coverage(),
                        "mostly": s.mostly,
                    })).collect::<Vec<_>>(),
                })),
            });
            println!("{}", serde_json::to_string_pretty(&json)?);
        }
        Format::Text => {
            print_speakers(&turns);
            if let Some((strict, collar)) = &scores {
                println!();
                print_score(strict, collar);
            }
        }
    }
    eprintln!(
        "\n{audio_secs:.1}s audio · load {load_secs:.2}s · diarize {secs:.2}s · {:.0}× real time",
        audio_secs / secs.max(f64::EPSILON)
    );
    Ok(())
}

/// Diarizes a recording's system track (or reuses the cached analysis)
/// and relabels its "Others" segments as "Speaker 1", "Speaker 2"…
/// Returns how many speakers the transcript ends up with.
pub fn label_others(
    dir: &Path,
    segments: &mut Vec<quill_stt::Segment>,
    speakers: Option<usize>,
    models_dir: &Path,
) -> anyhow::Result<usize> {
    use quill_core::speakers;
    let analysis = match speakers::cached_analysis(dir, speakers::VOICE_MODEL) {
        Some(analysis) => analysis,
        None => {
            for model in speakers::required_models() {
                crate::models::download_with_progress(model, models_dir)?;
            }
            let diarizer = speakers::diarizer(models_dir)?;
            match speakers::analyze(dir, speakers::VOICE_MODEL, &diarizer)? {
                Some(analysis) => analysis,
                None => return Ok(0),
            }
        }
    };
    let (labelled, count) = speakers::label(segments, &analysis, speakers);
    *segments = labelled;
    Ok(count)
}

fn speaker_name(speaker: usize) -> String {
    format!("Speaker {}", speaker + 1)
}

fn print_speakers(turns: &[Turn]) {
    let speakers = quill_diarize::speaker_count(turns);
    println!("{speakers} speakers");
    for s in 0..speakers {
        let mine: Vec<_> = turns.iter().filter(|t| t.speaker == s).collect();
        let talk: u64 = mine.iter().map(|t| t.duration_ms()).sum();
        println!(
            "  {:<11} {:>6.1}s in {:>3} turns, first at {}",
            speaker_name(s),
            talk as f64 / 1000.0,
            mine.len(),
            clock(mine.first().map_or(0, |t| t.start_ms)),
        );
    }
}

fn print_score(strict: &Report, collar: &Report) {
    let pct = |x: f64| format!("{:.1}%", x * 100.0);
    println!(
        "DER {} (missed {}, false alarm {}, confusion {}) · {} with a 250 ms collar",
        pct(strict.der()),
        pct(strict.missed()),
        pct(strict.false_alarm()),
        pct(strict.confusion()),
        pct(collar.der()),
    );
    println!(
        "{} speakers in the reference, {} found",
        strict.reference_speakers, strict.hypothesis_speakers
    );
    for s in &strict.speakers {
        let found = match (s.matched, s.mostly) {
            (Some(m), Some(most)) if m != most => format!(
                "{} ({} of their speech), mostly labelled {}",
                speaker_name(m),
                pct(s.coverage()),
                speaker_name(most)
            ),
            (Some(m), _) => format!(
                "{} ({} of their speech)",
                speaker_name(m),
                pct(s.coverage())
            ),
            (None, Some(most)) => format!("merged into {}", speaker_name(most)),
            (None, None) => "missed".into(),
        };
        println!(
            "  {:<14} {:>6.1}s  → {found}",
            s.name,
            s.talk_ms as f64 / 1000.0
        );
    }
}

fn clock(ms: u64) -> String {
    format!("{}:{:02}", ms / 60_000, ms / 1000 % 60)
}
