"""Scores who a labelled transcript says spoke each line, against what was
really said: does a change of speaker land on the right word?

    quill-cli transcribe data/handoffs_call --diarize --speakers 2 -f json > out.json
    python -I score_lines.py out.json --script handoffs.txt
    python -I score_lines.py out.json --rttm data/ami_ES2004a.rttm

--script (a make_tts_meeting.py script) scores word by word: the
transcript's words are aligned to the script's, each transcript speaker is
mapped to the script voice most of its words belong to, and then

- words: the share of the script's words given to the right person;
- boundaries: changes of speaker in the script where the transcript changes
  speaker too, in the same direction, within one word;
- mixed lines: transcript lines holding words of two people.

--rttm (real meetings, where only who spoke when is known) scores time: the
share of the time one person was speaking, inside a transcript line, that
the line gives to someone else ("line confusion"; speakers mapped the same
way). Lines are what readers and the notes see, so this is diarization's
confusion as it reaches them.

"Me" lines are left out: these are scores for the call track.
"""

import argparse
import json
import re
from collections import Counter, defaultdict


def norm(word):
    return re.sub(r"[^0-9a-z]", "", word.lower())


def script_words(path):
    words = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            voice, text = line.lstrip("~>").split("|", 1)
            words += [(w, voice) for w in map(norm, text.split()) if w]
    return words


def transcript_lines(path):
    with open(path, encoding="utf-8-sig") as f:
        segments = json.load(f)
    return [s for s in segments if s.get("speaker") != "Me"]


def align(ref, hyp):
    """Levenshtein alignment: for each ref index, the hyp index it lines up
    with (match or substitution), or None."""
    n, m = len(ref), len(hyp)
    cost = [[0] * (m + 1) for _ in range(n + 1)]
    for i in range(n + 1):
        cost[i][0] = i
    for j in range(m + 1):
        cost[0][j] = j
    for i in range(1, n + 1):
        for j in range(1, m + 1):
            cost[i][j] = min(
                cost[i - 1][j - 1] + (ref[i - 1] != hyp[j - 1]),
                cost[i - 1][j] + 1,
                cost[i][j - 1] + 1,
            )
    out = [None] * n
    i, j = n, m
    while i > 0 and j > 0:
        if cost[i][j] == cost[i - 1][j - 1] + (ref[i - 1] != hyp[j - 1]):
            out[i - 1] = j - 1
            i, j = i - 1, j - 1
        elif cost[i][j] == cost[i - 1][j] + 1:
            i -= 1
        else:
            j -= 1
    return out


def mapping(pairs):
    """Each transcript label → the reference speaker it overlaps most."""
    votes = defaultdict(Counter)
    for label, truth, weight in pairs:
        votes[label][truth] += weight
    return {label: c.most_common(1)[0][0] for label, c in votes.items()}


def score_script(lines, script):
    ref = script_words(script)
    hyp = []  # (word, label, line index)
    for k, s in enumerate(lines):
        hyp += [(w, s.get("speaker"), k) for w in map(norm, s["text"].split()) if w]
    to_hyp = align([w for w, _ in ref], [w for w, _, _ in hyp])
    names = mapping((hyp[j][1], ref[i][1], 1) for i, j in enumerate(to_hyp) if j is not None)

    # Who the transcript says said each script word; a word it missed takes
    # the nearest aligned word before it.
    said, last = [], None
    for j in to_hyp:
        if j is not None:
            last = names[hyp[j][1]]
        said.append(last)
    right = sum(said[i] == ref[i][1] for i in range(len(ref)))

    changes = [i for i in range(1, len(ref)) if ref[i][1] != ref[i - 1][1]]
    placed = 0
    for i in changes:
        before, after = ref[i - 1][1], ref[i][1]
        if any(
            0 < k < len(ref) and said[k - 1] == before and said[k] == after
            for k in (i - 1, i, i + 1)
        ):
            placed += 1

    voices = defaultdict(set)
    for i, j in enumerate(to_hyp):
        if j is not None:
            voices[hyp[j][2]].add(ref[i][1])
    mixed = sum(len(v) > 1 for v in voices.values())

    print(f"words: {right}/{len(ref)} to the right person ({100 * right / len(ref):.1f}%)")
    print(f"boundaries: {placed}/{len(changes)} within one word")
    print(f"lines: {len(lines)}, {mixed} with two people's words")
    print("labels: " + ", ".join(f"{k} = {v}" for k, v in sorted(names.items(), key=str)))


def score_rttm(lines, rttm):
    frame = 10  # ms
    truth = defaultdict(set)
    with open(rttm, encoding="utf-8") as f:
        for row in f:
            p = row.split()
            if not p or p[0] != "SPEAKER":
                continue
            start, dur = round(float(p[3]) * 1000), round(float(p[4]) * 1000)
            for t in range(start // frame, (start + dur) // frame):
                truth[t].add(p[7])
    said = {}
    for s in lines:
        for t in range(s["start_ms"] // frame, s["end_ms"] // frame):
            said[t] = s.get("speaker")
    single = [(t, next(iter(v))) for t, v in truth.items() if len(v) == 1 and t in said]
    names = mapping((said[t], who, 1) for t, who in single)
    wrong = sum(names[said[t]] != who for t, who in single)
    print(f"line confusion: {100 * wrong / len(single):.1f}% of {len(single) * frame / 1000:.0f} s")
    print(f"lines: {len(lines)}, labels: {len(names)}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("transcript", help="quill-cli transcribe -f json output")
    parser.add_argument("--script", help="the make_tts_meeting.py script it was read from")
    parser.add_argument("--rttm", help="reference RTTM")
    args = parser.parse_args()
    lines = transcript_lines(args.transcript)
    if args.script:
        score_script(lines, args.script)
    if args.rttm:
        score_rttm(lines, args.rttm)


if __name__ == "__main__":
    main()
