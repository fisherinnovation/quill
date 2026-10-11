# Quill (working name) — Local-First Meeting Notes & Dictation


## 1. Vision

A cross-platform (macOS + Windows) desktop app, built in Rust, that:

1. **Priority 1 — Meeting notes:** records a meeting (mic + system audio), transcribes it locally with Whisper, and generates a detailed, structured summary via LLMs dispatched through OpenRouter (or a local model).
2. **Priority 2 — Dictation:** hold a global hotkey, speak, and formatted text is inserted at the cursor in any app (a Wispr Flow–style experience).

Guiding principles:
- **Local-first.** Audio never leaves the machine. Only transcripts go to an LLM, and only when the user opts in to a cloud model.
- **Model-agnostic.** The STT engine and the LLM provider are swappable behind traits.
- **Measurable.** Summary quality and cost are evaluated with a repeatable harness, not vibes.
- **Learning project.** The owner is new to Rust. Prefer clear, idiomatic code over clever code; explain non-obvious Rust patterns in comments or PR notes.

### Non-goals (for v1)
- Mobile apps, cloud sync, team sharing, calendar integration.
- Real-time streaming transcription during meetings (batch after the meeting is fine for v1; live captions are a stretch goal).
- Training or fine-tuning models.

---

## 2. Tech Stack

| Concern | Choice | Notes |
|---|---|---|
| App shell | **Tauri v2** | Rust backend, web UI frontend, small binaries, plugins for hotkeys/updater/tray |
| Frontend | **Svelte + TypeScript** (or React if preferred) | Keep UI thin; logic lives in Rust |
| Async runtime | `tokio` | Tauri already uses it |
| Mic capture | `cpal` | Cross-platform input devices |
| System audio (macOS) | ScreenCaptureKit (`screencapturekit` crate) or Core Audio process taps (macOS 14.4+) | Requires Screen Recording / audio capture permission |
| System audio (Windows) | WASAPI loopback (`wasapi` crate, or `cpal` loopback on the default output device) | |
| Resampling | `rubato` | Whisper wants 16 kHz mono `f32` |
| Audio files | `hound` (WAV) | Store raw tracks for re-processing |
| Voice activity detection | Silero VAD (`voice_activity_detector` crate or via `sherpa-rs`) | Trims silence, speeds up STT, enables chunking |
| Speech-to-text | `whisper-rs` (whisper.cpp bindings) | Metal on Apple Silicon; CUDA/Vulkan on Windows via feature flags |
| Optional fast STT / diarization | `sherpa-rs` (sherpa-onnx) | NVIDIA Parakeet, speaker diarization, VAD |
| LLM client | `reqwest` + `serde` against OpenAI-compatible API | OpenRouter primary; Ollama/LM Studio for local |
| Storage | `rusqlite` (or `sqlx` with SQLite) + FTS5 | Meetings, transcripts, summaries, eval runs |
| Secrets | `keyring` | API keys in OS keychain/Credential Manager, never in plain config |
| Hotkeys | `tauri-plugin-global-shortcut` | |
| Text insertion | `arboard` (clipboard) + `enigo` (simulate Cmd/Ctrl+V) | Restore the user's clipboard afterwards |
| Logging | `tracing` + `tracing-subscriber` | |
| Errors | `thiserror` (library code) + `anyhow` (app/binary edges) | |
| Config | `serde` + TOML in the app data dir | |
| CLI tools | `clap` | For the eval harness and dev utilities |

---

## 3. Architecture

### 3.1 Workspace layout (Cargo workspace)

```
quill/
├── Cargo.toml                # [workspace]
├── docs/PROJECT_PLAN.md      # this file
├── crates/
│   ├── quill-audio/          # capture (mic + system), resample, VAD, WAV writing
│   ├── quill-stt/            # Transcriber trait + whisper-rs impl (+ sherpa impl later)
│   ├── quill-llm/            # LlmProvider trait + OpenRouter / Ollama impls, prompts
│   ├── quill-core/           # domain types, pipeline orchestration, storage (SQLite)
│   ├── quill-eval/           # run prompts across models, log cost/latency, judge
│   ├── quill-insert/         # clipboard + keystroke text insertion (dictation)
│   └── quill-cli/            # binary: dev CLI over the library crates (transcribe, summarize, …)
├── app/                      # Tauri v2 app (src-tauri/ + frontend/)
├── prompts/                  # versioned prompt templates (.md / .toml)
└── eval/
    ├── transcripts/          # sample transcripts (gitignored if private)
    └── results/              # eval outputs (gitignored)
```

Keeping the core logic in plain library crates (no Tauri dependency) means each piece can be built and tested from a CLI first. That's faster to iterate on and easier to learn Rust with.

### 3.2 Core traits (sketch)

```rust
// quill-stt
pub struct Segment { pub start_ms: u64, pub end_ms: u64, pub text: String, pub speaker: Option<String> }

pub trait Transcriber: Send + Sync {
    fn transcribe(&self, pcm_16k_mono: &[f32], opts: &TranscribeOptions) -> Result<Vec<Segment>, SttError>;
}

// quill-llm
#[async_trait::async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError>;
}

pub struct CompletionResponse {
    pub text: String,
    pub model: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cost_usd: Option<f64>,
    pub latency_ms: u64,
}
```

### 3.3 Meeting pipeline

```
[Start recording]
   ├─ mic track  (cpal)            ──► mic.wav
   └─ system track (SCK / WASAPI)  ──► system.wav
[Stop recording]
   ► resample both to 16 kHz mono
   ► VAD → speech chunks (keep timestamps)
   ► Whisper per chunk, per track
   ► merge segments by timestamp; label mic = "Me", system = "Others"
     (optional: diarize the system track into Speaker 1..N)
   ► store transcript in SQLite
   ► summarize (LLM): single pass if it fits the context budget,
     else map-reduce (chunk summaries → final summary)
   ► store summary + model + cost + prompt version
```

**Two-track trick:** recording mic and system audio separately gives reliable "me vs. them" labels for free, and avoids most echo problems.

### 3.4 Dictation pipeline

```
[Hotkey down] → start mic capture (ring buffer, pre-warmed stream)
[Hotkey up]   → VAD trim → Whisper (turbo/small, or Parakeet)
              → LLM "polish" (fast/cheap model or local) with a strict formatting prompt
              → save clipboard → write text → simulate paste → restore clipboard
```

**Latency budget target:** under ~1.0 s from key release to text appearing for a 10-second utterance. Keep the Whisper model loaded in memory; never reload per utterance.

### 3.5 Summary output format (default template)

```markdown
# {Meeting title} — {date}
## TL;DR (3–5 bullets)
## Decisions
## Action items (owner — task — due date if mentioned)
## Discussion by topic
## Open questions / risks
## Notable quotes (optional, short)
```

Prompt templates live in `prompts/` with a version number. Every stored summary records which prompt version and model produced it.

---

## 4. Milestones

Each milestone ends with something runnable. Don't start the next until the acceptance criteria pass.

### M0 — Rust & tooling foundations
- [x] Install toolchain (`rustup`, stable), `cargo-watch`, `cargo-nextest`, clippy, rustfmt. *(Installed on the Windows dev machine.)*
- [x] Create the Cargo workspace with empty crates; CI (GitHub Actions) running `fmt --check`, `clippy -D warnings` and tests on macOS + Windows.
- [ ] Short learning pass: ownership/borrowing, `Result`/`?`, traits, `Arc<Mutex<_>>`, channels, async basics.

**Accept:** `cargo build` and `cargo test` pass on both OSes in CI.

### M1 — Transcribe a file (CLI)
- [x] `quill-stt`: load a ggml Whisper model, transcribe a WAV file, print timestamped segments.
- [x] Model download helper (Hugging Face ggml models → app data dir, with checksum).
- [x] Feature flags: Metal is always on for macOS builds (target-specific dependency, `--cpu` to compare); `cuda` / `vulkan` opt-in features (not yet tested on Windows); CPU fallback.
- [x] Benchmark tiny/base/small/turbo/large-v3 on your hardware; record the real-time factor in `eval/benchmarks.md`. *(Mac done; Windows pending.)*

**Accept:** `quill-cli transcribe meeting.wav --model turbo` produces an accurate timestamped transcript.

### M2 — Capture audio (CLI)
- [x] Mic capture with `cpal` → WAV. *(Verified on macOS.)*
- [x] System audio capture: macOS (ScreenCaptureKit / process tap) and Windows (WASAPI loopback) behind a common trait. *(cpal 0.18 does both: an input stream on the output device uses a Core Audio process tap on macOS 14.6+ and WASAPI loopback on Windows. So cpal's API is the common interface, and there's no platform code of our own. Verified on macOS; Windows untested.)*
- [x] Record both tracks simultaneously with a shared start timestamp. *(Each track is padded with silence so that sample 0 is the shared start. Measured on macOS: the mic hears speaker playback 50–70 ms after the system track records it.)*
- [x] Resample to 16 kHz mono; add Silero VAD. *(VAD is on by default; see the M2 notes.)*
- [x] Merge tracks into one "Me" / "Others" transcript, dropping speaker echo from the mic track (`quill-cli transcribe <recording dir>`).

**Accept:** Record a real Zoom/Meet/Teams call on both OSes; both tracks are clean, in sync and transcribable.

### M3 — Summarize via OpenRouter (CLI)
- [ ] `quill-llm`: OpenRouter client (OpenAI-compatible chat completions), API key from `keyring`.
- [ ] Capture token usage and cost per request (OpenRouter can return usage/cost accounting in the response — check current docs for the exact request option).
- [ ] Provider privacy settings: prefer providers that don't collect or retain data (OpenRouter supports provider routing preferences — verify the current parameter names).
- [ ] Ollama/LM Studio provider using the same trait.
- [ ] Map-reduce summarization for long transcripts.

**Accept:** `quill-cli summarize transcript.json --model <id>` produces the template summary and prints its cost and latency.

### M4 — Evaluation harness (`quill-eval`)
- [ ] Input: a folder of transcripts × a list of model IDs × prompt version(s).
- [ ] Run all combinations concurrently (bounded with a semaphore); store outputs, tokens, cost and latency in SQLite plus a CSV export.
- [ ] LLM-as-judge scoring (a strong model scores each summary 1–10 against a rubric: completeness, accuracy, action-item recall, hallucinations, formatting). Randomize order and hide model names from the judge.
- [ ] Manual review mode: side-by-side markdown report for spot checks.
- [ ] Output a leaderboard: quality score vs. cost per meeting-hour.

**Accept:** One command produces a ranked table that answers which model is best for the money. Choose the default model from this.

### M5 — Tauri app: meeting notes MVP
- [ ] Tauri v2 shell with a tray/menu-bar icon: Start/Stop recording, recording indicator.
- [ ] Meetings list, transcript view (with speaker labels and timestamps), summary view, re-summarize with a different model/prompt.
- [ ] Settings: STT model, LLM provider/model, API key, storage location, prompt template.
- [ ] Permission onboarding (mic, screen/system audio on macOS) with clear explanations.
- [ ] Full-text search over transcripts (SQLite FTS5).
- [ ] Export summary as Markdown; copy to clipboard.

**Accept:** Daily-driveable for real meetings on macOS and Windows.

### M6 — Dictation
- [ ] Global push-to-talk hotkey (configurable) plus a toggle mode.
- [ ] Pre-warmed mic stream and a resident Whisper model for low latency.
- [ ] Polish step: fast LLM (or local) with a strict prompt that removes fillers, fixes punctuation and formats lists, and never adds content.
- [ ] Text insertion via clipboard + simulated paste, restoring the previous clipboard.
- [ ] Custom vocabulary (fed as Whisper initial prompt) and replacement rules.
- [ ] Per-app tone (optional): detect the frontmost app and adjust the polish prompt.

**Accept:** Under ~1 s perceived latency on Apple Silicon for short utterances; works in Slack, browser, VS Code and email.

### M7 — Polish & distribution
- [ ] Code signing + notarization (macOS), signing (Windows), installers via Tauri bundler.
- [ ] Auto-update (`tauri-plugin-updater`).
- [ ] Crash/error logging to a local file; opt-in only for anything remote.
- [ ] Data retention settings (auto-delete audio after N days, keep transcripts).

### Stretch goals
- Live captions during meetings (streaming chunks).
- Local speaker diarization with named speakers (voice fingerprints).
- "Ask your meetings" — semantic search / chat across past transcripts (embeddings in SQLite via `sqlite-vec`).
- Auto-detect meeting start (mic in use + Zoom/Teams/Meet active).
- Notes pad: user's typed notes merged with the transcript in the summary (Granola-style).

---

## 5. Known Risks & Bottlenecks

| Risk | Mitigation |
|---|---|
| System audio capture is OS-specific and permission-heavy | Do it early (M2); study Meetily / OpenWhispr implementations; isolate behind a trait |
| Whisper too slow on weak Windows hardware | Model tiers; GPU features; Parakeet via sherpa; cloud STT fallback (Groq) as an opt-in |
| Whisper hallucinations on silence ("Thanks for watching!") | VAD before STT; drop low-confidence/no-speech segments |
| Dictation latency | Keep models resident; small STT model; tiny/fast polish model; skip polish for short utterances |
| Text insertion edge cases (secure fields, clipboard clobbering) | Restore clipboard; detect failures; fall back to "copied — press paste" |
| Privacy of transcripts sent to cloud LLMs | Provider routing preferences for no data retention; local Ollama option; clear UI indicator of where data goes |
| Long meetings exceed context or cost | Token estimation before sending; map-reduce; cache chunk summaries |
| Code signing / notarization pain | Budget time in M7; Apple Developer account needed for distribution |
| Rust learning curve (async + FFI + threads) | Build CLI-first in library crates; keep audio threads simple (channels, not shared state) |

---

## 6. Conventions (for humans and coding agents)

- **Rust style:** `rustfmt` defaults; `clippy -D warnings`. No `unwrap()`/`expect()` outside tests and `main` setup; use `?` with typed errors.
- **Errors:** `thiserror` enums per crate; `anyhow` only at app/CLI boundaries.
- **Concurrency:** audio callbacks must never block or allocate heavily. Push samples into a lock-free ring buffer or channel and process on a worker thread.
- **Unsafe:** none outside FFI wrappers; every `unsafe` block gets a `// SAFETY:` comment.
- **Tests:** unit tests per crate; golden-file tests for prompt rendering and transcript merging; a short sample WAV checked in for STT smoke tests.
- **Secrets:** never log API keys or full transcripts at info level.
- **Commits:** small, one concern each; conventional commit prefixes (`feat:`, `fix:`, `refactor:`).
- **Teaching mode:** when introducing a new Rust concept (lifetimes, trait objects, `Pin`, FFI), add a 2–3 line explanation in the PR description or a code comment.

---

## 7. Reference Projects (read, borrow ideas, respect licenses)

- **OpenWhispr** — dictation + meeting notes; whisper.cpp, sherpa-onnx (Electron, but useful for feature design).
- **Meetily** — local meeting notes; system audio capture and summarization pipeline (MIT).
- **Hyprnote** — Granola-style notes merged with transcripts (GPL-3.0: don't copy code into a non-GPL project).
- **Handy** — cross-platform Tauri dictation app; closest architectural match for the Rust side.
- **VoiceInk** — macOS dictation UX reference.
- **whisper.cpp / whisper-rs** — STT engine and bindings.
- **sherpa-onnx / sherpa-rs** — Parakeet, VAD, diarization.

---

## 8. Status

| Milestone | Status | Notes |
|---|---|---|
| M0 Foundations | ✅ Done | Workspace (6 lib crates + `quill-cli`), lint config and CI workflow; first CI run green on macOS + Windows (2026-10-11). Learning pass is ongoing alongside M1. |
| M1 Transcribe file | 🟡 In progress | `quill-cli transcribe` / `models` / `bench` working on macOS with Metal (turbo ≈ 24× real time, see `eval/benchmarks.md`). Resampling pulled forward into `quill-audio`. Beam search is the default after greedy decoding sent large-v3 into a repetition loop. CI green on macOS + Windows (whisper.cpp builds with MSVC). Left: Windows benchmark. |
| M2 Capture audio | 🟡 In progress | `record` / VAD / track merge working on macOS: real mic + system capture in sync, echo dropped in the merged transcript. Left: record a real Zoom/Meet/Teams call; Windows loopback test. |
| M3 Summarize | ⬜ | |
| M4 Eval harness | ⬜ | |
| M5 Meeting MVP | ⬜ | |
| M6 Dictation | ⬜ | |
| M7 Distribution | ⬜ | |

**Current focus:** M2 acceptance: record a real call on macOS, then test on Windows. Primary dev machine is now the MacBook (M3 Pro); Windows for platform testing.
**Open decisions:** project name; license; Svelte vs React; default summary model (decided by M4); whether `quill-eval` ships its own binary or a `quill-cli eval` subcommand (M4).

**Workspace notes (M0):**
- Section 6 conventions are enforced as lints in the root `Cargo.toml`: `unsafe_code` is denied, and clippy's `unwrap_used`, `expect_used` and `undocumented_unsafe_blocks` are warnings, which CI's `-D warnings` turns into errors. `clippy.toml` allows unwrap/expect in tests; `main` setup must opt in with a local `#[allow]`.
- `.gitattributes` forces LF line endings so golden-file tests compare the same on Windows.
- Windows needs Visual Studio 2022 Build Tools (C++ workload) for the MSVC linker.

**Notes (M1):**
- The default transcription model is `turbo` (large-v3-turbo). `turbo-q5` is just as fast on Metal at a third of the download size, so it's a candidate for dictation, where the model stays in memory.
- The ggml Metal backend compiles its shaders the first time a model loads after a build (~6 s). macOS caches the result, and later loads take under 1 s.
- whisper.cpp has built-in Silero VAD (`FullParams::enable_vad`, using a ggml VAD model). For M2, compare it with a separate VAD crate before adding one.
- Building whisper.cpp needs CMake and a C++ compiler. On macOS: `brew install cmake` plus the Xcode Command Line Tools (full Xcode isn't needed, because the Metal shaders are embedded). On Windows, the VS 2022 Build Tools C++ workload includes CMake.

**Notes (M2):**
- **whisper.cpp's built-in VAD does nothing through whisper-rs.** whisper.cpp only runs it in `whisper_full()`, and whisper-rs calls `whisper_full_with_state()`. `quill-stt` therefore runs Silero itself (`WhisperVadContext`), packs the speech regions together with 0.1 s gaps, and maps segment times back onto the original audio. The ignored test `vad_prevents_hallucinations_in_silence` guards this.
- **VAD is on by default.** In 65 s of faint room noise around the JFK clip, every model without VAD invented text ("I'm not sure what you're doing." ×13, "Thank you.") and misplaced timestamps. With VAD the output was clean. On continuous speech VAD costs about 20% speed and doesn't change turbo's or small's accuracy. Each track of a two-track meeting is roughly half silence, so it matters there.
- **Mapping segment times back** snaps a segment's start to the first speech region it substantially overlaps. Whisper often starts a segment in the previous region's trailing silence, which put replies before the questions they answered.
- **Echo:** with speakers instead of headphones, the mic hears the other people. A mic segment is dropped when at least 60% of its words appear in the system segments overlapping it (within ±1.5 s). On a synthetic 4-person meeting with 30% echo, all 54 segments were labelled and ordered correctly.
- **On macOS, opening a device blocks with no timeout** while a privacy prompt waits for an answer, and fails with "Unknown property" if nobody answers. `record` prints a hint after 3 s. The real app must request permissions up front (M5 onboarding).
- Tracks are saved at the device's native rate as 32-bit float WAV, plus a `recording.json` with each track's device name, lead-in, dropped samples and peak. A peak of exactly 0 is flagged as a likely missing permission.
- The macOS system-audio tap delivers its first buffer only when something starts playing, then continues through silence (checked across a 4 s pause). The lead-in padding handles the late start. The shared start time is taken just before the streams start, so time spent on a permission prompt isn't recorded as silence.
