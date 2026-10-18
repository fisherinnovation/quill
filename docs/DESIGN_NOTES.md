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
- **Live captions** (`quill_core::live`, on by default): while listening,
  a pass every 0.6 s re-transcribes the audio not yet locked in (no
  temperature retries: audio ending mid-word makes Whisper unsure, and
  retries took seconds). A segment locks in when two passes agree on it, it
  isn't the last one, there's a real pause after it to cut at (150 ms below
  20% of its speech level, within 0.8 s of Whisper's end time, which runs
  early in short-clip mode), and its words fit the audio (at most 4.5 a
  second: short-clip timestamps can put a long stretch's end seconds early,
  and cutting there transcribed it twice). On release only the rest is
  transcribed, primed with the locked text, minus any words repeating its
  end. If the result has a Whisper loop (8 words repeated), one pass over
  everything is used instead, when that one hasn't.
- **Live captions, measured** (2026-10-17, M3 Pro, turbo; `quill-cli
  dictate --stream` over 54 saved dictations): release to text went from a
  median of 1.9 s to 0.53 s for dictations over 30 s (6.5 s to 0.15 s for
  two minutes; 75% locked in by release), 0.65 s to 0.39 s at 15–30 s, and
  unchanged under 15 s. Passes take ~0.3 s (rarely 2–3 s before anything
  locks). The text matched a single pass but for 68 of 1,633 words: 57 were
  a single pass's Whisper loops (repeated phrases) that live avoided, 11
  one-word differences. The TTS clips match a single pass exactly but one.
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

## Noticing calls

- **Signal:** which apps are recording from a microphone, from Core Audio's
  process objects (macOS 14.2+: PID, bundle id, `IsRunningInput`). No
  permission needed, and no audio, windows or tabs read. Checked once a
  second (`calls.rs`), turned into starts and ends by
  `quill_core::calls::Detector`.
- **Measured** with `quill-cli mic-users --watch` (2026-10-17): Zoom shows
  as `us.zoom.xos`; a Google Meet tab as `com.google.Chrome.helper` (so
  call apps match by bundle-id prefix); Quill's dictation as its own id,
  which is excluded. Safari and Firefox not yet checked.
- **Timing:** a call app must hold the mic for 5 s before Quill asks (a
  voice search or mic test doesn't), and release it for 15 s before the
  call counts as over (rejoining or switching devices doesn't end it).
- **Browsers** show as the browser, not the site: telling Meet from any
  other tab would mean reading tabs or window titles (Screen Recording or
  Accessibility), against the promise to read only app names.
- **Prompt:** its own always-on-top window at the bottom centre (above the
  Dock, where it's hard to miss) that takes clicks but never focus, so it
  doesn't pull you out of the call. It rises and fades in, and fades out
  when answered. It's a floating panel (below). Unanswered,
  it goes after 30 s (60 s for "Stop recording?"). Not shown while
  recording, before onboarding, or with the setting off. Windows support is
  planned (the microphone ConsentStore in the registry).

## Floating panels

- **The call prompt and live notes** take clicks without bringing Quill
  forward (`app/src-tauri/src/floating.rs`). On macOS a click on any of an
  app's windows activates the app, which raises all its windows: the main
  window jumped over the meeting whenever the live notes were touched.
- **Handing focus back** after the click (noting the app in front and
  reactivating it, `focus.rs`) wasn't enough: AppKit had already raised
  the main window, and the race showed. Tauri windows can't be made
  non-activating panels (tao's window class carries state, so swapping the
  class at runtime would corrupt it).
- **So on macOS** each window's web view moves into an `NSPanel` Quill makes
  itself, styled non-activating, floating, on every desktop and over
  full-screen apps, and placed in points on the screen under the mouse. The
  Tauri window is never shown. Windows shows the window as is: an
  unfocusable window there doesn't activate the app.

## Live notes

- **During a recording** a worker follows `mic.wav` and `system.wav` as
  they're written (`quill_audio::growing`: nothing on the capture path
  changes) and transcribes ~10 s chunks, cut where both tracks pause, as
  You (mic, echo-cancelled per chunk) and Others (call). It's a draft for
  the notes; the real transcript is still made after the recording.
- **Every minute** (or on "update now") the summary model gets the
  notes so far as JSON, the items dismissed, and the last 15 minutes of
  transcript (`prompts/live-v1`), and returns updated notes: gist, ask
  before the end, follow-ups, decisions. The 15-minute window keeps each
  call's cost flat however long the meeting; the notes carry what came
  before. Item ids stay stable; dismissed items are matched by id or text
  so they can't come back renumbered; the model marks items done when the
  conversation deals with them, and the user can tick them too.
- **Panel:** a floating panel (below) at the right edge, content-protected
  (hidden from screen sharing and recording), collapsible to a tab with the open questions'
  count. On by default (the owner's choice while they're its only user);
  Settings → Meetings turns it off. Notes and transcript live in memory
  only.
- **Cadence:** every two minutes, with 20 s chunks, felt behind the
  conversation in the first real use: by the time a note appeared, the
  moment to act on it had passed. Now every minute with ~10 s chunks, so a
  note reflects what was said within about a minute. A 15-minute window
  per update keeps that affordable (~$0.0004 per update with Haiku).
- **Controls on the panel:** the recording's timer, Pause/Resume and Stop
  (which asks once). Pausing is in the recorder itself: the track writers
  keep draining their buffers but discard what arrives, so both files skip
  the same stretch and stay aligned; paused time is left out of the clock
  and the meeting's length. Also in the menu bar.
- **Measured** (`quill-cli live-notes eval/bench/meeting.wav`, 3.9 min
  synthetic product sync, turbo + claude-haiku-5.5): live transcription
  took 15 s of compute for the whole meeting; each update ~1.5 s and
  together $0.0007. First draft of the prompt repeated owners' names in the
  text, marked a decision "done" and listed one task twice; live-v1 says
  not to, and the merge strips a repeated name and never marks decisions
  done. It raised "make sure pricing and hiring get covered" and ticked it
  off itself once they were.

## App layout

- **The sidebar is navigation only** (Home, Meetings, Dictations, Settings,
  and the record button). It used to list every meeting and
  dictation, which grew without end and was more noise than use: meetings
  are looked back on, dictations rarely. Each now has its own page, grouped
  by day, with a back link from an opened item.
- **Meeting tiles** on the Meetings page each carry the summary's first
  TL;DR point, who was there (dots in the transcript's speaker colours),
  length, action items and your share of the talking, over a faint
  two-lane thread of the conversation: your turns on one lane, the call's
  on the other, so the back and forth reads as a zig-zag. It's clear for
  the left 56% of the tile, where the gist wraps, and fades in towards the
  right edge, so it never runs under text. `quill_core::overview` computes it all in two queries
  (speaker turns without text, latest summaries), cut into 96 slices; a
  bar per slice sized by speech looked like a barcode, because nearly
  every slice of a meeting has speech.
- **Tray icon:** the logo's feather and pulse line, drawn by
  `app/scripts/tray_icon.py` from the same paths as the app's `Logo`. On
  macOS it's a template image (black on transparent; macOS tints it), with
  the pulse line kept clear of the feather so they don't merge at 22 pt.
  On Windows it's the full-colour logo, since a black glyph disappears on
  a dark taskbar.
- **Collapsing the sidebar** leaves icons only: 92 px on macOS, centred
  under the window buttons (about 59 pt wide on macOS 26; at 78 px they
  nearly touched the border), 60 px on Windows. Each icon is 40 px wide
  like the record button and named by a tooltip that includes its status
  ("Meetings · Recording"); badges become dots.
  ⌘\ or Ctrl+\ toggles it. The choice is kept in the webview's
  localStorage: a per-computer view preference, not a setting.
- **Dictations page:** the hero has a waveform behind it (`VoiceWave.tsx`,
  a canvas) shaped by words per day over the last 30 days, mirrored like
  a voice's waveform. It breathes when idle, follows the mic level while
  listening (the level the pill gets is also sent to the main window),
  and a shimmer crosses it while typing. It stops drawing when the window
  is hidden and holds still with reduced motion. Below it, the all-time
  numbers count up once; each day in the history shows its totals, and
  each dictation a badge for its app.
- **Dictation colours** say what happened to each one, one colour per
  outcome so they don't compete: not typed (left on the clipboard,
  fuchsia, the one to act on) before corrected (grape) before tidied up
  (sky); typed as spoken, the usual case, stays plain. The filter chips
  are the legend. App badges are neutral, so colour means only this.
  Questions, lists and dictations over 20 s get grey tags.
- **Help** is a page in the app (sidebar, and Help → Quill Help on macOS),
  not a website: it works offline and can say "Settings → Dictation" for
  the settings the user actually has. Articles live in
  `app/src/help/content.ts` as Markdown, with platform wording swapped in
  and Mac-only sections left out on Windows. Search runs over titles,
  extra keywords and text: every word must match unless no section has
  them all, words like "how do I" are ignored, and a title match ranks
  highest. A table of contents follows the reading.
- **Insights are part of Home**, not a page of their own, beside or under
  the meetings. Two pages of numbers about the same use were one too many.
- **Home's quill:** the mark's feather as a fine outline (fuchsia to
  grape), large and faint, cropped by the top right of Home and fading out
  before the content. It replaced a wordmark (the quill beside "Quill" in
  a serif, with a signature line), which looked dated at the top of the
  page.
- **Two columns on Home** once the main area is 900 px wide (the default
  window is): meetings on the left, usage on the right. Narrower, or with
  no usage yet, one column.
- **Background wash:** the main area has a faint gradient of the palette
  (fuchsia from the top right, sky or grape from the top left), the one
  exception to flat colours: a plain sheet looked flat in both themes. It
  stays put while pages scroll. The meeting page's sticky header spans the
  whole content area (container units) and is see-through, so it doesn't
  sit on the wash as a box.
- **See-through boxes:** cards, the latest notes and meeting tiles have a
  translucent background (`--bg-panel`), a soft border and a faint light
  top edge, so the wash carries under them instead of stopping at opaque
  sheets. Floating windows (the call prompt, live notes) stay solid: they
  sit over other apps.
- **Home leads with content, not chrome.** A generated-dashboard look
  (greeting, equal cards, a row of KPI tiles, explanatory copy) read as
  robotic. Home now opens with a plain sentence about the day, then the
  latest meeting's TL;DR and action items (read from the summary's
  `## TL;DR` and `## Action items`) as the page's only card, then the
  meetings before it as a plain list, then usage as one line of facts and
  unboxed details. Record lives in the sidebar, not in a second button on
  Home. Keep it that way: hierarchy from type and space, not more boxes.
- **Settings has tabs** (General, Meetings, Dictation, Models, Storage), so
  dictation's settings could move there from the Dictations page without
  making one endless page. The Dictations page keeps what isn't a setting:
  whether dictation is ready, the history, and fixes waiting for an OK.
- **Theme:** System (the default: follows the OS, live), Light or Dark,
  under Settings → General. Each window's page sets `data-theme` from the
  setting before its first render, so there's no flash of the OS theme,
  and follows the `theme` event; the CSS holds the dark tokens twice (the
  OS media query, and `[data-theme="dark"]`). Rust sets the native window
  theme too (`theme.rs`), so the title bar, scrollbars and menus match.
- Dictation settings are saved on their own (`save_dictation_settings`
  rebinds the shortcut, and `save_settings` ignores them), so the Settings
  page keeps them in separate state with their own autosave.

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
