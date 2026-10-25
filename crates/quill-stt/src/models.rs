//! Catalog of downloadable speech models and a checksum-verified downloader.
//!
//! Models come from Hugging Face: Whisper from the official whisper.cpp ggml
//! conversions (`ggerganov/whisper.cpp`), the Silero VAD model from
//! `ggml-org/whisper-vad`, and the diarization models from sherpa-onnx's ONNX
//! exports (`csukuangfj/...`). Sizes and SHA-256 hashes are the Git LFS
//! metadata of those files.

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
    /// File name on disk, and on Hugging Face unless `remote_name` is set.
    pub file_name: &'static str,
    /// Path within the Hugging Face repository, when it differs from
    /// `file_name` (e.g. a generic `model.onnx`).
    pub remote_name: Option<&'static str>,
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
        remote_name: None,
        size_bytes: 77_691_713,
        sha256: "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
        description: "39M params; fastest, lowest accuracy",
    },
    ModelInfo {
        name: "base",
        repo: WHISPER_REPO,
        file_name: "ggml-base.bin",
        remote_name: None,
        size_bytes: 147_951_465,
        sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
        description: "74M params",
    },
    ModelInfo {
        name: "small",
        repo: WHISPER_REPO,
        file_name: "ggml-small.bin",
        remote_name: None,
        size_bytes: 487_601_967,
        sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        description: "244M params",
    },
    ModelInfo {
        name: "medium",
        repo: WHISPER_REPO,
        file_name: "ggml-medium.bin",
        remote_name: None,
        size_bytes: 1_533_763_059,
        sha256: "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208",
        description: "769M params",
    },
    ModelInfo {
        name: "turbo-q5",
        repo: WHISPER_REPO,
        file_name: "ggml-large-v3-turbo-q5_0.bin",
        remote_name: None,
        size_bytes: 574_041_195,
        sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        description: "large-v3-turbo quantized to 5 bits; ~1/3 the size of turbo",
    },
    ModelInfo {
        name: "turbo",
        repo: WHISPER_REPO,
        file_name: "ggml-large-v3-turbo.bin",
        remote_name: None,
        size_bytes: 1_624_555_275,
        sha256: "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
        description: "large-v3 with a 4-layer decoder; near large-v3 accuracy, much faster",
    },
    ModelInfo {
        name: "large-v3",
        repo: WHISPER_REPO,
        file_name: "ggml-large-v3.bin",
        remote_name: None,
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
    remote_name: None,
    size_bytes: 885_098,
    sha256: "2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987",
    description: "Silero VAD v6.2: skips silence so Whisper doesn't invent text",
};

/// pyannote's segmentation-3.0 (MIT), exported to ONNX by sherpa-onnx: finds
/// who speaks when, up to three overlapping speakers per 10 s window, but not
/// who is who across windows. That's the voice models' job.
pub const SEGMENTATION_MODEL: ModelInfo = ModelInfo {
    name: "segmentation",
    repo: "csukuangfj/sherpa-onnx-pyannote-segmentation-3-0",
    file_name: "pyannote-segmentation-3.0.onnx",
    remote_name: Some("model.onnx"),
    size_bytes: 5_992_913,
    sha256: "220ad67ca923bef2fa91f2390c786097bf305bceb5e261d4af67b38e938e1079",
    description: "pyannote segmentation 3.0: where speaker turns are",
};

const VOICE_REPO: &str = "csukuangfj/speaker-embedding-models";

/// Speaker-embedding ("voice fingerprint") models for diarization, from
/// sherpa-onnx's exports. Several are listed so they can be compared with
/// `quill-cli diarize --voice`.
pub const VOICE_MODELS: &[ModelInfo] = &[
    ModelInfo {
        name: "voice-campplus",
        repo: VOICE_REPO,
        file_name: "3dspeaker_speech_campplus_sv_en_voxceleb_16k.onnx",
        remote_name: None,
        size_bytes: 29_596_978,
        sha256: "357a834f702b80161e5b981182c038e18553c1f2ca752ed6cec2052365d4129b",
        description: "3D-Speaker CAM++, VoxCeleb (Apache-2.0)",
    },
    ModelInfo {
        name: "voice-eres2net",
        repo: VOICE_REPO,
        file_name: "3dspeaker_speech_eres2net_sv_en_voxceleb_16k.onnx",
        remote_name: None,
        size_bytes: 26_485_263,
        sha256: "c59158379255ad66e161679cca6af8d52d51e389e3224ab7d7a7baae295c2db5",
        description: "3D-Speaker ERes2Net, VoxCeleb (Apache-2.0)",
    },
    ModelInfo {
        name: "voice-eres2net-200k",
        repo: VOICE_REPO,
        file_name: "3dspeaker_speech_eres2net_base_200k_sv_zh-cn_16k-common.onnx",
        remote_name: None,
        size_bytes: 39_593_765,
        sha256: "e2d2048292e055f7b61cdec3db010503f35369b245bf0b3bbad021c9a91e4053",
        description: "3D-Speaker ERes2Net base, 200k speakers (Apache-2.0)",
    },
    ModelInfo {
        name: "voice-resnet34",
        repo: VOICE_REPO,
        file_name: "wespeaker_en_voxceleb_resnet34_LM.onnx",
        remote_name: None,
        size_bytes: 26_530_550,
        sha256: "e9848563da86f263117134dfd7ad63c92355b37de492b55e325400c9d9c39012",
        description: "WeSpeaker ResNet34-LM, VoxCeleb (CC BY 4.0)",
    },
    ModelInfo {
        name: "voice-titanet-small",
        repo: VOICE_REPO,
        file_name: "nemo_en_titanet_small.onnx",
        remote_name: None,
        size_bytes: 40_257_283,
        sha256: "ad4a1802485d8b34c722d2a9d04249662f2ece5d28a7a039063ca22f515a789e",
        description: "NVIDIA NeMo TitaNet small (CC BY 4.0)",
    },
    ModelInfo {
        name: "voice-titanet-large",
        repo: VOICE_REPO,
        file_name: "nemo_en_titanet_large.onnx",
        remote_name: None,
        size_bytes: 101_405_493,
        sha256: "d51abcf31717ef28162f26acb9d44dd4127c3d44c9b8624f699f3425daca8e77",
        description: "NVIDIA NeMo TitaNet large (CC BY 4.0)",
    },
];

/// Search by meaning (`quill_core::semantic`): Microsoft's
/// multilingual-e5-small (MIT), weights and tokenizer, run by `quill-embed`.
/// Both files are needed. Most of the weights are its 250k-token
/// vocabulary, which is what covers ~100 languages.
pub const SEARCH_MODELS: &[ModelInfo] = &[
    ModelInfo {
        name: "search",
        repo: "intfloat/multilingual-e5-small",
        file_name: "multilingual-e5-small.safetensors",
        remote_name: Some("model.safetensors"),
        size_bytes: 470_641_600,
        sha256: "1a55775f53449dac10a2bcbc312469fac40b96d53198c407081a831f81c98477",
        description: "multilingual-e5-small: finds moments by meaning, in ~100 languages",
    },
    ModelInfo {
        name: "search-tokenizer",
        repo: "intfloat/multilingual-e5-small",
        file_name: "multilingual-e5-small-tokenizer.json",
        remote_name: Some("tokenizer.json"),
        size_bytes: 17_082_730,
        sha256: "0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39",
        description: "multilingual-e5-small's tokenizer",
    },
];

/// Every non-Whisper model: VAD, the diarization models and search.
pub fn support_models() -> impl Iterator<Item = &'static ModelInfo> {
    [&VAD_MODEL, &SEGMENTATION_MODEL]
        .into_iter()
        .chain(VOICE_MODELS)
        .chain(SEARCH_MODELS)
}

/// Looks up any model by its short name: Whisper, `"vad"`, or a
/// diarization model.
pub fn find(name: &str) -> Option<&'static ModelInfo> {
    MODELS
        .iter()
        .chain(support_models())
        .find(|m| m.name == name)
}

impl ModelInfo {
    /// Where this model lives inside `models_dir`.
    pub fn path_in(&self, models_dir: &Path) -> PathBuf {
        models_dir.join(self.file_name)
    }

    /// Where a download in progress is written, until its checksum passes.
    pub fn part_path_in(&self, models_dir: &Path) -> PathBuf {
        self.path_in(models_dir).with_extension("bin.part")
    }

    /// True if the file exists with the expected size. (Hashing a 3 GB file on
    /// every run would be slow, so the full checksum is only done on download.)
    pub fn is_downloaded(&self, models_dir: &Path) -> bool {
        fs::metadata(self.path_in(models_dir)).is_ok_and(|m| m.len() == self.size_bytes)
    }

    fn url(&self) -> String {
        format!(
            "https://huggingface.co/{}/resolve/main/{}",
            self.repo,
            self.remote_name.unwrap_or(self.file_name)
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
    let part_path = model.part_path_in(models_dir);
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
        let all: Vec<_> = MODELS.iter().chain(support_models()).collect();
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
