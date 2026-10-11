//! End-to-end smoke test: real model, real audio.
//!
//! Ignored by default because it needs a downloaded model. Run it with:
//!
//! ```text
//! cargo run -p quill-cli -- models download tiny
//! QUILL_TEST_MODEL="$HOME/Library/Application Support/com.fisherinnovation.Quill/models/ggml-tiny.bin" \
//!     cargo test -p quill-stt -- --ignored
//! ```

use std::path::{Path, PathBuf};

use quill_stt::{TranscribeOptions, Transcriber, WhisperConfig, WhisperTranscriber};

#[test]
#[ignore = "needs a Whisper model; set QUILL_TEST_MODEL"]
fn transcribes_jfk_sample() {
    let model = PathBuf::from(std::env::var("QUILL_TEST_MODEL").expect("set QUILL_TEST_MODEL"));
    let wav = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/jfk.wav");

    let pcm = quill_audio::load_for_stt(&wav).unwrap();
    let transcriber = WhisperTranscriber::load(&model, &WhisperConfig::default()).unwrap();
    let opts = TranscribeOptions {
        language: Some("en".into()),
        ..Default::default()
    };
    let segments = transcriber.transcribe(&pcm, &opts).unwrap();

    let text = segments
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    // Even the tiny model gets the famous line right.
    assert!(
        text.contains("ask not what your country can do for you"),
        "{text}"
    );
    // Timestamps are ordered and fall within the 11-second clip.
    assert!(segments.windows(2).all(|w| w[0].start_ms <= w[1].start_ms));
    assert!(segments.iter().all(|s| s.end_ms <= 11_500));
}
