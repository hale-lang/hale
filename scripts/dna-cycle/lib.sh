# scripts/dna-cycle/lib.sh — what every phase of the cycle shares. Sourced, never run.
#
# HW        the hale checkout whose target/release/hale the cycle runs (default: this one)
# CYCLE_DIR where runs are kept (default: ${TMPDIR:-/tmp}/dna-cycle); `current` names the run in hand
#
# The organism is the todo repository (dna/tests/onboarding/todo) in local mode, its
# models canned (no tokens): the Leader and the editor answer from canned/rules.txt and
# canned/answers, a leg's model from canned-gateway.py, an OpenAI-compatible server
# over real HTTP.

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
HW=${HW:-$(cd "$HERE/../.." && pwd)}
CYCLE_DIR=${CYCLE_DIR:-${TMPDIR:-/tmp}/dna-cycle}
mkdir -p "$CYCLE_DIR"

# the fixed ports: the todo api's (its compose file says them), the head's, the gateway's
PORTS="8080 8081 8090 8792 8793 8795 8796"
HEAD_API=http://127.0.0.1:8793
GATEWAY=http://127.0.0.1:8796

export PATH="$HW/target/release:$PATH" HALE_BIN="$HW/target/release/hale" HALE_SKIP_STALE_CHECK=1 HALE_DNA_DISCOVER=off
export GIT_CONFIG_NOSYSTEM=1
unset ANTHROPIC_API_KEY OPENAI_API_KEY

current_run() { cat "$CYCLE_DIR/current" 2>/dev/null; }

# A step: a command, PASS or FAIL with its time, in the run's report ($R); its output in $LOG.
step() {
    local name=$1; shift
    local t0; t0=$(date +%s.%N)
    if "$@" >> "$LOG" 2>&1; then
        printf -- '- PASS %s (%.1fs)\n' "$name" "$(echo "$(date +%s.%N) - $t0" | bc)" >> "$R"
    else
        local rc=$?
        printf -- '- FAIL %s (%.1fs) rc=%s\n' "$name" "$(echo "$(date +%s.%N) - $t0" | bc)" "$rc" >> "$R"
        return 1
    fi
}
note() { printf -- '  - %s\n' "$*" >> "$R"; }
# check <what> ok|<anything else> [why]
check() { if [ "$2" = ok ]; then printf -- '- PASS %s\n' "$1" >> "$R"; else printf -- '- FAIL %s: %s\n' "$1" "${3:-}" >> "$R"; fi; }

# decide <label> <review args...>: a verdict that settles; `hale dna review` exits 0 on a refused one,
# so its answer is read
decide() {
    local label=$1; shift; local out
    out=$(timeout 180 hale dna review "$@" 2>&1); echo "$out" >> "$LOG"
    if echo "$out" | grep -q 'settled: approve' && ! echo "$out" | grep -q 'refused'; then check "$label" ok; else check "$label" no "$(echo "$out" | tail -2 | tr '\n' ' ' | cut -c1-240)"; fi
}

# The fixed ports something already listens on, one `<port> <pid> <command>` a line: a cycle
# never takes a port it does not own, and never stops a process it did not start.
ports_taken() {
    for port in $PORTS; do
        for pid in $(ss -ltnp 2>/dev/null | grep ":$port " | grep -o 'pid=[0-9]*' | cut -d= -f2 | sort -u); do
            echo "$port $pid $(ps -o args= -p "$pid" 2>/dev/null | cut -c1-80)"
        done
    done
}
