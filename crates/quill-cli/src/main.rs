//! `quill-cli`: command-line front end for Quill's library crates.
//!
//! Each milestone adds a subcommand here (M1 `transcribe`, M3 `summarize`) so
//! features can be exercised long before the Tauri app exists.

use clap::Parser;
use tracing_subscriber::EnvFilter;

/// Local-first meeting notes and dictation.
//
// `#[derive(Parser)]` generates the argument-parsing code from this struct at
// compile time: fields become flags/arguments, and the `///` doc comment above
// becomes the `--help` text.
#[derive(Debug, Parser)]
#[command(name = "quill-cli", version)]
struct Cli {}

// Returning `anyhow::Result` from `main` lets us use `?` on any error type;
// if an error reaches here it is printed with its chain of causes and the
// process exits with a non-zero code. (Libraries use typed `thiserror` enums
// instead; `anyhow` is only for app edges like this one.)
fn main() -> anyhow::Result<()> {
    init_tracing();
    let _cli = Cli::parse();

    tracing::info!("no subcommands yet: `transcribe` arrives in M1, `summarize` in M3");
    Ok(())
}

/// Sends log output to stderr, keeping stdout clean for command output.
/// The level comes from `RUST_LOG` (e.g. `RUST_LOG=debug`), defaulting to info.
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
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
