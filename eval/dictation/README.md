# eval/dictation/

Latency and behaviour checks for dictation (M6).

```sh
sh eval/dictation/make_clips.sh            # TTS clips from utterances.txt → clips/ (gitignored)
quill-cli dictate eval/dictation/clips/*.wav                       # Whisper + polish, with timings
quill-cli dictate --always-polish --polish-models a,b,c clips/*.wav  # compare polish models
```

`utterances.txt` covers a short reply, a Slack update, fillers, a
self-correction, a spoken list, a question (the polish model must not answer
it), an email, code identifiers, numbers, and a clip whose end is cut off
(early key release). The voices are macOS text-to-speech: clean, so good for
latency and for catching hallucinations, not for judging accuracy on real
voices.

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
