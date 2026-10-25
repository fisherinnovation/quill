//! End-to-end smoke tests for Parakeet: real model, real audio.
//!
//! Ignored by default because they need the downloaded model. Run them with:
//!
//! ```text
//! cargo run -p quill-cli -- models download parakeet-v2
//! cargo run -p quill-cli -- models download vad
//! QUILL_TEST_MODELS_DIR="$(cargo run -q -p quill-cli -- models path)" \
//!     cargo test -p quill-stt --test parakeet_smoke -- --ignored
//! ```
//!
//! On Windows, copy sherpa-onnx's DLLs from `target/debug` into
//! `target/debug/deps` first: Windows 11 has an older `onnxruntime.dll` in
//! System32, which is found before anything on `PATH` and crashes the test
//! (`STATUS_ACCESS_VIOLATION`). The app and the CLI have theirs beside the
//! executable, which comes first.
//!
//! `fixtures/jfk.wav` is described in `whisper_smoke.rs`.

use std::path::{Path, PathBuf};

use quill_stt::models::{self, VAD_MODEL};
use quill_stt::{ParakeetConfig, ParakeetTranscriber, TranscribeOptions, Transcriber};

// Test helpers: clippy.toml allows unwrap/expect in #[test] fns only.
#[allow(clippy::unwrap_used, clippy::expect_used)]
fn load() -> ParakeetTranscriber {
    let dir =
        PathBuf::from(std::env::var("QUILL_TEST_MODELS_DIR").expect("set QUILL_TEST_MODELS_DIR"));
    let config = ParakeetConfig {
        vad_model: Some(VAD_MODEL.path_in(&dir)),
        ..ParakeetConfig::default()
    };
    let model = models::find_parakeet("parakeet-v2").unwrap();
    ParakeetTranscriber::load(model, &dir, &config).unwrap()
}

#[allow(clippy::unwrap_used)]
fn jfk() -> Vec<f32> {
    let wav = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/jfk.wav");
    quill_audio::load_for_stt(&wav).unwrap()
}

fn text(segments: &[quill_stt::Segment]) -> String {
    segments
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[test]
#[ignore = "needs Parakeet v2 and VAD; set QUILL_TEST_MODELS_DIR"]
fn transcribes_jfk_sample_as_a_meeting_and_a_dictation() {
    let transcriber = load();
    // Silence around it, as in a meeting track: times are the original's.
    let mut pcm = vec![0.0; 3 * 16_000];
    pcm.extend(jfk());
    pcm.extend(vec![0.0; 3 * 16_000]);
    for short_clip in [false, true] {
        let opts = TranscribeOptions {
            short_clip,
            ..Default::default()
        };
        let segments = transcriber.transcribe(&pcm, &opts).unwrap();
        let text = text(&segments);
        assert!(
            text.contains("ask not what your country can do for you"),
            "{text}"
        );
        assert!(segments.windows(2).all(|w| w[0].start_ms <= w[1].start_ms));
        assert!(segments.first().unwrap().start_ms >= 2_800, "{segments:?}");
        assert!(segments.iter().all(|s| s.end_ms <= 14_500), "{segments:?}");
    }
    // Silence alone is nothing.
    let silence = vec![0.0; 5 * 16_000];
    assert!(
        transcriber
            .transcribe(&silence, &TranscribeOptions::default())
            .unwrap()
            .is_empty()
    );
    assert!(transcriber.speech_end(&silence).is_none());
}

/// sherpa-onnx exits the process or throws on some hotwords; every one of
/// these must be skipped or used, never fatal.
#[test]
#[ignore = "needs Parakeet v2 and VAD; set QUILL_TEST_MODELS_DIR"]
fn any_vocabulary_is_safe() {
    let transcriber = load();
    let opts = TranscribeOptions {
        short_clip: true,
        vocabulary: [
            "Kennedy", ":3", "#general", "@alex", "TCP/IP", "café", "日本", "a  b", "",
        ]
        .map(String::from)
        .to_vec(),
        ..Default::default()
    };
    let text = text(&transcriber.transcribe(&jfk(), &opts).unwrap());
    assert!(text.contains("your country"), "{text}");
}
