# Contributing to Quill

Thanks for your interest in Quill! Bug reports, testing (especially on
Windows), prompt and model evaluations, documentation and code are all
welcome.

## Before you start

- **Bugs:** open an issue with your OS version, what you did, what you
  expected and what happened. For crashes, include the relevant lines from
  the log file (Settings → Troubleshooting → Show log files). Logs contain
  events, timings and errors, never transcripts or dictated text, but please
  glance over anything you paste.
- **Features and larger changes:** open an issue first to discuss the
  approach, so your time isn't spent on something that doesn't fit. The
  design notes in [`docs/DESIGN_NOTES.md`](docs/DESIGN_NOTES.md) explain why
  things are the way they are.
- **Security issues:** don't open a public issue. See [SECURITY.md](SECURITY.md).

## Setting up

Follow [Building from source](README.md#building-from-source) in the README.
You'll need stable Rust, Node.js 24, and CMake with a C/C++ compiler.

```sh
cd app && npm ci && npm run app   # runs the app with hot reload
```

The first build downloads prebuilt sherpa-onnx libraries (used to tell
speakers apart) from the project's GitHub release. To use a copy checked
against pinned hashes instead, as CI does:
`export SHERPA_ONNX_LIB_DIR="$(scripts/fetch-sherpa-onnx.sh)"`.

On Windows, building needs the Vulkan SDK too, for whisper.cpp's GPU
backend: `export VULKAN_SDK="$(scripts/install-vulkan-sdk.sh)"` installs
the hash-checked version CI uses.

On macOS, test dictation with `npm run app:bundle`, which installs a signed
`Quill.app` (see the tip in the README). Permissions survive rebuilds.

## Making changes

### Checks

Run these before opening a pull request. CI runs the same on macOS and
Windows, and a pull request must pass them:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
(cd app && npm run build)
```

If you touch transcription, also run the model-backed tests:

```sh
QUILL_TEST_MODEL=<path/to/ggml-base.bin> \
QUILL_TEST_VAD_MODEL=<path/to/ggml-silero-v6.2.0.bin> \
cargo test -p quill-stt -- --ignored
```

(`cargo run -p quill-cli -- models download base` fetches a small model;
`models list` shows where models are stored.)

### Code conventions

The workspace enforces most of these through lints:

- **Formatting:** `rustfmt` defaults. TypeScript follows the existing style.
- **Errors:** a `thiserror` enum per library crate; `anyhow` only at the edges
  (CLI, app commands). No `unwrap()`/`expect()` outside tests and startup code.
- **Unsafe code** is denied except in small platform wrappers, and every
  `unsafe` block needs a `// SAFETY:` comment explaining why it's sound.
- **Audio callbacks** must never block, lock or allocate: hand samples to a
  worker thread through a lock-free ring buffer.
- **Logging:** never log API keys, transcripts, summaries or dictated text.
  Counts and timings are fine.
- **Where logic lives:** in the library crates (`crates/`), not in the CLI or
  the app, which should stay thin. The interface displays state and calls
  commands; Rust does the work.
- **Comments** explain *why*, especially for anything learned the hard way
  (a platform quirk, a model behaviour, a measured trade-off).
- **Tests** for new behaviour, at the level where it lives. Timing- or
  hardware-dependent tests go behind `#[ignore]` with a note on how to run them.

### Prompts and models

- Prompt templates are versioned in [`prompts/`](prompts). Don't edit a
  version that has shipped: copy it to the next version (`summary-v3/`),
  change that, and point the code at it. Stored summaries record their
  prompt version, so results stay comparable.
- Back a change of default model or prompt with numbers from the evaluation
  tools (`quill-cli eval` for summaries, `quill-cli dictate` for dictation).
  Include the method and results in the pull request.
- **Never commit real meetings.** Put your own transcripts in
  `eval/transcripts/private/`, which is gitignored.

### Interface work

`cd app && npm run dev`, then open http://localhost:1420. In a plain browser,
the interface runs against sample data ([`app/src/dev/mock.ts`](app/src/dev/mock.ts)),
so you can work on it without building the Rust side. URL flags show
particular states, such as `?onboarding`, `?listening` or `?light`. Check both
light and dark themes.

## Commits and pull requests

- Keep each pull request to one concern, with small, focused commits.
- Use [Conventional Commits](https://www.conventionalcommits.org) prefixes:
  `feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`, with an optional
  scope (`feat(dictation): …`).
- In the description, say what changed and why, how you tested it (and on
  which OS), and anything a reviewer should look at closely.
- Update the README when behaviour changes, and
  [`docs/DESIGN_NOTES.md`](docs/DESIGN_NOTES.md) when decisions do.

## Licensing of contributions

Quill is dual-licensed under MIT and Apache-2.0. Unless you explicitly state
otherwise, any contribution you intentionally submit for inclusion is dual
licensed as above, without any additional terms or conditions. Only submit
work you have the right to license this way.
