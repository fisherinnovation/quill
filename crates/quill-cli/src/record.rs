//! `quill-cli record`: capture the microphone and/or system audio to WAVs.

use std::io::BufRead;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, bail};
use quill_audio::capture::{self, Source, TrackRequest, TrackSummary};

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Record to this folder instead of a new meeting in the library
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
    // By default the recording becomes a new meeting in the library (and
    // shows up in the app); `--out` records to a plain folder instead.
    let (out_dir, meeting) = match args.out {
        Some(dir) => (dir, None),
        None => {
            let library = crate::library::open()?;
            let meeting = library.create_meeting()?;
            (
                library.meeting_dir(&meeting.id),
                Some((library, meeting.id)),
            )
        }
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
    quill_core::meeting::write_recording_info(&out_dir, &tracks)
        .with_context(|| format!("writing {}", out_dir.join("recording.json").display()))?;
    if let Some((library, id)) = meeting {
        let longest = tracks
            .iter()
            .map(TrackSummary::duration_secs)
            .fold(0.0, f64::max);
        library.set_duration(&id, (longest * 1000.0) as u64)?;
        eprintln!(
            "Saved as meeting {id}. Next: quill-cli transcribe {}",
            out_dir.display()
        );
    }
    println!("{}", out_dir.display());
    Ok(())
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
