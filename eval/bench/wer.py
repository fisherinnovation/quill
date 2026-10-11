"""Word error rate of `quill-cli transcribe -f json` output against a script.

Usage: python3 eval/bench/wer.py eval/bench/meeting_script.txt out1.json [out2.json ...]

The script has one "Voice|line" per line (see make_bench_audio.sh). Text is
lowercased and stripped of punctuation before comparing, so "OK" vs "Okay"
still counts as an error but "Friday," vs "Friday" does not.
"""
import json
import re
import sys


def words(text):
    return re.sub(r"[^a-z0-9' ]", " ", text.lower().replace("-", " ")).split()


def edit_distance(ref, hyp):
    row = list(range(len(hyp) + 1))
    for i, r in enumerate(ref, 1):
        diag, row[0] = row[0], i
        for j, h in enumerate(hyp, 1):
            diag, row[j] = row[j], min(row[j] + 1, row[j - 1] + 1, diag + (r != h))
    return row[-1]


ref = words(" ".join(l.split("|", 1)[1] for l in open(sys.argv[1]) if "|" in l))
for path in sys.argv[2:]:
    hyp = words(" ".join(s["text"] for s in json.load(open(path))))
    print(f"{path}: WER {100 * edit_distance(ref, hyp) / len(ref):.1f}%")
