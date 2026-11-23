#!/bin/sh
# Usage: check-release.sh <tag> <notes-file>
# Fails unless the tag is vX.Y.Z, Cargo.toml has the version X.Y.Z and CHANGELOG.md has an entry "## X.Y.Z"; writes
# that entry to <notes-file>. Runs from the root of the repo.
set -eu

tag="$1"
notes="$2"

if ! printf '%s\n' "$tag" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+$'; then
  echo "::error::The tag ${tag} has not the form vX.Y.Z." >&2
  exit 1
fi
version="${tag#v}"

manifest="$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[0].version')"
if [ "$manifest" != "$version" ]; then
  echo "::error::Cargo.toml has version ${manifest}, the tag is ${tag}." >&2
  exit 1
fi

if [ ! -f CHANGELOG.md ]; then
  echo "::error::CHANGELOG.md is missing." >&2
  exit 1
fi
# The lines after "## X.Y.Z" up to the next "## ", without blank lines at the start and the end.
awk -v heading="## ${version}" '
  $0 == heading { found = 1; next }
  found && /^## / { exit }
  found { lines[++n] = $0 }
  END {
    first = 1; while (first <= n && lines[first] == "") first++
    last = n; while (last >= first && lines[last] == "") last--
    for (i = first; i <= last; i++) print lines[i]
  }
' CHANGELOG.md > "$notes"
if [ ! -s "$notes" ]; then
  echo "::error::CHANGELOG.md has no entry for ${version}." >&2
  exit 1
fi
