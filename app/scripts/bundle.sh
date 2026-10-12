#!/bin/sh
# Builds Quill as a real app bundle at ~/Applications/Quill.app and opens it.
#
# Why: `npm run app` runs a bare executable from target/, which macOS won't
# let you add to Privacy & Security lists (Accessibility, needed for
# dictation to paste), and which macOS attributes to whatever app launched
# it. A bundle opened with `open` is its own app.
#
# Signing: macOS remembers permissions (Accessibility, microphone, keychain
# "Always Allow") by the app's designated requirement. An ad hoc signature's
# default requirement is the hash of that exact build, so every rebuild
# looked like a new app and lost its permissions. We therefore sign with an
# explicit requirement, "any build with Quill's identifier", which stays
# the same across rebuilds. It's weaker (anything signed with that
# identifier would match), which is fine for development builds; releases
# get Developer ID signing.
#
# If a code-signing certificate named "Quill Development" exists (Keychain
# Access → Certificate Assistant → Create a Certificate…, Self-Signed Root,
# Code Signing), it's used instead. Override the name with
# QUILL_SIGNING_IDENTITY.
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
    codesign --force --deep --sign - \
        -r='designated => identifier "com.fisherinnovation.quill"' "$built"
    echo "Signed ad hoc, with a requirement that stays the same across rebuilds."
fi

# Only one Quill at a time: they'd fight over the menu bar and the shortcut.
pkill -x quill-app 2>/dev/null && sleep 1 || true

mkdir -p "$HOME/Applications"
rm -rf "$installed"
ditto "$built" "$installed"
# Leave one copy: a second Quill.app in target/ shows up in Spotlight and in
# the Privacy & Security "+" picker, and adding that one doesn't help.
rm -rf "$built"
open "$installed"
echo "Opened $installed"
