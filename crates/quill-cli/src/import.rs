//! `quill-cli import <dir>`: add a recording made elsewhere (e.g. with
//! `record --out`) to the meeting library.

use std::path::PathBuf;

use anyhow::{Context, bail};
use quill_core::meeting::{MIC_FILE, MIC_FLAC, SYSTEM_FILE, SYSTEM_FLAC};
use quill_stt::Segment;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Folder with mic and/or system audio (.wav or .flac), and optionally
    /// transcript.json
    dir: PathBuf,
    /// Meeting title [default: from its summary, else the import time]
    #[arg(long)]
    title: Option<String>,
}

pub fn run(args: Args) -> anyhow::Result<()> {
    let has = |name: &str| args.dir.join(name).is_file();
    if !quill_core::meeting::has_audio(&args.dir) && !has("transcript.json") {
        bail!("{} has no audio or transcript.json", args.dir.display());
    }
    let library = crate::library::open()?;
    let meeting = library.create_meeting()?;
    let dest = library.meeting_dir(&meeting.id);

    for name in [
        MIC_FILE,
        SYSTEM_FILE,
        MIC_FLAC,
        SYSTEM_FLAC,
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
