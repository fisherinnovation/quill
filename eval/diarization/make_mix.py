"""Builds a many-speaker "call" from real voices: clean single-speaker turns
taken from several recordings (e.g. AMI meetings, via fetch_ami.sh) are
stitched into one conversation with uneven participation, then optionally
passed through Opus like call audio.

    python3 -I make_mix.py out/mix10 data/ami_ES2004a data/ami_IS1009a \
        data/ami_TS3003a --speakers 10 --minutes 10 [--codec]

Each source is <path>.wav (16 kHz mono 16-bit) plus <path>.rttm. Writes
<out>.wav and <out>.rttm. Speakers from different recordings also differ in
room and microphone, which makes them a little easier to tell apart than
people on one call; same-recording speakers don't have that help.
"""

import argparse
import array
import os
import random
import subprocess
import sys
import wave

RATE = 16000
MIN_TURN_MS, MAX_TURN_MS = 1000, 15000


def read_wav(path):
    with wave.open(path, "rb") as w:
        assert w.getframerate() == RATE and w.getnchannels() == 1 and w.getsampwidth() == 2, path
        samples = array.array("h", w.readframes(w.getnframes()))
    if sys.byteorder == "big":
        samples.byteswap()
    return samples


def clean_turns(rttm_path):
    """(start_ms, end_ms, speaker) turns that no other speaker overlaps."""
    turns = []
    with open(rttm_path, encoding="utf-8") as f:
        for line in f:
            p = line.split()
            if p and p[0] == "SPEAKER":
                start = round(float(p[3]) * 1000)
                turns.append((start, start + round(float(p[4]) * 1000), p[7]))
    clean = []
    for i, (s, e, spk) in enumerate(turns):
        if not MIN_TURN_MS <= e - s <= MAX_TURN_MS:
            continue
        if any(j != i and o_s < e and o_e > s and o_spk != spk for j, (o_s, o_e, o_spk) in enumerate(turns)):
            continue
        clean.append((s, e, spk))
    return clean


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("out")
    parser.add_argument("sources", nargs="+", help="paths without extension")
    parser.add_argument("--speakers", type=int, default=10)
    parser.add_argument("--minutes", type=float, default=10)
    parser.add_argument("--codec", action="store_true")
    parser.add_argument("--seed", type=int, default=3)
    args = parser.parse_args()
    rng = random.Random(args.seed)

    pool = {}  # speaker -> [(source index, start_ms, end_ms)]
    audio = []
    for i, src in enumerate(args.sources):
        audio.append(read_wav(src + ".wav"))
        for s, e, spk in clean_turns(src + ".rttm"):
            pool.setdefault(f"{os.path.basename(src)}:{spk}", []).append((i, s, e))
    # Speakers with the most material first, so the chatty roles are filled.
    speakers = sorted(pool, key=lambda k: -sum(e - s for _, s, e in pool[k]))[: args.speakers]
    if len(speakers) < args.speakers:
        sys.exit(f"only {len(speakers)} speakers with clean turns")
    rng.shuffle(speakers)
    # Uneven participation: a facilitator, a few regulars, some who barely talk.
    weights = [30, 15, 12, 10, 8, 8, 6, 5, 4, 2][: len(speakers)] + [3] * max(0, len(speakers) - 10)
    for spk in speakers:
        rng.shuffle(pool[spk])

    mix = array.array("h")
    turns = []
    target = int(args.minutes * 60 * RATE)
    last = None
    while len(mix) < target:
        spk = rng.choices(speakers, weights=weights)[0]
        if spk == last or not pool[spk]:
            if all(not pool[s] for s in speakers):
                break
            continue
        src, s, e = pool[spk].pop()
        clip = audio[src][s * RATE // 1000 : e * RATE // 1000]
        mix.extend([0] * (rng.randint(200, 800) * RATE // 1000))
        start = len(mix)
        mix.extend(clip)
        turns.append((start, len(mix), spk))
        last = spk
    mix.extend([0] * RATE)

    wav_path = args.out + ".wav"
    os.makedirs(os.path.dirname(os.path.abspath(wav_path)), exist_ok=True)
    out = array.array("h", mix)
    if sys.byteorder == "big":
        out.byteswap()
    with wave.open(wav_path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(out.tobytes())
    if args.codec:
        opus = args.out + ".opus"
        ffmpeg = ["ffmpeg", "-hide_banner", "-loglevel", "error", "-y"]
        subprocess.run(ffmpeg + ["-i", wav_path, "-c:a", "libopus", "-b:a", "24k", opus], check=True)
        subprocess.run(ffmpeg + ["-i", opus, "-ar", str(RATE), "-ac", "1", wav_path], check=True)
        os.remove(opus)

    file_id = os.path.basename(args.out)
    with open(args.out + ".rttm", "w", encoding="utf-8") as f:
        for s, e, spk in turns:
            f.write(f"SPEAKER {file_id} 1 {s / RATE:.3f} {(e - s) / RATE:.3f} <NA> <NA> {spk} <NA> <NA>\n")
    talk = {spk: sum(e - s for s, e, k in turns if k == spk) / RATE for spk in speakers}
    print(f"wrote {wav_path} ({len(mix) / RATE:.0f} s, {len(turns)} turns)")
    for spk in sorted(talk, key=talk.get, reverse=True):
        print(f"  {spk:<22} {talk[spk]:6.1f} s")


if __name__ == "__main__":
    main()
