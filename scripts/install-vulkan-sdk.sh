#!/usr/bin/env bash
# Windows only. Downloads the LunarG Vulkan SDK installer, checks it against
# a pinned SHA-256 hash, installs it silently and prints the SDK's directory.
# whisper.cpp's Vulkan backend needs it to build (headers, vulkan-1.lib and
# the glslc shader compiler). Point the build at it:
#
#   export VULKAN_SDK="$(scripts/install-vulkan-sdk.sh)"
#
# The installer also puts the Vulkan loader (vulkan-1.dll) in System32; the
# release build ships that DLL next to quill-app.exe. Pick a VERSION at least
# two weeks old; the hash is LunarG's own, from
# https://sdk.lunarg.com/sdk/sha/$VERSION/windows/vulkansdk-windows-X64-$VERSION.exe.txt
set -euo pipefail

VERSION=1.4.363.0
SHA256=94a82d378f7a5e3e54c9db7d2fb7016af136e14ac0a18dbf0f2f67a36352d141

case "$(uname -s)" in
    MINGW* | MSYS* | CYGWIN*) ;;
    *)
        echo "the Vulkan SDK is only needed on Windows" >&2
        exit 1 ;;
esac

DEST="${1:-C:/VulkanSDK/$VERSION}"

if [ ! -f "$DEST/Lib/vulkan-1.lib" ]; then
    installer="$(mktemp -d)/vulkansdk-windows-X64-$VERSION.exe"
    curl -fsSL -o "$installer" \
        "https://sdk.lunarg.com/sdk/download/$VERSION/windows/vulkansdk-windows-X64-$VERSION.exe"
    actual=$(sha256sum "$installer" | cut -d' ' -f1)
    if [ "$actual" != "$SHA256" ]; then
        echo "checksum mismatch for the Vulkan SDK $VERSION: expected $SHA256, got $actual" >&2
        rm -f "$installer"
        exit 1
    fi
    # A Qt Installer Framework installer; these flags run it unattended.
    "$installer" --root "$(cygpath -w "$DEST")" --accept-licenses --default-answer \
        --confirm-command install >&2
    rm -f "$installer"
fi

cygpath -w "$DEST"
