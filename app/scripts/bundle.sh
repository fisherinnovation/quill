#!/bin/sh
# Builds Quill as a real app bundle at ~/Applications/Quill.app and opens it.
#
# Why: `npm run app` runs a bare executable from target/, which macOS won't
# let you add to Privacy & Security lists (Accessibility, needed for
# dictation to paste), and which macOS attributes to whatever app launched
# it. A bundle opened with `open` is its own app.
#
# Signing: macOS remembers permissions by code signature. With a code-signing
# certificate named "Quill Development" in your login keychain (create it
# once in Keychain Access → Certificate Assistant → Create a Certificate…,
# Identity Type: Self-Signed Root, Certificate Type: Code Signing), every
# build is signed the same way, so permissions and the keychain's "Always
# Allow" survive rebuilds. Without it the build is signed ad hoc, and after
# each rebuild you must remove Quill from Accessibility and add it again.
# Override the certificate name with QUILL_SIGNING_IDENTITY.
set -eu

# Shells that didn't load ~/.zshenv (e.g. some IDE tasks) may lack cargo.
if ! command -v cargo >/dev/null 2>&1 && [ -f "$HOME/.cargo/env" ]; then
    . "$HOME/.cargo/env"
fi

cd "$(dirname "$0")/.."
identity="${QUILL_SIGNING_IDENTITY:-Quill Development}"
built="../target/debug/bundle/macos/Quill.app"
installed="$HOME/Applications/Quill.app"

# Debug profile (fast to build, keeps debug assertions) with the frontend
# embedded, as in a release.
npx tauri build --debug --bundles app

if security find-identity -p codesigning | grep -q "\"$identity\"" &&
    codesign --force --deep --sign "$identity" "$built"; then
    echo "Signed with \"$identity\"."
else
    codesign --force --deep --sign - "$built"
    echo "Signed ad hoc (no \"$identity\" certificate): after each rebuild, remove Quill"
    echo "from Privacy & Security → Accessibility and add it again. See this script's header."
fi

# Only one Quill at a time: they'd fight over the menu bar and the shortcut.
pkill -x quill-app 2>/dev/null && sleep 1 || true

mkdir -p "$HOME/Applications"
rm -rf "$installed"
ditto "$built" "$installed"
open "$installed"
echo "Opened $installed"
