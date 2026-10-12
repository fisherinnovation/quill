"""Builds a synthetic meeting and its reference RTTM from a script read by
macOS text-to-speech voices (macOS only: needs `say`; and ffmpeg for --codec).

    python3 -I make_tts_meeting.py ten_people.txt out/ten_people [--codec]

Writes <out>.wav (16 kHz mono) and <out>.rttm. Script lines are
"Voice|text"; a line starting with "~" overlaps the end of the previous line,
like an interjection. Reference turns are the voiced part of each line
(leading and trailing silence trimmed). With --codec, the mix goes through
Opus at 24 kbit/s, like call audio.

TTS voices are much more distinct and steady than people, so treat scores on
these files as an optimistic sanity check, not a measure of real calls.
"""

import argparse
import array
import os
import random
import subprocess
import sys
import tempfile
import wave

RATE = 16000
THRESHOLD = 400  # int16 amplitude counted as voiced, for trimming
OVERLAP_MS = 500


def say(voice, text, path):
    subprocess.run(
        ["say", "-v", voice, "--file-format=WAVE", f"--data-format=LEI16@{RATE}", "-o", path, text],
        check=True,
    )
    with wave.open(path, "rb") as w:
        assert w.getframerate() == RATE and w.getnchannels() == 1 and w.getsampwidth() == 2
        samples = array.array("h", w.readframes(w.getnframes()))
    if sys.byteorder == "big":
        samples.byteswap()
    voiced = [i for i, s in enumerate(samples) if abs(s) > THRESHOLD]
    if not voiced:
        return array.array("h")
    return samples[voiced[0] : voiced[-1] + 1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("script")
    parser.add_argument("out", help="output path without extension")
    parser.add_argument("--codec", action="store_true", help="pass through Opus 24 kbit/s")
    parser.add_argument("--seed", type=int, default=7)
    args = parser.parse_args()
    rng = random.Random(args.seed)

    lines = []
    with open(args.script, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            overlap = line.startswith("~")
            voice, text = line.lstrip("~").split("|", 1)
            lines.append((voice, text, overlap))

    mix = array.array("h")
    turns = []
    cursor = 0  # sample where the previous line ended
    with tempfile.TemporaryDirectory() as tmp:
        for i, (voice, text, overlap) in enumerate(lines):
            clip = say(voice, text, os.path.join(tmp, f"{i}.wav"))
            if overlap:
                start = max(0, cursor - OVERLAP_MS * RATE // 1000)
            else:
                start = cursor + rng.randint(250, 900) * RATE // 1000
            end = start + len(clip)
            if len(mix) < end:
                mix.extend([0] * (end - len(mix)))
            for j, s in enumerate(clip):
                mix[start + j] = max(-32768, min(32767, mix[start + j] + s))
            turns.append((start, end, voice))
            cursor = max(cursor, end)
    mix.extend([0] * RATE)  # a second of trailing silence

    wav_path = args.out + ".wav"
    os.makedirs(os.path.dirname(os.path.abspath(wav_path)), exist_ok=True)
    out = mix
    if sys.byteorder == "big":
        out = array.array("h", mix)
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
        for start, end, voice in turns:
            name = voice.split(" ")[0]
            f.write(f"SPEAKER {file_id} 1 {start / RATE:.3f} {(end - start) / RATE:.3f} <NA> <NA> {name} <NA> <NA>\n")
    speakers = len({v for _, _, v in turns})
    print(f"wrote {wav_path} ({len(mix) / RATE:.0f} s, {speakers} speakers, {len(turns)} turns)")


if __name__ == "__main__":
    main()
