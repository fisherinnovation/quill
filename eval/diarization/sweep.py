"""Runs `quill-cli diarize` over test files, voice models and clustering
settings, and prints a table of DER and speakers found.

    python3 -I sweep.py data/ten_people_call.wav [more.wav ...] \
        [--voices voice-campplus,voice-titanet-small] [--thresholds 0.5,0.7,0.9]

Each WAV needs a reference <name>.rttm next to it. "known" rows pass the true
speaker count. Set QUILL_CLI to the binary (default: target/release/quill-cli
in the repository).
"""

import argparse
import json
import os
import subprocess

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
CLI = os.environ.get("QUILL_CLI", os.path.join(ROOT, "target", "release", "quill-cli"))
VOICES = "voice-campplus,voice-eres2net,voice-eres2net-200k,voice-resnet34,voice-titanet-small,voice-titanet-large"


def run(wav, voice, threshold=None, speakers=None):
    cmd = [CLI, "diarize", wav, "--voice", voice, "--format", "json", "--reference", wav[:-4] + ".rttm"]
    if threshold is not None:
        cmd += ["--threshold", str(threshold)]
    if speakers is not None:
        cmd += ["--speakers", str(speakers)]
    out = subprocess.run(cmd, check=True, capture_output=True, text=True).stdout
    return json.loads(out)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("wavs", nargs="+")
    parser.add_argument("--voices", default=VOICES)
    parser.add_argument("--thresholds", default="0.5,0.6,0.7,0.8,0.9")
    args = parser.parse_args()

    print(f"{'file':<22} {'voice':<20} {'setting':<8} {'DER':>6} {'conf':>6} {'collar':>7} {'found':>9} {'secs':>5}")
    for wav in args.wavs:
        name = os.path.basename(wav)[:-4]
        for voice in args.voices.split(","):
            rows = [(f"t={t}", run(wav, voice, threshold=float(t))) for t in args.thresholds.split(",")]
            truth = rows[0][1]["score"]["reference_speakers"]
            rows.append(("known", run(wav, voice, speakers=truth)))
            for setting, r in rows:
                s = r["score"]
                print(
                    f"{name:<22} {voice:<20} {setting:<8} {s['der']:>6.1%} {s['confusion']:>6.1%} "
                    f"{s['der_collar_250ms']:>7.1%} {r['speakers_found']:>4}/{truth:<4} {r['diarize_secs']:>5.1f}",
                    flush=True,
                )


if __name__ == "__main__":
    main()
