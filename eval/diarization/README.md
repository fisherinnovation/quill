# Diarization evaluation

Tools for measuring how well `quill-cli diarize` splits one audio track into
speakers. This matters for the call side of a meeting, where everyone else
is mixed into one "Others" track.

The test audio is not committed; the scripts rebuild it into `data/`
(gitignored).

| Script | Builds |
|---|---|
| `make_tts_meeting.py ten_people.txt data/ten_people_call --codec` | A 5-minute, 10-person call read by macOS voices, through Opus 24 kbit/s. Very uneven participation; two short overlapping interjections. |
| `fetch_ami.sh` | Three AMI test meetings (ES2004a, IS1009a, TS3003a): real, spontaneous 4-person meetings, headset mix. CC BY 4.0. References from pyannote's AMI setup. |
| `make_mix.py data/ami_mix10 data/ami_ES2004a data/ami_IS1009a data/ami_TS3003a --speakers 10 --codec` | A 10-person "call" stitched from real clean turns of 10 of the 12 AMI speakers, through Opus. Three people say only 4–5 s in total. |

Score one file with `quill-cli diarize <wav> --reference <rttm>`. Add
`--sweep` to run the models once and print the score at a range of
clustering thresholds, and with the true speaker count. `sweep.py` compares
voice models.

## How it works

1. sherpa-onnx runs pyannote segmentation-3.0 over 10 s windows (stepping
   2.5 s) and produces speaker turns, using its own clustering.
2. Quill splits those turns where they overlap someone else and into pieces
   of at most 6 s. It computes a voice embedding for each clean piece of at
   least 1 s.
3. Quill clusters the pieces by centroid distance, weighted by duration.
   Groups with under 4 s of speech are treated as outliers and folded into
   the nearest speaker, unless the caller gave a speaker count that needs
   them. Pieces too short to embed take their turn's label.

Step 3 replaces sherpa-onnx's clustering (complete linkage, no outlier
handling). With sherpa's own clustering and a speaker count of 10, the
10-person TTS call came out as 7 speakers at 33% DER. Stray embeddings took
clusters that real people needed.

## Results (October 2026, M3 Pro, CPU)

DER is missed + false alarm + confusion, as a share of reference speech, with
no collar. Missed and false-alarm speech come from segmentation, so they're
the same whichever voice model or clustering is used. **Confusion** (speech
given to the wrong person) is what clustering controls.

Default settings: TitaNet-small embeddings, threshold 0.45, window step 0.25.

| File | Speakers | Found | DER | Confusion | With true count: found / DER / confusion |
|---|---|---|---|---|---|
| AMI ES2004a (17 min) | 4 | 5 | 22.4% | 4.6% | 4 / 21.7% / 3.9% |
| AMI IS1009a (14 min) | 4 | 5 | 22.5% | 5.7% | 4 / 26.4% / 9.6% |
| AMI TS3003a (25 min) | 4 | 5 | 18.4% | 3.7% | 4 / 18.5% / 3.8% |
| AMI 10-person mix (7 min) | 10 | 7 | 25.3% | 8.6% | 10 / 22.5% / 5.9% |
| TTS 10-person call (5 min) | 10 | 8 | 24.7% | 20.8% | 10 / 6.3% / 2.4% |

For reference, pyannote 3.1 reports 18.8% DER on the AMI test set (headset
mix, no collar).

Speed: about 35× real time (17 min of audio in 30 s), so an hour-long call
takes under two minutes.

### What we learned

- **The speaker count is the most valuable input.** Without it, the
  threshold trades merging people against splitting them, and the best
  value differs between real voices (0.45–0.5) and synthetic ones (0.3).
  With it, every file came out with the right number of speakers.
- **People who barely talk are the weak spot.** In the 10-person mix,
  without a count, the three people with about 5 s of speech each were
  merged into others. With the count, they were found, but only 20–70% of
  their speech was attributed to them.
- **Voice models:**
  - TitaNet-small and ERes2Net (VoxCeleb) are about equally accurate;
    TitaNet-small is twice as fast.
  - CAM++ was unusable on the TTS call (62% DER).
  - WeSpeaker ResNet34 needs much lower thresholds than the others and was
    slow.
- **Segmentation step:** a 0.25 step (2.5 s) was as accurate as pyannote's
  default of 0.1, at under half the time. A 0.5 step lost about 2 points.
- **Opus at 24 kbit/s** made no visible difference to the TTS results.

## Known voices (recognising people across meetings)

`quill-cli voices eval` measures how well Quill recognises people it has
heard before (`quill_diarize::voices`, `quill_core::voices`). Each file needs
its RTTM beside it, with speaker names that identify people across files, as
AMI's participant ids do.

```sh
# The 16 AMI test meetings (4 series of the same 4 people) and 12 dev
# meetings (3 more series), 28 people in all
./fetch_ami.sh "ES2004a ES2004b ES2004c ES2004d IS1009a IS1009b IS1009c IS1009d TS3003a TS3003b TS3003c TS3003d EN2002a EN2002b EN2002c EN2002d"
AMI_SET=dev ./fetch_ami.sh "ES2011a ES2011b ES2011c ES2011d IS1008a IS1008b IS1008c IS1008d TS3004a TS3004b TS3004c TS3004d"
quill-cli voices eval data/ami_*.wav --cache data/cache
# Learn from the originals, recognise in copies through Opus 24 kbit/s
# (ffmpeg, as make_mix.py --codec does; name them ami_X_opus.wav + .rttm)
quill-cli voices eval data/ami_*.wav --test data/opus/*.wav --cache data/cache
```

Each recording is diarized as the app does (no speaker count), and each
speaker is taken to be the reference person who talks most in their turns.
Voices are learned as if the user had named everyone with enough speech
(speakers who are the same person merged, as typing the same name does), then
matched in another recording. Four scenarios:

- **one meeting, same people**: learned from another session of the same group;
- **one meeting, strangers**: learned from a meeting with none of these people;
- **everyone else**: learned from every other meeting (28 people known);
- **strangers only**: learned from every meeting without these people.

A wrong name is either a known person named as someone else, or a stranger
given a known name. `quill-cli voices match <a> <b>` shows the similarity of
every pair of speakers in two recordings (folders or audio files; nothing is
written).

### Results (October 2026, TitaNet-small)

Similarity of a speaker's voice to the voice learned for them from other
meetings: median 0.94 (5th percentile 0.44: speakers diarization had merged
with someone else). To the closest other person: median 0.46, 95th
percentile 0.72.

At least 30 s of speech, on the clean recordings (28 meetings):

| Threshold | Same people: named right / wrong | Strangers named (one meeting) | Everyone else: right / wrong | Strangers named (strangers only) | Wrong names of all given (on one-person speakers) |
|---|---|---|---|---|---|
| 0.50 | 93.1% / 1.8% | 4.0% of 2424 | 95.0% / 3.0% | 23.8% of 101 | 27.8% (34) |
| 0.60 | 92.8% / 1.8% | 0.2% | 95.0% / 2.0% | 1.0% | 5.4% (0) |
| 0.65 | 92.8% / 1.8% | 0.0% | 95.0% / 2.0% | 0.0% | 4.1% (0) |
| **0.70** | **92.1% / 1.8%** | **0.0%** | **95.0% / 2.0%** | **0.0%** | **3.8% (0)** |
| 0.75 | 91.7% / 1.8% | 0.0% | 95.0% / 2.0% | 0.0% | 3.8% (0) |
| 0.80 | 83.8% / 1.1% | 0.0% | 95.0% / 2.0% | 0.0% | 3.0% (0) |

Learned from the originals, recognised in the 16 test meetings through Opus
24 kbit/s, at 0.70: 89.0% / 1.8% (same people), 0.0% of 1416 strangers
named, 91.5% / 3.4% (everyone else), 0.0% (strangers only); 3.9% of names
wrong, none on one-person speakers.

With a 15 s minimum, recall is lower among the speakers that count (85%)
and wrong names rise (5.5% at 0.75): small, mixed clusters (overlap,
crosstalk) matched each other across meetings, and a voice learned from one
named small clusters everywhere.

### What we learned

- **From 0.6 up, every wrong name went to a speaker diarization had merged
  from two people** (ES2004d and ES2011a each merged two women). Their voice
  is half someone else's, so no threshold separates them (similarities
  0.87–0.94). Such a speaker is mislabelled as "Speaker 2" too; the fix is
  better diarization or the user setting **People on the call**.
- **Recall is flat from 0.45 to 0.75** while strangers named fall from 7% to
  0%, so a high threshold costs almost nothing. Above 0.75 it drops fast.
- **The codec costs about 3 points of recall** and no extra false names.
- **Caveat:** AMI's sessions of one group were recorded in the same room
  with the same headsets, so real calls (different microphones, days and
  connections) will match less often. That's the safe direction.
