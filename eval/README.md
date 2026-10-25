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
- `search/` holds the search-by-meaning check: `queries.json` (labelled
  queries, with the stretch of which meeting should answer each) over
  `product-sync.json` and four more synthetic transcripts (`launch-planning`
  from `diarization/ten_people.txt`; a support review, a one-on-one and a
  Spanish marketing meeting written for it). Run
  `quill-cli search --eval eval/search/queries.json` (`--scores` shows the
  closest chunks per query, `--scale-hours 100` times a bigger library).
