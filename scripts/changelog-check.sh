#!/usr/bin/env bash
# Check that every CHANGELOG fragment declares its breaking changes
# (STABILITY.md § Every release names its breaking changes, GH #1950).
#
# A fragment, unreleased/<pr-number>.md, says one of two things about
# breakage, explicitly:
#
#   - a line reading exactly `Breaking: none`, when nothing a correct
#     program relies on changes; or
#   - a `### Breaking` heading with at least one bullet under it, each
#     naming what breaks and how to migrate.
#
# Never both, and never neither: a fragment that says nothing is how a
# semantic change shipped unflagged in a patch release. At release,
# scripts/changelog-fold.sh gathers every fragment's `### Breaking`
# bullets under one `### Breaking` heading at the top of the version
# and drops the `Breaking: none` lines.
#
# Usage:
#   scripts/changelog-check.sh              # every fragment under unreleased/
#   scripts/changelog-check.sh FILE...      # just these
#   scripts/changelog-check.sh --self-test  # the checker against canned fragments
set -euo pipefail
cd "$(dirname "$0")/.."

# check_one FILE: prints a reason and returns 1 when FILE is not a
# well-declared fragment.
check_one() {
    local f=$1
    awk -v f="$f" '
        /^Breaking: none[[:space:]]*$/ { none++ ; next }
        /^### Breaking[[:space:]]*$/ { heading++; inside = 1; next }
        /^(#|##|###) / { inside = 0 }
        inside && /^[[:space:]]*[-*] [^[:space:]]/ { bullets++ }
        END {
            if (none && heading) { print f ": says both `Breaking: none` and `### Breaking`; keep one" > "/dev/stderr"; exit 1 }
            if (none > 1) { print f ": says `Breaking: none` more than once" > "/dev/stderr"; exit 1 }
            if (heading > 1) { print f ": has more than one `### Breaking` heading" > "/dev/stderr"; exit 1 }
            if (heading && !bullets) { print f ": its `### Breaking` heading has no bullet under it (say `Breaking: none` instead)" > "/dev/stderr"; exit 1 }
            if (!none && !heading) { print f ": declares no breaking changes; add a line `Breaking: none`, or a `### Breaking` heading with a bullet per break and its migration" > "/dev/stderr"; exit 1 }
        }
    ' "$f"
}

self_test() {
    local dir rc=0
    dir=$(mktemp -d)
    printf '### Fixed\n- a fix.\n\nBreaking: none\n' > "$dir/ok-none.md"
    printf '### Breaking\n- `x` is refused; write `y`.\n\n### Added\n- y.\n' > "$dir/ok-heading.md"
    printf '### Fixed\n- a fix.\n' > "$dir/bad-silent.md"
    printf '### Breaking\n\n### Fixed\n- a fix.\n' > "$dir/bad-empty.md"
    printf '### Breaking\n- x.\n\nBreaking: none\n' > "$dir/bad-both.md"
    printf '### Fixed\n- Breaking: none of this.\n' > "$dir/bad-inline.md"
    for f in "$dir"/ok-*.md; do
        check_one "$f" >/dev/null 2>&1 || { echo "self-test: $(basename "$f") should pass"; rc=1; }
    done
    for f in "$dir"/bad-*.md; do
        if check_one "$f" >/dev/null 2>&1; then echo "self-test: $(basename "$f") should fail"; rc=1; fi
    done
    rm -rf "$dir"
    [ "$rc" -eq 0 ] && echo "changelog-check: self-test passed"
    return "$rc"
}

if [ "${1:-}" = "--self-test" ]; then
    self_test
    exit
fi

files=("$@")
if [ "${#files[@]}" -eq 0 ]; then
    while IFS= read -r name; do
        [ -n "$name" ] && files+=("unreleased/$name")
    done < <(ls unreleased 2>/dev/null | grep -E '^[0-9]+\.md$' | sort -n || true)
fi

bad=0
for f in ${files[@]+"${files[@]}"}; do
    check_one "$f" || bad=1
done
if [ "$bad" -ne 0 ]; then
    echo "changelog-check: see unreleased/README.md for the fragment convention" >&2
    exit 1
fi
echo "changelog-check: ${#files[@]} fragment(s) declare their breaking changes"
