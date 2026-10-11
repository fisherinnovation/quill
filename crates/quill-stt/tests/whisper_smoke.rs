//! End-to-end smoke test: real model, real audio.
//!
//! Ignored by default because it needs a downloaded model. Run it with:
//!
//! ```text
//! cargo run -p quill-cli -- models download tiny
//! cargo run -p quill-cli -- models download vad
//! M="$HOME/Library/Application Support/com.fisherinnovation.Quill/models"
//! QUILL_TEST_MODEL="$M/ggml-tiny.bin" QUILL_TEST_VAD_MODEL="$M/ggml-silero-v6.2.0.bin" \
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

/// Long stretches of near-silence make Whisper invent text ("Thank you.",
/// repeated phrases) and misplace timestamps. With VAD, only the real speech
/// is transcribed, at its real position.
#[test]
#[ignore = "needs Whisper + VAD models; set QUILL_TEST_MODEL and QUILL_TEST_VAD_MODEL"]
fn vad_prevents_hallucinations_in_silence() {
    let model = PathBuf::from(std::env::var("QUILL_TEST_MODEL").expect("set QUILL_TEST_MODEL"));
    let vad =
        PathBuf::from(std::env::var("QUILL_TEST_VAD_MODEL").expect("set QUILL_TEST_VAD_MODEL"));
    let wav = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/jfk.wav");

    // 25 s of faint hiss, the 11 s JFK clip, then 40 s more hiss.
    let mut rng = 0x2545_f491_u32;
    let mut hiss = |secs: usize| -> Vec<f32> {
        (0..secs * 16_000)
            .map(|_| {
                // xorshift: tiny deterministic noise generator, no extra crate.
                rng ^= rng << 13;
                rng ^= rng >> 17;
                rng ^= rng << 5;
                (rng as f32 / u32::MAX as f32 - 0.5) * 0.004
            })
            .collect()
    };
    let mut pcm = hiss(25);
    pcm.extend(quill_audio::load_for_stt(&wav).unwrap());
    pcm.extend(hiss(40));

    let config = WhisperConfig {
        vad_model: Some(vad),
        ..WhisperConfig::default()
    };
    let transcriber = WhisperTranscriber::load(&model, &config).unwrap();
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
    assert!(text.contains("ask not what your country"), "{text}");
    // Every segment sits inside the speech (25.0–36.0 s), give or take.
    for s in &segments {
        assert!(s.start_ms >= 24_500 && s.end_ms <= 36_500, "{s:?}");
    }
}
