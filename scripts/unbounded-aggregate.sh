#!/usr/bin/env bash
# Sum, per `@unbounded` and `@bounded_by(<name>)` decorator written in
# dna/ and in the stdlib's own source (crates/hale-stdlib/hl/), what it
# silences across every program that reaches it (GH #1490).
#
# `hale check --report-unbounded-use` prints, for one program, each
# decorated fn it reaches with the memory-bound survey's sites and the
# hot-path allocation lint's warnings the decorator silences there.
# DNA's core is compiled into many programs, and what a decorator
# silences depends on the program, so this adds the program's counts
# up. scripts/unbounded-unused.sh answers the narrower question (which
# decorators silence nothing in every program) over the same programs;
# both pick them through scripts/unbounded-reports.sh: every DNA target,
# the corpus and tests/hale (the stdlib has no `.hl` programs of its
# own; those reach it). Pass targets as arguments to sum over those
# instead.
#
# Per decorator: its place (`path:line`, relative to the repository
# root), its fn as the report spells it, its kind, the programs that
# reach it and how many of those it silences something in, and the
# survey sites and hot-path warnings it silences summed over them. A
# decorator no program reaches is unjudged and counts as neither
# silencing nothing nor something.
#
# Output, on stdout: totals by kind (and by tree: dna/, dna/'s tests,
# the stdlib), how many silence nothing anywhere and how many silence
# something (with the sites and warnings they silence), how the
# survey sites concentrate, the top 20 by survey sites silenced, the
# decorators that silence nothing, the unjudged ones, and every
# decorator by place. A plain aligned table by default; `--markdown`
# prints GitHub markdown for posting, the long lists folded. On stderr:
# the programs that did not check and reported nothing (`unreported`)
# and any reported line no decorator is written on (`stray`, none
# expected).
#
# Usage:
#   scripts/unbounded-aggregate.sh [--markdown] [target ...]
#   HALE_BIN=path/to/hale JOBS=8 scripts/unbounded-aggregate.sh
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

format=plain
if [ "${1:-}" = "--markdown" ]; then
    format=markdown
    shift
fi
case "${1:-}" in
    -h|--help)
        sed -n '2,/^set -uo/p' "$0" | sed '$d; s/^# \{0,1\}//'
        exit 0
        ;;
    -*)
        echo "usage: scripts/unbounded-aggregate.sh [--markdown] [target ...]" >&2
        exit 2
        ;;
esac

# shellcheck source=scripts/unbounded-reports.sh
. scripts/unbounded-reports.sh || exit 1
unbounded_reports unbounded-aggregate "$@"

# One row per decorator in the inventory:
# place, fn, kind, tree, reaching, using, survey, hot.
cat "$work"/out/*[0-9] 2>/dev/null | awk -F'\t' -v OFS='\t' -v inventory="$work/inventory" -v stray="$work/stray" '
    $1 == "unbounded-use" {
        k = $2
        if (!(k in name)) name[k] = $3
        # A report before GH #1490 had no sixth field: `@unbounded`.
        if (!(k in kind)) kind[k] = ($6 == "" ? "unbounded" : $6)
        reaching[k]++
        if ($4 + $5 > 0) using[k]++
        survey[k] += $4
        hot[k] += $5
    }
    END {
        while ((getline line < inventory) > 0) {
            split(line, f, "\t")
            k = f[1]
            written[k] = 1
            if (k ~ /^crates\/hale-stdlib\/hl\//) tree = "stdlib"
            else if (k ~ /(^|\/)tests\// || k ~ /_test\.hl:/) tree = "dna/ tests"
            else tree = "dna/"
            print k, (k in name ? name[k] : "-"), (k in kind ? kind[k] : f[2]), tree, reaching[k] + 0, using[k] + 0, survey[k] + 0, hot[k] + 0
        }
        for (k in name) {
            if (k in written) continue
            # The report names the decorators of every program it checks;
            # only those written in dna/ and the stdlib are summed here.
            if (k ~ /^dna\// || k ~ /^crates\/hale-stdlib\/hl\//) print "stray", k > stray
        }
    }
' > "$work/rows" || exit 1
sort -t: -k1,1 -k2,2n "$work/rows" > "$work/byplace" || exit 1
sort -t"$(printf '\t')" -k7,7nr -k8,8nr -k1,1 "$work/rows" > "$work/bysurvey" || exit 1

# The report as tagged lines, rendered below: `H` a heading, `P` a
# paragraph, `T` a table's header, `R` its row, `E` its end, `D`/`/D`
# a folded block (markdown) around a list.
awk -F'\t' -v OFS='\t' -v ntargets="$ntargets" -v reporting="$reporting" -v none="$none" -v failed="$failed" '
    function pct(a, b) { return b == 0 ? "0.0%" : sprintf("%.1f%%", 100 * a / b) }
    function cls(kind) { return kind ~ /^bounded_by\(/ ? "@bounded_by" : "@unbounded" }
    function add(g) {
        n[g]++
        if ($5 == 0) unjudged[g]++
        else if ($6 == 0) nothing[g]++
        else { something[g]++; s[g] += $7; h[g] += $8 }
        if (!(g in seen)) { seen[g] = 1; order[++ng] = g }
    }
    function totals(g, label) {
        print "R", label, n[g] + 0, unjudged[g] + 0, nothing[g] + 0, something[g] + 0, s[g] + 0, h[g] + 0
    }
    function row(r, rank) {
        split(r, f, "\t")
        print "R", (rank == "" ? "" : rank OFS) "`" f[1] "`", "`" f[2] "`", "`" f[3] "`", f[6] "/" f[5], f[7], f[8]
    }
    # bysurvey first: the totals, the concentration and the top 20.
    FILENAME == ARGV[1] {
        bys[++nb] = $0
        add("all")
        add(cls($3))
        add(cls($3) ", " $4)
        totsurvey += $7
        next
    }
    { byp[++np] = $0 }
    END {
        print "H", "Programs"
        print "P", ntargets " programs checked: " reporting " reach a decorator, " none " reach none, " failed " did not check (listed on stderr)."
        print "H", "Totals"
        print "T", "group", "decorators", "unjudged", "silence nothing", "silence something", "survey sites", "hot-path warnings"
        for (i = 1; i <= ng; i++) {
            g = order[i]
            if (g == "all") continue
            if (g ~ /, /) continue
            totals(g, "`" g "`")
            for (j = 1; j <= ng; j++) {
                t = order[j]
                if (index(t, g ", ") == 1) totals(t, "`" g "`, " substr(t, length(g) + 3))
            }
        }
        totals("all", "all")
        print "E"
        judged = n["all"] - unjudged["all"]
        print "P", nothing["all"] + 0 " of " judged " judged decorators (" pct(nothing["all"], judged) ") silence nothing in any program; " something["all"] + 0 " (" pct(something["all"], judged) ") silence " s["all"] + 0 " survey sites and " h["all"] + 0 " hot-path warnings, summed over the programs that reach them. Reached by no program: " unjudged["all"] + 0 "."
        acc = 0; half = 0; most = 0; top = 0
        for (i = 1; i <= nb; i++) {
            split(bys[i], f, "\t")
            acc += f[7]
            if (i <= 20) top = acc
            if (!half && totsurvey > 0 && acc * 2 >= totsurvey) half = i
            if (!most && totsurvey > 0 && acc * 10 >= totsurvey * 8) most = i
        }
        if (totsurvey > 0)
            print "P", "Concentration: the top 20 silence " top " of " totsurvey " survey sites (" pct(top, totsurvey) "); the top " half " silence half of them, the top " most " (" pct(most, judged) " of the judged) 80%."
        print "H", "Top 20 by survey sites silenced"
        print "T", "#", "place", "fn", "kind", "programs (silencing/reaching)", "survey sites", "hot-path warnings"
        for (i = 1; i <= nb && i <= 20; i++) row(bys[i], i)
        print "E"
        print "D", "Silence nothing in any program (" nothing["all"] + 0 ")"
        if (nothing["all"] + 0 > 0) {
            print "T", "place", "fn", "kind", "programs (silencing/reaching)", "survey sites", "hot-path warnings"
            for (i = 1; i <= np; i++) { split(byp[i], f, "\t"); if (f[5] > 0 && f[6] == 0) row(byp[i], "") }
            print "E"
        } else print "P", "None."
        print "/D"
        print "D", "Reached by no program (" unjudged["all"] + 0 ")"
        if (unjudged["all"] + 0 > 0) {
            print "T", "place", "fn", "kind", "programs (silencing/reaching)", "survey sites", "hot-path warnings"
            for (i = 1; i <= np; i++) { split(byp[i], f, "\t"); if (f[5] == 0) row(byp[i], "") }
            print "E"
        } else print "P", "None."
        print "/D"
        print "D", "Every decorator, by place (" n["all"] + 0 ")"
        print "T", "place", "fn", "kind", "programs (silencing/reaching)", "survey sites", "hot-path warnings"
        for (i = 1; i <= np; i++) row(byp[i], "")
        print "E"
        print "/D"
    }
' "$work/bysurvey" "$work/byplace" > "$work/report" || exit 1

awk -F'\t' -v format="$format" '
    # Plain: backticks dropped, tables aligned, numbers right-aligned.
    function flush(   i, j, line, cell) {
        for (i = 1; i <= nrows; i++) {
            line = ""
            for (j = 1; j <= ncols; j++) {
                cell = cells[i, j]
                if (j > 1) line = line "  "
                if (cells[2, j] ~ /^[0-9]+(\/[0-9]+)?$/) line = line sprintf("%" width[j] "s", cell)
                else line = line sprintf("%-" width[j] "s", cell)
            }
            sub(/ +$/, "", line)
            print line
        }
        nrows = 0
    }
    function md(   i, j, line) {
        for (i = 1; i <= nrows; i++) {
            line = "|"
            for (j = 1; j <= ncols; j++) line = line " " cells[i, j] " |"
            print line
            if (i == 1) {
                line = "|"
                for (j = 1; j <= ncols; j++) line = line (j == 1 || cells[2, j] !~ /^[0-9]+(\/[0-9]+)?$/ ? " --- |" : " ---: |")
                print line
            }
        }
        nrows = 0
    }
    function plain(s) { gsub(/`/, "", s); return s }
    $1 == "H" { if (format == "markdown") print "### " $2 "\n"; else print "== " $2 " =="; next }
    $1 == "P" { if (format == "markdown") print $2 "\n"; else print plain($2) "\n"; next }
    $1 == "D" { if (format == "markdown") print "<details><summary>" $2 "</summary>\n"; else print "== " $2 " =="; next }
    $1 == "/D" { if (format == "markdown") print "</details>\n"; next }
    $1 == "T" || $1 == "R" {
        if ($1 == "T") { nrows = 0; ncols = NF - 1; for (j = 1; j <= ncols; j++) width[j] = 0 }
        nrows++
        for (j = 1; j <= ncols; j++) {
            c = (format == "markdown" ? $(j + 1) : plain($(j + 1)))
            cells[nrows, j] = c
            if (length(c) > width[j]) width[j] = length(c)
        }
        next
    }
    $1 == "E" { if (format == "markdown") md(); else flush(); print ""; next }
' "$work/report" || exit 1

if [ -s "$work/failed" ]; then
    cat "$work/failed" >&2
fi
if [ -s "$work/stray" ]; then
    # A reported line no decorator is written on: the report and the
    # inventory disagree about where a decorator is.
    cat "$work/stray" >&2
fi
exit 0
