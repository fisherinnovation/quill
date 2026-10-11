//! Connecting CLI commands to the meeting library the app uses.

use std::path::Path;

use anyhow::Context;
use quill_core::library::Library;

pub fn open() -> anyhow::Result<Library> {
    let data_dir = quill_core::paths::data_dir().context("could not find a home directory")?;
    Library::open(&data_dir).context("opening the meeting library")
}

/// If `dir` is a meeting folder inside the library, the library and the
/// meeting's id, so results can be saved to it as well as to files.
pub fn meeting_for(dir: &Path) -> anyhow::Result<Option<(Library, String)>> {
    let library = open()?;
    let (Ok(dir), Ok(root)) = (dir.canonicalize(), library.recordings_dir().canonicalize()) else {
        return Ok(None);
    };
    if dir.parent() != Some(root.as_path()) {
        return Ok(None);
    }
    let Some(id) = dir.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return Ok(None);
    };
    Ok(library.get(&id).is_ok().then_some((library, id)))
}
