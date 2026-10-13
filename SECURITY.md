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

Quill is pre-1.0 and has no released builds yet. Security fixes go into the
`main` branch.

## How Quill handles data

This section describes the intended design, so you can check it and report
where reality differs.

**Network.** Quill connects only to:

- the summary and clean-up model you configure: OpenRouter (`openrouter.ai`),
  or an Ollama server (`localhost:11434` by default, or `OLLAMA_HOST`);
- Hugging Face, to download speech models. Each download is checked against
  a pinned SHA-256 hash before use.
- OpenRouter's public model list (no key, nothing about you), and the local
  Ollama server's list of installed models, to fill the model pickers in
  Settings.

There's no telemetry, analytics or crash reporting.

**What is sent.** Audio is never sent anywhere. Meeting transcripts are sent
to the summary model, and dictated text to the clean-up model, only when
those steps run, and only to the provider you chose. OpenRouter requests ask
for providers that don't collect data (`data_collection: deny`), and
optionally for zero data retention.

**Secrets.** API keys are stored in the macOS Keychain or Windows Credential
Manager, never written to configuration files, and never logged. For
development, a key can also come from the `QUILL_OPENROUTER_API_KEY`
environment variable.

**Data at rest.** Meetings, transcripts, notes, dictation history and audio
are stored **unencrypted** in Quill's data folder (see the README), protected
by your operating-system account. Use full-disk encryption (FileVault or
BitLocker) if that matters for you. Corrections you make to dictations are kept in the history (and deleted with it). The rules they suggest are stored as word pairs only. The "About you" text is sent to the clean-up model with dictations being tidied. Telling speakers apart stores voice
fingerprints (embeddings) of the people on a call in that meeting's folder
(`speakers.json`); they're deleted with the meeting and never sent anywhere. Logs contain events, timings and errors,
never transcripts, notes, dictated text or keys.

**Permissions.**

| Permission | Why |
|---|---|
| Microphone | Recording your side of meetings, and dictation. |
| System audio recording (macOS) | Recording the call. Windows needs no permission for this. |
| Accessibility (macOS) | Simulating ⌘V to paste dictated text. Without it, text is left on the clipboard. With fn as the dictation key, also watching for fn being pressed: Quill looks only at whether fn is down and whether another key was pressed with it, never at which keys or what's typed. |

For dictation, Quill reads the frontmost app's identifier and name, never its
windows or contents. Dictated text passes through the clipboard: it's marked
so clipboard managers don't record it, and your previous clipboard is put back.

**Desktop app hardening.**

- The web interface runs under a strict Content Security Policy and can only
  call Quill's own commands. Tauri's built-in APIs are limited to the
  defaults plus window dragging.
- File and folder pickers open from Rust, so the interface never supplies a
  path to write to.
- The dictation indicator window can't take keyboard focus.

**Known gaps** (planned before a public release):

- Builds aren't yet code-signed or notarized.
- Data at rest isn't encrypted by Quill itself.
