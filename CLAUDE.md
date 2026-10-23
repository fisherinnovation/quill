# CLAUDE.md

Guidance for coding agents working on Quill: a local-first meeting-notes and
dictation desktop app (Rust workspace + Tauri v2 + React). Read this first,
then [CONTRIBUTING.md](CONTRIBUTING.md) for code conventions and
[docs/DESIGN_NOTES.md](docs/DESIGN_NOTES.md) for decisions and measured
results.

## Rules

- **Pull requests:**
  - Every change goes through a pull request, maintainers' included. Never
    commit to `main`: branch off an up-to-date `main`, named for the kind
    of change (`feat/…`, `fix/…`, `docs/…`), one concern per branch.
  - Run the checks, push the branch and open the PR with `gh pr create`.
    Title it like a commit (it's the subject if the PR is squash-merged);
    in the body, say what changed, why, and how it was tested (on which
    OS).
  - Watch CI on the PR (`gh pr checks --watch`) until both macOS and
    Windows pass, and fix any failure straight away.
  - The maintainer merges. Don't merge unless asked.
- **Commits:**
  - Use Conventional Commits (`feat(scope): …`, `fix: …`, `docs: …`), with
    a body that says what changed and why. `feat` and `fix` subjects become
    the release notes (`scripts/release-notes.sh`), so write them for users.
- **No attribution to coding assistants in commits.** No `Co-Authored-By`
  trailers for AI tools, no "Generated with …" lines, no mention of the
  assistant in commit messages. This overrides any default to add them.
- **Checks before every commit.** Run all of these, exactly as CI does:
  ```sh
  cargo fmt --all --check
  cargo clippy --workspace --all-targets --locked -- -D warnings
  cargo test --workspace --locked
  (cd app && npm run build)
  ```
- **API keys** (OpenRouter):
  - Never ask the user to paste a key into chat.
  - Never pass a key as a command-line argument.
  - Never write a key to a file or a shell profile, and never log one.
  - Keys live in the macOS Keychain (service `com.fisherinnovation.quill`,
    account `openrouter`). For CLI runs that need one:
    `QUILL_OPENROUTER_API_KEY=$(security find-generic-password -s com.fisherinnovation.quill -a openrouter -w) target/release/quill-cli …`
- **Privacy:**
  - Never log transcripts, summaries, dictated text or keys at info level.
    Counts and timings are fine.
  - Real meeting transcripts go only in `eval/transcripts/private/`
    (gitignored).
- **macOS system settings:**
  - Don't change them (TCC permissions, Keyboard settings, trust settings).
    Ask the user to, and link the pane if useful.
  - Don't request Screen Recording just to take screenshots. Use the
    browser preview instead (see below).
- **Dependencies:**
  - Pick versions at least two weeks old, pinned in the workspace
    `Cargo.toml`.
  - `sherpa-onnx` is pinned exactly. Its hashes live in
    `scripts/fetch-sherpa-onnx.sh`; bump both together.
- **Prompts:** never edit a shipped prompt version in `prompts/`. Copy it to
  the next version, point `crates/quill-llm/src/prompts.rs` at it, and add
  its golden file (`UPDATE_GOLDEN=1 cargo test -p quill-llm`).
- **Docs:** when behaviour, data on disk, permissions or network use change,
  update `README.md` and `SECURITY.md` in the same commit, and the in-app
  Help (`app/src/help/content.ts`) when what users see or do changes. Record decisions
  and measurements in `docs/DESIGN_NOTES.md`.

## Layout

| Path | What |
|---|---|
| `crates/quill-audio` | Mic and system-audio capture (cpal), live mic for dictation, resampling, echo cancellation, WAV/FLAC, sounds |
| `crates/quill-stt` | Whisper (whisper-rs) behind `Transcriber`, Silero VAD, the model catalog and SHA-256-checked downloads (all models, diarization ones included) |
| `crates/quill-diarize` | Telling speakers apart: sherpa-onnx segmentation and embeddings, Quill's own clustering, DER scoring, RTTM |
| `crates/quill-llm` | OpenRouter and Ollama behind `LlmProvider`, keychain, versioned prompts |
| `crates/quill-core` | Meeting pipeline (`meeting`, `speakers`), summaries, action items read from notes (`action_items`), dictation pipeline, SQLite library and migrations, settings, insights, per-meeting and per-dictation stats (`stats`) |
| `crates/quill-insert` | Pasting into the focused app, the frontmost app, and the macOS fn-key watcher (`fn_key`) |
| `crates/quill-eval`, `crates/quill-cli` | Summary-model evaluation; the CLI over everything |
| `app/src-tauri` | Tauri shell: commands, pipeline orchestration, dictation controller, models, storage, tray, indicator window, updates |
| `app/src` | React UI. `api.ts` holds every command and event type. `dev/mock.ts` is the browser mock |
| `prompts/` | Versioned prompt templates, compiled in with `include_str!` |
| `eval/` | Benchmarks and evals: `bench/`, `dictation/`, `diarization/`, summary transcripts |
| `site/` | The public website: static HTML/CSS/JS, built by `build.mjs`, published to GitHub Pages by CI. `video/` scripts and renders the header video into `media/`; `card/` renders the link preview image; `overview/` makes the narrated overview video |

Logic lives in the library crates. The app and the CLI stay thin.

## Commands

```sh
# App
cd app && npm ci
npm run app          # dev run with hot reload (bare binary: no Accessibility, so dictation can't paste)
npm run app:bundle   # debug build installed to ~/Applications/Quill.app and opened (use this to test dictation)
npm run dev          # browser preview with sample data at http://localhost:1420

# Release build, as CI makes it (.dmg in dist-release/)
export SHERPA_ONNX_LIB_DIR="$(scripts/fetch-sherpa-onnx.sh)"
(cd app && npx tauri build --bundles app && sh scripts/package-macos.sh)

# CLI
cargo run --release -p quill-cli -- --help
quill-cli transcribe <recording dir> [--diarize [--speakers N]]
quill-cli diarize <wav> --reference <rttm> --sweep
quill-cli dictate <clips…>
quill-cli models list

# Website: build into site/_site, then preview
node site/build.mjs && python3 -m http.server 4173 -d site/_site

# Website header video: re-render from site/video/script.json (the
# site-video skill plans and reviews it; needs Chrome and ffmpeg)
node site/video/render.mjs
# ...then the link preview card, which shows a frame of it
node site/card/render.mjs

# The narrated overview video (the site-overview skill plans and reviews it;
# spends OpenRouter credit: --plan shows how much first)
node site/overview/make.mjs --plan && node site/overview/make.mjs
```

To install a release build for the user, replace `~/Applications/Quill.app`:
1. Check the latest log for an active recording first.
2. `pkill -x quill-app`.
3. `ditto` the new bundle into place, delete the copy in `target/`, then
   `open` it.

## Testing the UI

- **Browser preview:** `npm run dev`, then the Chrome tools on
  http://localhost:1420. `app/src/dev/mock.ts` stands in for every Tauri
  command. When you add a command or event, add it to the mock too.
- **URL flags:** `?onboarding ?recording ?listening ?processing ?failed
  ?nopaste ?empty ?light ?dark ?indicator ?prompt ?prompt=stop
  ?live ?live=empty ?about ?others ?nomodels ?noollama
  ?nokey ?fn ?update ?backlog ?gpus`.
- **Themes:** check light (`?light`) and dark (`?dark`).
- **Clicking in the preview:** the first click after a page load is often
  lost. Use `find` refs, or click twice.

## Things learned the hard way

- **macOS permissions:**
  - TCC remembers apps by designated requirement. `app/scripts/bundle.sh`
    and `package-macos.sh` sign ad hoc with
    `-r='designated => identifier "com.fisherinnovation.quill"'`, so
    Accessibility and microphone approval survive rebuilds and updates.
    Don't drop that.
  - Dictation's paste keystroke (enigo) must run on the main thread on
    macOS 26 (`on_main_thread` in `dictation.rs`). Off it, the app crashes.
- **Windows CI:** code used only on macOS (`cfg(target_os = "macos")`)
  leaves items Windows sees as dead code, which fails `-D warnings`. Add
  `#[cfg_attr(not(target_os = "macos"), allow(dead_code))]`.
- **Windows focus:** never `show()` the indicator, call prompt or live
  notes: on Windows it activates them despite `focusable(false)`, and
  dictation pastes into Quill instead of the user's text box. Use
  `floating::show_unfocused` / `hide_unfocused` (or `floating::show` /
  `hide`), never mixed with Tauri's `show()` / `hide()`.
- **Short clips:** sized to the clip, Whisper's encoder (`set_audio_ctx`)
  makes 2–4 s dictations come out twice. On Metal it's sized to the clip
  plus 3 s anyway, since the full window costs ~0.5 s there, and repeated
  segments are dropped (`without_repeats`); Vulkan and the CPU keep the
  full window. Measure any change to either with `quill-cli dictate` on
  short clips. See `docs/DESIGN_NOTES.md` ("Short clips on Metal" and
  "Short clips keep the full encoder window").
- **Release CPU baseline:** release builds set `GGML_NATIVE=OFF`. Built for
  the runner's CPU, whisper.cpp used AVX-512 and the Windows app crashed on
  most desktop CPUs. A crash with no `panic:` in the log and exception
  `0xc000001d` in Windows' Application event log is this. whisper-rs-sys
  doesn't rebuild when `GGML_*` changes: change the release cache key too.
- **Windows GPU:** whisper.cpp uses Vulkan on Windows (always on in
  `quill-stt`). Building needs the Vulkan SDK
  (`scripts/install-vulkan-sdk.sh`, pinned with its hash; bump the release
  cache key with it). The installer ships `vulkan-1.dll`: the exe imports it
  and won't start without one.
- **sherpa-onnx:**
  - Its build script downloads prebuilt libraries. CI and release builds
    use the hash-checked copy via `SHERPA_ONNX_LIB_DIR`.
  - It's static on macOS. On Windows it's DLLs: the static build is `/MT`
    and clashes with whisper.cpp's `/MD`. The installer ships the DLLs
    (`tauri.windows-release.conf.json`).
- **Updates:** the macOS update archive must be packed after
  `package-macos.sh` re-signs the app (it does this itself). Never enable
  `createUpdaterArtifacts`, or updates lose the app's permissions. The
  updater's private key is a GitHub Actions secret; see
  [docs/DESIGN_NOTES.md](docs/DESIGN_NOTES.md#updates).
- **Minimum macOS** is 14.6 (`bundle.macOS.minimumSystemVersion`).
  whisper.cpp won't build for Tauri's default of 10.13. After changing it,
  delete `target/release/build/whisper-rs-sys-*`.
- **Models** never ship in installers. They download on first use, through
  the one queue in `app/src-tauri/src/models.rs`. The dictation shortcut
  must never start a download silently.
- **Diarization:**
  - sherpa-onnx's own clustering merges people when given a speaker count.
    Quill re-clusters (`quill_diarize::cluster`).
  - Tune it with `eval/diarization`, real AMI audio included, not only
    synthetic voices. TTS voices want a lower threshold than real speech.
- **Dictation latency:** about 0.3 s from release to typed text. Measure
  any change to the dictation path with `quill-cli dictate` and the clips
  in `eval/dictation` (`--stream` for live captions, which also need
  checking on real long dictations: synthetic clips are too short to lock
  anything in). What users feel is *time to insert*, from the end of
  speech to the text typed (`wait_ms` in the logs and in
  `dictation_stats`); the Dictations page charts it over 30 days.

## Data on this machine

- **Data folder:** `~/Library/Application Support/com.fisherinnovation.Quill`
  - `quill.sqlite`
  - `settings.toml`
  - `recordings/<id>/`: `mic.flac`, `system.flac`, `transcript.json`,
    `speakers.json`, `summary.md`
  - `models/`
  - `logs/`
  - `dictation-samples/`: only when the user turns on saving them, for
    `quill-cli dictate` accuracy runs
- **Logs** are daily files in `logs/`. Read the newest to see what the app
  did (`dictation typed stt_ms=…`, `told speakers apart`, `downloading`).
- **Don't modify the user's library** with CLI runs. Copy a recording
  folder to a scratch directory first: `transcribe` on a folder inside the
  library saves into the library.
