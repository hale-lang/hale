#!/usr/bin/env bash
# scripts/dna-cycle/cycles.sh — N cycles from scratch (default 3): up, walk, down, each under a memory
# watchdog (a process of the cycle past LIMIT_MB, default 6000, stops the run's units). One line per
# cycle in $CYCLE_DIR/cycles.md, with every failing step under it; exit 0 only when every cycle is
# green from up to teardown.
#
#   cargo build --release && scripts/dna-cycle/cycles.sh 3
#
# Needs docker (compose), systemd --user, python3 (with yaml, for local-ci.py) and a release build of
# this checkout (the whole workspace). Nothing is spent: every model answer is canned.
set -u
source "$(dirname "$0")/lib.sh"
N=${1:-3}; LIMIT_MB=${LIMIT_MB:-6000}; out=$CYCLE_DIR/cycles.md; green=0
# the cycles run a snapshot of these scripts: editing the checkout's copy while a cycle reads it
# would change the lines bash has yet to read
export HW
SNAP=$CYCLE_DIR/scripts-$(date +%s); cp -r "$HERE" "$SNAP"; HERE=$SNAP
echo "# cycles $(date +%H:%M) on $(git -C "$HW" rev-parse --short HEAD)" >> "$out"
for i in $(seq 1 "$N"); do
    peak=$CYCLE_DIR/cycle-$i.peak; echo "0 -" > "$peak"; rm -f "$CYCLE_DIR/cycle-$i.watchdog"
    (
        while true; do
            top=$(ps -eo rss,comm --sort=-rss | awk 'NR==2{print int($1/1024)" "$2}'); mb=${top%% *}
            [ "$mb" -gt "$(cut -d' ' -f1 "$peak")" ] && echo "$top" > "$peak"
            if [ "$mb" -gt "$LIMIT_MB" ]; then
                echo "WATCHDOG: $top MB over $LIMIT_MB" >> "$CYCLE_DIR/cycle-$i.watchdog"
                for u in $(systemctl --user list-units 'dna-cycle-*' --no-legend --plain | awk '{print $1}'); do systemctl --user stop "$u"; done
            fi
            sleep 3
        done
    ) & WD=$!
    t0=$(date +%s)
    timeout 1800 bash "$HERE/up.sh" > "$CYCLE_DIR/cycle-$i-up.out" 2>&1
    timeout 3000 bash "$HERE/walk.sh" > "$CYCLE_DIR/cycle-$i-walk.out" 2>&1
    t1=$(date +%s)
    bash "$HERE/down.sh" > "$CYCLE_DIR/cycle-$i-down.out" 2>&1
    kill "$WD" 2>/dev/null; wait "$WD" 2>/dev/null
    RUN=$(current_run)
    up_pass=$(grep -c '^- PASS' "$RUN/report.md"); up_fail=$(grep -c '^- FAIL' "$RUN/report.md")
    w_pass=$(grep -c '^- PASS' "$RUN/walk.md" 2>/dev/null); w_fail=$(grep -c '^- FAIL' "$RUN/walk.md" 2>/dev/null); w_pass=${w_pass:-0}; w_fail=${w_fail:-0}
    [ -f "$RUN/walk.md" ] || w_fail=1
    printf -- '- cycle %s: up+down %s pass %s fail, walk %s pass %s fail, %ss up+walk, peak %s MB (%s), watchdog %s; %s\n' \
        "$i" "$up_pass" "$up_fail" "$w_pass" "$w_fail" "$((t1 - t0))" "$(cut -d' ' -f1 "$peak")" "$(cut -d' ' -f2 "$peak")" \
        "$( [ -f "$CYCLE_DIR/cycle-$i.watchdog" ] && echo FIRED || echo no)" "$RUN" >> "$out"
    if [ "$up_fail" = 0 ] && [ "$w_fail" = 0 ]; then green=$((green + 1)); else grep -h '^- FAIL' "$RUN/report.md" "$RUN/walk.md" 2>/dev/null | cut -c1-300 >> "$out"; fi
done
echo "- $green of $N green" >> "$out"
tail -n $((N * 4 + 2)) "$out"
[ "$green" = "$N" ]
