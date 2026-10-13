//! `quill-cli`: command-line front end for Quill's library crates.
//!
//! Every stage of the pipeline has a subcommand here, so each can be run,
//! scripted and measured on its own, without the desktop app.

mod bench;
mod compact;
mod diarize;
mod dictate;
mod eval;
mod import;
mod keys;
mod library;
mod models;
mod record;
mod summarize;
mod transcribe;

use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

/// Local-first meeting notes and dictation.
//
// `#[derive(Parser)]` generates the argument-parsing code from this struct at
// compile time: fields become flags/arguments, and the `///` doc comment above
// becomes the `--help` text.
#[derive(Debug, Parser)]
#[command(name = "quill-cli", version)]
struct Cli {
    /// Directory holding Whisper models [default: the app data dir]
    #[arg(long, global = true, env = "QUILL_MODELS_DIR")]
    models_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

// Each enum variant is a subcommand; its fields are that subcommand's args.
#[derive(Debug, Subcommand)]
enum Command {
    /// Transcribe a WAV file and print timestamped segments
    Transcribe(transcribe::Args),
    /// List, download or locate Whisper models
    #[command(subcommand)]
    Models(models::Command),
    /// Measure transcription speed of several models on one file
    Bench(bench::Args),
    /// Record the microphone and system audio to WAV files
    Record(record::Args),
    /// Find who spoke when in an audio file, optionally scored against a
    /// reference
    Diarize(diarize::Args),
    /// Run the dictation pipeline on WAV files, with timings
    Dictate(dictate::Args),
    /// Summarize a transcript into meeting notes with an LLM
    Summarize(summarize::Args),
    /// Add a recording folder made elsewhere to the meeting library
    Import(import::Args),
    /// Shrink recordings by replacing their WAVs with 16 kHz FLAC
    Compact(compact::Args),
    /// Compare models' summaries: cost, latency and LLM-judged quality
    Eval(eval::Args),
    /// Store or remove LLM API keys in the OS keychain
    #[command(subcommand)]
    Keys(keys::Command),
}

// Returning `anyhow::Result` from `main` lets us use `?` on any error type;
// if an error reaches here it is printed with its chain of causes and the
// process exits with a non-zero code. (Libraries use typed `thiserror` enums
// instead; `anyhow` is only for app edges like this one.)
fn main() -> anyhow::Result<()> {
    init_tracing();
    let cli = Cli::parse();

    let models_dir = match cli.models_dir {
        Some(dir) => dir,
        None => quill_core::paths::models_dir().context("could not find a home directory")?,
    };

    match cli.command {
        Command::Transcribe(args) => transcribe::run(args, &models_dir),
        Command::Models(cmd) => models::run(cmd, &models_dir),
        Command::Bench(args) => bench::run(args, &models_dir),
        Command::Record(args) => record::run(args),
        Command::Diarize(args) => diarize::run(args, &models_dir),
        Command::Dictate(args) => dictate::run(args, &models_dir),
        Command::Summarize(args) => summarize::run(args),
        Command::Keys(cmd) => keys::run(cmd),
        Command::Eval(args) => eval::run(args),
        Command::Import(args) => import::run(args),
        Command::Compact(args) => compact::run(args),
    }
}

/// Sends log output to stderr, keeping stdout clean for command output.
/// The level comes from `RUST_LOG` (e.g. `RUST_LOG=debug`), defaulting to info
/// for our code and errors only for whisper.cpp, which is very chatty (even
/// its warnings are mostly informational, like which Metal features it found).
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,whisper_rs=error,flacenc=warn"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

// `#[cfg(test)]` compiles this module only for `cargo test`, so test code
// never ends up in the release binary.
#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// Catches mistakes in the clap definitions (duplicate flags, conflicting
    /// names) at test time instead of when someone runs the command.
    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
