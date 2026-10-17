# Sourced, not run: the part scripts/unbounded-unused.sh and
# scripts/unbounded-aggregate.sh share (GH #1490). It picks the programs,
# runs `hale check --report-unbounded-use` over each in parallel, and
# lists the decorators written in dna/ and in the stdlib's own source.
#
# The programs: every directory under dna/ holding a `.hl` file that is
# not a `*_test.hl` (checked as a seed), every `*_test.hl` under dna/
# (checked as a program), every entry of the in-tree corpus
# (crates/hale-codegen/tests/fixtures/examples/: each directory, and
# each `.hl` file beside them), and every `*_test.hl` under tests/hale/.
# node_modules is skipped. The stdlib has no `.hl` tests of its own; the
# corpus and the language's tests under tests/hale/ reach it. Targets
# given as arguments replace that list.
#
# The caller has cd'd to the repository root. Usage:
#   . scripts/unbounded-reports.sh
#   unbounded_reports <name for messages> [target ...]
#
# Sets `hale`, `work` (a temporary directory, removed on exit),
# `ntargets`, `reporting` (programs that reported a decorator), `none`
# (programs that checked and contain none) and `failed` (programs that
# did not check and reported nothing). Leaves in $work:
#   targets     one target per line
#   out/<N>     target N's `unbounded-use` lines
#   out/<N>.code  its check's exit code
#   failed      `unreported<TAB>target` per failed program (may be absent)
#   inventory   `path:line<TAB>kind` per decorator written in dna/ or in
#               crates/hale-stdlib/hl/, sorted (`kind` is `unbounded` or
#               `bounded_by(<name>)`, as the report spells it)
#   written     the inventory's `path:line` column
# Honours HALE_BIN (default target/release/hale) and JOBS (default nproc).
unbounded_reports() {
    local me=$1
    shift
    hale=${HALE_BIN:-target/release/hale}
    if [ ! -x "$hale" ]; then
        echo "$me: no hale binary at $hale (build the workspace, or set HALE_BIN)" >&2
        exit 1
    fi
    case "$hale" in
        /*) ;;
        */*) hale="$PWD/$hale" ;;
    esac
    local jobs=${JOBS:-$(nproc 2>/dev/null || echo 4)}

    work=$(mktemp -d "${TMPDIR:-/tmp}/$me.XXXXXX") || exit 1
    # shellcheck disable=SC2064 # expand $work now: the trap outlives this function
    trap "rm -rf '$work'" EXIT

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
        echo "$me: no targets" >&2
        exit 1
    fi

    # One report per program, in parallel: report N holds target N's
    # lines, and N.code its check's exit code.
    mkdir "$work/out" || exit 1
    export UNBOUNDED_REPORTS_OUT="$work/out"
    # shellcheck disable=SC2016 # the inner script expands its own arguments
    nl -ba -w1 -s' ' "$work/targets" | xargs -P "$jobs" -L1 sh -c '
        "$0" check "$2" --report-unbounded-use > "$UNBOUNDED_REPORTS_OUT/$1.raw" 2>/dev/null
        echo $? > "$UNBOUNDED_REPORTS_OUT/$1.code"
        grep "^unbounded-use	" "$UNBOUNDED_REPORTS_OUT/$1.raw" > "$UNBOUNDED_REPORTS_OUT/$1"
        exit 0
    ' "$hale"
    reporting=0
    none=0
    failed=0
    local i=0 t
    while IFS= read -r t; do
        i=$((i + 1))
        if [ ! -f "$work/out/$i.code" ]; then
            echo "$me: $t was not checked" >&2
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
        | sed -E 's/^([^:]*:[0-9]+):.*@(unbounded|bounded_by\([^)]*\))([[:space:]].*)?$/\1\t\2/' \
        | sort -u > "$work/inventory" || exit 1
    cut -f1 "$work/inventory" > "$work/written" || exit 1
}
