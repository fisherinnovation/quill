//! `quill-cli record`: capture the microphone and/or system audio to WAVs.

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
use quill_audio::capture::{self, Source, TrackRequest, TrackSummary};
use serde_json::json;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Directory for the WAV files [default: <data dir>/recordings/<timestamp>]
    #[arg(long, short)]
    out: Option<PathBuf>,
    /// Stop automatically after this many seconds (otherwise press Enter)
    #[arg(long, short)]
    duration: Option<f64>,
    /// Record only the microphone
    #[arg(long, conflicts_with = "system_only")]
    mic_only: bool,
    /// Record only system audio
    #[arg(long)]
    system_only: bool,
}

pub fn run(args: Args) -> anyhow::Result<()> {
    let out_dir = match args.out {
        Some(dir) => dir,
        None => default_dir()?,
    };
    std::fs::create_dir_all(&out_dir).with_context(|| format!("creating {}", out_dir.display()))?;

    let mut requests = Vec::new();
    if !args.system_only {
        requests.push(TrackRequest {
            source: Source::Microphone,
            path: out_dir.join("mic.wav"),
        });
    }
    if !args.mic_only {
        requests.push(TrackRequest {
            source: Source::SystemAudio,
            path: out_dir.join("system.wav"),
        });
    }

    // On macOS, opening a device blocks (with no timeout) while a privacy
    // prompt is waiting for an answer. Say so if starting takes a while, so it
    // doesn't look like a hang.
    let (started_tx, started_rx) = mpsc::channel::<()>();
    std::thread::spawn(move || {
        if let Err(mpsc::RecvTimeoutError::Timeout) =
            started_rx.recv_timeout(Duration::from_secs(3))
        {
            eprintln!(
                "Still opening audio devices… if a macOS permission prompt is showing \
                 (microphone or system audio recording), answer it to continue."
            );
        }
    });
    let recording = capture::start(&requests);
    // Wakes the watchdog early (success or error), so it stays quiet.
    let _ = started_tx.send(());
    let recording = recording?;
    match args.duration {
        Some(secs) => {
            if !secs.is_finite() || secs <= 0.0 {
                bail!("--duration must be a positive number of seconds");
            }
            eprintln!("Recording for {secs}s…");
            std::thread::sleep(Duration::from_secs_f64(secs));
        }
        None => {
            eprintln!("Recording… press Enter to stop.");
            wait_for_enter();
        }
    }
    let tracks = recording.stop()?;

    for t in &tracks {
        eprintln!(
            "{:<12} {:.1}s  {} Hz × {} ch  peak {:.2}  lead-in {} ms  dropped {}  ({})",
            label(t.source),
            t.duration_secs(),
            t.sample_rate,
            t.channels,
            t.peak,
            t.lead_in_ms,
            t.dropped_samples,
            t.device,
        );
        if t.peak == 0.0 {
            eprintln!(
                "  warning: {} is pure silence. {}",
                label(t.source),
                hint(t.source)
            );
        }
    }
    write_metadata(&out_dir, &tracks)?;
    println!("{}", out_dir.display());
    Ok(())
}

fn default_dir() -> anyhow::Result<PathBuf> {
    let base = quill_core::paths::data_dir().context("could not find a home directory")?;
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    Ok(base.join("recordings").join(format!("recording-{secs}")))
}

/// Blocks until the user presses Enter (or stdin closes).
fn wait_for_enter() {
    // Read stdin on a helper thread so a closed or odd stdin can't wedge us;
    // the channel just carries the "go" signal.
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = std::io::stdin().lock().lines().next();
        let _ = tx.send(());
    });
    let _ = rx.recv();
}

fn label(source: Source) -> &'static str {
    match source {
        Source::Microphone => "microphone",
        Source::SystemAudio => "system audio",
    }
}

fn hint(source: Source) -> &'static str {
    match (source, cfg!(target_os = "macos")) {
        (Source::Microphone, true) => {
            "Check System Settings › Privacy & Security › Microphone for the app running this command (Terminal, VS Code, …)."
        }
        (Source::SystemAudio, true) => {
            "Was anything playing? If so, check System Settings › Privacy & Security › Screen & System Audio Recording › System Audio Recording Only."
        }
        (Source::Microphone, false) => "Check the OS microphone privacy settings.",
        (Source::SystemAudio, false) => "Was anything playing on the default output device?",
    }
}

/// Saves what was recorded next to the WAVs, for the transcribe step and for
/// debugging sync issues later.
fn write_metadata(dir: &Path, tracks: &[TrackSummary]) -> anyhow::Result<()> {
    let tracks: Vec<_> = tracks
        .iter()
        .map(|t| {
            json!({
                "source": match t.source {
                    Source::Microphone => "microphone",
                    Source::SystemAudio => "system",
                },
                "file": t.path.file_name().map(|f| f.to_string_lossy()),
                "device": t.device,
                "sample_rate": t.sample_rate,
                "channels": t.channels,
                "frames": t.frames,
                "lead_in_ms": t.lead_in_ms,
                "dropped_samples": t.dropped_samples,
                "peak": t.peak,
            })
        })
        .collect();
    let path = dir.join("recording.json");
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&json!({ "tracks": tracks }))?,
    )
    .with_context(|| format!("writing {}", path.display()))
}
