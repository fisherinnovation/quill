#!/bin/sh
# Builds eval/bench/meeting.wav: a ~4-minute synthetic four-person meeting read
# by macOS text-to-speech voices from meeting_script.txt ("Voice|line" per
# line). Synthetic speech is cleaner than a real call, so use it for speed
# (real-time factor) comparisons, not accuracy. macOS only (needs `say`).
set -eu
cd "$(dirname "$0")"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

i=0
while IFS='|' read -r voice text; do
    [ -n "$voice" ] || continue
    i=$((i + 1))
    # [[slnc 400]] adds a 400 ms pause after each speaker turn.
    say -v "$voice" --file-format=WAVE --data-format=LEI16@16000 \
        -o "$tmp/$(printf %03d "$i").wav" "$text [[slnc 400]]"
done < meeting_script.txt

# Concatenate the per-line WAVs (all 16 kHz mono 16-bit) into one file.
python3 -I - "$tmp" meeting.wav <<'PY'
import glob, sys, wave
parts = sorted(glob.glob(f"{sys.argv[1]}/*.wav"))
with wave.open(sys.argv[2], "wb") as out:
    for i, path in enumerate(parts):
        with wave.open(path, "rb") as part:
            if i == 0:
                out.setparams(part.getparams())
            out.writeframes(part.readframes(part.getnframes()))
PY
echo "wrote $(pwd)/meeting.wav"
