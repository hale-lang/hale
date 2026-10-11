#!/usr/bin/env bash
# List the `@unbounded` and `@bounded_by(<name>)` decorators in dna/
# that silence nothing in any program that compiles them (GH #1490).
#
# `@unbounded` on a fn or lifecycle hook drops two things: the
# memory-bound survey's sites in it and the hot-path allocation lint's
# warnings for it. `@bounded_by(<name>)` drops the same two and names
# the input that bounds the growth; the two are judged alike and listed
# with their kind. `hale check --report-unbounded-use` prints, for one
# program, every decorated fn it contains (its imported seeds'
# included) with how many of each the decorator silences there. A
# seed's decorator is judged in every program that compiles the seed
# (dna/core is in most of them), and what it silences depends on the
# program: a fn is invoked unboundedly only where a consumer's bus
# handler or run loop reaches it. So a decorator is listed here only
# when it silences nothing in EVERY program that contains it.
#
# The programs: every directory under dna/ holding a `.hl` file that is
# not a `*_test.hl` (checked as a seed), and every `*_test.hl` under
# dna/ (checked as a program). node_modules is skipped. A program that
# does not resolve reports nothing; it is counted on stderr, and a
# decorator no program reported is listed on stderr as unjudged, never
# on stdout. Pass targets as arguments to judge over those instead.
#
# Out of scope for now: the stdlib's own `@unbounded` decorators
# (crates/hale-stdlib/hl/*.hl). They sit in the stdlib's analysis copy,
# which the per-program report leaves out.
#
# Output: stdout, one `path:line<TAB>fn<TAB>decorator` per decorator
# that silences nothing anywhere (`decorator` is `unbounded` or
# `bounded_by(<name>)`, as the report spells it), sorted; stderr, the programs that did not check and
# reported nothing (`unreported`), the decorators no program reported
# (`unjudged`), any reported line no decorator is written on (`stray`,
# none expected), and a summary. A decorator's place is `path:line`
# relative to the repository root, so the same decorator reported by
# different programs is one entry.
#
# Usage:
#   scripts/unbounded-unused.sh [target ...]
#   HALE_BIN=path/to/hale JOBS=8 scripts/unbounded-unused.sh
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

hale=${HALE_BIN:-target/release/hale}
if [ ! -x "$hale" ]; then
    echo "unbounded-unused: no hale binary at $hale (build the workspace, or set HALE_BIN)" >&2
    exit 1
fi
case "$hale" in
    /*) ;;
    */*) hale="$PWD/$hale" ;;
esac
jobs=${JOBS:-$(nproc 2>/dev/null || echo 4)}

work=$(mktemp -d "${TMPDIR:-/tmp}/unbounded-unused.XXXXXX") || exit 1
trap 'rm -rf "$work"' EXIT

if [ "$#" -gt 0 ]; then
    printf '%s\n' "$@" > "$work/targets" || exit 1
else
    {
        find dna -path '*/node_modules' -prune -o -name '*.hl' ! -name '*_test.hl' -print | sed 's|/[^/]*$||' | sort -u
        find dna -path '*/node_modules' -prune -o -name '*_test.hl' -print | sort
    } > "$work/targets" || exit 1
fi
ntargets=$(wc -l < "$work/targets") || exit 1
if [ "$ntargets" -eq 0 ]; then
    echo "unbounded-unused: no targets" >&2
    exit 1
fi

# One report per program, in parallel: report N holds target N's lines,
# and N.code its check's exit code.
mkdir "$work/out" || exit 1
export UNBOUNDED_UNUSED_OUT="$work/out"
nl -ba -w1 -s' ' "$work/targets" | xargs -P "$jobs" -L1 sh -c '
    "$0" check "$2" --report-unbounded-use > "$UNBOUNDED_UNUSED_OUT/$1.raw" 2>/dev/null
    echo $? > "$UNBOUNDED_UNUSED_OUT/$1.code"
    grep "^unbounded-use	" "$UNBOUNDED_UNUSED_OUT/$1.raw" > "$UNBOUNDED_UNUSED_OUT/$1"
    exit 0
' "$hale"
reporting=0
none=0
failed=0
i=0
while IFS= read -r t; do
    i=$((i + 1))
    if [ ! -f "$work/out/$i.code" ]; then
        echo "unbounded-unused: $t was not checked" >&2
        exit 1
    fi
    if [ -s "$work/out/$i" ]; then
        reporting=$((reporting + 1))
    elif [ "$(cat "$work/out/$i.code")" = 0 ]; then
        none=$((none + 1))
    else
        failed=$((failed + 1))
        echo "unreported	$t" >> "$work/failed"
    fi
done < "$work/targets"

# Every decorator written in dna/, by `path:line`: `@unbounded`, or
# `@bounded_by(<name>)`.
grep -rnE --include='*.hl' --exclude-dir=node_modules \
    '^[[:space:]]*(@[A-Za-z_]+(\([^)]*\))?[[:space:]]+)*@(unbounded|bounded_by\([^)]*\))([[:space:]]|$)' dna \
    | cut -d: -f1,2 | sort -u > "$work/written" || exit 1
: > "$work/unjudged" || exit 1
: > "$work/stray" || exit 1

cat "$work"/out/*[0-9] 2>/dev/null | awk -F'\t' -v written="$work/written" -v unjudged="$work/unjudged" -v stray="$work/stray" -v summary="$work/summary" '
    $1 == "unbounded-use" {
        seen[$2] = 1
        if (!($2 in name)) name[$2] = $3
        # A report before GH #1490 had no sixth field: `@unbounded`.
        if (!($2 in kind)) kind[$2] = ($6 == "" ? "unbounded" : $6)
        if ($4 + $5 > 0) used[$2] = 1
        if ($4 > 0) survey[$2] = 1
        programs[$2]++
        if ($4 + $5 > 0) using[$2]++
    }
    END {
        nw = 0; nt = 0
        while ((getline w < written) > 0) {
            all[w] = 1; nw++
            if (w ~ /(^|\/)tests\// || w ~ /_test\.hl:/) nt++
            if (!(w in seen)) print w > unjudged
        }
        nseen = 0; nunused = 0; nhot = 0; nsurvey = 0; nsingle = 0; nbound = 0; nbunused = 0
        for (k in seen) {
            nseen++
            bound = (kind[k] ~ /^bounded_by\(/)
            if (bound) nbound++
            if (!(k in all)) print k > stray
            if (!(k in used)) { print k "\t" name[k] "\t" kind[k]; nunused++; if (bound) nbunused++ }
            if ((k in used) && !(k in survey)) nhot++
            if (k in survey) nsurvey++
            if ((k in used) && using[k] == 1 && programs[k] > 1) nsingle++
        }
        printf "%d %d %d %d %d %d %d %d %d\n", nw, nt, nseen, nunused, nsurvey, nhot, nsingle, nbound, nbunused > summary
    }
' | sort -t: -k1,1 -k2,2n || exit 1

read -r nw nt nseen nunused nsurvey nhot nsingle nbound nbunused < "$work/summary" || exit 1
if [ -s "$work/failed" ]; then
    cat "$work/failed" >&2
fi
if [ -s "$work/unjudged" ]; then
    sed 's/^/unjudged\t/' "$work/unjudged" >&2
fi
if [ -s "$work/stray" ]; then
    # A reported line no decorator is written on: the report and the
    # inventory disagree about where a decorator is.
    sed 's/^/stray\t/' "$work/stray" >&2
fi
nunjudged=$(wc -l < "$work/unjudged") || exit 1
nstray=$(wc -l < "$work/stray") || exit 1
{
    echo "unbounded-unused: $nw decorators in dna/ ($((nw - nt)) outside tests, $nt in tests);" \
        "$ntargets programs checked: $reporting reported decorators, $none contain none," \
        "$failed did not check and reported none (listed as unreported)"
    echo "unbounded-unused: $nseen judged: $nunused silence nothing anywhere," \
        "$nsurvey silence a survey site somewhere, $nhot silence only hot-path warnings," \
        "$nsingle silence something in exactly one of the several programs that contain them;" \
        "$nunjudged unjudged (no program reported them); $nstray stray;" \
        "of the judged, $nbound are \`@bounded_by\`, $nbunused of them silencing nothing"
} >&2
