<div align="center">

<img src="app/app-icon.svg" alt="" width="96" height="96">

# Quill

**Local-first meeting notes and dictation for macOS and Windows.**

Record any call and get a transcript plus structured notes. Or hold a key, speak, and have clean text typed into whatever app you're using. Audio is processed on your computer and never uploaded.

[![CI](https://github.com/fisherinnovation/quill/actions/workflows/ci.yml/badge.svg)](https://github.com/fisherinnovation/quill/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

</div>

## Contents

- [Features](#features)
- [Privacy](#privacy)
- [Platform support](#platform-support)
- [Installing](#installing)
- [Building from source](#building-from-source)
- [Using Quill](#using-quill)
- [Summary and clean-up models](#summary-and-clean-up-models)
- [Command-line tool](#command-line-tool)
- [Where your data lives](#where-your-data-lives)
- [How it works](#how-it-works)
- [Performance](#performance)
- [Development](#development)
- [Roadmap](#roadmap)
- [Known limitations](#known-limitations)
- [Contributing](#contributing) · [Security](#security) · [License](#license) · [Acknowledgments](#acknowledgments)

## Features

### Meeting notes

- **Works with any call app.** Zoom, Google Meet, Teams, a browser tab, a phone app mirrored to your computer. Nothing joins the call as a bot.
- **Two tracks, real speaker labels.** Your microphone is recorded as **You** and the call audio separately, so your words are never mixed up with anyone else's.
- **Tells the other people apart.** Voices on the call are separated into **Speaker 1**, **Speaker 2**… on your computer ([pyannote](https://github.com/pyannote/pyannote-audio) segmentation and voice fingerprints via [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx)). Rename them in the transcript (typing an existing name merges two), or say how many people were on the call to split them again instantly. The notes use the names.
- **Recognizes people you've named.** Name someone once and Quill remembers their voice (on your computer), then names them in later meetings when their voice clearly matches, marked so you can confirm or correct it. Unsure matches stay **Speaker 2**. Known voices are listed, and forgotten, in **Settings → Meetings**.
- **Echo cancellation.** If you use speakers instead of headphones, a WebRTC echo canceller removes the call's audio from your microphone track before transcription.
- **Local transcription** with [Whisper](https://github.com/openai/whisper) through [whisper.cpp](https://github.com/ggerganov/whisper.cpp), GPU-accelerated: Metal on Apple Silicon, Vulkan on Windows (Nvidia, AMD and Intel graphics; Quill prefers a dedicated card, and **Settings → Models → Graphics card** picks another). Silence is skipped first, so Whisper doesn't invent text in quiet stretches.
- **Structured notes** written by a model you choose: TL;DR, decisions, action items with owners, discussion by topic and open questions. Long meetings are summarized in parts and combined.
- **Daily summaries.** On a day with two or more meetings, **Summarize this day** on the Meetings page writes one summary from their notes: what the day came to, the decisions (as they ended up, if a later meeting changed them), your action items and others', threads that ran across meetings, and open questions. Written only when you ask, and marked when the day's notes change.
- **Search everything.** <kbd>Ctrl</kbd>+<kbd>K</kbd> (<kbd>⌘</kbd>+<kbd>K</kbd> on a Mac) searches meeting titles, every transcript, each meeting's notes, action items and your dictation history at once, by their words and, optionally, by meaning: "when did we talk about hiring" finds "we're bringing on two engineers", in about 100 languages, with a small model that runs on your computer. Click-to-rename titles, export to Markdown, all stored in a local SQLite database.
- **Compact storage.** Recordings are kept as 16 kHz FLAC once transcribed, about 70 MB per hour.

### Dictation

- **Hold a shortcut and speak** (<kbd>⌥ Option</kbd> + <kbd>Space</kbd> by default), and the text is typed wherever your cursor is. Or tap the shortcut to start and tap again to stop. <kbd>Esc</kbd> cancels.
- **Fast.** About 0.3 s from releasing the key to typed text on an M3 Pro, because Whisper stays loaded and runs in a mode tuned for short clips.
- **Clean-up only when it helps.** A model removes "um"s, applies self-corrections ("at 3, no wait, 4" becomes "at 4") and formats spoken lists. It only runs when there's something to fix, and its output is checked before use, so it can't type an answer to your question instead of your question.
- **Per-app style.** Casual in chat apps, formal in email, exact names and commands in code editors and terminals. Built-in defaults, and you can change any app.
- **Your words.** A vocabulary list for names and jargon, plus replacement rules ("open router" → "OpenRouter").
- **A second speech engine.** NVIDIA's [Parakeet](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2) can transcribe dictation (or meetings) instead of Whisper: **Settings → Models → Speech recognition for dictation**. It runs on the CPU, so on a computer without a usable graphics card it's far quicker than Whisper (about 0.2 s against 7 s for a short dictation on an i9-14900KF); with one, Whisper is as quick or quicker, so it stays the default. v2 is English only; v3 knows 25 European languages and detects which. It takes no language setting, and your vocabulary reaches it as hotwords rather than a prompt.
- **A floating indicator** with a live level meter, start and stop sounds, and a menu-bar status.
- **Your clipboard is preserved.** Text is inserted by pasting; whatever you had copied is put back, and the dictated text is hidden from clipboard managers.

### Insights (on Home)

- Words dictated, time saved compared with typing, speaking speed, streaks, a 30-day activity chart, time of day and top apps.
- Meeting count and hours, and your share of the talking.
- Built from counts only: Quill keeps numbers about dictations, not what you said.

## Privacy

Quill is built so that the sensitive parts stay on your computer.

| Data | Where it goes |
|---|---|
| **Audio** (meetings and dictation) | Never leaves your computer. Transcription runs locally. |
| **Meeting transcripts** | Sent to the summary model you choose: [OpenRouter](https://openrouter.ai) (routed only to providers that don't store or train on your data; zero-data-retention mode available), or a local [Ollama](https://ollama.com) model, so nothing leaves your machine. Summaries are optional. With live notes on (the default), the last 15 minutes of the conversation also go to that model every minute *during* a recording. A daily summary, when you ask for one, sends that day's meetings' notes (not their transcripts) to the same model. |
| **Dictated text** | Sent to the clean-up model only when clean-up runs (fillers, corrections, lists). Clean-up can be turned off or pointed at a local Ollama model. |
| **API keys** | Stored in the macOS Keychain or Windows Credential Manager, never in config files or logs. |
| **Logs** | Local files with events, timings and errors. Never transcripts, notes, dictated text or keys. |
| **Voice fingerprints** | Never leave your computer. Telling speakers apart and recognizing known voices use numeric voice embeddings, stored locally (see [Where your data lives](#where-your-data-lives)). |
| **Search** | Local. Search by meaning runs its model on your computer; transcripts, notes, dictations and what you search for are never sent anywhere. |
| **Telemetry** | None. Quill makes no network requests other than the model calls above, downloading the speech and search models, and a daily check on GitHub for a new version (which can be turned off). |

For dictation, Quill reads the *name* of the frontmost app (for per-app style and insights), never its windows or contents, and shows the installed app's icon beside your dictations. To offer to record calls (macOS), it checks twice a second which apps are using a microphone (their identifiers only), never their audio, windows or contents.

## Platform support

| | macOS | Windows |
|---|---|---|
| Minimum OS | macOS 14.6 (system-audio capture uses Core Audio process taps) | Windows 10 / 11 |
| Transcription | Metal GPU on Apple Silicon | Vulkan GPU (Nvidia, AMD, Intel), or the CPU without one; CUDA in the CLI with a build flag |
| Meeting notes | ✅ | ✅ |
| Dictation | ✅ (<kbd>fn</kbd> key or a shortcut) | ✅ (a shortcut) |
| Installers | `.dmg`, Apple Silicon, not yet notarized | `.exe` / `.msi`, not yet signed |

Apple Silicon is strongly recommended on macOS: Whisper runs several times faster on its GPU. Linux isn't supported.

## Installing

Installers are attached to each [release](https://github.com/fisherinnovation/quill/releases), or you can [build from source](#building-from-source):

- **macOS:** `Quill_<version>_aarch64.dmg` (Apple Silicon). Open it and drag Quill to Applications.
- **Windows:** `Quill_<version>_x64-setup.exe` (installs for your user account; recommended), or the `.msi`.

The builds aren't signed by a registered developer yet, so the first launch needs one extra approval:

- **macOS:** opening Quill shows a message that Apple couldn't check it for malicious software. Click **Done**, then open **System Settings → Privacy & Security**, scroll to Security, and click **Open Anyway** next to the message about Quill. This is needed only once. Later versions keep your microphone, system-audio and Accessibility permissions.
- **Windows:** SmartScreen shows "Windows protected your PC". Click **More info**, then **Run anyway**.

Speech models download the first time they're needed (see [First run](#first-run)). The installers are built by [GitHub Actions](.github/workflows/release.yml) from the tagged source.

### Updates

Quill looks for a new version once a day. When there is one, Home, **Settings → Updates** and the About window offer **Restart to update**; nothing downloads or installs until you click it, and it waits if you're recording, dictating or a meeting is being written up. Updates are checked against Quill's signing key before they install, and your recordings, settings, models and permissions stay as they are. Turn the daily check off in **Settings → Updates** (where **Check now** still works). Version 0.1.0 predates the updater, so install the next version from the releases page once; later versions update themselves.

## Building from source

### Prerequisites

- **Rust**, latest stable, via [rustup](https://rustup.rs).
- **Node.js 24** and npm, for the app's interface.
- **CMake and a C/C++ compiler**, to build whisper.cpp:
  - macOS: `xcode-select --install` (the Command Line Tools are enough; full Xcode isn't needed) and `brew install cmake`.
  - Windows: [Visual Studio 2022 Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) with the *Desktop development with C++* workload, which includes CMake.
- **Windows: the [Vulkan SDK](https://vulkan.lunarg.com/sdk/home#windows)**, for whisper.cpp's GPU backend. Install it yourself (it sets `VULKAN_SDK`), or from Git Bash run `export VULKAN_SDK="$(scripts/install-vulkan-sdk.sh)"`, which installs the version CI uses after checking its hash.

### Build and run

```sh
git clone https://github.com/fisherinnovation/quill.git
cd quill/app
npm ci

npm run app          # development: hot reload for the interface
npm run app:bundle   # macOS: build ~/Applications/Quill.app and open it
npx tauri build      # release bundle (.app/.dmg on macOS, .msi/.exe on Windows)
```

The first build compiles whisper.cpp and takes several minutes; later builds are incremental.

> [!TIP]
> **Developing on macOS:** use `npm run app:bundle` (not `app`) to test
> dictation, because macOS only lets real app bundles into the Accessibility
> list. It signs the build with a fixed identity, so microphone, system-audio
> and Accessibility permissions survive rebuilds. See [`app/README.md`](app/README.md).

### First run

1. A welcome screen explains what Quill records, then runs an **access check**: a few seconds of recording plus a short chime, which also brings up the permission prompts. Allow **Microphone** and **System Audio Recording**.
2. For dictation, open **Dictations** and click **Allow…** to grant **Accessibility**, which Quill needs to paste for you. Without it, dictated text is left on your clipboard.
3. The welcome screen also offers to download the models (about 1.7 GB): Whisper `turbo` (1.5 GB), the Silero VAD model (1 MB), and the speaker models (46 MB). They come from Hugging Face, and each is checked against its SHA-256 hash. If you skip that, they download the first time they're needed; pressing the dictation shortcut before then opens the Dictation page with the download's progress. **Settings → Models** lists each job that needs a model (speech recognition, silence detection, telling speakers apart, meeting notes and dictation clean-up), which model does it, and whether it's downloaded. Pick another Whisper size and it downloads straight away. Each local model can be re-downloaded (a fresh copy, checked against its hash again) or deleted; a deleted one downloads again when next needed. Downloads that no job uses are gathered at the end to delete in one go. Models live outside the app, so updates don't download them again. The search model (multilingual-e5-small, 488 MB) is the one exception to downloading on first use: it downloads only when you ask, from the search window or **Settings → Models**.
4. Meeting notes and dictation clean-up use OpenRouter models by default, which need an API key. The welcome screen has a place to paste one, with a link to create it, or add it later in **Settings → OpenRouter**. To keep everything local, choose an Ollama model instead (see below).

## Using Quill

Quill opens on **Home**: a line about your day (how many meetings, how long on calls, words dictated this week), the TL;DR and action items from your latest meeting's notes, the meetings before it, and your usage: the last 30 days, meetings and dictation in detail, and top apps. While a meeting is recording, it shows at the top with its timer. The sidebar leads to **Home**, **Meetings**, **Action items**, **Dictations**, **Help** and **Settings**, and collapses to icons with the button at its foot or <kbd>⌘</kbd><kbd>\\</kbd> (<kbd>Ctrl</kbd>+<kbd>\\</kbd> on Windows); hover an icon for its name; **Meetings** opens with what's happening (ready, recording or writing up) over a moving weave of your recent conversations, then this week's numbers (meetings, time on calls, your share of the talking, action items) and filters (action items, new, needs attention, and the people you meet most), and lists every meeting by day (each with the first point of its notes, who was there, its length, action items and how much you spoke, over a faint picture of the conversation's back and forth) and searches their transcripts, **Dictations** shows your dictation numbers over a waveform of your last 30 days (it follows your voice while you dictate) and lists what you've dictated, each with the app's icon, colour-coded by whether clean-up tidied it, you corrected it, or it was left on the clipboard (with filters for each), and **Help** explains every feature, with search (<kbd>⌘</kbd><kbd>F</kbd> or <kbd>/</kbd>) and a table of contents (also **Help → Quill Help** in the macOS menu bar). Settings has tabs: **General** (theme, OpenRouter, audio access, updates, logs), **Meetings**, **Dictation**, **Models** and **Storage**. The theme follows your computer's light or dark setting unless you pick **Light** or **Dark** under **Settings → General → Appearance**.

### Recording a meeting

Click **Record meeting** at the top of the sidebar, or use the menu-bar (tray) icon, when the call starts. Stop when it ends. You can pause and resume from the menu-bar icon or the live notes panel; nothing is recorded while paused, and the timer stands still. On macOS Quill also notices calls: when Zoom, Teams, Webex, FaceTime, Slack, Discord or a browser (for Google Meet and other web calls) has been using your microphone for about two seconds, a small prompt at the bottom of the screen asks whether to record, and when the call lets go of the mic, whether to stop. It sees only *which app* is using the microphone, so a browser call shows as "Google Chrome is using your microphone". Turn it off in **Settings → Meetings → Offer to record calls**. Quill then:

1. Cleans speaker echo out of your microphone track.
2. Transcribes both tracks locally and merges them into one transcript, with your lines labelled **You**.
3. Tells the people on the call apart by voice (*Tell speakers apart* in Settings), as **Speaker 1**, **Speaker 2**…
4. Writes the notes, if *Summarize automatically* is on (Settings).
5. Compacts the audio to FLAC.

You can watch it happen: the meeting shows each step with its progress, roughly how long is left, and the lines being transcribed as they come; Home and the meeting's tile on **Meetings** show it too. When it's done, the meeting says what came out (people, action items, decisions, words, how long it took) and the notes rise into view. Elsewhere in Quill a card says the notes are ready and the meeting is marked **New**; if live notes were showing, the panel stays after you stop to follow the write-up and offers **Open notes**; and if Quill isn't in front, a system notification says so (click it to open the meeting; macOS asks permission the first time; off in **Settings → Meetings → Tell me when notes are ready**). In the **Transcript** tab, click a speaker to name them. With 10 people on a call, Quill reliably separates everyone who talks for more than a minute or so, but people who say only a sentence or two may be merged into someone else; choosing the number of people under **People on the call** helps most. Telling speakers apart adds about 2 minutes per hour of audio, and the voice models (46 MB) download the first time. From a meeting you can re-summarize, copy the notes as Markdown, export them to a `.md` file, search its transcript, or delete it along with its audio.

The **Stats** tab shows how the conversation went: who talked and for how long, each person's pace, turns and questions, a timeline of who spoke when (click a stretch to read it in the transcript), moments like the longest monologue, the longest silence and the two people who went back and forth most, and the words and phrases that came up most. It's worked out on your computer from the transcript, and nothing extra is stored. Each dictation shows its own: words, pace against your usual, time saved compared with typing, how long you waited for it (see *Time to insert* below), and what clean-up took out. Word lists are English-first, so in other languages the most-used words can include common words.

If a recording turns out to have nothing said in it, it's marked **Nothing said** in the list of meetings, and opening it offers **Delete recording** or **Keep**. **Settings → Storage** lists how many such recordings there are and deletes them all at once, after you confirm. Quill never deletes them on its own.

Closing the window keeps Quill running in the menu bar, so a recording isn't cut off. Bring it back by clicking Quill in the Dock, with **Window → Quill** (<kbd>⌘</kbd> <kbd>0</kbd>), or from the menu-bar icon's **Open Quill** (on Windows, the tray icon's). Quit from the menu-bar icon.

**About Quill** (in the Quill menu on macOS, or the menu-bar icon's menu) shows the version and build, the models in use, links, and credits. **Copy diagnostic info** there copies what a bug report needs (no keys or text). On macOS, **Help → Report a Problem…** opens a new issue.


**Live notes.** While you record, a panel at the right edge of your screen keeps notes as the meeting goes: **Ask before the end** (questions left open, decisions without an owner or date), **Follow-ups** and **Decisions**, with a line on where the conversation is. Quill transcribes what's said every ten seconds or so (roughly, from your mic as **You** and the call as **Others**), and every minute asks your notes model to update the notes; items the conversation has dealt with tick themselves off. Tick items off yourself, dismiss ones you don't want (they won't come back), update now, or collapse the panel to a tab on the screen edge. Its header has the recording's timer and **Pause** and **Stop** (Stop asks once, so a stray click doesn't end the meeting). It's hidden from screen sharing, so others on the call don't see it, and it never takes focus from your call. The notes model gets the last 15 minutes of the conversation each time (with OpenRouter, the provider rules above apply; use an Ollama model to keep it on your computer). Turn it off in **Settings → Meetings → Live notes during meetings**. The meeting's real transcript and notes are still written after the recording, as before.

### Action items

**Action items** in the sidebar gathers what your meetings agreed needs doing, from every meeting's notes, so it doesn't stay buried in them. Yours come first, most pressing at the top (overdue, then due soonest, then undated); then **Waiting on others**, by person, for following up. The number beside **Action items** in the sidebar is how many of yours are open. Tick an item off there, in the meeting's notes or on Home: it's the same item everywhere. Due dates are worked out from the day of the meeting ("by Thursday" said on a Tuesday is that Thursday; "end of next week" the Friday after), and anything that isn't a date shows as written. Items are read from the notes when they're written; re-summarizing keeps the ticks on items that are still there, even reworded.

### Dictating

Put your cursor in any text field, then:

- **Hold** <kbd>⌥ Option</kbd> + <kbd>Space</kbd> while you speak and let go to type, or
- **Tap** it, speak, and tap again.

On macOS you can use <kbd>fn</kbd> (🌐) on its own instead: choose **fn** under **Settings → Dictation → Shortcut**. fn with another key (fn+Delete, fn+arrows, fn+F-keys) keeps working as usual. If macOS's own **Press 🌐 key to** setting isn't **Do Nothing**, pressing fn also opens the emoji picker or switches the input source; the Dictation page links to that setting. Using fn needs the same Accessibility permission as pasting.

Press <kbd>Esc</kbd> to cancel. A pill near the bottom of the screen shows the mic level while listening and "Typing…" while processing.

In **Settings → Dictation** you can change the shortcut, choose the language (your system language by default; auto-detection is available but about 0.5 s slower), pick the sound it plays to start and stop (Classic, the default, or Soft, Bubble, Wood, Glass, Marimba, Chirp, Digital or none; choosing one plays it), turn clean-up on or off, set a style per app, and add vocabulary and replacements. The **Dictations** page shows whether dictation is ready and lists your dictations by day, newest first, with **Show more** for older ones. Each has a **Copy** button beside it (in case text didn't land where you meant it to); open one to correct it or delete it.

**Live captions.** While you speak, what you've said so far appears above the dictation pill, the latest words fainter until Quill is sure of them. Most of a long dictation is transcribed before you let go, so it's typed sooner (a minute of speech in about half a second, not three). The captions are on your screen, so they're visible in screen shares; turn them off in **Settings → Dictation → Show what you're saying**.

**Time to insert.** The wait you notice when dictating is from your last word to the text appearing: the moment you keep holding the shortcut after speaking, plus Quill transcribing and typing after you let go (the "release to typed text" time under [Performance](#performance)). Quill finds where your speech ended with Silero VAD, after the text is typed so it adds no delay, and records the wait with each dictation's numbers. The **Dictations** page shows the last week's median against the week before, how it splits between holding the key and Quill, and 30 days of daily medians with the slowest 1 in 10 marked; each dictation shows its own. It needs **Skip silence** on (the default), and counts dictations from this version on.

**Teaching Quill.** When Quill gets a name or term wrong, open that dictation from **Dictations**, click **Correct** and fix the text. Quill compares your fix with what it typed and suggests what to learn, under **Learn from your fixes** at the top of the **Dictations** page:

- A misspelled name or term ("Quil" → "Quill") becomes a vocabulary word, which steers both the speech model and the clean-up.
- Words typed apart ("open router" → "OpenRouter") become a replacement.

Nothing is learned until you click **Add**, and learned rules can be removed under **Settings → Dictation → Words**. Ordinary word changes ("their" → "there") aren't learned, because as a rule for every dictation they'd be wrong as often as right. With clean-up on, **About you** tells the clean-up model about you (your role, names you use, spelling preferences) so it gets them right. It's never added to your text.

**Testing accuracy.** **Settings → Dictation → Save dictations for testing** (off by default) keeps the audio of each dictation and what was typed, on your computer only. **Review…** in the same section plays each one beside what was typed; fix it into what you said (or keep it) and Quill shows how many words it got right, as typed and from Whisper alone, and your commonest mistakes. The fixes are references for `quill-cli dictate`, to measure changes on your own voice; see [`eval/dictation/README.md`](eval/dictation/README.md). It takes about 1 MB per minute of speech, and the same section shows how many are saved and deletes them.

### Per-app styles

| Style | Built in for | Effect |
|---|---|---|
| Casual | Slack, Messages, Discord, WhatsApp, Telegram, Teams, Signal | One-sentence messages lose their final period, the way people type in chat. Question marks stay. |
| Formal | Mail, Outlook, Spark, Superhuman, Word, Pages | Complete, properly punctuated sentences. |
| Technical | VS Code, Cursor, Zed, Xcode, JetBrains IDEs, Terminal, iTerm, Warp, Ghostty | Code names, file names, commands and flags kept exactly as spoken. |
| Default | Everything else | Your words, tidied up. |

The Casual period rule is applied locally, with no model call. The other styles shape the clean-up step when it runs. A style never makes clean-up run more often, so it adds no delay.

## Summary and clean-up models

Quill talks to models through [OpenRouter](https://openrouter.ai) (one API key, hundreds of models) or a local [Ollama](https://ollama.com) server.

| Use | Default | Why |
|---|---|---|
| Meeting notes | `anthropic/claude-haiku-5.5` | Accurate and cheap: a one-hour meeting costs well under a cent. |
| Dictation clean-up | `google/gemini-3.5-flash-lite` | The fastest consistent model in our comparison (0.66 s median); see [`eval/dictation/README.md`](eval/dictation/README.md). |

Choose them in **Settings → Models** (clean-up also in **Settings → Dictation**). Each picker offers:

- a few recommended models, with what each is good at and, for notes, the rough cost of an hour-long meeting from OpenRouter's current prices;
- the models installed in Ollama, to stay fully local;
- **Other OpenRouter model…**, which checks the id you type against OpenRouter's model list and shows its prices.

From the CLI, `-m` takes any OpenRouter id or `ollama:<model>`.

**OpenRouter key:** in **Settings → OpenRouter**, click **Add key…** (or **Change key…**) and paste it, or from a terminal run `quill-cli keys set openrouter`. **Get a key** opens openrouter.ai/keys. The key is stored in the macOS Keychain or Windows Credential Manager. On Windows, Settings shows whether a key is stored. On macOS, opening Settings never reads the keychain, because that alone can ask for your password, so it can't tell until the key is used. Quill reads the key only when it writes notes or tidies dictation, once per launch. For development, the `QUILL_OPENROUTER_API_KEY` environment variable overrides the keychain.

**Data handling:** requests always ask OpenRouter to route only to providers that don't collect your data. *Zero data retention only* (Settings → OpenRouter) is stricter and supported by fewer models.

## Command-line tool

`quill-cli` exposes the same engine for scripting, testing and evaluation:

```sh
cargo run --release -p quill-cli -- --help
```

| Command | What it does |
|---|---|
| `record` | Record the microphone and system audio into a new library meeting (or `--out <dir>`). |
| `transcribe <file or dir>` | Transcribe a WAV/FLAC file or a recording folder (echo cancellation and VAD on by default; `--diarize [--speakers N]` tells the people on the call apart). |
| `diarize <file or dir>` | Who spoke when, optionally scored against a reference RTTM (see [`eval/diarization`](eval/diarization)). |
| `summarize <dir or transcript.json>` | Write meeting notes (`-m` picks the model, e.g. `-m ollama:qwen3:8b`). |
| `dictate <clips…>` | Run the dictation pipeline on audio files and show each step's output and timing; `--polish-models a,b` compares clean-up models; `--stream` replays them through live captions and compares the text and the wait after release with a single pass. Clips with a reference (`clip.txt`, what should have been typed) are scored by word error rate. |
| `models list` / `models download <name>` | Manage models: Whisper (`tiny`, `base`, `small`, `medium`, `turbo-q5`, `turbo`, `large-v3`), Parakeet (`parakeet-v2`, `parakeet-v3`), VAD and the diarization models. `transcribe -m` and `dictate --stt-model` take any of the speech models. |
| `bench <file>` | Compare transcription speed across models (`--short-clip` for dictation settings). |
| `eval` | Compare summary models on cost, latency and quality judged by an LLM. |
| `import <dir>` | Add a recording folder made elsewhere to the library. |
| `compact [dirs…]` | Convert recordings' WAVs to FLAC (the app does this automatically). |
| `keys set\|delete openrouter` | Manage the API key in the OS keychain. |
| `gpus` | List the graphics cards Quill can transcribe on, and the one it picks by itself. |
| `voices match <a> <b>` / `voices eval <wavs…>` | Known voices: compare the people in two recordings by voice and show who in the second would be recognized, or measure recognition and false matches on audio with reference RTTMs (see [`eval/diarization`](eval/diarization)). Neither writes to the library. |
| `mic-users [--watch]` | List the apps using a microphone (macOS), e.g. to see how a call shows up: Zoom as `us.zoom.xos`, a Meet tab as `com.google.Chrome.helper`. Reads only which apps, never audio. |
| `search <query>` | Search the library by words and meaning, as the app does: transcripts, or with `--everything` titles, notes, action items and dictations too (`--words-only` for words alone; `--index` first indexes what isn't indexed yet, writing to the library). `--eval eval/search/queries.json` measures search quality and speed on labelled transcripts in a scratch library, `--scale-hours N` also times it on a bigger one, and `--scale-dictations N` times searching everything with a long dictation history. |
| `live-notes <recording or audio file>` | Replay a recording through live notes (the panel during meetings) and print the notes at each update (every minute, as in the app), with timings and cost. |

The CLI and the app share one library, so meetings recorded or imported in either appear in both.

## Where your data lives

**Settings → Storage** shows how much space Quill takes in total, split into meeting audio, models, notes and transcripts, logs, and the app itself.

| | macOS | Windows |
|---|---|---|
| Data folder | `~/Library/Application Support/com.fisherinnovation.Quill` | `%APPDATA%\fisherinnovation\Quill\data` |

Inside it:

- `quill.sqlite`: meetings, transcripts, summaries, daily summaries (each deleted with any of its day's meetings), action items (and which are done), dictation history and insight counts. Also known voices: the name of each person you named on a call, with one voice fingerprint (a 192-number embedding, not audio) per meeting you named them in, and each meeting's average voice fingerprint per person on the call, kept so naming them later can teach Quill their voice. And the search index: a copy of each meeting's title, its latest notes (point by point), its action items and each dictation's text, for searching everything; and for search by meaning, a vector of 384 numbers for every half minute or so of transcript (about 200 KB per hour) and for each notes point, action item and dictation. Each entry is deleted with what it came from: a meeting, a dictation, or the whole dictation history when you clear it.
- `recordings/<meeting id>/`: audio (`mic`/`system`, `.wav` while recording and `.flac` afterwards), `transcript.json` (the two-track transcript), `speakers.json` (voice fingerprints of the people on the call, used to re-split speakers; deleted with the meeting), `summary.md`, `recording.json`. The recordings folder can be moved in **Settings → Storage**.
- `models/`: downloaded Whisper, Parakeet, VAD, speaker and search models.
- `settings.toml`: settings (safe to edit while Quill isn't running).
- `logs/`: daily log files, kept for two weeks.
- `dictation-samples/`: only with **Save dictations for testing** on. Each dictation's audio (`.flac`), what was typed (`.typed.txt`), the app it went into (`.json`), and once reviewed, what you said (`.txt`).

Deleting a meeting in the app removes its database rows and its audio folder. Dictation history can be turned off or cleared, and insight counts can be reset separately (**Reset** at the bottom of Home). Saved dictation samples are deleted from **Settings → Dictation → Testing accuracy**. Known voices are forgotten one by one or all at once in **Settings → Meetings**; deleting a meeting deletes its per-meeting voice fingerprints, but not a known voice learned from it.

## How it works

```mermaid
flowchart LR
    subgraph Meeting
        A[Mic + system audio<br/>cpal] --> B[Echo cancellation<br/>WebRTC AEC3]
        B --> C[Silero VAD]
        C --> D[Whisper<br/>whisper.cpp]
        D --> E[Merge tracks<br/>You / the call]
        E --> S[Tell speakers apart<br/>pyannote + voice embeddings]
        S --> F[Summary<br/>OpenRouter or Ollama]
        F --> G[(SQLite + FTS5)]
    end
```

```mermaid
flowchart LR
    subgraph Dictation
        H[Shortcut held<br/>warm mic + pre-roll] --> I[Whisper<br/>short-clip mode]
        I --> J{Needs clean-up?}
        J -- yes --> K[Clean-up model<br/>+ per-app style]
        J -- no --> L[Local style rules]
        K --> L
        L --> M[Paste via clipboard<br/>then restore it]
    end
```

The Rust workspace is split so that the app and the CLI are thin layers over library crates:

| Crate | Responsibility |
|---|---|
| [`quill-audio`](crates/quill-audio) | Microphone and system-audio capture (cpal: Core Audio process taps on macOS, WASAPI loopback on Windows), live mic for dictation, resampling, echo cancellation, WAV/FLAC, sounds. |
| [`quill-stt`](crates/quill-stt) | Speech-to-text behind a `Transcriber` trait: whisper.cpp via whisper-rs, NVIDIA Parakeet via sherpa-onnx, Silero VAD, model catalog and verified downloads. |
| [`quill-llm`](crates/quill-llm) | LLM providers behind an `LlmProvider` trait (OpenRouter, Ollama), keychain access, versioned prompt templates. |
| [`quill-diarize`](crates/quill-diarize) | Telling speakers apart: sherpa-onnx segmentation and voice embeddings, Quill's own clustering, DER scoring. |
| [`quill-core`](crates/quill-core) | Meeting pipeline, track merging, speaker labelling, summaries, the dictation pipeline, the SQLite library, settings, insights. |
| [`quill-insert`](crates/quill-insert) | Typing text into the focused app (clipboard + simulated paste), identifying that app, and watching the fn key on macOS. |
| [`quill-embed`](crates/quill-embed) | Text embeddings for search by meaning behind an `Embedder` trait: multilingual-e5-small run with [candle](https://github.com/huggingface/candle) on the CPU. |
| [`quill-eval`](crates/quill-eval) | The evaluation harness for summary models. |
| [`quill-cli`](crates/quill-cli) | The `quill-cli` command-line tool. |
| [`app/`](app) | The desktop app: [Tauri v2](https://tauri.app) in Rust ([`app/src-tauri`](app/src-tauri)) with a React + TypeScript interface ([`app/src`](app/src)). |

Prompts live as versioned Markdown in [`prompts/`](prompts) and are compiled into the binary. Every summary records which prompt version and model produced it. Design decisions and the reasoning behind them are recorded in [`docs/DESIGN_NOTES.md`](docs/DESIGN_NOTES.md).

## Performance

Measured on a MacBook Pro (M3 Pro, 36 GB):

| | Result |
|---|---|
| Meeting transcription (`turbo`, Metal) | about 24× real time: a one-hour meeting takes roughly 2½ minutes |
| Dictation, release to typed text | about 0.3 s, or about 0.95 s when clean-up runs |
| Meeting audio on disk | about 70 MB per hour once transcribed (about 2 GB per hour while recording) |
| Word error rate on the synthetic benchmark meeting | 2.6% (`turbo`) |

Details and methods: [`eval/benchmarks.md`](eval/benchmarks.md) (transcription) and [`eval/dictation/README.md`](eval/dictation/README.md) (dictation). The benchmark audio is synthetic speech, so real-call error rates will be higher.

## Development

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
(cd app && npm run build)          # type-checks and builds the interface
```

CI runs exactly these on macOS and Windows for every push and pull request.

**Releases:** bump `version` in `app/src-tauri/tauri.conf.json`, `app/package.json` and the workspace `Cargo.toml` in a pull request; once it's merged, tag `main` to match and push the tag (`git tag v0.2.0 && git push origin v0.2.0`). The [release workflow](.github/workflows/release.yml) builds the macOS `.dmg` and the Windows installers and attaches them to a draft GitHub Release to review and publish. Its notes are written from the `feat` and `fix` commits since the last tag ([`scripts/release-notes.sh`](scripts/release-notes.sh); run it to preview them); edit them in the draft before publishing. Running the workflow by hand (Actions → Release → Run workflow) builds the same installers as downloadable artifacts, without a release. Locally, `cd app && npx tauri build --bundles app && sh scripts/package-macos.sh` makes the `.dmg` in `dist-release/`.

- **Tests that need models** are ignored by default. Run them with `QUILL_TEST_MODEL=<path to ggml model> QUILL_TEST_VAD_MODEL=<path to VAD model> cargo test -p quill-stt -- --ignored`; the Parakeet ones need `QUILL_TEST_MODELS_DIR` (see `crates/quill-stt/tests/parakeet_smoke.rs`).
- **Previewing the interface without Rust:** `cd app && npm run dev`, then open http://localhost:1420. In a plain browser the app uses sample data ([`app/src/dev/mock.ts`](app/src/dev/mock.ts)). URL flags show particular states: `?onboarding`, `?recording`, `?listening`, `?processing`, `?failed`, `?nopaste`, `?empty`, `?light`, `?indicator`, `?about` (the About window), `?others` (call audio as one "Others" speaker), `?nomodels` (nothing downloaded), `?noollama` (Ollama not running), `?nokey` (no OpenRouter key), `?fn` (fn as the dictation shortcut), `?dark` (the dark theme), `?prompt` or `?prompt=stop` (the call prompt), `?gpus` (Windows' graphics card choice), `?voices` (a speaker recognized by voice), `?nosearch` (search by meaning without its model), `?indexing` (meetings being indexed for it), `?daily` (today's daily summary already written).
- **Evaluating summary models:** see [`eval/README.md`](eval/README.md). Put your own meeting transcripts in `eval/transcripts/private/`, which is gitignored.
- **Changing a prompt:** copy it to the next version directory (for example `prompts/summary-v3/`) rather than editing a shipped one, so results stay comparable.
- **The website** lives in [`site/`](site): static HTML, CSS and a little JavaScript, with no third-party requests. `node site/build.mjs` builds it into `site/_site`, filling in the version, the latest release's downloads and the source links. CI builds it on every run and publishes it to GitHub Pages from `main`. The header video is recorded from the app's browser preview by `node site/video/render.mjs` (see [`.claude/skills/site-video`](.claude/skills/site-video/SKILL.md) for how it's planned). The narrated overview video behind "Watch the overview" is made by `node site/overview/make.mjs`, which generates its voice, footage and music through OpenRouter (see [`.claude/skills/site-overview`](.claude/skills/site-overview/SKILL.md)).

The codebase enforces a few rules through lints: no `unsafe` outside small, documented platform wrappers; no `unwrap`/`expect` outside tests; typed errors in libraries. [CONTRIBUTING.md](CONTRIBUTING.md) has the details.

## Roadmap

Planned:

- Signed and notarized builds
- Data retention settings, such as deleting audio after a set time while keeping transcripts
- Remembering voices across meetings, so regular colleagues are named automatically

Ideas being considered: live captions during meetings, asking questions across past meetings, detecting when a meeting starts, merging your own typed notes into the summary, and taking speaker names from the call app (opt-in).

## Known limitations

- **Builds aren't code-signed or notarized yet,** so the first launch needs one extra approval (see [Installing](#installing)). Updates after that install without it.
- **Windows** has had less real-world use than macOS. Its GPU transcription (Vulkan) is new and so far measured on few machines. On a PC with both integrated and dedicated graphics, Quill picks the dedicated card; if that's the wrong choice, pick another in **Settings → Models → Graphics card**.
- **People who barely speak** on a large call may be merged into someone else, and two similar voices are sometimes treated as one. Fix either in the transcript. Voices aren't remembered between meetings yet.
- **Several people in one room on one microphone** are all "You".
- **Language:** Whisper supports about 100 languages, but the summary prompts and the dictation tests are English-first.
- **The floating indicator** may not appear over full-screen apps on macOS.
- **The default dictation shortcut** (<kbd>⌥ Space</kbd>) is also the default for some launchers and assistants. If another app already uses it, Quill says so and you can choose another.
- Transparent windows use a Tauri option built on private macOS interfaces. That's fine for distribution outside the Mac App Store, but would block an App Store release.

## Contributing

Contributions are welcome: bug reports, testing on Windows, prompt and model evaluations, and code. Please read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request, and open an issue first for anything large.

## Security

Please don't report security issues in public issues. See [SECURITY.md](SECURITY.md) for how to report them privately, and for a summary of how Quill handles keys and data.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.

Speech models downloaded at runtime have their own licenses: [Whisper](https://github.com/openai/whisper) (MIT), [Silero VAD](https://github.com/snakers4/silero-vad) (MIT), [pyannote segmentation-3.0](https://huggingface.co/pyannote/segmentation-3.0) (MIT) and NVIDIA NeMo [TitaNet-small](https://catalog.ngc.nvidia.com/orgs/nvidia/teams/nemo/models/titanet_small) (CC BY 4.0), the last two as exported to ONNX by sherpa-onnx, and, if you choose it, NVIDIA [Parakeet TDT 0.6B](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2) v2 or [v3](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3) (CC BY 4.0), as exported to ONNX and quantized by sherpa-onnx. The search model, Microsoft's [multilingual-e5-small](https://huggingface.co/intfloat/multilingual-e5-small), is MIT-licensed.

## Acknowledgments

Quill stands on excellent open-source work, including:

- [whisper.cpp](https://github.com/ggerganov/whisper.cpp) and [whisper-rs](https://codeberg.org/tazz4843/whisper-rs), with OpenAI's [Whisper](https://github.com/openai/whisper) models
- [Silero VAD](https://github.com/snakers4/silero-vad) for voice activity detection
- [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx), [pyannote](https://github.com/pyannote/pyannote-audio) and NVIDIA [NeMo](https://github.com/NVIDIA/NeMo) for telling speakers apart, and the [AMI Meeting Corpus](https://groups.inf.ed.ac.uk/ami/corpus/) for testing it
- [Tauri](https://tauri.app) for the desktop shell
- [cpal](https://github.com/RustAudio/cpal) for audio capture, and [rubato](https://github.com/HEnquist/rubato) for resampling
- [sonora](https://github.com/dignifiedquire/sonora), a Rust port of WebRTC's audio processing, for echo cancellation
- [flacenc](https://github.com/yotarok/flacenc-rs) and [claxon](https://github.com/ruuda/claxon) for FLAC
- [enigo](https://github.com/enigo-rs/enigo) and [arboard](https://github.com/1Password/arboard) for typing and the clipboard
- [OpenRouter](https://openrouter.ai) and [Ollama](https://ollama.com) for model access

Projects that informed Quill's design: [Meetily](https://github.com/Zackriya-Solutions/meeting-minutes), [Handy](https://github.com/cjpais/Handy), [OpenWhispr](https://github.com/HeroTools/open-whispr) and [VoiceInk](https://github.com/Beingpax/VoiceInk).
