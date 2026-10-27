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
  - whisper.cpp takes the card at `gpu_device` among ggml's GPU devices,
    which Vulkan lists dedicated and integrated alike in the driver's
    order, and by default the first: on a PC that lists its integrated
    graphics first, the slower one. So Quill chooses (`quill_stt::gpu`):
    the card picked in **Settings → Models → Graphics card**, by name
    (positions shift with drivers and hardware), else a dedicated card
    with the most memory, else integrated graphics. Dedicated comes before
    memory because integrated graphics report shared system memory, which
    can be more than a card's. Or the CPU, for a card that misbehaves.
    Logged when Whisper loads (`gpu=`); `quill-cli gpus` lists the cards.
    Not yet measured on a PC with both.
  - List cards through ggml's device registry (`ggml_backend_dev_*`), the
    path whisper.cpp itself takes when loading: its Vulkan registration
    catches a missing driver and lists no GPU. Don't call
    `whisper_rs::vulkan::list_devices()`: it creates a logical device per
    GPU, and with no driver ggml's instance setup throws through FFI.
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

## Daily summaries

On the Meetings page, a day with two or more meetings that have notes gets
**Summarize this day** under its heading; the result is a card there
(`quill_core::daily`, `prompts/daily-v1/`, `DailySummary.tsx`).

- **From notes, not transcripts.** The notes already hold what each meeting
  came to, so a day is one short request (about 950 prompt tokens for three
  meetings) with the summary model, and nothing goes to the model that its
  notes didn't already send. Re-reading transcripts would cost tens of
  thousands of tokens a day to repeat what the notes distilled. If a day's
  notes don't fit the model's context, only their essential sections are
  sent (TL;DR, decisions, action items, open questions).
- **Only when asked.** No model calls the user didn't ask for: a day may
  never be looked at, and with OpenRouter each summary costs money. One
  meeting's notes are already their own summary, hence the two-meeting
  minimum.
- **Outdated, not rewritten.** A summary stores the notes it was written
  from (`meeting id@summary id` for each), so new notes for one of its
  meetings, or another meeting that day getting notes, mark it **Notes
  changed** until the user rewrites it. A plain list rather than a hash,
  so it stays comparable across Rust versions.
- **Deleted with its meetings.** It holds its meetings' content, so
  deleting any of them deletes it: `daily_summary_meetings` links it to
  each meeting with `ON DELETE CASCADE`, and a trigger on that table
  deletes the summary (migration 13). Saving checks each meeting still
  exists, so a meeting deleted while its day was being summarized doesn't
  leave its content behind.
- **Days are local.** Keyed `YYYY-MM-DD` in the computer's time zone, in
  both Rust (jiff) and the UI (`toLocaleDateString("en-CA")`), matching how
  the Meetings page groups meetings.
- **The prompt** asks for TL;DR, Decisions (as they ended up, when a later
  meeting changed one), Your action items, Others' action items, Across
  meetings (topics in two or more meetings, at most five) and Open
  questions, each citing its meeting by title. The meetings are given as
  `### Title (09:30, 31 min)` headings with their notes demoted under them.
  An earlier draft numbered them ("Meeting 3: …"), and the model then
  cited "(Meeting 3)" instead of the title.
- **Checked with a real model:** `crates/quill-core/tests/daily_smoke.rs`
  (ignored by default; `QUILL_TEST_DAILY_MODEL=ollama:…`) gives three
  meetings whose notes disagree: a launch planned for Friday in one, moved
  to Monday in another. With local `qwen3.5:9b` (Windows, RTX 4070 SUPER),
  the summary kept the launch on Monday and said why, linked the analytics
  fix and the hiring thread across meetings, kept every action item with
  its owner, cited meetings by title and named no one who wasn't in the
  notes. It took 6.8 s with the model loaded (53 s the first time,
  loading it), for 940 prompt and 365 completion tokens.
- **No thinking first.** 0.6.0 let the model reason and capped the answer
  at 2,048 tokens. With `anthropic/claude-haiku-5.5` on OpenRouter, the
  three test meetings took 1,359 completion tokens for a summary of about
  450: the rest was reasoning. A real day's longer notes used up the cap
  thinking, and the model returned no text. The request now asks for no
  reasoning (dropped automatically for models whose reasoning is
  mandatory) and allows 4,096 tokens, as meeting notes do. On the same
  test: 670 completion tokens, 3.1 s instead of 6.0 s, half the cost
  ($0.00046), and the same checks pass. The notes are already distilled,
  so there's little left to think through.

## Action items

- **Read from the notes, not asked for separately.** The summary prompts
  already write `- **Owner** — task — due date` under "## Action items",
  so `quill_core::action_items` parses that section when notes are saved
  (in the same transaction) instead of a second model call or a new prompt
  version. In the owner's 7 real summaries, every item followed that format
  and every other bullet was a placeholder ("None", "No action items were
  assigned"), so placeholders are skipped; counting them is also what made
  the Meetings page show "1 action item" for meetings with none.
- **Stored with their ticks** in `action_items`, one row per item, synced
  from the latest notes. Re-summarizing rewords items, so an item keeps its
  id and tick if its task shares at least 60% of its words with an earlier
  one (Jaccard over words longer than two letters); items the new notes
  drop are deleted. The notes themselves are never rewritten: the Summary
  tab and Home render the section as a checklist from the table instead.
  Libraries from before it read their existing notes once (migration 8).
- **Yours, then others'.** An item is yours when its owner is "You" (or
  "Me", in v1 notes), alone or with others; the rest are "Waiting on
  others", by person, for following up.
- **Due dates** are worked out from the meeting's local day: weekdays mean
  the coming one ("next Thursday" the one in the following week), "end of
  the week" Friday, plus today/tomorrow, end of month, "in two weeks" and
  calendar dates (a date without a year more than 60 days before the
  meeting is next year's). Anything else, like "after the launch", stays
  as written and isn't flagged.

## Dictation

- **Latency:** Whisper `short_clip` mode (greedy decoding, VAD trimming
  only the outer silence, and on Metal an encoder sized to the clip) takes
  about 0.15 s median for a dictation under 4 s on turbo (M3 Pro), 0.2 s
  on an RTX 4070 SUPER. Details in `eval/dictation/README.md`.
- **Short clips on Metal: encoder sized to the clip, repeats dropped.**
  The full 30 s window (below) costs ~0.6 s on an M3 Pro whatever the
  clip's length: 0.13 s → 0.61 s median for dictations under 4 s, which
  broke the ~0.3 s promise. So on Metal the encoder is sized to the clip
  plus 3 s (`FIT_MARGIN_FRAMES`, 150 frames), and `without_repeats` drops
  the segments Whisper didn't hear: one that ends the segment before it
  word for word, one of 3+ words already said (compared on letters and
  digits), and one after the first with more than 8 words a second. The
  repeat is always a separate segment, an exact or near copy, starting
  where the one before ends; in 334 clips the segments Whisper heard came
  at most 4.3 words a second, the copies 86 and up. Measured (2026-10-20,
  `quill-cli dictate --no-polish`): 230 short TTS clips doubled 38 → 0
  (WER 26.6% → 4.2%; full window 3.0%), the user's 104 saved dictations
  11 doubled and 2 with an earlier sentence repeated → 0, Whisper median
  0.15 s under 4 s (full window: 0.61 s). The old 1.3 s margin left more
  misheard short words (TTS WER 8.0% even with the de-dup); 2 s let a
  doubling through, 5 s a repeated sentence, and floors of 500–1000 frames
  cost 0.22–0.40 s. `no_timestamps` made Whisper loop. Vulkan and the CPU
  keep the full window, as measured below. The one long doubled dictation
  (23.7 s) was this bug too, in a single pass: the encoder was sized to
  anything under ~28.7 s. Stitching live-caption passes never repeated
  text (36 dictations over 15 s, `--stream`, all three builds).
- **Short clips keep the full encoder window** on Vulkan and the CPU.
  Sizing the encoder to the clip (`set_audio_ctx`, the length plus ~1.3 s)
  was 4.5× faster for a 4 s clip on Metal, but on Windows half of all
  2–4 s dictations were typed twice ("Can you send it over? Can you send it over?"): 13 of the 26 in
  one user's history, and 10 of 40 TTS clips. Whisper repeats the sentence
  within one decode, so `single_segment` doesn't help, and it happens on the
  CPU and on Vulkan with or without flash attention, so it isn't a GPU bug.
  Its timestamps also ran past the end (0–15 s for a 4 s clip). A floor of
  400–750 frames still doubled 1–3 clips. The full window made 0 of 40 and
  cut WER from 28% to 2.6% (live captions: 10.8% to 2.6%), for ~50 ms more
  on an RTX 4070 SUPER (Vulkan, flash attention). On Metal it costs ~0.5 s,
  hence the entry above.
- **Live captions** (`quill_core::live`, on by default): while listening,
  a pass every 0.6 s re-transcribes the audio not yet locked in (no
  temperature retries: audio ending mid-word makes Whisper unsure, and
  retries took seconds). A segment locks in when two passes agree on it, it
  isn't the last one, there's a real pause after it to cut at (150 ms below
  20% of its speech level, within 0.8 s of Whisper's end time, which runs
  early with the shortened encoder), and its words fit the audio (at most
  4.5 a second: shortened-encoder timestamps can put a long stretch's end
  seconds early, and cutting there transcribed it twice). On release only the rest is
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
- **Language:** dictation defaults to the system language. Auto-detection
  on the shortened encoder window produces nonsense, so on Metal "auto"
  runs a separate full-window detection pass first (~0.5 s); with the full
  window (Vulkan, CPU) Whisper's own detection works in the same pass, ~50
  ms slower than a fixed language (RTX 4070 SUPER, 40 TTS clips, same WER).
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
- **The indicator must never take focus** (Windows). It's built unfocusable
  (`WS_EX_NOACTIVATE`), but that only stops clicks from activating it:
  tao's `show()` uses `ShowWindow(SW_SHOW)`, which activates it anyway
  (`focused(false)` only covers a window built visible). The dictation
  shortcut gives Quill the right to take the foreground, so showing the
  indicator, including the switch to "Typing…" just before the paste, could
  take the focus from the text box and the Ctrl+V went to the indicator:
  often enough to notice, and the log still said `inserted=Pasted`. The
  indicator, call prompt and live notes are shown with
  `ShowWindowAsync(SW_SHOWNOACTIVATE)` and hidden with `SW_HIDE`
  (`floating::show_unfocused`); tao then doesn't know they're visible, so
  its own `hide()` would do nothing.
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
- **Reviewing samples in the app:** writing references by hand in a text
  editor didn't happen, so the app has a review page (Settings →
  Dictation → Testing accuracy → Review…). Each sample plays as it's shown
  beside what was typed, already in an editable box: keeping it (⌘↵) says
  it was right, editing and saving records what was said. Most samples
  are right, so most take one keystroke. It scores what was typed and
  Whisper's own text against the references as it goes (word error rate,
  now in `quill_core::wer` so the app can use it), and lists the
  commonest mistakes, the candidates for vocabulary and replacements.
- **Planned:** opt-in learning by watching the inserted text through
  Accessibility for about a minute. First check which apps expose it.

- **Sounds are synthesized**, not files (`quill_audio::cue`): each cue is a
  few notes with an attack, an exponential decay and a fade to zero at the
  end, so nothing clicks. Sets differ in timbre: overtones (inharmonic for
  wood and glass, the 4th harmonic for marimba, odd harmonics for the
  digital beep) and pitch glides (bubble, chirp). Start rises and stop falls
  in every set; cancel is two low notes. Classic, the default, is the
  original pair of blips, kept sample for sample (a test checks it against
  the old formula). Levels were tuned so each set's peak is within
  0.5–1.6× Classic's; Soft and Chirp sit a little lower on purpose (gentle,
  and 2–3 kHz sounds louder than its peak). The access-check chime doesn't
  follow the setting.
- **Time to insert** is the measure of dictation speed that matches what
  users feel: from the end of speech to the text typed. `latency_ms`
  starts at the key's release, but people keep holding the key a moment
  after their last word, and that's waiting too. The end of speech comes
  from Silero VAD on the whole clip (`WhisperTranscriber::speech_end`: the
  end of its last speech region), run after the paste so measuring adds no
  delay; `wait_ms` = clip length − end of speech + `latency_ms`. Stored
  with each dictation and in `dictation_stats` (migration 9), NULL when
  there's no VAD model. Insights give the median, 90th percentile and its
  split (median hold, median Quill) over the 30-day chart, the last 7 days
  against the 7 before, and per-day medians and 90th percentiles for the
  chart. The 90th percentile is there because the slow tail is what's
  remembered. A tap-to-stop dictation counts the time to the second tap as
  holding.
## Insights and logs

- Dictation numbers come from a `dictation_stats` table (counts only, no
  text), so they survive turning history off or clearing it. Time saved
  is measured against typing at 40 wpm.
- App icons beside dictations, top apps and per-app styles come from the
  installed app, looked up by the id Quill records (`quill_insert::icon`):
  on macOS the bundle id, through NSWorkspace, even when the app isn't
  running; on Windows the shell's icon for the executable. A Windows id is
  only the executable's name, so the full path is remembered in memory
  when the app is seen in front (a path names the user's folders, so it's
  never stored); until then, and for uninstalled apps, the app's letter
  shows. Icons are cached in memory as PNG data URLs, never written to
  disk. `dictations.app_id` (filled for older rows from `dictation_stats`
  by app name) lets the history find them. Microsoft Store apps show as
  their host process, so neither their name nor icon is right yet.
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
  permission needed, and no audio, windows or tabs read. Checked twice a
  second (`calls.rs`), turned into starts and ends by
  `quill_core::calls::Detector`.
- **Measured** with `quill-cli mic-users --watch` (2026-10-17): Zoom shows
  as `us.zoom.xos`; a Google Meet tab as `com.google.Chrome.helper` (so
  call apps match by bundle-id prefix); Quill's dictation as its own id,
  which is excluded. Safari and Firefox not yet checked.
- **Timing:** a call app must hold the mic for 2 s before Quill asks, and
  release it for 15 s before the call counts as over (rejoining or
  switching devices doesn't end it). It was 5 s, polled once a second, and
  in use (Google Meet, 2026-10-18) the prompt felt about 10 s into the
  call: too late, with the start of the meeting already unrecorded. Now
  2 s, polled twice a second, and a gap of up to 3 s keeps an app's start
  time, so a browser letting go of the mic between a preview screen and
  the call doesn't restart the clock. A long voice search can bring the
  prompt up; it goes on its own, which costs less than a missed start.
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

## After the recording

- **The write-up is followed step by step** (`quill_core::processing`):
  getting the audio ready, transcribing you, transcribing the call,
  telling speakers apart, writing the notes. Whisper reports its progress
  and each line as it's recognised (`quill_stt::Progress`), so the
  transcription steps show a percentage and the lines being heard; the
  others show time so far. The progress event carries the whole report;
  a running job sends at most about seven a second, and step changes at
  once.
- **Time left** starts from typical speeds for the recording's length
  (transcription about 25x real time, speakers about 2 minutes an hour,
  notes 12 s plus 15 s an hour) and switches to the measured pace once a
  transcription step is 8% in. It isn't shown for the first 1.5 s.
- **Done** reports what came out: words and people from the transcript,
  action items and decisions counted from the notes' sections.
- **Where you're told** depends on where you are: on the meeting, the
  card becomes "Your notes are ready" and the notes rise in; elsewhere in
  Quill, a toast and a "New" mark until it's opened; on the live notes
  panel (if it was showing), the panel stays after Stop as a small card
  sized to its content, then offers Open notes and goes after 45 s; in
  another app, a system notification (macOS's User Notifications, asked
  for the first time; a toast on Windows) that opens the meeting when
  clicked. Only one of these says it: no notification while the panel or
  the window is showing it.

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
- **Meetings page:** the hero is woven from recent meetings
  (`ConversationWeave.tsx`, a canvas): each a ribbon of its 96-slice
  conversation shape, coloured by speaker as on the tiles, drifting
  sideways, rows in alternate directions; faster and brighter while
  recording or writing up. With fewer than three meetings it weaves made-up
  ones. Below, this week's numbers count up (the same tiles as Dictations),
  and filters pick meetings with action items, new ones, ones needing
  attention, or with one of the up to three people met at least twice.
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
- **Release notes** come from `scripts/release-notes.sh`: the `feat` and
  `fix` subjects since the previous tag, as New and Fixes (without the
  website's), then the downloads by file name and where to start. GitHub's
  `--generate-notes` lists merged pull requests, and with changes landing
  on main directly that's only a changelog link; 0.1.0's were written by
  hand. The release is a draft, so they're edited before publishing (an
  opening line, refinements of one feature folded together). First-launch
  steps link to the README rather than being copied, so they can't go stale.

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
- **The hero video on wide screens** (over 960 px) is a pane beside the
  copy: 70% as tall as it, turned 50° in 3D to face it, so the far,
  inner edge is the smaller one, and that edge fades in from transparent.
  As tall as the copy, it overpowered the headline. The angle is what let
  a 16:10 pane that tall fit beside it inside the page's width: at 40° it
  ran under the headline. Between about
  960 and 1100 px, where the headline wraps less, the pane is capped by its
  column's width (container units) and sits centred. The video is out of
  the layout's flow, so the copy alone sets the row's height. Phones and
  narrow windows keep the flat, stacked video.
- Captions sit on a scrim in the app's background colour, present from a
  scene's first frame so crossfades don't flash text through it. Light and
  dark videos are separate files (about 1.2 MB each); `main.js` picks one
  by `prefers-color-scheme` and leaves the poster with controls under
  `prefers-reduced-motion`.
- **Quill's cost** in Compare is a year at 10 hours of meetings a week on
  the default models: about 1 cent an hour for notes and dictation
  clean-up, plus live notes (on by default) at about 2 cents an hour (60
  updates of ~$0.0004 with a full 15-minute window; less while the window
  fills), so about 3 cents an hour and ~$17 a year. Redo it when defaults,
  prices or live notes' cadence change.
- CI builds the site on every run, but only deploys when the repository
  variable `SITE_DEPLOY` is `true`, so CI stays green before Pages is
  enabled.
- **Being found.** "Quill" is a crowded name, so the `<title>` and
  description use the words people search for (private, AI meeting notes,
  dictation, Mac, Windows), with the description under the ~155 characters
  results show. The page carries a link preview card (`og:image`,
  `twitter:card`), a canonical URL and schema.org `SoftwareApplication`
  data built from the release (version, systems, free, download links).
  Google only shows the rich, starred result for apps with ratings or
  reviews, which Quill doesn't have, but the data still tells search
  engines what the page is. All of these need absolute URLs, so they're
  only built with `QUILL_SITE_URL` set, as are `sitemap.xml` and
  `robots.txt`.
- **The site's address** is `quill.fisherinnovation.com`: a CNAME to
  `fisherinnovation.github.io` at Namecheap, with the custom domain set in
  the repository's Pages settings (a `CNAME` file does nothing for a site
  deployed by Actions) and `fisherinnovation.com` verified on the GitHub
  account, so no one else can claim its subdomains. A domain of its own
  keeps what it earns in search if hosting moves, and puts `robots.txt` at
  a root, the only place crawlers read it. The old
  `fisherinnovation.github.io/quill/` redirects there. `QUILL_SITE_URL` in
  CI must match it.
- The preview card (`site/media/card.jpg`, 1200×630) is
  `site/card/index.html` captured by headless Chrome
  (`node site/card/render.mjs`). It shows the light header video's poster,
  so it's re-rendered with the video. It's a JPEG because, with a
  screenshot in it, a PNG is twice the size.
- **The overview video** (`site/media/overview.mp4`, 47 s) is the narrated
  rundown behind the hero's "Watch the overview" button, made by
  `node site/overview/make.mjs` from `site/overview/script.json` (the
  `site-overview` skill). The header video shows the app; this one is
  meant to feel human, so it adds a voice, people and music:
  - **Voice:** ElevenLabs (`elevenlabs/eleven-v4`, voice "jessica") through
    OpenRouter, one request per line, so a line can be redone alone and its
    length sets its scene's.
  - **People:** Veo 3.1 at 1080p without sound, 8 s clips at $0.20 a
    second. Laptop screens are kept turned away in the prompts, because
    generated screens show nonsense. `cuts` jump-cut a clip when its best
    moment comes late. Two prompts for the same person give two different
    people.
  - **Music:** Lyria 3 Pro ($0.08 a track), about 10 dB under the voice
    and dipped under each line.
  - **App shots:** recorded by `site/video/render.mjs` (`--clips`) from the
    header video's scenes, each held as long as its line (`minSeconds`).
  - **The edit** is Remotion (`site/overview/remotion`, its own
    dependencies, so nothing new reaches the app or the site). Remotion's
    licence is free for individuals and companies of up to three people.
    Its packages pin `zod` 4.5.4, which has a memory-exhaustion advisory
    (fixed in 4.6.6), and every Remotion release up to 4.0.538 still pins
    it, so `package.json` overrides `zod` to 4.6.6. Drop the override once
    Remotion pins a fixed version itself. The render is levelled to -16 LUFS with peaks under -1.5 dB, two-pass
    `loudnorm` with linear gain, so the voice and music keep their
    balance. Straight out of the edit it was -23.6 LUFS.
  - **Cost:** everything generated is cached by what made it, and
    `make.mjs` refuses to spend more than `--budget` ($10). The first
    version cost $4.94, nearly all of it footage.
  - **On the site:** the MP4 is served as rendered (about 35 MB at CRF 18).
    The page doesn't load it until the dialog opens, and without
    JavaScript the button links to the file.

## Known voices

- **What:** once the user names a speaker, Quill remembers that voice and
  names a matching speaker in later meetings (`quill_diarize::voices` for
  matching, `quill_core::voices` for storage and learning).
- **A voice** is the duration-weighted average of a speaker's clean-piece
  embeddings in one meeting (the centroid clustering already works with).
  A known voice keeps one sample per meeting the user named or confirmed
  them in, and is matched by the speech-weighted average of its samples.
  Samples record the voice model; embeddings from different models are
  never compared.
- **Only the user teaches it.** Naming, merging (typing an existing name)
  and confirming an automatic name (✓) add that meeting's sample. An
  automatic match adds nothing, so a wrong match can't reinforce itself;
  renaming it removes that meeting's sample from the wrong person (and
  forgets them if it was their only one). "Me"/"You", "Others" and
  "Speaker N" are never remembered; the mic track never has a voice.
- **Matching:** greedy by similarity, each known voice to at most one
  speaker, only above **0.70** cosine similarity and only with at least
  **30 s** of speech behind the speaker's voice (the same minimum applies to
  learning). Tuned with `quill-cli voices eval` on 28 AMI meetings (28
  people), clean and through Opus 24 kbit/s:
  - 92% of known people named correctly (89% through Opus), 1.8% named as
    someone else;
  - no stranger named in 2,424 trials (1,416 through Opus); 1% at 0.60,
    24% at 0.50;
  - every remaining wrong name was on a speaker diarization had merged from
    two people, which no threshold separates;
  - with a 15 s minimum, small mixed clusters (overlap, crosstalk) matched
    each other across meetings.

  Details and the full sweep: eval/diarization/README.md. AMI sessions of
  one group share a room and headsets, so real calls will match less often,
  which is the safe direction.
- **Storage:** `known_voices`, `voice_samples` and `meeting_voices` in
  SQLite. `meeting_voices` holds each meeting's people on the call by
  transcript label, kept in step with renames and replaced when speakers are
  split again; samples are copied from it. A known voice outlives the
  meetings it was learned from, until deleted in Settings → Meetings.
  Meetings told apart before this existed have no `meeting_voices`, so
  naming people there teaches nothing until they're split again.
- **Privacy:** embeddings are biometric-like. They stay in the local
  library, are never logged (only counts: `known_voices`, `recognized`), and
  can be deleted one by one or all at once. **Recognize voices** turns off
  both matching and learning.

## Search by meaning

Search finds moments by meaning as well as by words: "when did we talk
about hiring" finds "we're bringing on two engineers". It adds to the FTS5
keyword search rather than replacing it (`quill_core::semantic`).

- **Model: multilingual-e5-small** (Microsoft, MIT): a 12-layer BERT with
  384 dimensions, trained for retrieval in about 100 languages. Whisper
  transcribes ~100 languages, so an English-only model (all-MiniLM, bge-small)
  was ruled out, though they're 3–4× smaller: e5-small's 250k-token
  vocabulary is ~80% of its weights. The download is 488 MB (fp32
  safetensors and tokenizer, from `intfloat/multilingual-e5-small`,
  SHA-256 checked, catalogued as `search` and `search-tokenizer`).
  `config.json` is compiled in. E5 wants `query: ` / `passage: ` prefixes
  and mean pooling; vectors are normalised, so cosine is a dot product.
- **Runtime: candle, pure Rust, CPU only** (`quill-embed`, behind an
  `Embedder` trait). Not `ort`: sherpa-onnx already links onnxruntime
  (statically on macOS), and a second copy risks clashing symbols.
  sherpa-onnx 1.13.8 can't run a text-embedding model: its API covers
  speech recognition, TTS, VAD, keyword spotting, punctuation, audio
  tagging, diarization and speaker embeddings, nothing for text. Not Ollama by default:
  it isn't installed for most users, and the index would depend on a
  server being up; an Ollama `Embedder` would fit behind the same trait
  later. candle is pinned to 0.9.2 because 0.10 and later make
  candle-core depend on `tokenizers` with Oniguruma (C). `tokenizers` is
  built without default features (`unstable_wasm` is what picks the
  pure-Rust regex engine), so there's no new C or C++ in the build. The
  model is small enough that the CPU suffices; no Metal or Vulkan for it.
- **Never downloads by itself.** The setting (Settings → Meetings → Search
  by meaning) is on by default, but until the user asks for the model
  (the line at the foot of the search window, or Settings → Models), search
  is by words alone. It isn't in the onboarding download either:
  `models::required` doesn't list it.
- **Chunks, not segments.** Single segments are often a few words ("Sounds
  good."), too little to carry meaning, so consecutive segments are
  grouped until ~80 words (never past 140; a pause over a minute starts a
  new chunk): ~100 chunks per hour of meeting, each about half a minute.
  A result found by meaning opens at the line in its chunk that shares
  most words with the query (ignoring filler words like "about", and
  counting "crash" for "crashes"), or else its first line. Notes, action
  items and dictations are embedded too, as their own kinds of result:
  see [Search everything](#search-everything).
- **Storage:** migration 11 (after known voices' 10), `semantic_chunks` (segment range, times,
  model, the vector as a little-endian f32 BLOB) and `semantic_index`
  (which meetings are indexed, by which model). `save_transcript` deletes
  a meeting's rows in the same transaction, so a new transcript is
  indexed again; vectors from another model id are ignored, so changing
  the model rebuilds everything. Indexing reads, embeds without holding
  the library lock, then writes only if the transcript is unchanged.
- **Indexing in the background** (`app/src-tauri/src/semantic.rs`): a
  worker indexes one meeting at a time, newest first, woken when a
  transcript is saved, the model arrives or the setting changes. It waits
  while recording, transcribing, moving recordings or dictating, and
  stops between batches of 16 chunks when one starts. The model loads on
  first use (~0.7 s; opening the search window loads it ahead), and is
  unloaded after 10 minutes unused: resident it's ~0.5 GB, and loading
  peaks at ~0.9 GB because the weights are read into memory, then copied
  into tensors. (Memory-mapping would avoid the copy but needs `unsafe`
  and holds the file open, so Windows couldn't delete it.)
- **Threads:** half the cores, at most 4. On a 32-thread Windows PC, 4
  threads indexed at 7.3 s per hour of transcript and 16 at 10.3 s: the
  model is too small to keep more busy.
- **Ranking: weighted reciprocal rank fusion.** Each result scores
  weight / (60 + rank) per list it's in. Keyword hits stay one per
  matching line, as before. A chunk found by meaning that contains a
  keyword hit lifts it (`both`); one without becomes a result of its own
  (`meaning`, shown as **Related**). Meaning has weight 0.5, so a result
  found only by meaning always ranks after every line with the words:
  for a query like "Thursday" the nearest chunks by meaning are only
  loosely related, while exact words are the surer sign.
- **Cut-offs.** E5's similarities are bunched: unrelated chunks score
  0.75–0.82, good matches 0.82–0.87. A chunk counts if it scores at least
  0.80 *and* is within 0.04 of the query's best; at most 10, and none for
  queries under 3 characters (while typing). Chosen by sweeping
  `quill-cli search --eval --min-score --spread`: 0.78 let in noise for
  nonsense queries, 0.82 lost true matches, and the spread changed
  little between 0.03 and 0.06.
- **Brute force is enough.** Each query reads every vector from SQLite
  and takes dot products. Measured on Windows (32 threads, 4 for the
  model), `quill-cli search --eval eval/search/queries.json`:

  | | |
  |---|---|
  | Indexing | 6–10 s per hour of transcript (machine shared with other builds) |
  | Index size | 1.5 KB per chunk, ~100 chunks per hour: the database grows ~200 KB per hour |
  | Query, the 16-minute eval library | 7 ms median (embedding the query is most of it) |
  | Query, padded to 100 hours (9,906 chunks, 15 MB of vectors) | 38 ms median (words alone 0.6 ms) |
  | Query, padded to 500 hours (49,426 chunks, 76 MB) | 145 ms median |

  A personal library of a few hundred hours stays well under the 150 ms
  the search box waited before searching (the search window waits 120 ms).
  Past that, keep the vectors in memory (or as f16/int8) instead of
  reading them from SQLite each time.
- **Quality** on 24 labelled queries over five synthetic meetings
  (`eval/search/`: four English, one Spanish), counting a result that
  opens the meeting at most 15 s before the right stretch:

  | | Right result first | In the top 3 |
  |---|---:|---:|
  | Words only (before) | 2 / 22 | 2 / 22 |
  | Words + meaning | 13 / 22 | 18 / 22 |

  (Two of the 24 are nonsense queries with no answer; both return
  nothing.) Paraphrases work well ("app crashes when you turn the phone
  sideways" finds "a crash on older Android phones when you rotate the
  screen"), as do Spanish queries over the Spanish meeting and a Spanish
  query over an English one. English queries over the Spanish meeting
  mostly failed ("trade show budget" didn't reach "el presupuesto para la
  feria"), as did the single word "vacation" for "two weeks off": a lone
  word gives the model little to go on.

## Search everything

One search window (Ctrl+K / ⌘K, the sidebar's Search, or the Meetings
page's search box, which opens it narrowed to meetings) looks through
meeting titles, transcripts, each meeting's latest notes, action items and
dictation history (`quill_core::search`, `SearchPalette.tsx`). It replaced
the transcript-only search box on the Meetings page.

- **One table for everything but transcripts.** Migration 12 adds
  `search_items` (kind: title, notes, action or dictation; the row it came
  from; for notes, the heading; the text; and the embedding with its
  model) and an external-content FTS5 index over it, `search_fts`, kept in
  step by the table's own triggers. Transcripts keep their own
  `segments_fts` and `semantic_chunks`: a moment needs its time, and
  there are ~100 chunks per meeting-hour against a few dozen notes points.
- **The database keeps it in step.** Triggers on `meetings` (title),
  `action_items` (owner and task) and `dictations` (text and correction)
  insert and update the rows; a change of text clears the embedding, so
  the worker embeds it again. They fire only when the text actually
  changes: re-saving notes updates every action item, and an unchanged one
  keeps its vector. Deletes cascade through foreign keys, and FTS5 is told
  by `search_items`' delete trigger (cascades fire triggers), so deleting
  a meeting or a dictation, or clearing the history, leaves nothing
  behind; tests check the FTS index is empty afterwards. Notes are the one
  kind written from Rust (`sync_notes`, in `save_summary`'s transaction),
  because they have to be parsed; a point unchanged word for word keeps
  its row and vector. Existing libraries are read in once after the
  migration, like action items were.
- **Dictations as corrected**, and only while the history is kept: the
  index follows `dictations`, so turning history off adds nothing new and
  clearing it removes everything. Titles are searched by words only (a few
  words embed poorly, and the meeting's notes cover what it was about).
- **Notes by line, not by section.** Each bullet (or paragraph, for the
  TL;DR) is its own item, under its heading; the "Action items" section is
  left out because each item is its own row. Sections were tried first:
  a section holds unrelated points ("Hire one backend engineer", "Move the
  pricing page redesign"), and embedded together it came out close to
  every query, an unwanted Related hit for 8 of the 12 test queries.
- **Cut-offs against the best match anywhere.** Each kind is ranked and
  fused like transcripts (words, then meaning at half weight), but "within
  0.04 of the best" is measured from the best match across everything the
  search covers: with the spread measured per kind, every kind showed its
  own best, however loosely related, and a clear answer elsewhere didn't
  stop it. Narrowed to one kind, the best is that kind's. The query is
  embedded once, and every vector in scope is scored once.
- **Cut-offs measured on short texts.** The transcript cut-offs (0.80,
  within 0.04) were tuned on half-minute chunks; notes points, action
  items and dictations are a sentence or two. `crates/quill-core/tests/
  search_smoke.rs` (ignored by default; needs the model) has 12 queries over
  sample notes, action items and dictations: 10 paraphrases with a known
  answer ("presentation for the directors" for "send me the slides before
  the board meeting") and 2 nonsense queries. With the defaults, 8 of the
  10 paraphrases put the right answer first, both nonsense queries find
  nothing, and 9 other results come back as Related, about half of them
  fair ("Hire one backend engineer" for "recruiting a designer"). E5's
  similarities are bunched even tighter on short texts (right answers
  0.805–0.836, wrong ones up to 0.82): 0.81 removes the noise but loses 2
  of the 8. The same defaults stay, since meaning-only results always rank
  after every match by words and are marked Related. Misses: "exponential
  backoff" for "the retry loop never caps its delay" (0.767, below
  unrelated items) and "reserve a location for the team retreat" for
  "book the venue for the offsite" (0.772).
- **The window.** A few of each kind (5) across everything, 50 of one
  kind when narrowed, with "Show all" when there are more. Groups keep a
  fixed order (meetings, then notes, action items, dictations), since
  ranks from different lists don't compare. Typing waits 120 ms, and an
  answer to an older query never replaces a newer one. The model loads
  when the window opens.
- **Speed**, Windows (32 threads, 4 for the model), `quill-cli search
  --eval eval/search/queries.json` with `--scale-hours` and
  `--scale-dictations` (padded dictations share one vector; scoring cost
  doesn't depend on the values):

  | Library | Searching everything (median) |
  |---|---|
  | 16-minute eval library | 7 ms |
  | 100 hours of meetings (9,906 chunks) + 20,000 dictations | 49 ms |
  | 500 hours (49,426 chunks) + 50,000 dictations | 145 ms |

  20,000 dictations is about two years of heavy use. Each query still
  reads every vector from SQLite; the next step, if libraries grow past
  this, is keeping them in memory (or as f16/int8).

## Parakeet

- **A second engine.** NVIDIA's Parakeet TDT 0.6B (CC BY 4.0) runs through
  sherpa-onnx 1.13.8's offline recognizer, already linked for diarization
  (`model_type = "nemo_transducer"`). Two models: **v2**, English only, and
  **v3**, 25 European languages, detected (it takes no language). Both as
  sherpa-onnx's int8 exports (`csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-*-int8`,
  about 660 MB each: encoder, decoder, joiner and `tokens.txt`, downloaded
  and hash-checked like any model). int8 was good enough not to try fp32
  (2.4 GB): on these clips its words were within a point or two of Whisper
  turbo's. Unmeasured: fp32.
- **Settings.** The engine follows the model: `stt_model` takes a Whisper
  size or `parakeet-v2`/`-v3`, and `dictation.stt_model` (new, `None` by
  default) gives dictation its own. The app keeps both loaded when they
  differ (`AppState::transcriber` holds up to two), so a meeting never
  reloads dictation's model. Defaults are unchanged: Whisper turbo for both.
- **CPU only.** sherpa-onnx's prebuilt libraries bring ONNX Runtime's CPU
  provider, so Parakeet ignores the graphics card. 4 threads: on an
  i9-14900KF, 4 and 8 were equally fast (short clips 174–187 ms median),
  16 four to five times slower (862–871 ms; the efficiency cores and
  hyperthreads hold the rest up).
- **What it doesn't take.** No prompt and no language. The vocabulary goes
  in as sherpa-onnx **hotwords**, which needs beam search
  (`modified_beam_search`; greedy gave the same text on 108 clips, a few
  percent faster) and a SentencePiece `.vocab` the export lacks: Quill
  writes one from `tokens.txt` (score = −id, BPE's merge order) to a temp
  file while loading. Learned the hard way:
  - with no `modeling_unit` (or anything but `bpe`/`cjkchar`), sherpa-onnx
    `exit()`s the process on the first per-stream hotword;
  - a hotword word starting with `:` (or `#`/`@` once encoded) reaches
    `std::stof` and throws across FFI; `/` is read as a line break.
  So terms with those, or with letters the model can't write, are left
  out (`usable_hotword`). On the eval clips with `fetchUser, loadUser,
  standup` as vocabulary, WER went 10.4% → 7.1% (Whisper's prompt: 9.4% →
  7.8%).
- **Segments.** Parakeet gives a time per token, not segments. Quill
  splits at sentence ends, and for meetings at pauses between VAD regions
  when the text so far ends in punctuation or the pause was a second or
  more. The punctuation that ends a turn often comes with the next turn's
  first token, so the pause is looked for after the last *word*. Packed
  with Whisper's 0.1 s gaps, Parakeet ran sentences and different
  speakers' turns together with commas (one 32 s segment had three
  speakers); 0.5 s gaps made a full stop at every pause ("Before we dive
  in."); 0.25 s (`PACK_GAP_SAMPLES`) punctuates like Whisper. Long
  recordings are decoded in windows of at most 30 s, cut at pauses: its
  encoder attends over the whole input.
- **Windows 11 has its own `onnxruntime.dll`** in System32, older, and
  found before `PATH`: a test binary in `target/debug/deps` without the
  sherpa-onnx DLLs beside it crashes (`STATUS_ACCESS_VIOLATION`). The app
  and the CLI have theirs beside the executable, which wins.
- **Measured** (2026-10-25, Windows 11, i9-14900KF, RTX 4070 SUPER;
  `quill-cli dictate --no-polish -l en`, medians of two runs on a quiet
  machine). Clips: the 20 eval clips (10 utterances × the two SAPI voices,
  `eval/dictation/make_clips.ps1`), 80 short ones (40 sentences under 4 s ×
  2 voices) and 8 long ones (four 25 s paragraphs × 2 voices), synthetic,
  not in the repo but for the script:

  | | Whisper turbo, Vulkan | turbo, CPU | Parakeet v2 | Parakeet v3 |
  |---|---:|---:|---:|---:|
  | 72 clips under 4 s | 209–215 ms | 7.1 s | **180–184 ms** | 187–188 ms |
  | 20 eval clips (4–15 s) | **277–297 ms** | 7.3 s | 386–391 ms | 407 ms |
  | 8 long clips (25 s) | **634–650 ms** | 7.9 s | 1.33–1.36 s | 1.33–1.36 s |
  | Long, live captions: wait after release | 389 ms | – | **340 ms** | – |
  | WER, 80 short clips (428 words) | **0.7%** | 0.7% | 1.2% | 1.9% |
  | WER, 20 eval clips (308 words) | **9.4%** | 9.4% | 10.4% | 12.7% |
  | WER, 8 long clips (514 words) | 0% | 0% | 0% | 0.2% |
  | Meeting (272 s, `eval/bench`), WER / × real time | **1.9%** / 20× | – | 2.4% / 22× | 2.2% / 22× |

  The eval clips' errors are mostly the same in both engines (fillers,
  the self-correction and the spoken list, which clean-up removes);
  Parakeet's extra ones on short clips were single words ("poll" for
  "pull", "merch" for "merge"). Live captions work unchanged: same text as
  a single pass on 6 of 8 long clips for both engines (the others differ
  only in commas), passes while speaking 251 ms median. The meeting is
  `meeting_script.txt` read by the two SAPI voices; turbo made 50
  segments, Parakeet v2 75, none over 15 s.
- **Decision: Whisper stays the default.** With a graphics card, Parakeet
  is no faster for short dictations (30 ms quicker under 4 s, 0.1–0.7 s
  slower on longer ones, live captions hiding most of that), a little less
  accurate, and switching would download 660 MB for everyone. It wins
  where there's no usable GPU: Whisper turbo on the CPU takes over 7 s
  for any dictation (the full 30 s encoder window), Parakeet 0.2–0.4 s, at
  the same accuracy. So it's offered for dictation (Settings → Models →
  Speech recognition for dictation), and suggested there for computers
  without a graphics card. Not measured: macOS (Parakeet on Apple Silicon's
  CPU, against Whisper on Metal), real voices, v3 in other languages, a PC
  with no GPU at all. Worth deciding with those numbers: defaulting
  dictation to Parakeet when no GPU is found.

## AI assistants (MCP)

`quill-cli mcp` lets an AI assistant (Claude Desktop, Claude Code, any MCP
client) read the meeting library, so it can answer questions from meeting
notes. What it sees is decided in `quill_core::assistants`; the CLI's
`mcp.rs` only maps that onto MCP tools with the official Rust SDK, `rmcp`.

- **stdio, not a port.** The assistant starts `quill-cli mcp` and talks
  over stdin/stdout. Nothing listens on the network, there's no token to
  leak, and it works whether or not the app is running. A Streamable HTTP
  server inside the app was the alternative. It would mean a port, a token,
  checks against DNS rebinding, and it would only work while the app was
  open. Shipping a server binary in the app bundle (a Tauri sidecar) is
  left for later; for now it's the CLI.
- **Off unless the user turns it on, in the app.** `assistant_access` is
  switched in Settings → Meetings, not by the CLI. The app keeps settings
  in memory and writes the whole file when anything changes, so a flag set
  from the CLI would be turned off again by the app's next save. The
  server re-reads `settings.toml` on every tool call, so turning it off
  stops a connected assistant at once. With it off, `quill-cli mcp`
  refuses to start.
- **Read-only, meetings only.** Tools: `search`, `list_meetings`,
  `get_meeting`, `action_items`, `daily_summary`. Dictations are left out,
  even from search (`assistants::search` drops them from
  `search_everything`'s results): they're often private messages typed into
  other apps, not meeting content. Writes, such as ticking off an action
  item, would need consent of their own.
- **Notes first, transcripts in pages.** `get_meeting` returns the notes
  and a line count. The transcript comes only when asked for, 300 lines at
  a time (about 10k tokens), with where the next page starts. An hour's
  transcript is 30–40k tokens, and the notes answer most questions.
  Results are Markdown or JSON text rather than MCP structured content:
  the library's types don't derive JSON Schema, and models read text well.
- **Search by meaning, if it's ready.** The search model loads on the first
  search, and only if search by meaning is on and the model is downloaded.
  This never starts a download, and otherwise it searches by words. The
  server never indexes; the app does that.
- **Sharing the database with the app.** rusqlite sets a 5 s busy timeout
  on every connection, and Quill's queries are short, so the two processes
  share `quill.sqlite` in its rollback-journal mode without WAL.
- **Logs.** The tool name, time taken and whether it worked are logged.
  What was asked or returned never is.
