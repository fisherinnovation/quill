//! `quill-cli compact`: replace recordings' WAVs with 16 kHz FLAC (about
//! 1/20 of the size; see `quill_core::meeting::compact_recording`).

use std::path::PathBuf;

use anyhow::bail;
use quill_core::meeting::{self, Compacted};

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Recording folders [default: every meeting in the library]
    dirs: Vec<PathBuf>,
}

pub fn run(args: Args) -> anyhow::Result<()> {
    let dirs = if args.dirs.is_empty() {
        let library = crate::library::open()?;
        library
            .list()?
            .iter()
            .map(|m| library.meeting_dir(&m.id))
            .collect()
    } else {
        args.dirs
    };
    let mut total = Compacted::default();
    let mut failed = 0;
    for dir in &dirs {
        match meeting::compact_recording(dir) {
            Ok(done) if done.tracks > 0 => {
                println!(
                    "{}: {} → {}",
                    dir.display(),
                    mb(done.bytes_before),
                    mb(done.bytes_after)
                );
                total.tracks += done.tracks;
                total.bytes_before += done.bytes_before;
                total.bytes_after += done.bytes_after;
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("{}: {e:#}", dir.display());
                failed += 1;
            }
        }
    }
    println!(
        "Compacted {} tracks: {} → {}",
        total.tracks,
        mb(total.bytes_before),
        mb(total.bytes_after)
    );
    if failed > 0 {
        bail!("{failed} folder(s) failed");
    }
    Ok(())
}

fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1e6)
}
