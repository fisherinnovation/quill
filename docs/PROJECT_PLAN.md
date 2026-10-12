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
- [x] `quill-llm`: OpenRouter client (OpenAI-compatible chat completions), API key from `keyring`. *(`quill-cli keys set openrouter`; the key is stored in the macOS Keychain or Windows Credential Manager. `QUILL_OPENROUTER_API_KEY` overrides it.)*
- [x] Capture token usage and cost per request. *(As of 2026-10 OpenRouter always includes `usage`, with `usage.cost` in credits (USD). The old `usage: {include: true}` option is deprecated and has no effect.)*
- [x] Provider privacy settings. *(Sent as `provider: {data_collection: "deny"}` by default; `--zdr` adds `zdr: true`, and `--allow-data-collection` turns the default off.)*
- [x] Ollama provider using the same trait. *(LM Studio not yet. Ollama uses the native `/api/chat` endpoint with `num_ctx` 32k and `think: false`: its OpenAI-compatible endpoint can't set the context window, so input beyond the small default would be silently cut off.)*
- [x] Map-reduce summarization for long transcripts. *(Single pass when the transcript fits the model's context minus 6k tokens; otherwise parts of ≤15k tokens, run in parallel on cloud models and one at a time locally, then a combining step.)*

**Accept:** `quill-cli summarize transcript.json --model <id>` produces the template summary and prints its cost and latency.

### M4 — Evaluation harness (`quill-eval`)
- [x] Input: a folder of transcripts × a list of model IDs × prompt version(s). *(`quill-cli eval -m a,b,ollama:c [--prompts summary-v1,…]`; reads `eval/transcripts/*.json` and `private/*.json`.)*
- [x] Run all combinations concurrently (bounded by `--concurrency`, using `buffer_unordered`); store outputs, tokens, cost and latency in SQLite (`eval/results/eval.sqlite`, appended on every run) plus a CSV export. A failed combination is recorded, not fatal.
- [x] LLM-as-judge scoring against the rubric: completeness, accuracy, action items, faithfulness, formatting, overall (`prompts/judge-v1`). Per transcript, the judge sees all candidates at once under shuffled letter labels and no model names. Default judge: `anthropic/claude-opus-5.5`.
- [x] Manual review mode: `review.md` shows each transcript's summaries side by side with scores and the judge's notes.
- [x] Output a leaderboard: quality score vs. cost per meeting-hour (`leaderboard.md`, also printed).

**Accept:** One command produces a ranked table that answers which model is best for the money. Choose the default model from this.

### M5 — Tauri app: meeting notes MVP
- [x] Tauri v2 shell with a tray/menu-bar icon: Start/Stop recording, recording indicator. *(Template mic icon with a "● m:ss" timer while recording. Closing the window keeps Quill running in the menu bar.)*
- [x] Meetings list, transcript view (with speaker labels and timestamps), summary view, re-summarize. *(Re-summarize uses the model currently set in Settings; picking a prompt version in the UI isn't done yet.)*
- [ ] Settings: STT model, LLM provider/model, API key, storage location, prompt template. *(Done: STT model, language, VAD, summary model, ZDR, auto-summarize, and the API key, which is saved to the keychain by the app. Still to do: storage location, prompt template.)*
- [ ] Permission onboarding (mic, screen/system audio on macOS) with clear explanations. *(The Info.plist usage strings are in place. The onboarding screen isn't done.)*
- [x] Full-text search over transcripts (SQLite FTS5). *(Search as you type; clicking a hit jumps to that transcript line.)*
- [ ] Export summary as Markdown; copy to clipboard. *(Copy is done; exporting to a file isn't.)*

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
- [x] Installers built in CI (`.github/workflows/release.yml`): a version tag builds a macOS `.dmg` (Apple Silicon, signed ad hoc with Quill's stable designated requirement, so permissions survive updates) and Windows NSIS/MSI installers (with the sherpa-onnx DLLs), and attaches them to a draft release.
- [ ] Code signing + notarization (macOS: Developer ID, set `APPLE_SIGNING_IDENTITY` in `app/scripts/package-macos.sh`, plus notarytool), signing (Windows).
- [ ] Auto-update (`tauri-plugin-updater`).
- [ ] Crash/error logging to a local file; opt-in only for anything remote.
- [x] Models on a fresh install: onboarding offers the download (size shown, progress, can continue meanwhile); the dictation shortcut never starts a silent download, it opens the Dictation page with progress instead; Settings → Models lists and downloads models, and deletes or re-downloads any of them (or all), including those in use, with a confirmation; the loaded Whisper model is released so the next use loads the fresh copy. All downloads go through one queue (`app/src-tauri/src/models.rs`), so two features never write the same file.
- [ ] Data retention settings (auto-delete audio after N days, keep transcripts).

### Stretch goals
- Live captions during meetings (streaming chunks).
- Local speaker diarization with named speakers (voice fingerprints).
  - [x] Phase 1 (trial): `quill-diarize` crate, `quill-cli diarize` with scoring, and `quill-cli transcribe <dir> --diarize [--speakers N]` (splits "Others" into Speaker 1..N). Results are in `eval/diarization/README.md`.
  - [x] Phase 2: in the app. Speakers are told apart after transcribing (Settings → *Tell speakers apart*, on by default). The Transcript tab's speaker bar renames (a name another speaker has merges them) and re-splits into a chosen number of people, instantly from the cached analysis (`speakers.json`). Prompt `summary-v3` handles "Speaker N" and given names; on the test meeting it named Daniel, Moira and Karen from how they addressed each other and never wrote "Speaker N".
  - [ ] Phase 3: remembered voices across meetings, from stored per-speaker embeddings.
  - [ ] Optional: speaker names from the meeting app's active-speaker display (opt-in; reads window contents).
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
| M3 Summarize | 🟡 In progress | `summarize` working end to end with Ollama (qwen3:8b, local, 34 s for a 4-min meeting). Prompts are in `prompts/summary-v1/`. Left: first OpenRouter run (needs a key). |
| M4 Eval harness | 🟡 In progress | Harness complete and tested end to end with local models. Left: a cloud run with a strong judge (needs the OpenRouter key), more transcripts (real meetings in `private/`), then choose the default model. |
| M5 Meeting MVP | 🟡 In progress | Record → transcribe → summarize works end to end in the app (first real run: 27 s test meeting, Haiku summary in 4.1 s for $0.0004). Meeting library in SQLite with FTS5 search, shared with the CLI. First-run onboarding with an audio access check, Markdown export, and a movable recordings folder. Left: prompt choice, Windows check. |
| M6 Dictation | 🟡 In progress | Engine measured with `quill-cli dictate`: 0.3 s to type without polish, about 0.95 s with. App: global ⌥ Space hotkey (hold to talk or tap to toggle; Esc cancels), menu-bar status, start/stop sounds, paste into the focused app with the clipboard restored, vocabulary and replacements, history. Left: real-world testing, a floating indicator, per-app tone, a Windows check. |
| M7 Distribution | ⬜ | |

**Current focus:** M6 (dictation). Pending on real use: M2/M4 acceptance (a real call; a cloud eval with a strong judge). Primary dev machine is now the MacBook (M3 Pro); Windows for platform testing.
**Open decisions:** project name; default summary model (decided by M4; `anthropic/claude-haiku-5.5` until then). 

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

**Notes (M3):**
- The default model until M4 is `anthropic/claude-haiku-5.5` ($0.10/M input, $0.50/M output tokens, 1M context), so a one-hour meeting costs well under a cent. `-m ollama:<model>` keeps everything local.
- Prompts are compiled into the binary with `include_str!`, and rendering is strict: a placeholder without a value, or a value without a placeholder, is an error. A golden-file test covers the rendered single-pass prompt (`UPDATE_GOLDEN=1` regenerates it).
- `transcribe <dir>` saves `transcript.json` into the recording folder; `summarize <dir>` then writes `summary.md` and `summary.json` (model, prompt version, tokens, cost, latency) next to it.
- First local result (qwen3:8b on the synthetic meeting): accurate, and real names were inferred from context. It missed two decisions that appear in the TL;DR but not under "Decisions". That's a candidate for the M4 rubric and for summary-v2.

**Notes (M4):**
- Decided: the harness is the `quill-eval` library plus a `quill-cli eval` subcommand (one binary).
- Rubric lesson: the first version had a "hallucinations" criterion where 10 meant *none*. The local judge read "0 hallucinations" and scored 0. Renamed to "faithfulness" so every criterion points the same way (10 = best).
- Judge caveats: a judge that is also a candidate, or from the same family, may favour its own style. A small judge also misses subtle omissions (qwen3:8b gave itself 10/10 although its notes left two decisions out of "Decisions"). Use a strong judge from a different family than the leading candidates when choosing the default, and spot-check `review.md`.
- One synthetic transcript isn't enough to choose a default model. Add real meetings to `eval/transcripts/private/` (from `quill-cli transcribe <recording dir>` → `transcript.json`).

**Notes (M5):**
- Decided: Tauri v2 with React + TypeScript (Vite). Rust does all the work; the UI calls commands and listens for `recording-state` / `meeting-progress` events.
- The library (`quill.sqlite`, with audio in `recordings/<id>/`) is shared by the app and the CLI: `record` creates library meetings, and `import <dir>` adds existing recording folders.
- Recording runs on its own thread, controlled through channels (cpal streams aren't guaranteed to be movable between threads). Start/stop run off the main thread, because opening devices can block on a macOS permission prompt.
- Keychain with unsigned dev builds: a program that didn't create the item gets a password prompt, and "Always Allow" fails for unsigned binaries (it looks like a wrong password; "Allow" works). Mitigations: the app reads the key once per launch, and saving the key from Settings makes the app the item's owner. For development, `npm run app` (`app/scripts/dev.sh`) reads the key once with Apple's signed `security` tool (where "Always Allow" sticks) and passes it in `QUILL_OPENROUTER_API_KEY`, so rebuilds never touch the keychain. Code signing (M7) is the real fix.
- **Echo cancellation** (`quill_audio::aec`, WebRTC AEC3 via the pure-Rust `sonora` crate). After recording, the mic track is cleaned using the system track as the reference, before transcription. On the first real call (ChatGPT voice on speakers, 2:22):

  | | Echo leaked into "Me" words (`eval/bench/echo_leak.py`) | Notes |
  |---|---:|---|
  | No AEC (before) | 7.8% | "Me" text all lowercase, no punctuation; one of my sentences lost |
  | **AEC (now the default)** | **0.0%** | Proper punctuation/capitals; lost sentence recovered; 13% faster overall |
  | No AEC + punctuated prompt | 13.8% | Longer segments mixed in more echo |
  | AEC + punctuated prompt | 0.0% | 30 s+ segments, coarse timestamps, a sentence dropped |

  The missing punctuation was caused by the echo confusing Whisper, not by Whisper's style, so no initial prompt is needed. `quill-cli transcribe <dir> --no-aec` turns AEC off for comparison. The text-level echo filter in `merge_tracks` stays as a second line of defence.
- **Onboarding / access check** (`quill_audio::check`). macOS reports a denied permission as silence, not an error, so the first-run screen records 4 s from both sources while playing a quiet chime. The system tap records every process, Quill included, so hearing nothing (not even the chime) means system-audio access was denied; a working mic always picks up some noise. Running the check also triggers both permission prompts before the first real meeting. It can be re-run from Settings, and the test audio is deleted afterwards.
- **Recordings folder** (`Settings.recordings_dir`, default `<data dir>/recordings`). Changing it moves every meeting folder (rename, or copy then delete across drives). It's all or nothing, and refused while recording or processing. The database stays in the data directory. Recording writes about 2 GB per hour (32-bit float, 48 kHz, stereo system track); once a meeting is transcribed the tracks are replaced with 16 kHz mono 16-bit FLAC, about 70 MB per hour (the 2:22 test call went from 81.7 MB to 2.8 MB). It's the exact audio the pipeline uses; word error rate on the benchmark meeting is the same (2.6%) from the FLAC as from the WAV, though beam search makes the wording not byte-identical. Older meetings are compacted at startup; `quill-cli compact` does it by hand.
- File and folder pickers (export, recordings folder) open from Rust via `tauri-plugin-dialog`, so the web view never passes in a path to write to and needs no plugin permissions.

**Notes (diarization trial):**
- **Library:** the official `sherpa-onnx` Rust crate (1.13.8), pinned exactly. Its build script downloads prebuilt sherpa-onnx and onnxruntime libraries from the project's GitHub release. These are static on macOS. On Windows the DLLs are used instead, because the static build uses `/MT` and whisper.cpp uses `/MD`. That download isn't hash-checked, so CI (and anyone who wants to) uses `scripts/fetch-sherpa-onnx.sh` instead. It checks the archive against pinned SHA-256 hashes and sets `SHERPA_ONNX_LIB_DIR`. Release builds should do the same.
- **Models:** pyannote segmentation-3.0 (MIT, 6 MB) and NeMo TitaNet-small (CC BY 4.0, 40 MB). Both go through the same SHA-256-checked downloader as Whisper.
- **Clustering:** sherpa-onnx's clustering (complete linkage, no outlier handling) merged real people as soon as a speaker count was given. Quill re-clusters embeddings of clean pieces of each turn by centroid distance, and folds groups with under 4 s of speech into the nearest speaker. On a 10-person call (real AMI voices through Opus):
  - without a speaker count, 7 of 10 speakers are found, with 8.6% confusion; the people who said about 5 s were merged into others;
  - with the count, 10 of 10, with 5.9% confusion.
- **AMI accuracy:** about 18–22% DER, in line with pyannote's published 18.8%. Confusion is 4–6%; the rest is segmentation.
- **Speed:** about 35× real time on the M3 Pro CPU.

**Notes (M6):**
- Latency: Whisper `short_clip` mode (greedy decoding, the encoder sized to the clip, VAD trimming only the outer silence) takes 0.27 s median on turbo, versus 0.6–0.7 s with meeting settings. The polish step (default `google/gemini-3.5-flash-lite`, 0.66 s median) runs only when `needs_polish` finds fillers, corrections, lists or stutters; on clean speech the model returned Whisper's text unchanged 7 times out of 9. Details are in `eval/dictation/README.md`.
- Language: auto-detection on the shortened encoder window produced nonsense, and doing it properly costs about 0.5 s, so dictation defaults to the system language.
- Polish safety: `check_polish` rejects output that's much longer (the model answered or embellished), much shorter (it summarized) or starts with a preamble, and the raw transcript is typed instead.
- The hotkey (`tauri-plugin-global-shortcut`) works two ways: hold it to talk (release after at least 350 ms), or tap it to toggle. Esc is bound globally only while listening. Registering a shortcut blocks on the main thread, so it's only ever done from the dictation controller's thread.
- The mic stays open for 30 s after each dictation (with a 300 ms pre-roll) so back-to-back dictations start instantly; macOS shows the orange mic indicator meanwhile.
- Insertion: `quill-insert` copies to the clipboard (marked concealed, so clipboard managers skip it), simulates ⌘V, then restores the previous text or image. Without Accessibility permission the text is left on the clipboard and the UI says so.
- Logs: daily files in `<data dir>/logs` (two weeks kept), including panics; Settings → Troubleshooting opens the folder.
- Insights page: dictation words, time saved against typing at 40 wpm, speaking speed, streak, a 30-day activity chart and time of day; meeting count and time, your talk share, summary cost. Dictation numbers come from a `dictation_stats` table (counts only, no text), so they survive turning history off or clearing it. Computed on demand in `quill_core::insights`.
- Per-app tone: when the shortcut is pressed, Quill reads the frontmost app (bundle id and name via NSWorkspace on macOS; executable via the Win32 API on Windows; nothing about its windows or contents). Each app has a style: Casual for chat (a one-sentence message loses its final period, applied locally with no model call), Formal for email, Technical for editors and terminals, or Default. Built-ins cover common apps and can be overridden per app on the Dictation page. When clean-up runs, prompt `dictation-v2` says where the text is going; tone doesn't make clean-up run more often. On the clips the model kept question marks and was never rejected. Insights lists the top apps, and history shows the app.
- License decided: MIT OR Apache-2.0 (the Rust ecosystem's usual dual license). README, CONTRIBUTING.md and SECURITY.md were added for open-source distribution.
- Dev signing: `app:bundle` signs ad hoc with an explicit designated requirement (`identifier "com.fisherinnovation.quill"`) instead of the default per-build cdhash, so macOS permissions and keychain "Always Allow" survive rebuilds. That's weaker than a certificate (anything signed with that identifier matches) but fine for development; releases need Developer ID signing (M7).
