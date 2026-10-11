//! `quill-cli models ...`: manage downloaded Whisper models.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use clap::Subcommand;
use indicatif::{ProgressBar, ProgressStyle};
use quill_stt::models::{self, MODELS, ModelInfo};

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Show every known model and whether it is downloaded
    List,
    /// Download a model and verify its checksum
    Download {
        /// Model name, e.g. `turbo` or `vad` (see `models list`)
        name: String,
    },
    /// Print the models directory
    Path,
}

pub fn run(cmd: Command, models_dir: &Path) -> anyhow::Result<()> {
    match cmd {
        Command::List => {
            println!("Models directory: {}\n", models_dir.display());
            for m in MODELS.iter().chain([&models::VAD_MODEL]) {
                let mark = if m.is_downloaded(models_dir) {
                    "✓"
                } else {
                    " "
                };
                println!(
                    "{mark} {:<9} {:>8}  {}",
                    m.name,
                    format_size(m.size_bytes),
                    m.description
                );
            }
        }
        Command::Download { name } => {
            let model = lookup(&name)?;
            let path = download_with_progress(model, models_dir)?;
            println!("{}", path.display());
        }
        Command::Path => println!("{}", models_dir.display()),
    }
    Ok(())
}

/// Finds a model by name, listing the valid names if it doesn't exist.
pub fn lookup(name: &str) -> anyhow::Result<&'static ModelInfo> {
    match models::find(name) {
        Some(m) => Ok(m),
        None => {
            let names: Vec<_> = MODELS.iter().map(|m| m.name).collect();
            bail!(
                "unknown model `{name}`; choose one of: {}",
                names.join(", ")
            )
        }
    }
}

/// Resolves a model name to its file, with a helpful error if it is missing.
pub fn resolve(name: &str, models_dir: &Path) -> anyhow::Result<PathBuf> {
    let model = lookup(name)?;
    if !model.is_downloaded(models_dir) {
        bail!(
            "model `{name}` is not downloaded ({}); run: quill-cli models download {name}",
            format_size(model.size_bytes)
        );
    }
    Ok(model.path_in(models_dir))
}

/// The VAD model path if VAD is enabled, downloading the (1 MB) model the
/// first time so VAD can be on by default.
pub fn vad_model(enabled: bool, models_dir: &Path) -> anyhow::Result<Option<PathBuf>> {
    if !enabled {
        return Ok(None);
    }
    download_with_progress(&models::VAD_MODEL, models_dir).map(Some)
}

fn download_with_progress(model: &ModelInfo, models_dir: &Path) -> anyhow::Result<PathBuf> {
    if model.is_downloaded(models_dir) {
        return Ok(model.path_in(models_dir));
    }
    let bar = ProgressBar::new(model.size_bytes);
    if let Ok(style) = ProgressStyle::with_template(
        "{msg} [{bar:40}] {bytes}/{total_bytes} {bytes_per_sec} eta {eta}",
    ) {
        bar.set_style(style.progress_chars("=> "));
    }
    bar.set_message(model.name);

    let path = models::download(model, models_dir, |done, _total| bar.set_position(done))
        .with_context(|| format!("downloading {}", model.name))?;
    bar.finish_with_message(format!("{} ✓ checksum verified", model.name));
    Ok(path)
}

fn format_size(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    let mb = bytes as f64 / MB;
    if mb >= 1024.0 {
        format!("{:.1} GB", mb / 1024.0)
    } else {
        format!("{mb:.0} MB")
    }
}
