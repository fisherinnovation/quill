#!/bin/sh
# Builds clips/<id>.wav from utterances.txt: macOS text-to-speech with 0.4 s
# of silence before and 0.8 s after (like pressing the key, speaking, then
# releasing), or with the end cut off when `cut` > 0. 16 kHz mono 16-bit.
# Synthetic speech: good for latency and for catching hallucinations, not
# for judging accuracy on real voices. macOS only (needs `say`).
set -eu
cd "$(dirname "$0")"
mkdir -p clips
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
grep -v '^#' utterances.txt | while IFS='|' read -r id voice cut text; do
    [ -n "$id" ] || continue
    say -v "$voice" --file-format=WAVE --data-format=LEI16@16000 -o "$tmp/$id.wav" "$text"
    python3 -I - "$tmp/$id.wav" "clips/$id.wav" "$cut" <<'PY'
import sys, wave
src, dst, cut = sys.argv[1], sys.argv[2], float(sys.argv[3])
with wave.open(src) as w:
    params, data = w.getparams(), w.readframes(w.getnframes())
rate, width = params.framerate, params.sampwidth
silence = lambda secs: b"\0" * (int(rate * secs) * width)
if cut > 0:
    data = data[: len(data) - int(rate * cut) * width] + silence(0.05)
else:
    data += silence(0.8)
with wave.open(dst, "wb") as out:
    out.setparams(params)
    out.writeframes(silence(0.4) + data)
PY
done
echo "wrote $(ls clips | wc -l | tr -d ' ') clips to $(pwd)/clips"
