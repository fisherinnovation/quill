"""How much of the far end's speech leaked into "Me" lines.

Usage: python3 eval/bench/echo_leak.py transcript.json [more.json ...]

A "Me" word counts as leaked when it is part of a 3-word sequence that also
appears in an "Others" segment within a few seconds of it. Three-word
sequences rather than single words, so common words ("the", "yeah") shared
by both sides don't count. Lower is better; 0% means no echo in Me lines.
"""
import json
import re
import sys

WINDOW_MS = 4000


def words(text):
    return re.sub(r"[^a-z0-9' ]", " ", text.lower().replace("-", " ")).split()


def trigrams(ws):
    return {tuple(ws[i:i + 3]) for i in range(len(ws) - 2)}


for path in sys.argv[1:]:
    segs = json.load(open(path))
    others = [s for s in segs if s.get("speaker") == "Others"]
    total = leaked = 0
    examples = []
    for s in segs:
        if s.get("speaker") != "Me":
            continue
        ws = words(s["text"])
        near = set()
        for o in others:
            if o["start_ms"] <= s["end_ms"] + WINDOW_MS and s["start_ms"] <= o["end_ms"] + WINDOW_MS:
                near |= trigrams(words(o["text"]))
        hit = [False] * len(ws)
        for i in range(len(ws) - 2):
            if tuple(ws[i:i + 3]) in near:
                hit[i] = hit[i + 1] = hit[i + 2] = True
        total += len(ws)
        leaked += sum(hit)
        if any(hit):
            examples.append(" ".join(w.upper() if h else w for w, h in zip(ws, hit)))
    pct = 100 * leaked / total if total else 0.0
    print(f"{path}: {leaked}/{total} Me words leaked ({pct:.1f}%)")
    for e in examples[:6]:
        print("   ", e[:150])
