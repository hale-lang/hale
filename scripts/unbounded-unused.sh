#!/usr/bin/env bash
# List the `@unbounded` and `@bounded_by(<name>)` decorators in dna/ and
# in the stdlib's own source (crates/hale-stdlib/hl/) that silence
# nothing in any program that reaches them (GH #1490).
#
# `@unbounded` on a fn or lifecycle hook drops two things: the
# memory-bound survey's sites in it and the hot-path allocation lint's
# warnings for it. `@bounded_by(<name>)` drops the same two and names
# the input that bounds the growth; the two are judged alike and listed
# with their kind. `hale check --report-unbounded-use` prints, for one
# program, every decorated fn it contains (its imported seeds' included,
# and the stdlib's it reaches: the appended api runtime's and the
# analysis copy's fns and hooks it reaches) with how many of each the
# decorator silences there; a stdlib fn's are judged in that program as
# though its rows were the program's own, since the advisory reports the
# program's own sites alone. A seed's decorator is judged in every
# program that compiles the seed (dna/core is in most of them), and what
# it silences depends on the program: a fn is invoked unboundedly only
# where a consumer's bus handler or run loop reaches it. So a decorator
# is listed here only when it silences nothing in EVERY program that
# reports it; one no program reports is unjudged, never listed.
#
# The programs: every directory under dna/ holding a `.hl` file that is
# not a `*_test.hl` (checked as a seed), every `*_test.hl` under dna/
# (checked as a program), every entry of the in-tree corpus
# (crates/hale-codegen/tests/fixtures/examples/: each directory, and
# each `.hl` file beside them), and every `*_test.hl` under tests/hale/.
# node_modules is skipped. The stdlib has no `.hl` tests of its own; the
# language's tests under tests/hale/ exercise it. A program that does
# not resolve reports nothing; it is counted on stderr. Pass targets as
# arguments to judge over those instead.
#
# Judged: the decorators written in dna/ and in crates/hale-stdlib/hl/.
# A program's own decorators elsewhere (the corpus's, tests/hale's) are
# reported by their program and ignored here.
#
# Output: stdout, one `path:line<TAB>fn<TAB>decorator` per decorator
# that silences nothing anywhere (`decorator` is `unbounded` or
# `bounded_by(<name>)`, as the report spells it), sorted, the dna/ and
# stdlib ones alike; stderr, the programs that did not check and
# reported nothing (`unreported`), the decorators no program reported
# (`unjudged`), any reported line no decorator is written on (`stray`,
# none expected), and a summary: one line of programs, one of dna/'s
# decorators, one of the stdlib's (total, silence nothing, silence
# something, unjudged). A decorator's place is `path:line` relative to
# the repository root (a stdlib one's is `crates/hale-stdlib/hl/<file>`,
# as the report spells it), so the same decorator reported by different
# programs is one entry.
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
        find crates/hale-codegen/tests/fixtures/examples -mindepth 1 -maxdepth 1 \( -type d -o -name '*.hl' \) -print | sort
        find tests/hale -name '*_test.hl' -print | sort
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

# Every decorator written in dna/ and in the stdlib, by `path:line`:
# `@unbounded`, or `@bounded_by(<name>)`.
grep -rnE --include='*.hl' --exclude-dir=node_modules \
    '^[[:space:]]*(@[A-Za-z_]+(\([^)]*\))?[[:space:]]+)*@(unbounded|bounded_by\([^)]*\))([[:space:]]|$)' dna crates/hale-stdlib/hl \
    | cut -d: -f1,2 | sort -u > "$work/written" || exit 1
: > "$work/unjudged" || exit 1
: > "$work/stray" || exit 1

cat "$work"/out/*[0-9] 2>/dev/null | awk -F'\t' -v written="$work/written" -v unjudged="$work/unjudged" -v stray="$work/stray" -v summary="$work/summary" '
    # Which judged tree a decorator is in: dna/, the stdlib, or neither
    # (one a corpus or tests/hale program declares itself, ignored).
    function tree(k) {
        if (k ~ /^dna\//) return "dna"
        if (k ~ /^crates\/hale-stdlib\/hl\//) return "std"
        return ""
    }
    $1 == "unbounded-use" && tree($2) != "" {
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
        nw = 0; nt = 0; sw = 0
        while ((getline w < written) > 0) {
            all[w] = 1
            if (tree(w) == "std") sw++
            else {
                nw++
                if (w ~ /(^|\/)tests\// || w ~ /_test\.hl:/) nt++
            }
            if (!(w in seen)) { print w > unjudged; if (tree(w) == "std") sun++ }
        }
        nseen = 0; nunused = 0; nhot = 0; nsurvey = 0; nsingle = 0; nbound = 0; nbunused = 0
        sseen = 0; sunused = 0
        for (k in seen) {
            if (!(k in all)) print k > stray
            if (!(k in used)) print k "\t" name[k] "\t" kind[k]
            if (tree(k) == "std") {
                sseen++
                if (!(k in used)) sunused++
                continue
            }
            nseen++
            bound = (kind[k] ~ /^bounded_by\(/)
            if (bound) nbound++
            if (!(k in used)) { nunused++; if (bound) nbunused++ }
            if ((k in used) && !(k in survey)) nhot++
            if (k in survey) nsurvey++
            if ((k in used) && using[k] == 1 && programs[k] > 1) nsingle++
        }
        printf "%d %d %d %d %d %d %d %d %d %d %d %d %d\n", nw, nt, nseen, nunused, nsurvey, nhot, nsingle, nbound, nbunused, sw, sseen, sunused, sun + 0 > summary
    }
' | sort -t: -k1,1 -k2,2n || exit 1

read -r nw nt nseen nunused nsurvey nhot nsingle nbound nbunused sw sseen sunused sunjudged < "$work/summary" || exit 1
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
nunjudged=$(( $(wc -l < "$work/unjudged") - sunjudged )) || exit 1
nstray=$(wc -l < "$work/stray") || exit 1
{
    echo "unbounded-unused: $ntargets programs checked: $reporting reported decorators, $none contain none," \
        "$failed did not check and reported none (listed as unreported); $nstray stray"
    echo "unbounded-unused: dna/: $nw decorators ($((nw - nt)) outside tests, $nt in tests);" \
        "$nseen judged: $nunused silence nothing anywhere," \
        "$nsurvey silence a survey site somewhere, $nhot silence only hot-path warnings," \
        "$nsingle silence something in exactly one of the several programs that contain them;" \
        "$nunjudged unjudged (no program reported them);" \
        "of the judged, $nbound are \`@bounded_by\`, $nbunused of them silencing nothing"
    echo "unbounded-unused: stdlib: $sw decorators; $sseen judged: $sunused silence nothing anywhere," \
        "$((sseen - sunused)) silence something somewhere; $sunjudged unjudged (no program reaches them)"
} >&2
