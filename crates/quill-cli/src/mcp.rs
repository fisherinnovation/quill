//! `quill-cli mcp`: the MCP server (`quill_mcp`) over stdin/stdout, for an
//! assistant that starts it itself (`claude mcp add quill -- quill-cli mcp`).
//! The app serves the same tools on a localhost URL instead.
//!
//! It serves only while Settings → Meetings → Let AI assistants read your
//! meetings is on, which it reads from `settings.toml` on every request.
//! stdout carries the protocol, so logs go to stderr, as always in the CLI.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use anyhow::{Context as _, bail};
use quill_core::library::Library;
use quill_core::semantic;
use quill_core::settings::Settings;
use quill_embed::Embedder;

pub fn run(models_dir: &Path) -> anyhow::Result<()> {
    let data_dir = quill_core::paths::data_dir().context("could not find a home directory")?;
    let settings = Settings::load(&data_dir).context("reading settings")?;
    if !settings.assistant_access {
        bail!("{}", quill_mcp::TURNED_OFF);
    }
    let library = Library::open(&data_dir, &settings.recordings_dir(&data_dir))
        .context("opening the meeting library")?;
    let context = Arc::new(CliContext {
        library,
        data_dir,
        models_dir: models_dir.to_path_buf(),
        embedder: OnceLock::new(),
    });
    tracing::info!("serving the library to an AI assistant over stdio");
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(quill_mcp::serve_stdio(context))?;
    Ok(())
}

struct CliContext {
    library: Library,
    data_dir: PathBuf,
    models_dir: PathBuf,
    /// Loaded on the first search, if search by meaning is on and the model
    /// is downloaded (this never downloads it).
    embedder: OnceLock<Option<Arc<quill_embed::E5Embedder>>>,
}

impl quill_mcp::Context for CliContext {
    fn allowed(&self) -> bool {
        Settings::load(&self.data_dir).is_ok_and(|s| s.assistant_access)
    }

    fn library(&self) -> &Library {
        &self.library
    }

    fn embedder(&self) -> Option<Arc<dyn Embedder>> {
        let embedder = self.embedder.get_or_init(|| {
            let on = Settings::load(&self.data_dir).is_ok_and(|s| s.semantic_search);
            if !on || !semantic::model_ready(&self.models_dir) {
                return None;
            }
            semantic::load_embedder(&self.models_dir, semantic::default_threads())
                .map(Arc::new)
                .inspect_err(|e| tracing::warn!("search by words only: {e}"))
                .ok()
        });
        embedder.clone().map(|e| e as Arc<dyn Embedder>)
    }

    fn vocabulary(&self) -> Vec<String> {
        Settings::load(&self.data_dir)
            .map(|s| s.dictation.vocabulary)
            .unwrap_or_default()
    }

    /// Never: the app keeps its settings in memory, so a change saved here
    /// would be lost the next time it saves them.
    fn edit_vocabulary(
        &self,
        _edit: &quill_mcp::EditVocabulary<'_>,
    ) -> Result<Vec<String>, String> {
        Err(quill_mcp::VOCABULARY_APP_ONLY.into())
    }
}
