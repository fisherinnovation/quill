//! Where Quill keeps its files on each OS.

use std::path::PathBuf;

use directories::ProjectDirs;

/// Per-user data directory:
/// - macOS: `~/Library/Application Support/com.fisherinnovation.Quill`
/// - Windows: `%APPDATA%\fisherinnovation\Quill\data`
///
/// Keep the qualifier/organization/application in sync with the Tauri bundle
/// identifier in M5 so the CLI and the app share models and data.
///
/// Returns `None` only if the OS reports no home directory.
pub fn data_dir() -> Option<PathBuf> {
    ProjectDirs::from("com", "fisherinnovation", "Quill").map(|d| d.data_dir().to_path_buf())
}

/// Default directory for downloaded speech-to-text models.
pub fn models_dir() -> Option<PathBuf> {
    data_dir().map(|d| d.join("models"))
}
