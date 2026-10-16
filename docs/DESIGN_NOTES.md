# Design notes

Why Quill works the way it does: decisions, measurements and platform
quirks that the code alone doesn't explain. What Quill does is in the
[README](../README.md); how to work on it is in
[CONTRIBUTING.md](../CONTRIBUTING.md). Add a note here when you make a
decision someone might otherwise undo.

## Workspace

- Logic lives in library crates; the app and `quill-cli` are thin layers
  over them. The CLI came first, so every stage can be run and measured on
  its own.
- The conventions in CONTRIBUTING are enforced as lints in the root
  `Cargo.toml`: `unsafe_code` is denied, and clippy's `unwrap_used`,
  `expect_used` and `undocumented_unsafe_blocks` are warnings, which CI's
  `-D warnings` turns into errors. `clippy.toml` allows unwrap/expect in
  tests; `main` setup must opt in with a local `#[allow]`.
- `.gitattributes` forces LF line endings so golden-file tests compare the
  same on Windows.
- The app is Tauri v2 with React + TypeScript (Vite). Rust does all the
  work; the UI calls commands and listens for events.
- License: MIT OR Apache-2.0, the Rust ecosystem's usual dual license.

## Transcription

- The default model is `turbo` (large-v3-turbo), about 24× real time on an
  M3 Pro with Metal. `turbo-q5` is just as fast on Metal at a third of the
  download size.
- Beam search is the default: greedy decoding sent large-v3 into a
  repetition loop.
- The ggml Metal backend compiles its shaders the first time a model loads
  after a build (~6 s). macOS caches the result; later loads take under 1 s.
- **Release builds use `GGML_NATIVE=OFF`.** ggml otherwise builds for the
  build machine's CPU. The Windows runners have AVX-512, so the v0.1.0
  installer died with an illegal instruction (`0xc000001d`) on the first
  dictation on an i9-14900KF, which has none. Off, ggml targets AVX2, FMA
  and F16C on x64, and the M1 baseline on Apple Silicon, where Metal does
  the heavy lifting anyway. whisper-rs-sys passes `GGML_*` variables to
  CMake but doesn't rebuild when they change, so changing one also needs a
  new release cache key.
- **Windows uses Vulkan for the GPU** (since 2026-10). It runs on Nvidia,
  AMD and Intel alike, and its runtime comes with every GPU driver, where
  CUDA would mean Nvidia only and several hundred MB of CUDA DLLs in the
  installer. `quill-stt` always enables it on Windows, as it does Metal on
  macOS. Building needs the Vulkan SDK (`scripts/install-vulkan-sdk.sh`,
  pinned and hash-checked).
  - whisper-rs-sys links `vulkan-1.lib`, so `quill-app.exe` imports
    `vulkan-1.dll` and wouldn't start without it. The installer ships the
    loader next to the exe. With a loader but no usable GPU, ggml's Vulkan
    registration catches the error and whisper.cpp uses the CPU.
  - whisper.cpp takes the first GPU Vulkan lists, integrated or dedicated
    (`gpu_device` 0). Not yet measured on a PC with both.
  - Don't call `whisper_rs::vulkan::list_devices()` to show the device: it
    creates a logical device per GPU, and with no driver ggml's instance
    setup throws through FFI. whisper.cpp logs the device it picks at info
    (`RUST_LOG=info,whisper_rs=info`).
- **whisper.cpp's built-in VAD does nothing through whisper-rs.**
  whisper.cpp only runs it in `whisper_full()`, and whisper-rs calls
  `whisper_full_with_state()`. `quill-stt` therefore runs Silero itself
  (`WhisperVadContext`), packs the speech regions together with 0.1 s gaps,
  and maps segment times back onto the original audio. The ignored test
  `vad_prevents_hallucinations_in_silence` guards this.
- **VAD is on by default.** In 65 s of faint room noise around the JFK
  clip, every model without VAD invented text ("I'm not sure what you're
  doing." ×13, "Thank you.") and misplaced timestamps. With VAD the output
  was clean. On continuous speech VAD costs about 20% speed and doesn't
  change accuracy. Each track of a two-track meeting is roughly half
  silence, so it matters there.
- **Mapping segment times back** snaps a segment's start to the first
  speech region it substantially overlaps. Whisper often starts a segment in
  the previous region's trailing silence, which put replies before the
  questions they answered.

## Capture and echo

- Tracks are saved at the device's native rate as 32-bit float WAV, plus a
  `recording.json` with each track's device name, lead-in, dropped samples
  and peak. A peak of exactly 0 is flagged as a likely missing permission.
- **On macOS, opening a device blocks with no timeout** while a privacy
  prompt waits for an answer, and fails with "Unknown property" if nobody
  answers. So the app asks for permissions up front (onboarding), and
  start/stop run off the main thread.
- Recording runs on its own thread, controlled through channels (cpal
  streams aren't guaranteed to be movable between threads).
- The macOS system-audio tap delivers its first buffer only when something
  starts playing, then continues through silence. Lead-in padding handles
  the late start. The shared start time is taken just before the streams
  start, so time spent on a permission prompt isn't recorded as silence.
- **Echo cancellation** (`quill_audio::aec`, WebRTC AEC3 via the pure-Rust
  `sonora` crate) cleans the mic track after recording, with the system
  track as the reference. On the first real call (ChatGPT voice on
  speakers, 2:22):

  | | Echo leaked into "Me" words (`eval/bench/echo_leak.py`) | Notes |
  |---|---:|---|
  | No AEC | 7.8% | "Me" text all lowercase, no punctuation; a sentence lost |
  | **AEC (the default)** | **0.0%** | Proper punctuation and capitals; sentence recovered; 13% faster overall |
  | No AEC + punctuated prompt | 13.8% | Longer segments mixed in more echo |
  | AEC + punctuated prompt | 0.0% | 30 s+ segments, coarse timestamps, a sentence dropped |

  The missing punctuation came from the echo confusing Whisper, not from
  Whisper's style, so no initial prompt is used. `transcribe --no-aec`
  turns AEC off for comparison.
- A text-level echo filter in `merge_tracks` stays as a second line of
  defence: a mic segment is dropped when at least 60% of its words appear
  in overlapping system segments (within ±1.5 s).
- **Access check** (`quill_audio::check`). macOS reports a denied
  permission as silence, not an error, so onboarding records 4 s from both
  sources while playing a quiet chime. The system tap records every
  process, Quill included, so hearing nothing (not even the chime) means
  system-audio access was denied; a working mic always picks up some noise.
- **Storage.** Recording writes about 2 GB per hour; once transcribed, the
  tracks become 16 kHz mono 16-bit FLAC, about 70 MB per hour. That's the
  exact audio the pipeline uses: word error rate on the benchmark meeting
  is the same (2.6%) from FLAC as from WAV. Moving the recordings folder is
  all or nothing and refused while recording or processing.
- File and folder pickers open from Rust (`tauri-plugin-dialog`), so the
  web view never passes in a path to write to.

## Telling speakers apart

- **Library:** the official `sherpa-onnx` crate, pinned exactly. Its build
  script downloads prebuilt libraries without checking them, so CI and
  release builds use `scripts/fetch-sherpa-onnx.sh`, which checks pinned
  SHA-256 hashes. Static on macOS; DLLs on Windows, because the static
  build uses `/MT` and whisper.cpp uses `/MD`.
- **Models:** pyannote segmentation-3.0 (MIT, 6 MB) and NeMo TitaNet-small
  (CC BY 4.0, 40 MB), through the same SHA-256-checked downloader as
  Whisper. Other embedding models are in the catalog for comparison
  (`quill-cli diarize --voice`).
- **Clustering:** sherpa-onnx's own clustering (complete linkage, no
  outlier handling) merged real people as soon as a speaker count was
  given. Quill re-clusters embeddings of clean pieces of each turn by
  centroid distance, and folds groups with under 4 s of speech into the
  nearest speaker. On a 10-person call (real AMI voices through Opus):
  - without a speaker count, 7 of 10 speakers found, 8.6% confusion; the
    people who said about 5 s were merged into others;
  - with the count, 10 of 10, 5.9% confusion.
- **Accuracy:** about 18–22% DER on AMI, in line with pyannote's published
  18.8%. Confusion is 4–6%; the rest is segmentation. Tune with
  `eval/diarization` on real audio, not only synthetic voices.
- **Speed:** about 35× real time on the M3 Pro CPU.

## Summaries and prompts

- Default model `anthropic/claude-haiku-5.5`: a one-hour meeting costs
  well under a cent. `ollama:<model>` keeps everything local.
- Prompts are compiled in with `include_str!`, and rendering is strict: a
  placeholder without a value, or a value without a placeholder, is an
  error. Golden-file tests cover the rendered prompts. Shipped versions are
  never edited, so results stay comparable; every summary records its
  prompt version and model.
- Evaluation is the `quill-eval` library plus `quill-cli eval`. Rubric
  lesson: a "hallucinations" criterion where 10 meant *none* got scored 0
  by a local judge reading "0 hallucinations"; every criterion now points
  the same way (10 = best, "faithfulness").
- Judge caveats: a judge that is also a candidate, or from the same
  family, may favour its own style, and a small judge misses subtle
  omissions. Use a strong judge from a different family, spot-check
  `review.md`, and evaluate on real meetings in
  `eval/transcripts/private/`, not only the synthetic one.

## Dictation

- **Latency:** Whisper `short_clip` mode (greedy decoding, the encoder
  sized to the clip, VAD trimming only the outer silence) takes 0.27 s
  median on turbo, versus 0.6–0.7 s with meeting settings. Details in
  `eval/dictation/README.md`.
- **Clean-up only when needed:** `needs_polish` runs the model only for
  fillers, corrections, lists or stutters; on clean speech the model
  returned Whisper's text unchanged 7 times out of 9. Default
  `google/gemini-3.5-flash-lite`, 0.66 s median.
- **Clean-up safety:** `check_polish` rejects output that's much longer
  (the model answered or embellished), much shorter (it summarized) or
  starts with a preamble, and the raw transcript is typed instead.
- **Language:** auto-detection on the shortened encoder window produced
  nonsense, and doing it properly costs about 0.5 s, so dictation defaults
  to the system language.
- **Shortcut:** hold to talk (released after at least 350 ms) or tap to
  toggle. Esc is bound globally only while listening. Registering a
  shortcut blocks on the main thread, so it's only done from the dictation
  controller's thread. The paste keystroke must run on the main thread on
  macOS 26 (`on_main_thread`), or the app crashes.
- **Warm mic:** the mic stays open for 30 s after each dictation, with a
  300 ms pre-roll, so back-to-back dictations start instantly. macOS shows
  the orange mic indicator meanwhile.
- **Insertion:** copy to the clipboard (marked concealed, so clipboard
  managers skip it), simulate ⌘V, then restore the previous text or image.
  Without Accessibility the text is left on the clipboard.
- **Per-app style:** Quill reads only the frontmost app's identity (bundle
  id and name via NSWorkspace; the executable on Windows), never its
  windows or contents. The Casual rule (drop a one-sentence message's final
  period) is applied locally; the other styles go into the clean-up prompt
  and never make clean-up run more often.
- **Learning from corrections:** `quill_core::corrections::learn` aligns
  the typed and corrected words (LCS) and suggests only safe kinds of fix:
  a term spelled differently → vocabulary; several words typed as one term
  → replacement. Homophones, grammar edits, insertions, deletions and
  rewrites are ignored, because as rules they'd be wrong as often as
  right. Nothing is learned until the user accepts it.
- **About you** goes to the clean-up model (prompt `dictation-v3`). On the
  10 eval clips it changed one word and never leaked into the output.
- **Measuring accuracy:** the TTS clips are too clean to judge accuracy on
  real voices, so the app can keep real dictations (opt-in, off by
  default): the 16 kHz audio Whisper heard, the typed text and the app.
  A reference (`clip.txt`) is written by hand, and `quill-cli dictate`
  scores Whisper's text and the typed text by word error rate. Words are
  compared lowercased, without outer punctuation or list markers, so the
  score counts wrong words, not formatting; "10:30" vs "10.30" still
  counts. Where several forms are right ("section 4" or "four"), the
  reference uses the one Quill produces. Samples are FLAC (about 1 MB per
  minute) and outside the database, so deleting them is one folder.
- **Planned:** opt-in learning by watching the inserted text through
  Accessibility for about a minute. First check which apps expose it.

## Insights and logs

- Dictation numbers come from a `dictation_stats` table (counts only, no
  text), so they survive turning history off or clearing it. Time saved
  is measured against typing at 40 wpm.
- Logs are daily files kept for two weeks, panics included, and never
  contain transcripts, notes, dictated text or keys.
- Meeting and dictation stats (`quill_core::stats`) are computed from the
  stored transcript or entry each time they're shown; nothing is cached,
  so renaming or re-splitting speakers shows up straight away. A
  "stretch" is one person talking with pauses of at most 5 s and nobody
  else starting; turns, the longest monologue and back-and-forths are
  counted in stretches. Talk time merges each person's overlapping
  segments so nothing is counted twice.
- Interruptions aren't counted: Whisper's segments are coarse (a segment
  can span a pause), so overlap between them doesn't reliably mean anyone
  talked over anyone.
- Stats charts reuse the transcript's speaker colours, so a person keeps
  their colour everywhere, but identity never rests on colour alone: each
  bar and timeline lane is labelled with the name. The palette (built for
  avatars) fails the colour-vision checks for charts; re-stepping it would
  recolour the transcript too, so that's left as a separate change.

## macOS signing and the keychain

- TCC and the keychain remember apps by designated requirement.
  `app/scripts/bundle.sh` and `package-macos.sh` sign ad hoc with
  `identifier "com.fisherinnovation.quill"` as the requirement, instead of
  the default per-build hash, so permissions and keychain "Always Allow"
  survive rebuilds and updates. That's weaker than a Developer ID
  certificate (anything signed with that identifier matches), which
  distribution will need.
- Unsigned dev builds (`npm run app`) look like a new app on every
  rebuild, so `app/scripts/dev.sh` reads the key once with Apple's signed
  `security` tool and passes it in `QUILL_OPENROUTER_API_KEY`.
- On macOS even reading a keychain item's attributes can prompt for the
  password, so Settings never touches the keychain just to show the key's
  status: it's "unknown" until the key is read or saved. Windows Credential
  Manager never prompts, so there Settings checks for the key and says
  plainly when it's missing. The key is read only when it's needed, once
  per launch.

## Updates

- Tauri's updater plugin, driven from Rust (`app/src-tauri/src/updates.rs`)
  so the web view gets no updater permissions. It reads `latest.json` from
  `releases/latest/download/`, which GitHub serves only for published
  releases: a draft is invisible to installed copies until it's published.
- Check daily (a 1-hour tick compared with the wall clock, since a long
  `sleep` stops counting while the computer sleeps); install only when the
  user clicks, and never while recording, processing, dictating, moving
  recordings or downloading a model, since installing restarts the app.
  Nothing is downloaded until the click: keeping a ~50 MB update in memory
  in a menu-bar app all day wasn't worth the instant restart.
- Update files are signed with a minisign key separate from OS code
  signing, so updates work with ad hoc and unsigned builds. The private key
  and its password are the `TAURI_SIGNING_PRIVATE_KEY` and
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` Actions secrets; losing them means
  installed copies can never update again, so the maintainer keeps a
  backup. `requireSignedVersion` is on: each signature carries the version
  (`tauri signer sign --app-version`), so a tampered `latest.json` can't
  pair a new version number with an older, validly signed release.
- macOS: the update is a `.app.tar.gz` that `package-macos.sh` packs *after*
  re-signing with the stable designated requirement. Tauri's own
  `createUpdaterArtifacts` would archive the app before that, and every
  update would then lose the microphone and Accessibility permissions.
  The updater downloads with its own HTTP client, so the new app should get
  no quarantine flag and Gatekeeper shouldn't ask again (to confirm with the
  first real update).
- Windows: both installers are update files. `latest.json` lists
  `windows-x86_64-nsis` and `windows-x86_64-msi` (the plugin looks for
  `{os}-{arch}-{installer}` first), so an MSI install updates with the MSI;
  plain `windows-x86_64` falls back to NSIS. NSIS installs per user, so
  updating needs no UAC prompt. The installer runs in passive mode (a
  progress bar) and reopens Quill.

## Website

- `site/` is plain HTML, CSS and JavaScript with system fonts: no web
  fonts, analytics or other third-party requests, to match what the app
  promises about privacy.
- `build.mjs` fills the template at build time. Download buttons appear
  only when there's a published release *and* the repository is public
  (a private repository's release files only download for its members);
  source links appear once it's public. So the site catches up on its own
  when either changes, at the next CI run on `main`.
- The header video is recorded from the app's browser preview, not drawn
  or generated, so it can only show what the app really does, and it can
  be re-rendered whenever the app changes (`site/video/render.mjs`, planned
  by the `site-video` skill). It drives headless Chrome over the DevTools
  protocol with Node's built-in WebSocket and assembles with ffmpeg, so it
  adds no npm packages. The screencast only records at the device scale
  Chrome itself runs at, hence `--force-device-scale-factor`.
- Captions sit on a scrim in the app's background colour, present from a
  scene's first frame so crossfades don't flash text through it. Light and
  dark videos are separate files (about 1.2 MB each); `main.js` picks one
  by `prefers-color-scheme` and leaves the poster with controls under
  `prefers-reduced-motion`.
- CI builds the site on every run, but only deploys when the repository
  variable `SITE_DEPLOY` is `true`, so CI stays green before Pages is
  enabled.
