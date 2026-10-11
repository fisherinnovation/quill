//! Catalog of downloadable Whisper models and a checksum-verified downloader.
//!
//! Models come from the official whisper.cpp ggml conversions on Hugging Face
//! (Whisper models from `ggerganov/whisper.cpp`, the Silero VAD model from
//! `ggml-org/whisper-vad`). Sizes and SHA-256 hashes are the Git LFS metadata
//! of those files.

use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::SttError;

const WHISPER_REPO: &str = "ggerganov/whisper.cpp";

/// A Whisper model we know how to download and verify.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelInfo {
    /// Short name used on the command line, e.g. `"turbo"`.
    pub name: &'static str,
    /// Hugging Face repository the file is downloaded from.
    pub repo: &'static str,
    /// File name on Hugging Face and on disk.
    pub file_name: &'static str,
    pub size_bytes: u64,
    pub sha256: &'static str,
    pub description: &'static str,
}

/// Every model `quill-cli models download` accepts, smallest first.
pub const MODELS: &[ModelInfo] = &[
    ModelInfo {
        name: "tiny",
        repo: WHISPER_REPO,
        file_name: "ggml-tiny.bin",
        size_bytes: 77_691_713,
        sha256: "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
        description: "39M params; fastest, lowest accuracy",
    },
    ModelInfo {
        name: "base",
        repo: WHISPER_REPO,
        file_name: "ggml-base.bin",
        size_bytes: 147_951_465,
        sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
        description: "74M params",
    },
    ModelInfo {
        name: "small",
        repo: WHISPER_REPO,
        file_name: "ggml-small.bin",
        size_bytes: 487_601_967,
        sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        description: "244M params",
    },
    ModelInfo {
        name: "medium",
        repo: WHISPER_REPO,
        file_name: "ggml-medium.bin",
        size_bytes: 1_533_763_059,
        sha256: "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208",
        description: "769M params",
    },
    ModelInfo {
        name: "turbo-q5",
        repo: WHISPER_REPO,
        file_name: "ggml-large-v3-turbo-q5_0.bin",
        size_bytes: 574_041_195,
        sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        description: "large-v3-turbo quantized to 5 bits; ~1/3 the size of turbo",
    },
    ModelInfo {
        name: "turbo",
        repo: WHISPER_REPO,
        file_name: "ggml-large-v3-turbo.bin",
        size_bytes: 1_624_555_275,
        sha256: "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
        description: "large-v3 with a 4-layer decoder; near large-v3 accuracy, much faster",
    },
    ModelInfo {
        name: "large-v3",
        repo: WHISPER_REPO,
        file_name: "ggml-large-v3.bin",
        size_bytes: 3_095_033_483,
        sha256: "64d182b440b98d5203c4f9bd541544d84c605196c4f7b845dfa11fb23594d1e2",
        description: "1.55B params; most accurate, slowest",
    },
];

/// Silero voice activity detection model for whisper.cpp's built-in VAD.
pub const VAD_MODEL: ModelInfo = ModelInfo {
    name: "vad",
    repo: "ggml-org/whisper-vad",
    file_name: "ggml-silero-v6.2.0.bin",
    size_bytes: 885_098,
    sha256: "2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987",
    description: "Silero VAD v6.2: skips silence so Whisper doesn't invent text",
};

/// Looks up a Whisper model, or the VAD model (`"vad"`), by its short name.
pub fn find(name: &str) -> Option<&'static ModelInfo> {
    MODELS
        .iter()
        .chain(std::iter::once(&VAD_MODEL))
        .find(|m| m.name == name)
}

impl ModelInfo {
    /// Where this model lives inside `models_dir`.
    pub fn path_in(&self, models_dir: &Path) -> PathBuf {
        models_dir.join(self.file_name)
    }

    /// True if the file exists with the expected size. (Hashing a 3 GB file on
    /// every run would be slow, so the full checksum is only done on download.)
    pub fn is_downloaded(&self, models_dir: &Path) -> bool {
        fs::metadata(self.path_in(models_dir)).is_ok_and(|m| m.len() == self.size_bytes)
    }

    fn url(&self) -> String {
        format!(
            "https://huggingface.co/{}/resolve/main/{}",
            self.repo, self.file_name
        )
    }
}

/// Downloads `model` into `models_dir`, verifying its SHA-256, and returns the
/// final path. Does nothing if the model is already present.
///
/// `on_progress(downloaded_bytes, total_bytes)` is called as data arrives, so
/// a UI (CLI progress bar now, Tauri later) can show progress without this
/// crate knowing about either.
pub fn download(
    model: &ModelInfo,
    models_dir: &Path,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<PathBuf, SttError> {
    let final_path = model.path_in(models_dir);
    if model.is_downloaded(models_dir) {
        return Ok(final_path);
    }
    fs::create_dir_all(models_dir)?;

    // Write to a `.part` file and rename only after the checksum passes, so a
    // cancelled or corrupt download never looks like a usable model.
    let part_path = final_path.with_extension("bin.part");
    let result = download_to(model, &part_path, &mut on_progress);
    if let Err(e) = result {
        let _ = fs::remove_file(&part_path);
        return Err(e);
    }
    fs::rename(&part_path, &final_path)?;
    Ok(final_path)
}

fn download_to(
    model: &ModelInfo,
    part_path: &Path,
    on_progress: &mut impl FnMut(u64, u64),
) -> Result<(), SttError> {
    // Large models take minutes, so no overall timeout; only connecting is
    // time-limited.
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .timeout(None)
        .build()
        .map_err(|e| SttError::Download(e.to_string()))?;
    let mut response = client
        .get(model.url())
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|e| SttError::Download(e.to_string()))?;

    let mut file = BufWriter::new(File::create(part_path)?);
    let mut hasher = Sha256::new();
    let mut buf = vec![0_u8; 256 * 1024];
    let mut downloaded = 0_u64;
    loop {
        let n = response.read(&mut buf)?;
        if n == 0 {
            break;
        }
        // Hash while writing so we never have to re-read the file.
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])?;
        downloaded += n as u64;
        on_progress(downloaded, model.size_bytes);
    }
    file.flush()?;

    let actual = to_hex(&hasher.finalize());
    if actual != model.sha256 {
        return Err(SttError::ChecksumMismatch {
            file: model.file_name.to_string(),
            expected: model.sha256.to_string(),
            actual,
        });
    }
    Ok(())
}

fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_hashes_well_formed() {
        let all: Vec<_> = MODELS.iter().chain([&VAD_MODEL]).collect();
        for (i, m) in all.iter().enumerate() {
            assert!(all[i + 1..].iter().all(|o| o.name != m.name));
            assert_eq!(m.sha256.len(), 64, "{}", m.name);
            assert!(m.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn hex_encoding_matches_known_digest() {
        // SHA-256 of the empty input.
        assert_eq!(
            to_hex(&Sha256::digest(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn size_check_detects_partial_files() {
        let dir = tempfile::tempdir().unwrap();
        let tiny = find("tiny").unwrap();
        assert!(!tiny.is_downloaded(dir.path()));
        fs::write(tiny.path_in(dir.path()), b"truncated").unwrap();
        assert!(!tiny.is_downloaded(dir.path()));
    }
}
