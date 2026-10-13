# eval/

- `transcripts/` holds sample transcripts for the summary evaluation harness
  (`quill-cli eval`). Put real meetings in `transcripts/private/`, which is
  gitignored; `eval` reads both.
  - `product-sync.json`: a synthetic 4-minute, 4-person product meeting
    (script in `bench/meeting_script.txt`, read by macOS voices, recorded as
    two tracks with speaker echo, transcribed by turbo). Not a real meeting.
- `results/` holds harness outputs: `eval.sqlite` (every run), and per run a
  folder with `leaderboard.md`, `review.md` (summaries side by side with
  scores) and `results.csv`. Everything in it is gitignored.
- `benchmarks.md` records Whisper real-time factors per model and machine.
- `dictation/` and `diarization/` hold the dictation latency checks and the
  speaker diarization evaluation, each with its own README.
