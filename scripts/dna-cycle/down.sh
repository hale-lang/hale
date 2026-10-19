#!/usr/bin/env bash
# scripts/dna-cycle/down.sh — the cycle's teardown: the run's units stopped, the repository's compose
# project and the run's habitat (under $RUN/data, GH #1653) down with their volumes, the fixed ports
# freed; then what is left is counted in the run's report: the compose projects' containers, the run's
# units, and processes naming the run's directory.
set -u
source "$(dirname "$0")/lib.sh"
RUN=$(current_run); [ -n "$RUN" ] || exit 0
N=${RUN##*-}; R=$RUN/report.md; LOG=$RUN/down.log
for u in $(systemctl --user list-units "dna-cycle-*-$N.service" --all --no-legend --plain | awk '{print $1}'); do systemctl --user stop "$u" 2>/dev/null; done
sleep 2
if [ -d "$RUN/todo" ]; then
    (cd "$RUN/todo" && docker compose down -v >> "$LOG" 2>&1; note "todo compose down rc=$?")
fi
# the run's habitat: every project run from under the run's directory
hale habitat sweep "$RUN" >> "$LOG" 2>&1; note "habitat sweep rc=$?"
sleep 1
containers=0
if [ -d "$RUN/todo" ]; then
    containers=$(cd "$RUN/todo" && docker compose ps -aq 2>/dev/null | wc -l)
fi
containers=$(( containers + $(docker ps -aq --filter label=com.docker.compose.project.working_dir 2>/dev/null | xargs -r docker inspect --format '{{index .Config.Labels "com.docker.compose.project.working_dir"}}' 2>/dev/null | grep -c "^$RUN/") ))
units=$(systemctl --user list-units "dna-cycle-*-$N.service" --no-legend --plain | wc -l)
procs=$(ps -eo pid,args | awk -v r="$RUN" 'index($0, r) && !/awk/' | wc -l)
listening=0; for port in $PORTS; do ss -ltn 2>/dev/null | grep -q ":$port " && listening=$((listening + 1)); done
echo "left $containers $units $procs $listening" > "$RUN/left"
if [ "$containers$units$procs$listening" = 0000 ]; then check "teardown leaves nothing" ok
else check "teardown leaves nothing" no "$containers containers, $units units, $procs processes naming the run, $listening ports listening"; fi
echo "DOWN DONE"; tail -3 "$R"
