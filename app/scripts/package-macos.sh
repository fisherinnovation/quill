#!/bin/sh
# Signs the release Quill.app and packs it into dist-release/: a .dmg for
# new installs, and a .app.tar.gz for the in-app updater (the release
# workflow signs that with the updater key).
# Run from app/ after `npx tauri build --bundles app` (the release workflow
# does this).
#
# Signing: with APPLE_SIGNING_IDENTITY set (a Developer ID certificate in
# the keychain), the app is signed with it. Otherwise it's signed ad hoc
# with an explicit designated requirement, "any build with Quill's
# identifier", so macOS keeps its permissions (microphone, Accessibility)
# across updates; see scripts/bundle.sh. Ad hoc builds aren't notarized, so
# Gatekeeper asks the user to approve them once (README: Installing).
set -eu
cd "$(dirname "$0")/.."

app="../target/release/bundle/macos/Quill.app"
version=$(node -p "require('./src-tauri/tauri.conf.json').version")
out="../dist-release"
dmg="$out/Quill_${version}_aarch64.dmg"
update="$out/Quill_${version}_aarch64.app.tar.gz"

if [ -n "${APPLE_SIGNING_IDENTITY:-}" ]; then
    codesign --force --deep --options runtime --timestamp \
        --sign "$APPLE_SIGNING_IDENTITY" "$app"
else
    codesign --force --deep --sign - \
        -r='designated => identifier "com.fisherinnovation.quill"' "$app"
fi
codesign --verify --deep --strict "$app"
codesign -d -r- "$app" 2>&1 | grep designated

# A plain disk image: the app and a link to /Applications to drag it onto.
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
ditto "$app" "$stage/Quill.app"
ln -s /Applications "$stage/Applications"
mkdir -p "$out"
rm -f "$dmg"
hdiutil create -quiet -volname "Quill $version" -srcfolder "$stage" -format UDZO "$dmg"
echo "Wrote $dmg ($(du -h "$dmg" | cut -f1))"

# The updater's copy, packed after signing: an archive of the build Tauri
# signed itself would lack the designated requirement above, and macOS
# would forget Quill's permissions on every update. COPYFILE_DISABLE keeps
# macOS's ._ metadata files out of the archive.
rm -f "$update"
COPYFILE_DISABLE=1 tar -czf "$update" -C "$(dirname "$app")" Quill.app
echo "Wrote $update ($(du -h "$update" | cut -f1))"
