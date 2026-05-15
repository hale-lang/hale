#!/usr/bin/env bash
# bench/run.sh — Aperio bench harness.
#
# Builds each bench under bench/micro/ and bench/app/, runs it
# N times, takes the median of elapsed_ns + maxrss_kb, compares
# against the checked-in baseline with a per-bench tolerance
# band, emits a JSON report to bench/results/, and exits
# non-zero on regression.
#
# Comparative timing: for each <name>.ap, the harness also runs
# any sibling <name>.go / <name>.js / <name>.py whose toolchain
# is on PATH. The other-language numbers are emitted alongside
# Aperio's in the JSON report and printed as a ratio_vs_aperio
# line per language. Comparative results are informational only
# — they never gate exit code (per spec/testing.md Layer 3:
# "a regression in aperio-vs-X ratio is a developer signal, not
# a CI gate").
#
# Usage:
#   ./bench/run.sh                     # run all + comparatives, exit on Aperio regression
#   ./bench/run.sh --update-baselines  # overwrite baselines.json with new medians
#   ./bench/run.sh --bench=NAME        # run a single named bench
#   ./bench/run.sh --iters=N           # samples per bench (default 5)
#   ./bench/run.sh --no-build          # skip rebuilding (use stale binaries)
#   ./bench/run.sh --no-comparative    # Aperio only, skip go/node/python siblings
#   ./bench/run.sh --json              # quieter; JSON-only stdout

set -euo pipefail

BENCH_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$BENCH_DIR/.." && pwd)"
APERIO="$REPO_ROOT/target/release/aperio"
BASELINES="$BENCH_DIR/baselines.json"
RESULTS_DIR="$BENCH_DIR/results"

ITERS=5
UPDATE_BASELINES=0
SINGLE_BENCH=""
SKIP_BUILD=0
JSON_ONLY=0
SKIP_COMPARATIVE=0

for arg in "$@"; do
    case "$arg" in
        --update-baselines) UPDATE_BASELINES=1 ;;
        --bench=*)          SINGLE_BENCH="${arg#--bench=}" ;;
        --iters=*)          ITERS="${arg#--iters=}" ;;
        --no-build)         SKIP_BUILD=1 ;;
        --no-comparative)   SKIP_COMPARATIVE=1 ;;
        --json)             JSON_ONLY=1 ;;
        -h|--help)
            sed -n 's/^# \?//p' "${BASH_SOURCE[0]}" | sed -n '1,40p'
            exit 0
            ;;
        *)
            echo "unknown arg: $arg" >&2
            exit 2
            ;;
    esac
done

log() { [ "$JSON_ONLY" -eq 1 ] || echo "$@" >&2; }

# Ensure prerequisites.
command -v jq >/dev/null || { echo "jq not found on PATH" >&2; exit 2; }
command -v /usr/bin/time >/dev/null || { echo "/usr/bin/time not found" >&2; exit 2; }
[ -x "$APERIO" ] || { echo "aperio CLI not built at $APERIO" >&2; echo "run: cargo build --release -p aperio-cli" >&2; exit 2; }

# Comparative toolchain detection — silent if absent. Each entry
# in this map is "lang:cmd" where the command must be on PATH.
declare -A LANG_AVAILABLE
have() { command -v "$1" >/dev/null 2>&1; }
if [ "$SKIP_COMPARATIVE" -eq 0 ]; then
    have go      && LANG_AVAILABLE[go]=1            || true
    have go      && LANG_AVAILABLE[go-idiomatic]=1  || true
    have node    && LANG_AVAILABLE[node]=1          || true
    have python3 && LANG_AVAILABLE[python]=1        || true
fi

mkdir -p "$RESULTS_DIR"

# Discover benches as "kind:relative_path" pairs.
benches=()
for f in "$BENCH_DIR/micro"/*.ap; do
    [ -f "$f" ] || continue
    benches+=("micro:$f")
done
for f in "$BENCH_DIR/app"/*.ap; do
    [ -f "$f" ] || continue
    benches+=("app:$f")
done

# Median of an array of integers.
median() {
    local sorted
    sorted=$(printf '%s\n' "$@" | sort -n)
    local n
    n=$(echo "$sorted" | wc -l)
    local mid=$(( (n + 1) / 2 ))
    echo "$sorted" | sed -n "${mid}p"
}

# Look up a numeric field on a bench in baselines.json. Empty if absent.
baseline_field() {
    local name="$1"; local field="$2"
    [ -f "$BASELINES" ] || { echo ""; return; }
    jq -r --arg n "$name" --arg f "$field" \
        '.benches[$n][$f] // empty' "$BASELINES" 2>/dev/null
}

# Run a binary N times with /usr/bin/time -v. Populates globals:
#   _RUN_STATUS         — "ok" or "fail"
#   _RUN_ELAPSED_MEDIAN — median elapsed_ns
#   _RUN_MAXRSS_MEDIAN  — median maxrss_kb
#   _RUN_ELAPSED_JSON   — JSON array of samples
#   _RUN_MAXRSS_JSON    — JSON array of samples
# The binary must print exactly one `elapsed_ns=N` line on stdout.
time_binary() {
    local bin="$1"
    local n_iters="$2"
    shift 2
    local prefix_args=("$@")   # optional argv prefix (e.g. interpreter + script)

    local elapsed_samples=()
    local maxrss_samples=()
    local time_out
    for ((__r=1; __r<=n_iters; __r++)); do
        time_out=$(mktemp)
        if ! /usr/bin/time -f "__BENCH_TIME__ wall=%e maxrss=%M" \
                "${prefix_args[@]}" "$bin" >"$time_out.out" 2>"$time_out.err"; then
            _RUN_STATUS="fail"
            cat "$time_out.err" >&2
            rm -f "$time_out" "$time_out.out" "$time_out.err"
            return 0
        fi
        local elapsed maxrss
        elapsed=$(grep -oE 'elapsed_ns=[0-9]+' "$time_out.out" | head -1 | cut -d= -f2 || true)
        maxrss=$(grep -oE '__BENCH_TIME__ wall=[0-9.]+ maxrss=[0-9]+' "$time_out.err" | grep -oE 'maxrss=[0-9]+' | cut -d= -f2 || true)
        rm -f "$time_out" "$time_out.out" "$time_out.err"
        if [ -z "$elapsed" ] || [ -z "$maxrss" ]; then
            _RUN_STATUS="fail"
            return 0
        fi
        elapsed_samples+=("$elapsed")
        maxrss_samples+=("$maxrss")
    done

    _RUN_STATUS="ok"
    _RUN_ELAPSED_MEDIAN=$(median "${elapsed_samples[@]}")
    _RUN_MAXRSS_MEDIAN=$(median "${maxrss_samples[@]}")
    _RUN_ELAPSED_JSON=$(printf '%s\n' "${elapsed_samples[@]}" | jq -s .)
    _RUN_MAXRSS_JSON=$(printf '%s\n' "${maxrss_samples[@]}" | jq -s .)
}

# Per-run results gathered as one JSON object per bench.
results_json="[]"
regression_count=0

for entry in "${benches[@]}"; do
    kind="${entry%%:*}"
    src="${entry#*:}"
    name="$(basename "$src" .ap)"
    src_dir="$(dirname "$src")"

    if [ -n "$SINGLE_BENCH" ] && [ "$name" != "$SINGLE_BENCH" ]; then
        continue
    fi

    bin="${src%.ap}"

    # Build the Aperio binary (or trust an existing one).
    if [ "$SKIP_BUILD" -eq 0 ]; then
        log "[$kind/$name] building"
        if ! APERIO_SKIP_STALE_CHECK=1 "$APERIO" build "$src" >/dev/null 2>&1; then
            log "[$kind/$name] APERIO BUILD FAILED — skipping"
            results_json=$(jq --arg n "$name" --arg k "$kind" \
                '. + [{name: $n, kind: $k, status: "build_failed"}]' \
                <<<"$results_json")
            continue
        fi
    fi

    if [ ! -x "$bin" ]; then
        log "[$kind/$name] aperio binary missing at $bin — skipping"
        continue
    fi

    # Time the Aperio binary.
    time_binary "$bin" "$ITERS"
    if [ "$_RUN_STATUS" != "ok" ]; then
        log "[$kind/$name] APERIO RUN FAILED"
        results_json=$(jq --arg n "$name" --arg k "$kind" \
            '. + [{name: $n, kind: $k, status: "run_failed"}]' \
            <<<"$results_json")
        continue
    fi

    aperio_elapsed="$_RUN_ELAPSED_MEDIAN"
    aperio_maxrss="$_RUN_MAXRSS_MEDIAN"
    aperio_elapsed_json="$_RUN_ELAPSED_JSON"
    aperio_maxrss_json="$_RUN_MAXRSS_JSON"

    # Compare against baseline.
    baseline_elapsed=$(baseline_field "$name" "elapsed_ns")
    baseline_maxrss=$(baseline_field "$name" "maxrss_kb")
    tolerance=$(baseline_field "$name" "tolerance")
    # Default tolerance is wide: small-bench OS jitter routinely
    # swings sub-10ms medians by 20%+. Tighten per-bench in
    # baselines.json once a metric stabilizes.
    [ -z "$tolerance" ] && tolerance="0.30"
    status="ok"
    regression_note=""

    if [ -n "$baseline_elapsed" ] && [ "$UPDATE_BASELINES" -eq 0 ]; then
        if awk -v cur="$aperio_elapsed" -v base="$baseline_elapsed" -v tol="$tolerance" \
            'BEGIN { exit (cur > base * (1.0 + tol)) ? 0 : 1 }'; then
            status="regression"
            pct=$(awk -v cur="$aperio_elapsed" -v base="$baseline_elapsed" \
                'BEGIN { printf "%.1f", (cur/base - 1.0) * 100.0 }')
            regression_note="elapsed_ns ${aperio_elapsed} > baseline ${baseline_elapsed} (+${pct}%, tol ${tolerance})"
            regression_count=$((regression_count + 1))
            log "[$kind/$name] REGRESSION: $regression_note"
        fi
    fi

    log "[$kind/$name] aperio  elapsed_ns=$aperio_elapsed maxrss_kb=$aperio_maxrss status=$status"

    # Comparatives: for each language with sibling source AND
    # toolchain present, build (Go) and run N times.
    # `go-idiomatic` looks for <stem>.idiomatic.go and gets the
    # same build path as plain `go`. Benches that don't ship an
    # idiomatic.go sibling silently skip this column.
    comparatives_json="{}"
    for lang in go go-idiomatic node python; do
        [ -n "${LANG_AVAILABLE[$lang]:-}" ] || continue
        case "$lang" in
            go)            ext="go" ;;
            go-idiomatic)  ext="idiomatic.go" ;;
            node)          ext="js" ;;
            python)        ext="py" ;;
        esac
        sibling="$src_dir/$name.$ext"
        [ -f "$sibling" ] || continue

        if [ "$lang" = "go" ] || [ "$lang" = "go-idiomatic" ]; then
            go_bin="$src_dir/${name}.${ext}.bin"
            if [ "$SKIP_BUILD" -eq 0 ]; then
                if ! ( cd "$src_dir" && go build -o "$go_bin" "$name.$ext" >/dev/null 2>&1 ); then
                    log "[$kind/$name] $lang BUILD FAILED — skipping"
                    continue
                fi
            fi
            [ -x "$go_bin" ] || { log "[$kind/$name] $lang binary missing — skipping"; continue; }
            time_binary "$go_bin" "$ITERS"
        elif [ "$lang" = "node" ]; then
            time_binary "$sibling" "$ITERS" node
        elif [ "$lang" = "python" ]; then
            time_binary "$sibling" "$ITERS" python3
        fi

        if [ "$_RUN_STATUS" != "ok" ]; then
            log "[$kind/$name] $lang RUN FAILED — skipping"
            continue
        fi

        lang_elapsed="$_RUN_ELAPSED_MEDIAN"
        lang_maxrss="$_RUN_MAXRSS_MEDIAN"
        lang_elapsed_json="$_RUN_ELAPSED_JSON"
        lang_maxrss_json="$_RUN_MAXRSS_JSON"

        # ratio_vs_aperio = lang_elapsed / aperio_elapsed.
        # < 1.0 means this language is faster than Aperio.
        # > 1.0 means Aperio is faster than this language.
        ratio=$(awk -v lang="$lang_elapsed" -v ap="$aperio_elapsed" \
            'BEGIN { if (ap == 0) print "null"; else printf "%.4f", lang / ap }')
        log "[$kind/$name] $(printf '%-14s' "$lang:") elapsed_ns=$(printf '%-14s' "$lang_elapsed") ratio_vs_aperio=${ratio}x"

        comparatives_json=$(jq \
            --arg lang "$lang" \
            --argjson em "$lang_elapsed" --argjson rm "$lang_maxrss" \
            --argjson es "$lang_elapsed_json" --argjson rs "$lang_maxrss_json" \
            --argjson ratio "$ratio" \
            '. + {($lang): {
                elapsed_ns_median: $em, elapsed_ns_samples: $es,
                maxrss_kb_median: $rm, maxrss_kb_samples: $rs,
                ratio_vs_aperio: $ratio
            }}' \
            <<<"$comparatives_json")
    done

    results_json=$(jq \
        --arg n "$name" --arg k "$kind" --arg s "$status" \
        --arg note "$regression_note" \
        --argjson em "$aperio_elapsed" --argjson rm "$aperio_maxrss" \
        --argjson es "$aperio_elapsed_json" \
        --argjson rs "$aperio_maxrss_json" \
        --argjson be "${baseline_elapsed:-null}" \
        --argjson br "${baseline_maxrss:-null}" \
        --argjson comps "$comparatives_json" \
        '. + [{
            name: $n, kind: $k, status: $s,
            elapsed_ns_median: $em, elapsed_ns_samples: $es,
            maxrss_kb_median: $rm, maxrss_kb_samples: $rs,
            baseline_elapsed_ns: $be, baseline_maxrss_kb: $br,
            note: (if $note == "" then null else $note end),
            comparatives: $comps
        }]' \
        <<<"$results_json")
done

# Write report.
timestamp=$(date -u +%Y-%m-%dT%H:%M:%SZ)
report=$(jq --arg t "$timestamp" --argjson iters "$ITERS" \
    '{generated_at: $t, iters: $iters, benches: .}' <<<"$results_json")

results_path="$RESULTS_DIR/run-$(date -u +%Y%m%d-%H%M%S).json"
echo "$report" > "$results_path"

if [ "$JSON_ONLY" -eq 1 ]; then
    echo "$report"
else
    log ""
    log "Report: $results_path"
fi

# Update baselines if requested. Comparative numbers are NOT
# baselined — they're informational only.
if [ "$UPDATE_BASELINES" -eq 1 ]; then
    log ""
    log "Updating $BASELINES"
    existing_benches="{}"
    if [ -f "$BASELINES" ]; then
        existing_benches=$(jq '.benches // {}' "$BASELINES")
    fi
    new_benches=$(jq --argjson existing "$existing_benches" \
        '[.benches[] | select(.status == "ok") | {
            (.name): ({
                kind: .kind,
                elapsed_ns: .elapsed_ns_median,
                maxrss_kb: .maxrss_kb_median,
                tolerance: ($existing[.name].tolerance // 0.30)
            })
        }] | add // {}' \
        <<<"$report")
    jq -n --argjson b "$new_benches" --arg t "$timestamp" \
        '{updated_at: $t, benches: $b}' > "$BASELINES"
    log "Baselines updated."
fi

# Exit non-zero on Aperio regression (comparative numbers never gate).
if [ "$UPDATE_BASELINES" -eq 0 ] && [ "$regression_count" -gt 0 ]; then
    log ""
    log "FAILED: $regression_count regression(s) detected"
    exit 1
fi
