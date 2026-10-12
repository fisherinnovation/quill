#!/bin/sh
# Downloads AMI test meetings (headset mix, 16 kHz) and their reference RTTMs
# into data/. AMI is CC BY 4.0 (University of Edinburgh); the RTTMs are the
# "only_words" references from pyannote/AMI-diarization-setup (MIT), which
# published DER figures for pyannote are measured against.
set -eu
cd "$(dirname "$0")"
mkdir -p data
for m in "${@:-ES2004a IS1009a TS3003a}"; do
    for id in $m; do
        [ -f "data/ami_$id.wav" ] || curl -fL -o "data/ami_$id.wav" \
            "https://groups.inf.ed.ac.uk/ami/AMICorpusMirror/amicorpus/$id/audio/$id.Mix-Headset.wav"
        curl -fsL -o "data/ami_$id.rttm" \
            "https://raw.githubusercontent.com/pyannote/AMI-diarization-setup/main/only_words/rttms/test/$id.rttm"
    done
done
ls -l data/ami_*
