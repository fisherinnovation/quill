# eval/dictation/

Latency and behaviour checks for dictation.

```sh
sh eval/dictation/make_clips.sh            # TTS clips and references from utterances.txt → clips/ (gitignored)
quill-cli dictate eval/dictation/clips/*.wav                       # Whisper + polish, with timings and accuracy
quill-cli dictate --always-polish --polish-models a,b,c clips/*.wav  # compare polish models
```

`utterances.txt` covers a short reply, a Slack update, fillers, a
self-correction, a spoken list, a question (the polish model must not answer
it), an email, code identifiers, numbers, and a clip whose end is cut off
(early key release). The voices are macOS text-to-speech: clean, so good for
latency and for catching hallucinations, not for judging accuracy on real
voices.

## Live captions

```sh
quill-cli dictate --stream eval/dictation/clips/*.wav     # or saved samples: …/dictation-samples/*.flac
```

replays each clip through live dictation (`quill_core::live`): a pass every
`--step-ms` (600) of audio, then the final pass as if the key were released,
next to a single pass over the whole clip. It prints both texts, how much was
locked in before release, the wait after release for each, pass times, and
WER for clips with references.

The TTS clips are single sentences with short pauses, so nothing locks in and
live matches a single pass (9 of 10 identical, same 6.5% WER). Judge it on
real, long dictations. On 54 saved ones (2026-10-17, M3 Pro, turbo):

| Length | Clips | Single pass, median wait | Live, median wait | Locked in by release |
|---|---:|---:|---:|---:|
| Under 15 s | 39 | 0.23 s | 0.23 s | 5% |
| 15–30 s | 9 | 0.65 s | 0.39 s | 38% |
| Over 30 s | 6 | 1.95 s | 0.53 s | 75% |

The longest (126 s) went from 6.5 s to 0.15 s. Of 1,633 words, 68 differed
from a single pass: 57 were loops in the single pass (Whisper repeating a
phrase) that live avoided; the rest were single words.

## Accuracy

A clip with a reference beside it (`clip.txt` next to `clip.wav` or
`clip.flac`: what should have been typed) is scored by word error rate
(substitutions + deletions + insertions over the reference's words), for
Whisper's text and for what would be typed with each polish model, then
totalled over all the clips. Each scored clip lists its mistakes: `quil→quill`
(typed `quil` for `quill`), `-the` (missing), `+um` (extra).

Words are compared lowercased, without outer punctuation or list markers, so
formatting doesn't count but wrong words do ("10.30" for "10:30"). Where
several forms are right ("section 4" or "section four"), a reference uses the
one Quill produces, so every error is a real one.

`utterances.txt` holds the TTS clips' references in its last column.

### Real dictations

1. In the app, turn on **Settings → Dictation → Testing accuracy → Save
   dictations for testing**, then dictate as usual. Each dictation is saved in the data
   folder's `dictation-samples/` as `<time>.flac` (the audio Whisper heard),
   `<time>.typed.txt` (what was typed) and `<time>.json` (the app it went
   into, Whisper's text and the clean-up model).
2. Copy the clips worth keeping, with their files, to
   `eval/dictation/private/` (gitignored: real dictations never go in the
   repo).
3. For each one, copy `<time>.typed.txt` to `<time>.txt` and fix it to what
   you meant to type (listen to the clip if unsure).
4. Score them:

   ```sh
   QUILL_OPENROUTER_API_KEY=$(security find-generic-password -s com.fisherinnovation.quill -a openrouter -w) \
     target/release/quill-cli dictate eval/dictation/private/*.flac
   ```

   Each clip is cleaned up in the style of the app it was dictated into
   (from its `.json`); `--app` overrides that. Pass the app's settings that
   matter: `--vocabulary`, `--about`, `--language`.

Aim for variety: short replies and long paragraphs (30 s+), names and
jargon, code, numbers, a quiet room and a noisy one, the built-in mic and a
headset.

## Results (2026-10-11, MacBook Pro M3 Pro)

**Whisper** (`turbo`, resident, Metal; `short_clip` mode: greedy decoding, the
encoder sized to the clip, VAD trimming outer silence only): median 0.27 s,
max 0.46 s for 3–12 s clips. Same text as the meeting settings, which took
0.6–0.7 s (beam search, full 30 s encoder window).

- Auto-detecting the language on the shortened encoder window produced
  nonsense ("Tex Tex"); detecting it first on the full window fixes that but
  adds ~0.55 s. Dictation therefore defaults to the system language.
- Packing VAD regions tightly (as meetings do) made clips that end on the
  last word decode slowly via temperature retries (2.9 s); keeping pauses and
  adding 0.5 s of trailing silence fixed it.

**Polish** (OpenRouter, `dictation-v1` prompt, reasoning disabled where the
model allows; 3 runs × 9 clips):

| Model | Median | p90 | Max | Notes |
|---|---:|---:|---:|---|
| **google/gemini-3.5-flash-lite** (default) | **0.66 s** | **0.79 s** | 0.97 s | All outputs correct |
| deepseek/deepseek-v4.1-flash | 0.62 s | 1.50 s | 2.21 s | Slow tail |
| z-ai/glm-5.3-flash | 0.91 s | 1.19 s | 2.44 s | 11/27 empty (reasoning ate the budget) |
| anthropic/claude-haiku-5.5 | 1.12 s | 1.30 s | 2.62 s | Correct, but slow for dictation |
| google/gemini-3.8-flash | 1.77 s | 2.28 s | 2.47 s | 16/24 rejected by the output checks |
| mistralai/mistral-small-2603 | 0.53 s | 0.61 s | 0.78 s | 24/27 rate-limited (429) |
| qwen/qwen3.8-flash | 1.27 s | 6.16 s | 6.40 s | |

All models applied the self-correction ("3, no wait, 4 p.m." → "4 p.m."),
formatted the spoken list, and left the question unanswered. On 7 of 9 clips
the polished text equalled Whisper's, which is why `needs_polish` skips the
call unless there's something to fix (fillers, corrections, lists, stutters,
60+ words).

**End to end** (before pasting): ~0.3 s without polish, ~0.95 s with.

**Accuracy** (2026-10-15, TTS clips; Whisper `turbo`, polish
`google/gemini-3.5-flash-lite` when `needs_polish` says so):

| Typed by | WER | Errors (sub / del / ins) |
|---|---:|---:|
| Whisper only | 6.5% | 10 (2 / 0 / 8) |
| With polish when needed | 1.9% | 3 (2 / 0 / 1) |

Polish removed the self-correction ("3, no wait") and the spoken list's
"first… second… and third". All three remaining errors are in `fillers`:
Whisper wrote "stand up" for "standup" and "10.30" for "10:30", and dropped
the "um"s itself, so `needs_polish` didn't send it to the model.
