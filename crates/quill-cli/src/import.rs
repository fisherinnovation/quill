//! `quill-cli import <dir>`: add a recording made elsewhere (e.g. with
//! `record --out`) to the meeting library.

use std::path::PathBuf;

use anyhow::{Context, bail};
use quill_core::meeting::{MIC_FILE, SYSTEM_FILE};
use quill_stt::Segment;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Folder with mic.wav and/or system.wav (and optionally transcript.json)
    dir: PathBuf,
    /// Meeting title [default: from its summary, else the import time]
    #[arg(long)]
    title: Option<String>,
}

pub fn run(args: Args) -> anyhow::Result<()> {
    let has = |name: &str| args.dir.join(name).is_file();
    if !has(MIC_FILE) && !has(SYSTEM_FILE) && !has("transcript.json") {
        bail!(
            "{} has no {MIC_FILE}, {SYSTEM_FILE} or transcript.json",
            args.dir.display()
        );
    }
    let library = crate::library::open()?;
    let meeting = library.create_meeting()?;
    let dest = library.meeting_dir(&meeting.id);

    for name in [
        MIC_FILE,
        SYSTEM_FILE,
        quill_core::meeting::RECORDING_INFO_FILE,
        "transcript.json",
        "summary.md",
        "summary.json",
    ] {
        if has(name) {
            std::fs::copy(args.dir.join(name), dest.join(name))
                .with_context(|| format!("copying {name}"))?;
        }
    }
    if has("transcript.json") {
        let json = std::fs::read_to_string(args.dir.join("transcript.json"))?;
        let segments: Vec<Segment> =
            serde_json::from_str(&json).context("parsing transcript.json")?;
        library.save_transcript(&meeting.id, &segments)?;
    }
    if let Some(title) = args.title {
        library.rename(&meeting.id, &title)?;
    }
    eprintln!("Imported as meeting {}", meeting.id);
    println!("{}", dest.display());
    Ok(())
}
