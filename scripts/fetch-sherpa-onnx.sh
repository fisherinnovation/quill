#!/usr/bin/env bash
# Downloads the prebuilt sherpa-onnx libraries that the `sherpa-onnx` crate
# links (speaker diarization), checks them against pinned SHA-256 hashes,
# extracts them and prints the library directory. Point the build at it:
#
#   export SHERPA_ONNX_LIB_DIR="$(scripts/fetch-sherpa-onnx.sh)"
#
# Without SHERPA_ONNX_LIB_DIR, the crate's build script downloads the same
# archive itself, unchecked, and caches it under target/. CI always uses
# this script. Keep VERSION in step with the `sherpa-onnx` pin in Cargo.toml;
# hashes are the release assets' digests on GitHub.
set -euo pipefail

VERSION=1.13.8
case "$(uname -s)-$(uname -m)" in
    Darwin-arm64)
        ARCHIVE="sherpa-onnx-v$VERSION-osx-arm64-static-lib"
        SHA256=9091bf160dc7fdacedbc906b212badf53c2993f4e5277a0e03998e96c31d60da ;;
    MINGW*-x86_64 | MSYS*-x86_64 | CYGWIN*-x86_64)
        ARCHIVE="sherpa-onnx-v$VERSION-win-x64-shared-MT-Release-lib"
        SHA256=b8eedf41bd6d3779218887b48367bb7a3ece5aaa7667f01f69ee823a12b0a9e7 ;;
    *)
        echo "no pinned sherpa-onnx archive for $(uname -s)-$(uname -m)" >&2
        exit 1 ;;
esac

DEST="${1:-$(cd "$(dirname "$0")/.." && pwd)/.sherpa-onnx}"
mkdir -p "$DEST"
cd "$DEST"

if [ ! -d "$ARCHIVE/lib" ]; then
    curl -fsSL -o "$ARCHIVE.tar.bz2" \
        "https://github.com/k2-fsa/sherpa-onnx/releases/download/v$VERSION/$ARCHIVE.tar.bz2"
    if command -v sha256sum >/dev/null; then
        actual=$(sha256sum "$ARCHIVE.tar.bz2" | cut -d' ' -f1)
    else
        actual=$(shasum -a 256 "$ARCHIVE.tar.bz2" | cut -d' ' -f1)
    fi
    if [ "$actual" != "$SHA256" ]; then
        echo "checksum mismatch for $ARCHIVE.tar.bz2: expected $SHA256, got $actual" >&2
        rm -f "$ARCHIVE.tar.bz2"
        exit 1
    fi
    tar -xjf "$ARCHIVE.tar.bz2"
    rm "$ARCHIVE.tar.bz2"
fi

lib="$DEST/$ARCHIVE/lib"
# Windows builds need a native path.
if command -v cygpath >/dev/null; then
    lib=$(cygpath -w "$lib")
fi
echo "$lib"
