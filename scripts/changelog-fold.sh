#!/usr/bin/env bash
# Fold the CHANGELOG fragments under unreleased/ into CHANGELOG.md
# beneath the next version's heading.
#
# A change's CHANGELOG entry is not written into CHANGELOG.md; it is
# a fragment, unreleased/<pr-number>.md, worded exactly as it would
# read under `## Unreleased`. Two PRs that merge the same day then
# touch two files instead of one hunk, so the second never has to
# rebase over the first's changelog line.
#
# At release this script gathers the fragments in PR order, appends
# whatever still sits under `## Unreleased` first, and writes them
# all under `## <version> — <headline> (<date>)`. The result is the
# item-level record; the release page (one page at the release's
# altitude, see the v0.21.0 section) is written above it by hand in
# the same release PR. The fragments are `git rm`ed so the release
# commit carries their removal.
#
# Usage:
#   scripts/changelog-fold.sh vX.Y.Z "headline" [YYYY-MM-DD]
set -euo pipefail
cd "$(dirname "$0")/.."

usage='usage: scripts/changelog-fold.sh vX.Y.Z "headline" [YYYY-MM-DD]'
version=${1:?$usage}
headline=${2:?$usage}
date=${3:-$(date +%F)}
changelog=CHANGELOG.md
dir=unreleased

case "$version" in
    v[0-9]*.[0-9]*.[0-9]*) ;;
    *) echo "changelog-fold: version must look like vX.Y.Z, got '$version'" >&2; exit 2 ;;
esac

# Fragments are named by PR number; README.md and anything else in
# the directory is not a fragment.
fragments=()
while IFS= read -r name; do
    [ -n "$name" ] && fragments+=("$dir/$name")
done < <(ls "$dir" 2>/dev/null | grep -E '^[0-9]+\.md$' | sort -n || true)

unreleased_count=$(grep -c '^## Unreleased$' "$changelog" || true)
if [ "$unreleased_count" -ne 1 ]; then
    echo "changelog-fold: expected exactly one '## Unreleased' heading in $changelog, found $unreleased_count" >&2
    exit 1
fi
if grep -q "^## $version " "$changelog"; then
    echo "changelog-fold: $changelog already has a '## $version' section" >&2
    exit 1
fi

# Split CHANGELOG.md at the Unreleased heading: head (through the
# heading), the Unreleased body (up to the next '## ' heading, blank
# lines at either end trimmed), and the tail (the previous releases).
head_part=$(awk '{ print } /^## Unreleased$/ { exit }' "$changelog")
body_part=$(awk '
    /^## Unreleased$/ { inside = 1; next }
    inside && /^## / { exit }
    inside { print }
' "$changelog" | sed -e :a -e '/^\n*$/{$d;N;ba' -e '}' | sed '/./,$!d')
tail_part=$(awk '
    /^## Unreleased$/ { inside = 1; next }
    inside && /^## / { after = 1 }
    after { print }
' "$changelog")

if [ -z "$body_part" ] && [ "${#fragments[@]}" -eq 0 ]; then
    echo "changelog-fold: nothing to fold (no fragments under $dir/, nothing under '## Unreleased')" >&2
    exit 1
fi

tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT
{
    printf '%s\n\n' "$head_part"
    printf '## %s — %s (%s)\n\n' "$version" "$headline" "$date"
    if [ -n "$body_part" ]; then
        printf '%s\n\n' "$body_part"
    fi
    for f in "${fragments[@]}"; do
        # awk 1 re-terminates every line, so a fragment without a
        # trailing newline cannot run into the next one.
        awk 1 "$f" | sed -e :a -e '/^\n*$/{$d;N;ba' -e '}'
        printf '\n'
    done
    printf '%s\n' "$tail_part"
} > "$tmp"
mv "$tmp" "$changelog"
trap - EXIT

if [ "${#fragments[@]}" -gt 0 ]; then
    git rm -q -f -- "${fragments[@]}"
fi

echo "folded ${#fragments[@]} fragment(s) into $changelog under '## $version — $headline ($date)'"
echo "next: write the release page above the item-level list, then review 'git diff'"
