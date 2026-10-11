# Whisper benchmarks

Speed and accuracy of each Whisper model on our hardware, to choose defaults
for meeting transcription (accuracy first) and dictation (latency first).

## How to reproduce

```sh
eval/bench/make_bench_audio.sh                     # macOS: builds eval/bench/meeting.wav
quill-cli bench eval/bench/meeting.wav -l en --with-cpu
quill-cli transcribe eval/bench/meeting.wav -m turbo -l en -f json > out.json
python3 eval/bench/wer.py eval/bench/meeting_script.txt out.json
```

`bench` warms each model up on 3 s of audio before timing, so one-off GPU
setup isn't counted. "× real time" is audio length ÷ transcription time
(higher is faster). Load times assume the macOS Metal shader cache is warm;
the very first load after a fresh build takes ~6 s extra while Metal compiles
shaders.

**Caveat:** the clip is synthetic (macOS `say` voices, four speakers, no
crosstalk or background noise), so WER here is far lower than on real calls
and doesn't separate the models well. Treat it as a sanity check; rerun on
real meeting recordings once M2 can capture them.

## MacBook Pro M3 Pro, 36 GB, macOS 26.6 — 2026-10-11

Clip: `meeting.wav`, 231.5 s. whisper-rs 0.16 (whisper.cpp, Metal + flash
attention), 8 CPU threads, beam search (width 5), language `en`.

| Model | Size | Metal × RT | CPU × RT | Metal speed-up | WER |
|---|---:|---:|---:|---:|---:|
| tiny | 74 MB | 105.7 | 64.5 | 1.6× | 5.0% |
| base | 141 MB | 77.3 | 41.2 | 1.9× | 2.6% |
| small | 465 MB | 36.2 | 15.4 | 2.4× | 2.1% |
| medium | 1.4 GB | 14.5 | 5.5 | 2.6× | 1.9% |
| turbo-q5 | 547 MB | 22.8 | 6.2 | 3.7× | 2.7% |
| turbo | 1.5 GB | 23.7 | 6.2 | 3.8× | 2.7% |
| large-v3 | 2.9 GB | 8.5 | 3.3 | 2.6× | 3.7% |

In practice: an hour-long meeting takes about **2.5 minutes** with turbo on
Metal, 7 minutes with large-v3, and under a minute with base.

### Findings

- **Greedy decoding broke large-v3.** With greedy sampling, large-v3 fell into
  a repetition loop 90 s in ("I think that's a good thing." × 110 segments)
  and lost 60% of the meeting (WER 106%). Beam search (width 5) fixed it
  (WER 3.7%) at a ~10% speed cost for the other models, so beam search is the
  default. VAD (M2) should further reduce this risk; dictation (M6) may still
  use greedy for short clips.
- **turbo is the meeting default.** It's within noise of the best WER and ~3×
  faster than large-v3. Revisit on real recordings.
- **turbo-q5 is as fast as turbo on Metal, with ⅓ the download.** Quantization
  saves disk and memory, not time, on this GPU. That makes it a good default
  for download size and memory (dictation keeps the model resident).
- **Metal helps the big models most** (3.8× for turbo) and the tiny ones least,
  where fixed overheads dominate.
- **Dictation latency looks achievable.** base/small transcribe at 36–77×
  real time, i.e. ~0.15–0.3 s for a 10 s utterance before the LLM polish step.

### Raw `quill-cli bench` output (beam search)

| Model | Backend | Load (s) | Transcribe (s) | × real time |
|---|---|---:|---:|---:|
| tiny | Metal | 0.08 | 2.19 | 105.7 |
| tiny | CPU | 0.03 | 3.59 | 64.5 |
| base | Metal | 0.06 | 2.99 | 77.3 |
| base | CPU | 0.05 | 5.62 | 41.2 |
| small | Metal | 0.15 | 6.39 | 36.2 |
| small | CPU | 0.12 | 15.07 | 15.4 |
| medium | Metal | 0.44 | 15.97 | 14.5 |
| medium | CPU | 0.37 | 41.73 | 5.5 |
| turbo-q5 | Metal | 0.17 | 10.16 | 22.8 |
| turbo-q5 | CPU | 0.14 | 37.61 | 6.2 |
| turbo | Metal | 0.39 | 9.78 | 23.7 |
| turbo | CPU | 0.38 | 37.48 | 6.2 |
| large-v3 | Metal | 0.85 | 27.13 | 8.5 |
| large-v3 | CPU | 0.71 | 69.17 | 3.3 |

For comparison, greedy decoding on Metal ran 26.0× (turbo), 45.8× (small)
and 142.6× (tiny).

## Windows

Not yet measured. Copy `meeting.wav` over from a Mac (the generator needs
`say`), then run the same commands on the Windows machine (CPU, and with
`--features cuda` or `vulkan` if it has a suitable GPU) and add a section.
