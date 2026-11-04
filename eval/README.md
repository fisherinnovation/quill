# eval/

- `transcripts/` holds sample transcripts for the summary evaluation harness
  (`quill-cli eval`). Put real meetings in `transcripts/private/`, which is
  gitignored; `eval` reads both.
  - `product-sync.json`: a synthetic 4-minute, 4-person product meeting
    (script in `bench/meeting_script.txt`, read by macOS voices, recorded as
    two tracks with speaker echo, transcribed by turbo). Not a real meeting.
  - `all-hands-names.json`: a written 2-minute all-hands with numbered
    speakers, for checking which people the notes name (`quill-cli
    summarize` prints "Suggests Dana for Speaker 2"). Dana, Priya and Luis
    should be named (a handoff by name, a self-introduction, a call by
    name); Speakers 1 and 5 shouldn't, though Sam, Alex and Jordan are
    thanked or quoted, Jordan just before Speaker 5 speaks.
  - `one-on-one-names.json`: a written 1:1 ("Me" and "Others") where the
    other person, Priya, is named once in the small talk, the user is the
    new hire, and Dan is only mentioned. The notes should name Priya, never
    write "Me" as a name, and keep "new hire" out of the title.
  - `one-on-one-fidelity.json`: a written 1:1 full of traps from the notes
    fidelity feedback. The notes should give the migration as the main
    cause, keep the export-job and three-answers examples, keep the release
    train, shared pager and slow reviews in the past and Friday deploys
    stopped, credit "staging" to the user (it's talked over Jordan's
    sentence about documentation), add no time frames, quote Jordan's
    closing mood, and leave out the hospital in the small talk.
  - `team-names.json`: a written 1:1 for names, acronyms and years, with
    `team-names.vocabulary.txt`. The notes should write Priya Raman,
    Marguerite Feeney (said as "Marguerite" and "feeny"), Dan Okafor (said
    as "Okafor"), RIF, 2027 and 2025, and list the "Dan" drafting the
    business case under "Unresolved names" (Dan Ruiz or Dan Okafor).
  - A case's vocabulary, as **Settings → Dictation → Words** would give it,
    goes in `<case>.vocabulary.txt` next to it, one term per line.
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
