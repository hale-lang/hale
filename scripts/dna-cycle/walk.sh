#!/usr/bin/env bash
# scripts/dna-cycle/walk.sh — the cycle's walk over the organism up.sh left running
# ($CYCLE_DIR/current), each step checked against what it must come to (notes/process-harness-plan.md,
# "Skeleton"):
#
#   1. a position opened through the operation layer (`hale dna position open`), with its mandate,
#      ratified and filled, and a model-mapping default for it;
#   2. a feature ask the organization's own editor delivers (the Leader's plan and the editor's edit
#      canned), reviewed, checked and applied;
#   3. an ask routed to the position as a Patch Work, done by a leg running its own tool loop (the hat,
#      its hands, a model through the canned OpenAI-compatible gateway), reviewed, checked and applied;
#   4. a second one done by a scripted person session over `hale mcp`, applied the same way;
#   5. an Assessment Work done by a leg (a `tools` Work: its hands are the record's reads);
#   6. `hale dna ingest` at the later commit, filing the graph's difference, the application's tool
#      nodes among it, as one reviewed proposal the Board ratifies.
#
# The teardown is down.sh's.
#
# The canned answers (canned/): rules.txt is FakeModel's `role|needle|answer` lines (first match wins);
# answers/api/todo.hl is the whole file the editor's canned edit returns: the fixture's own api/todo.hl
# with the overdue sweep at two seconds. When dna/tests/onboarding/todo/api/todo.hl changes, so does it.
set -u
source "$(dirname "$0")/lib.sh"
RUN=$(current_run); APP=$RUN/todo; R=$RUN/walk.md; LOG=$RUN/walk.log; C=$HERE/canned
export XDG_STATE_HOME=$RUN/state HALE_DNA_API=$HEAD_API HALE_DNA_ID_TOKEN=$(cat "$RUN/id_token" 2>/dev/null)
# the head's memory, which `show org` reads the graph from (up.sh's `memory migrate` printed it)
HALE_DNA_MEMORY_DSN_HEAD=$(grep -o 'HALE_DNA_MEMORY_DSN_HEAD=[^ ]*' "$RUN/migrate.out" | head -1 | cut -d= -f2-); export HALE_DNA_MEMORY_DSN_HEAD
cd "$APP" || exit 1
POSITION=position:api/steward

rows() { git -C "$APP" show refs/dna/journal:journal.jsonl 2>/dev/null; }
# first <kind> [after] [needle]: the first row of a kind after row <after> whose entity or body holds <needle>
first() { rows | python3 -c 'import sys,json
k=sys.argv[1]; after=int(sys.argv[2]); needle=sys.argv[3]
for i,l in enumerate(sys.stdin):
    if i < after: continue
    try: e=json.loads(l)
    except Exception: continue
    if e.get("kind")==k and needle in (e.get("entity","")+" "+json.dumps(e.get("body",""))): print(e.get("entity")+"\t"+json.dumps(e.get("body",""))[:400]); break' "$1" "${2:-0}" "${3:-}"; }
wait_for() { local kind=$1 after=$2 secs=$3 needle=${4:-} t0; t0=$(date +%s); while [ $(( $(date +%s) - t0 )) -lt "$secs" ]; do hale dna sync >/dev/null 2>&1; r=$(first "$kind" "$after" "$needle"); [ -n "$r" ] && { echo "$r"; return 0; }; sleep 2; done; return 1; }
last_rows() { rows | tail -"${1:-4}" | python3 -c 'import sys,json
for l in sys.stdin:
    try: e=json.loads(l); print(e.get("kind"), e.get("entity"))
    except Exception: pass' | tr '\n' '|'; }
json_field() { python3 -c 'import sys,json
try: d=json.load(sys.stdin)
except Exception: sys.exit(0)
for k in sys.argv[1].split("."): d=d.get(k,{}) if isinstance(d,dict) else {}
print(d if not isinstance(d,(dict,list)) else json.dumps(d))' "$1"; }

# land <label> <after> <file> <needle>: the candidate after row <after> is routed for Review, signed,
# checked by the local CI and applied, and the working tree's <file> carries <needle>
land() {
    local label=$1 after=$2 file=$3 needle=$4 m sha v id out ci
    m=$(wait_for mutation.candidate "$after" 300) && check "$label: a candidate" ok || { check "$label: a candidate" no "no mutation.candidate in 300 s; last rows: $(last_rows)"; return 1; }
    sha=$(echo "$m" | grep -o -E '[0-9a-f]{40}' | head -1)
    if [ -n "$sha" ] && git -C "$APP" show "$sha:$file" 2>/dev/null | grep -q -F "$needle"; then check "$label: the candidate carries the change" ok; else check "$label: the candidate carries the change" no "commit '$sha', $file without '$needle'"; fi
    for _ in $(seq 1 24); do v=$(timeout 60 hale dna review 2>&1); echo "$v" | grep -q -E '^  m[0-9]+ needs' && break; sleep 5; done
    id=$(echo "$v" | grep -o -E '^  m[0-9]+' | head -1 | tr -d ' ')
    if [ -n "$id" ]; then out=$(timeout 180 hale dna review "$id" approve --as riley 2>&1 | tail -1); note "$label: approve $id: $(echo "$out" | cut -c1-160)"; check "$label: the Review is routed and signed" ok
    else check "$label: the Review is routed and signed" no "no pending mutation Review: $(echo "$v" | head -3 | tr '\n' '|')"; return 1; fi
    for _ in 1 2 3 4 5 6; do ci=$(python3 "$HERE/local-ci.py" "$APP" 2>&1); [ "$ci" != "local-ci: nothing to run" ] && { note "$label: local CI: $(echo "$ci" | tr '\n' '|' | cut -c1-240)"; break; }; sleep 10; done
    wait_for mutation.applied "$after" 300 > /dev/null && check "$label: applied to the genome" ok || { check "$label: applied to the genome" no "no mutation.applied in 300 s; last rows: $(last_rows)"; return 1; }
    grep -q -F "$needle" "$APP/$file" && check "$label: the working tree carries it" ok || check "$label: the working tree carries it" no "$file lacks '$needle'"
}

echo "# dna cycle walk $(date +%H:%M)" > "$R"

# ---- 1. a position, opened through the operation layer, with its model default. A solo organization:
# riley asks and, holding the Board since up.sh's fills, decides (a Board Review admits its asker)
base=$(rows | wc -l)
out=$(timeout 120 hale dna position open api/steward --under api --text "keeps the list's api" --mandate "decides how the list's api reads; may not change the browser; cites the README's axioms; escalates a new surface to the Board" --as riley 2>&1); note "position open: $(echo "$out" | tail -1 | cut -c1-200)"
echo "$out" | grep -q "proposed opening $POSITION" && check "1: the opening is proposed" ok || check "1: the opening is proposed" no "$(echo "$out" | tail -2 | tr '\n' ' ')"
for _ in $(seq 1 24); do hale dna review 2>&1 | grep -q '^  positions —' && break; sleep 5; done
decide "1: the Board ratifies the opening" positions approve --as riley
out=$(timeout 120 hale dna fill api/steward riley --as riley 2>&1); note "fill: $(echo "$out" | tail -1 | cut -c1-200)"
for _ in $(seq 1 24); do hale dna review 2>&1 | grep -q 'holding the position `api/steward`' && break; sleep 5; done
decide "1: the Board ratifies riley holding it" holds approve --as riley
# memory projects the record a few rows behind it: the chart is read until it shows the position
for _ in $(seq 1 30); do hale dna show org > "$RUN/org2.out" 2>&1; grep -q 'steward' "$RUN/org2.out" && break; sleep 2; done
grep -q 'steward' "$RUN/org2.out" && check "1: the org chart shows the position" ok || check "1: the org chart shows the position" no "$(head -5 "$RUN/org2.out" | tr '\n' '|')"
step "1: models rule $POSITION deep" hale dna models rule "$POSITION" deep --as riley
step "1: models category do.deep" hale dna models category do.deep canned-deep --tools --as riley
step "1: models category tools.standard" hale dna models category tools.standard canned-standard --tools --as riley
hale dna models map > "$RUN/models-map.out" 2>&1; grep -q "$POSITION" "$RUN/models-map.out" && grep -q 'canned-deep' "$RUN/models-map.out" && check "1: the mapping lists the default" ok || check "1: the mapping lists the default" no "$(head -6 "$RUN/models-map.out" | tr '\n' '|')"

# ---- 2. path A: the organization's own editor
base=$(rows | wc -l)
out=$(timeout 120 hale dna task create --as riley slow the overdue sweep to every two seconds 2>&1 | tail -2); note "A: ask: $(echo "$out" | tr '\n' ' ' | cut -c1-200)"
wait_for task.born "$base" 120 > /dev/null && check "A: the ask bears a Task" ok || check "A: the ask bears a Task" no "no task.born in 120 s"
p=$(wait_for model.called "$base" 60); echo "$p" | grep -q 'fake' && check "A: the Leader's plan came from the canned model" ok || check "A: the Leader's plan came from the canned model" no "$(echo "$p" | cut -c1-120)"
land A "$base" api/todo.hl 'sleep(2s)'

# ---- 3. path B: a leg's own tool loop, through the gateway
base=$(rows | wc -l); calls0=$(wc -l < "$RUN/gateway.jsonl")
out=$(timeout 120 hale dna task create --as riley --to "$POSITION" "leave the steward's note on the list's api" 2>&1 | tail -2); note "B: ask: $(echo "$out" | tr '\n' ' ' | cut -c1-200)"
wait_for attempt.admitted "$base" 120 "$POSITION" > /dev/null && check "B: admitted as the position's Work" ok || check "B: admitted as the position's Work" no "no attempt.admitted naming $POSITION; last rows: $(last_rows)"
out=$(timeout 600 hale dna work run --as "$POSITION" --performer model 2>&1); echo "$out" >> "$LOG"; note "B: run: $(echo "$out" | tail -1 | cut -c1-240)"
echo "$out" | grep -q '"state": "\(requested\|submitted\|settled\)"' && check "B: the leg's loop hands the change back" ok || check "B: the leg's loop hands the change back" no "$(echo "$out" | tail -2 | tr '\n' ' ' | cut -c1-300)"
calls=$(tail -n +"$((calls0 + 1))" "$RUN/gateway.jsonl")
python3 - "$calls" <<'EOF' > "$RUN/gateway-b.txt"
import json, sys
calls = [json.loads(l) for l in sys.argv[1].splitlines() if l.strip()]
patch = [c for c in calls if c["scenario"] == "patch"]
print("turns", len(patch))
print("models", sorted({c["model"] for c in patch}))
print("keys", bool(patch) and all(c["idempotency_key"] for c in patch), "authorized", bool(patch) and all(c["authorized"] for c in patch))
print("metadata", bool(patch) and all(c["metadata"].get("attempt") or c["metadata"].get("attempt_id") for c in patch))
print("tools", sorted({t for c in patch for t in c["tools"]}))
EOF
note "B: gateway: $(tr '\n' ';' < "$RUN/gateway-b.txt")"
grep -q '^turns 3' "$RUN/gateway-b.txt" && check "B: three turns crossed the gateway" ok || check "B: three turns crossed the gateway" no "$(head -1 "$RUN/gateway-b.txt")"
grep -q "^models \['canned-deep'\]" "$RUN/gateway-b.txt" && check "B: each asked for the model the mapping names" ok || check "B: each asked for the model the mapping names" no "$(grep '^models' "$RUN/gateway-b.txt")"
grep -q '^keys True authorized True' "$RUN/gateway-b.txt" && grep -q '^metadata True' "$RUN/gateway-b.txt" && check "B: with the habitat's key, an idempotency key and the attempt's metadata" ok || check "B: with the habitat's key, an idempotency key and the attempt's metadata" no "$(sed -n 3,4p "$RUN/gateway-b.txt" | tr '\n' ' ')"
grep -q "'edit'" "$RUN/gateway-b.txt" && check "B: offered the Work's hands as tools" ok || check "B: offered the Work's hands as tools" no "$(grep '^tools' "$RUN/gateway-b.txt")"
land B "$base" api/todo.hl "The steward's note"

# ---- 4. path C: a person's session over MCP
base=$(rows | wc -l)
out=$(timeout 120 hale dna task create --as riley --to "$POSITION" "leave the person's note on the list's api" 2>&1 | tail -2); note "C: ask: $(echo "$out" | tr '\n' ' ' | cut -c1-200)"
wait_for attempt.admitted "$base" 120 "$POSITION" > /dev/null && check "C: admitted as the position's Work" ok || check "C: admitted as the position's Work" no "no attempt.admitted naming $POSITION"
out=$(timeout 600 python3 "$HERE/mcp-session.py" "$APP" "$POSITION" "person's note" 2>>"$LOG"); echo "$out" >> "$LOG"; note "C: session: $(echo "$out" | cut -c1-300)"
echo "$out" | grep -q '"submitted": true' && check "C: the session works the hands and hands the change back" ok || check "C: the session works the hands and hands the change back" no "$(echo "$out" | json_field error)"
land C "$base" api/todo.hl "The person's note"

# ---- 5. an Assessment, by a leg
base=$(rows | wc -l); calls0=$(wc -l < "$RUN/gateway.jsonl")
out=$(timeout 120 hale dna task create --as riley --judgment "assess whether a Todos::count rpc is worth adding" 2>&1 | tail -1); note "Assessment: ask: $(echo "$out" | cut -c1-160)"
wait_for attempt.admitted "$base" 120 > /dev/null || true
out=$(timeout 300 hale dna work run --as position:agent --performer model 2>&1); echo "$out" >> "$LOG"; note "Assessment: run: $(echo "$out" | tail -1 | cut -c1-240)"
a=$(wait_for attempt.outcome "$base" 120) && check "Assessment: settled" ok || check "Assessment: settled" no "no attempt.outcome in 120 s; last rows: $(last_rows)"
echo "$a" | grep -q 'worth doing' && check "Assessment: the gateway's assessment is the outcome" ok || check "Assessment: the gateway's assessment is the outcome" no "$(echo "$a" | cut -c1-200)"
tail -n +"$((calls0 + 1))" "$RUN/gateway.jsonl" | grep '"scenario": "assess"' | grep -q '"record_status"' && check "Assessment: a tools Work, the record's reads offered" ok || check "Assessment: a tools Work, the record's reads offered" no "$(tail -n +"$((calls0 + 1))" "$RUN/gateway.jsonl" | head -1 | cut -c1-200)"

# ---- 6. the graph read again at the later commit
base=$(rows | wc -l)
out=$(timeout 300 hale dna ingest 2>&1); note "ingest: $(echo "$out" | tail -1 | cut -c1-200)"
rid=$(echo "$out" | grep -o 'k:[0-9a-f]\{12\}' | head -1)
echo "$out" | grep -q -E 'ingested [0-9a-f]{12}: [1-9][0-9]* added' && check "6: ingest proposes the difference" ok || check "6: ingest proposes the difference" no "$(echo "$out" | tail -2 | tr '\n' ' ')"
wait_for graph.ingested "$base" 30 > /dev/null && check "6: the commit is named" ok || check "6: the commit is named" no "no graph.ingested"
decide "6: the Board ratifies it" ingest approve --as riley
# the settlement is the Board's; the graph changes when the ratify workflow writes knowledge.ratified
wait_for knowledge.ratified "$base" 120 "$rid" > /dev/null && check "6: the ratified graph proposal is recorded" ok || check "6: the ratified graph proposal is recorded" no "no knowledge.ratified for $rid in 120 s"
out=$(timeout 300 hale dna ingest 2>&1); echo "$out" | grep -q 'nothing to review' && check "6: the record's graph is the repository's, the tools among it" ok || check "6: the record's graph is the repository's, the tools among it" no "$(echo "$out" | tail -1)"

echo "WALK DONE"; cat "$R"
