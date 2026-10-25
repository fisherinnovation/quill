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
2. Review them in the app: **Settings → Dictation → Testing accuracy →
   Review…** plays each one and shows what was typed; fix it into what you
   said (or keep it if it's right) and it's saved as `<time>.txt`, the
   reference, beside the clip. The page shows the accuracy of what was
   typed, and of Whisper alone, as you go, and the commonest mistakes.
   (By hand: copy `<time>.typed.txt` to `<time>.txt` and fix it.)
3. To keep clips for later runs, copy them with their files to
   `eval/dictation/private/` (gitignored: real dictations never go in the
   repo); `quill-cli dictate` can also read them where they are.
4. Score them again with the current settings, models or prompt:

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
0.6–0.7 s (beam search, full 30 s encoder window). The margin is now 3 s on
Metal, and Vulkan and the CPU use the full window; see the 2026-10-20 results
below.

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

## Results (2026-10-20, Windows 11, RTX 4070 SUPER, Vulkan)

**Short clips were typed twice.** With the encoder sized to the clip, half of
one user's 2–4 s dictations came out as the sentence twice ("Can you send it
over? Can you send it over?"); none shorter or longer did. Reproduced with 40
Windows text-to-speech clips (SAPI voices David and Zira, 0.4 s of silence
before and 0.8 s after, as `make_clips.sh` does; 2.5–8.5 s), `--no-polish -l en`:

| Encoder window | Doubled | WER | Whisper median |
|---|---:|---:|---:|
| Sized to the clip (+64 frames) | 10 / 40 | 28.0% | 155 ms |
| At least 400 frames (8 s) | 3 / 40 | 9.1% | 174 ms |
| At least 750 frames (15 s) | 1 / 40 | 4.7% | 202 ms |
| At least 1000 frames (20 s) | 0 / 40 | 2.6% | 202 ms |
| **Full 30 s (now)** | **0 / 40** | **2.6%** | 208 ms |

The repeat happens inside one decode (`single_segment` still doubled 4 of 8),
on the CPU too (8 / 40, 815 ms median) and on Vulkan without flash attention
(8 / 40). Timestamps were off as well: 0–15 s for a 4 s clip.

With the full window, `--stream` (live captions) matches a single pass on all
40 clips (before: 33), WER 2.6% (before: 10.8%); the wait after release went
from a median of 159 ms (max 914) to 215 ms (max 284), and passes while
speaking from 150 ms (max 546) to 202 ms (max 407). `-l auto` gives the same
WER at ~50 ms more per clip.

## Results (2026-10-20, MacBook Pro M3 Pro, Metal)

**The full window cost Metal 0.5 s.** Re-measured on the Mac, the fix above
stopped the doubling there too, but the encoder's 30 s window takes ~0.6 s
on an M3 Pro however short the clip, so a dictation under 4 s took 0.61 s
in Whisper instead of 0.13 s. On Metal the encoder is now sized to the clip
plus 3 s again, and the segments it then repeats are dropped (see "Short
clips on Metal" in `docs/DESIGN_NOTES.md`); Vulkan and the CPU keep the full
window.

`quill-cli dictate --no-polish` (turbo, system language), at 79529d8 (before
the fix), at 98d64f3 (full window) and with this change. Clips: the 10 eval
clips; 230 short TTS clips (45 sentences under 4 s, 10 macOS voices, padded
as `make_clips.sh` does; not in the repo); and 104 of one user's saved
dictations, which have no reviewed references, so they're compared with the
full window's text instead.

| | Before | Full window | **Now** |
|---|---:|---:|---:|
| Doubled, 230 short TTS clips | 38 | 0 | **0** |
| Doubled, 104 saved dictations | 11 (+2 repeating an earlier sentence) | 0 | **0** |
| WER, 230 short TTS clips | 26.6% | 3.0% | **4.2%** |
| WER, 10 eval clips | 6.5% | 9.1% | **7.8%** |
| Saved dictations, words differing from the full window | 5.8% | – | **0.5%** |
| Whisper median, short TTS clips | 130 ms | 615 ms | **158 ms** |
| Whisper median, saved under 4 s (28) | 129 ms | 608 ms | **151 ms** |
| Whisper median, saved 4–15 s (40) | 300 ms | 697 ms | **326 ms** |
| Whisper median, saved over 15 s (36) | 1.03 s | 1.05 s | **1.06 s** |

Release to typed text is Whisper plus ~82 ms (the app's own log: `latency_ms`
minus `stt_ms`, median of 161 dictations without clean-up), so about 0.23 s
for a dictation under 4 s now, against 0.69 s with the full window.

- **Margin.** Sizing the encoder to the clip plus 1.3 s, as before, also
  doubled nothing once repeats are dropped, but misheard more short words
  ("Sheep it", "The bilge is green again"): TTS WER 8.0%. Plus 2 s let a
  doubling through; plus 5 s let a repeated sentence through in the saved
  dictations; floors of 500, 750 and 1,000 frames took 0.22, 0.30 and 0.40 s.
  Plus 3 s had neither, for ~25 ms more than 1.3 s.
- **What doesn't work.** `no_timestamps` made Whisper loop (36 of 80 short
  clips repeated, 3.4 s median).
- **The repeats** are always a separate segment starting where the one
  before ends: an exact copy of it (one to three copies), a copy of its last
  sentence, or an earlier sentence spelled slightly differently. In 334
  clips, the segments Whisper actually heard (after the first) came at most
  4.3 words a second; the copies, 86 and up.

**Live captions** (`--stream`): on the 80 short TTS clips nothing locks in, so
live matches a single pass (before: 13 doubled, now 0), with a median wait
after release of 148 ms (full window: 608 ms) and passes while speaking of
120 ms (full window: 602 ms, slower than the 600 ms step). On the 104 saved
dictations: 0 doubled (before: 8), wait after release 308 ms median (full
window: 686 ms; over 15 s, 604 ms against 831 ms), passes while speaking 365
ms median (full window: 709 ms). On the 36 over 15 s, live text differs
from the full window's single pass in 0.7% of words (before: 2.3%; the full
window live: 1.0%). Stitching passes together never repeated text in any of
the three builds. The two live-only extras came from inside a final pass:
before, one 42 s
dictation's last pass (19 s, encoder sized to it) repeated a phrase; with
the 3 s margin, a 51 s one's last pass added a sentence of 7 words in 0.5 s
at its end. Dropping segments faster than 8 words a second removed it; both
now match a single pass (but for one word, as in every build).

**The long doubled dictation** (23.7 s, typed 2026-10-17 01:15 UTC) wasn't
live captions: they shipped the next evening, and its log line has no
`live_passes`. It was a single pass, and the encoder was sized to anything
under ~28.7 s, so it was the same bug: segment 1 (38 words, 0–22.4 s) and
then the same 38 words at 22.4–22.8 s. The 79529d8 build reproduces it; the
full window and this change both type it once.

## Parakeet (2026-10-25, Windows 11, i9-14900KF, RTX 4070 SUPER)

Windows has no `say`, so `make_clips.ps1` builds the same clips with its
text-to-speech voices (David and Zira), as `clips/<id>-<voice>.wav`:

```sh
powershell -ExecutionPolicy Bypass -File eval/dictation/make_clips.ps1
quill-cli models download parakeet-v2
quill-cli dictate --no-polish -l en --stt-model parakeet-v2 eval/dictation/clips/*.wav
quill-cli dictate --no-polish -l en --stt-model turbo --cpu eval/dictation/clips/*.wav   # Whisper without the GPU
```

`dictate` now ends with a speed table by clip length. The results, and why
Whisper stays the default, are in "Parakeet" in `docs/DESIGN_NOTES.md`: on
this PC Parakeet v2 took 0.18 s for a dictation under 4 s (Whisper turbo
on Vulkan 0.21 s, on the CPU 7.1 s) and 0.39 s for 4–15 s (Whisper 0.29 s),
with WER 10.4% on these 20 clips against Whisper's 9.4% (7.1% and 7.8%
with `--vocabulary fetchUser,loadUser,standup`).
