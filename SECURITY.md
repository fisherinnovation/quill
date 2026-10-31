# Security policy

## Reporting a vulnerability

Please **don't** report security issues in public GitHub issues, discussions
or pull requests.

Report them privately through GitHub's private vulnerability reporting: open
the repository's **Security** tab and choose **Report a vulnerability**
([direct link](https://github.com/fisherinnovation/quill/security/advisories/new)).

Please include:

- what the issue is and its impact,
- the steps to reproduce it (a proof of concept if you have one),
- the affected version or commit, and your OS.

You'll get an acknowledgement within a few days. We'll work with you on a fix
and on timing for disclosure, and credit you in the advisory unless you'd
rather not be named.

## Supported versions

Security fixes go into the `main` branch and the next release. Only the
latest release is supported.

## How Quill handles data

This section describes the intended design, so you can check it and report
where reality differs.

**Network.** Quill connects only to:

- the summary and clean-up model you configure: OpenRouter (`openrouter.ai`),
  or an Ollama server (`localhost:11434` by default, or `OLLAMA_HOST`);
- Hugging Face, to download speech models, and the search model when you
  ask for it. Each download is checked against a pinned SHA-256 hash before
  use. Choosing Parakeet in **Settings → Models** downloads its four files
  (about 660 MB) from sherpa-onnx's exports
  (`csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-*-int8`), checked the same
  way; like Whisper, it runs on your computer and sends nothing anywhere.
- OpenRouter's public model list (no key, nothing about you), and the local
  Ollama server's list of installed models, to fill the model pickers in
  Settings.
- GitHub, once a day, to see whether a newer Quill has been released
  (`latest.json` on the latest release; nothing about you or your meetings
  is sent). Turn it off in **Settings → Updates**. An update downloads only
  when you click **Restart to update**, and installs only if it's signed by
  Quill's update key (the public half is built into the app) for the version
  it claims to be. Debug builds never check on their own.

Quill opens a network port only while **Settings → Meetings → Let AI
assistants read your meetings** is on (off by default): a server for AI
assistants on `127.0.0.1` (port 47321 unless changed), which other computers
can't reach (see below).

There's no telemetry, analytics or crash reporting.

**Notifications.** When a meeting's notes are ready (or couldn't be
written) and Quill isn't in front, a system notification says so, with the
meeting's title and how many action items and decisions it has, or the
first sentence of the error. Notifications are local (macOS's notification
centre or a Windows toast), may show on a locked screen depending on your
system settings, and can be turned off in **Settings → Meetings → Tell me
when notes are ready**. On macOS, Quill asks permission the first time.

**What is sent.** Audio is never sent anywhere. Meeting transcripts are sent
to the summary model, and dictated text to the clean-up model, only when
those steps run, and only to the provider you chose. With live notes on
(Settings → Meetings, on by default), a rough transcript of the last 15
minutes of a meeting is sent to the summary model every minute while
it's being recorded; the live notes panel is excluded from screen sharing
and screen recording, and the live transcript and notes are kept only in
memory (they aren't saved or logged). A daily summary, written only when
you click **Summarize this day** on the Meetings page, sends that day's
meetings' notes (not their transcripts) to the summary model. OpenRouter
requests ask for providers that don't collect data (`data_collection:
deny`), and optionally for zero data retention.

**AI assistants.** Off by default, and turned on only after a confirmation
that says what's shared. With **Let AI assistants read your meetings** on,
the app serves MCP tools while it's open, at
`http://127.0.0.1:<port>/mcp`: search, listing meetings, reading a
meeting's notes and transcript, action items and daily summaries. It stops
the moment the setting is turned off, and every request re-checks the
setting too. What an assistant reads is sent to its provider under that
provider's terms, not Quill's. Dictations are never served, even in search
results.

- It listens on IPv4 loopback only, so nothing off the computer can connect.
- Every request needs a bearer token: 256 random bits, made on first use,
  kept in the macOS Keychain or Windows Credential Manager (account
  `assistant-token`), shown in Settings while the server runs, never logged,
  and compared in constant time. **New token…** replaces it, locking out
  every assistant set up with the old one.
- Requests from web pages are turned away (any `Origin` header), as are
  ones whose `Host` isn't a loopback name, which blocks DNS rebinding. Only
  the `/mcp` path is served.
- The server can't write to the library, start recordings or downloads, or
  read audio. It logs which tool was called, how long it took and whether
  it worked, and that a request without a valid token was turned away;
  never what was asked or returned.
- It can read the vocabulary (Settings → Dictation → Words). It changes
  nothing unless **Let them change your vocabulary** is also on (off by
  default, re-checked on every request). Then it can add terms to the
  vocabulary and take terms out, and nothing else: each term must be one
  line of at most six words and 60 characters, and the list stops at 500
  terms. The vocabulary goes into the prompt that writes meeting notes, so
  these limits keep what an assistant adds to names and terms rather than
  instructions. Changes go only through the app's address; `quill-cli mcp`
  only reads the vocabulary.
- With **Let them tick off action items** also on (off by default,
  re-checked on every request), it can tick an action item off or back on
  by its id, through either way in. That only sets when the item was done.

`quill-cli mcp` serves the same tools over stdin/stdout for an assistant
that starts it itself, with no port or token. It refuses to start while the
setting is off and re-reads it on every request. Any program running as you
could already read the library's files; the setting and the token are about
consent to send meetings to an assistant and keeping web pages and other
users out, not a boundary against software you run.

**Secrets.** API keys are stored in the macOS Keychain or Windows Credential
Manager, never written to configuration files, and never logged. For
development, a key can also come from the `QUILL_OPENROUTER_API_KEY`
environment variable.

**Data at rest.** Meetings, transcripts, notes, dictation history and audio
are stored **unencrypted** in Quill's data folder (see the README), protected
by your operating-system account. Use full-disk encryption (FileVault or
BitLocker) if that matters for you. Corrections you make to dictations are kept in the history (and deleted with it). The rules they suggest are stored as word pairs only. The "About you" text is sent to the clean-up model with dictations being tidied. With **Save dictations for testing** on (it's off by default), each dictation's audio, the text typed and the app it went into are kept in `dictation-samples/` until you delete them; they're never sent anywhere. Telling speakers apart stores voice
fingerprints (embeddings) of the people on a call in that meeting's folder
(`speakers.json`); they're deleted with the meeting and never sent anywhere.
Recognizing known voices (Settings → Meetings → Recognize voices, on by
default) keeps voice fingerprints in `quill.sqlite`: for each meeting, one
average fingerprint per person on the call (deleted with the meeting), and
for each person you name, their name and one fingerprint per meeting you
named or confirmed them in. Fingerprints are numeric embeddings from the
local voice model, not audio, but they can identify a voice, so treat them
like biometric data: they never leave your computer, are never logged, and
a known voice is kept until you forget it in Settings → Meetings (deleting
a meeting doesn't). Your own microphone track is never remembered.
Daily summaries are kept in `quill.sqlite`, one per day; since each holds
its meetings' content, deleting any of that day's meetings deletes it.
Search (Ctrl+K / ⌘K) keeps a word index in `quill.sqlite` holding a copy
of each meeting's title, latest notes (point by point) and action items, and
of each dictation in the history. Search by meaning (Settings → Meetings,
on by default, but only once its model is downloaded) stores an embedding
of each half minute or so of every transcript, and of each notes point,
action item and dictation. Both are computed on your computer, and each
entry is deleted with what it came from: the meeting, the dictation, or
the whole dictation history when you clear it. Embeddings are kept
(unused) if you turn search by meaning off. Search queries are never
logged, stored or sent anywhere. Logs contain events, timings and errors,
never transcripts, notes, dictated text or keys.

**Permissions.**

| Permission | Why |
|---|---|
| Microphone | Recording your side of meetings, and dictation. |
| System audio recording (macOS) | Recording the call. Windows needs no permission for this. |
| Accessibility (macOS) | Simulating ⌘V to paste dictated text. Without it, text is left on the clipboard. With fn as the dictation key, also watching for fn being pressed: Quill looks only at whether fn is down and whether another key was pressed with it, never at which keys or what's typed. |
| None (Windows) | With right Ctrl as the dictation key, Quill watches the keyboard with a low-level hook, which Windows allows without asking. It looks only at whether right Ctrl is down and whether another key was pressed with it, never at which keys or what's typed, and passes every key on unchanged. With any other shortcut there's no hook. |

For dictation, Quill reads the frontmost app's identifier and name, never its
windows or contents. When dictation starts, it also asks what kind of element
has keyboard focus (macOS Accessibility; Windows UI Automation), to leave the
final period off text going into a one-line box such as a search field. It
reads only the element's role (a text field, a text area, a search box) and
whether it's one line, never its text, and keeps nothing. Chromium-based apps
(Chrome, Edge, Electron) turn on their accessibility support when asked, as
they do for password managers and screen readers. To show apps' icons beside your dictations, it reads the
icon of the installed app (on Windows, of the executable it saw in front, whose
path is kept in memory only, never on disk). To offer to record calls (macOS, on by default, Settings →
Meetings), it checks twice a second which apps are using a microphone: their
process id and bundle identifier, from Core Audio, which needs no permission.
It never reads their audio, windows, tabs or contents, and logs only the call
app's name ("Zoom") when a call starts or ends. Dictated text passes through the clipboard: it's marked
so clipboard managers don't record it, and your previous clipboard is put back.

**GPU (Windows).** Transcription runs on the GPU through Vulkan. The
installer ships the Khronos Vulkan loader (`vulkan-1.dll`) next to Quill, so
it starts on PCs without one. As in any Vulkan app, the loader loads your
graphics driver and any Vulkan layers other software has registered
system-wide (overlays, for example) into Quill's process.

**Desktop app hardening.**

- The web interface runs under a strict Content Security Policy and can only
  call Quill's own commands. Tauri's built-in APIs are limited to the
  defaults plus window dragging.
- File and folder pickers open from Rust, so the interface never supplies a
  path to write to.
- The dictation indicator window can't take keyboard focus.

**Known gaps:**

- Builds aren't yet code-signed or notarized.
- Data at rest isn't encrypted by Quill itself.
