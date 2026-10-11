#!/bin/sh
# Runs the app in dev mode without keychain prompts on every rebuild.
#
# macOS remembers "Always Allow" per code signature. Dev builds are unsigned,
# so each rebuild looks like a new app and asks for the keychain password
# again. Instead, read the key once with Apple's `security` tool (signed and
# stable, so "Always Allow" sticks) and pass it to the app in the
# environment, which Quill checks before the keychain. The key stays in this
# process tree's memory only. Signed release builds use the keychain directly.
set -eu
# Shells that didn't load ~/.zshenv (e.g. some IDE tasks) may lack cargo.
if ! command -v cargo >/dev/null 2>&1 && [ -f "$HOME/.cargo/env" ]; then
    . "$HOME/.cargo/env"
fi
if [ -z "${QUILL_OPENROUTER_API_KEY:-}" ]; then
    QUILL_OPENROUTER_API_KEY=$(security find-generic-password \
        -s com.fisherinnovation.quill -a openrouter -w 2>/dev/null || true)
    export QUILL_OPENROUTER_API_KEY
fi
cd "$(dirname "$0")/.."
exec npx tauri dev "$@"
