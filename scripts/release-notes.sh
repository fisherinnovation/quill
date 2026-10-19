#!/usr/bin/env bash
# Release notes for a version, from the Conventional Commits since the
# previous tag: features under "New", fixes under "Fixes", in the order they
# landed, then the downloads and where to start. The release workflow
# attaches them to the draft release, to edit before publishing (an opening
# line, refinements of one feature folded together).
#
#   scripts/release-notes.sh [tag]      # default: HEAD, to preview
#
# Commits for the website (scope "site") are left out: it isn't part of the
# app. So are chores, docs, style and the like.
set -euo pipefail

ref="${1:-HEAD}"
repo="${GITHUB_REPOSITORY:-fisherinnovation/quill}"
url="https://github.com/$repo"
if [ "$ref" = HEAD ]; then
  version=$(sed -nE 's/^  "version": "([^"]+)".*/\1/p' app/src-tauri/tauri.conf.json)
else
  version="${ref#v}"
fi

if previous=$(git describe --tags --abbrev=0 --match 'v*' "$ref^" 2>/dev/null); then
  range="$previous..$ref"
else
  previous=""
  range="$ref"
fi

# "feat(app): a livelier Meetings page" -> "A livelier Meetings page".
section() {
  local type="$1" title="$2" lines
  lines=$(git log --reverse --no-merges --format=%s "$range" |
    grep -E "^$type(\([^)]*\))?!?: " |
    grep -Ev "^$type\(site\)!?: " |
    sed -E "s/^$type(\([^)]*\))?!?: //" |
    awk '!seen[$0]++ { print "- " toupper(substr($0, 1, 1)) substr($0, 2) }' || true)
  if [ -n "$lines" ]; then
    printf '## %s\n\n%s\n\n' "$title" "$lines"
  fi
}

section feat "New"
section fix "Fixes"

cat <<NOTES
## Download

| System | File |
|---|---|
| macOS 14.6 or later, Apple Silicon | \`Quill_${version}_aarch64.dmg\` |
| Windows 10 or 11 (64-bit) | \`Quill_${version}_x64-setup.exe\` (recommended), or \`Quill_${version}_x64_en-US.msi\` |

**New to Quill?** Builds aren't code-signed yet, so the first launch needs one extra approval: see [Installing]($url#installing).
NOTES

# 0.1.0 had no updater: its users download the next version themselves.
if [ "$previous" = v0.1.0 ]; then
  echo
  echo "**Using 0.1.0?** It can't update itself: download this version once, and Quill offers later updates itself."
elif [ -n "$previous" ]; then
  echo
  echo "**Already using Quill?** It offers this update itself, or use **Settings → Updates → Check now**."
fi

cat <<NOTES

Known issues are in the [README]($url#known-limitations), and how Quill handles your data is in [SECURITY.md]($url/blob/main/SECURITY.md). Please report problems in [Issues]($url/issues).
NOTES

if [ -n "$previous" ]; then
  name=$([ "$ref" = HEAD ] && echo main || echo "$ref")
  echo
  echo "**Full changelog:** $url/compare/$previous...$name"
fi
