#!/usr/bin/env bash
# scripts/dna-cycle/up.sh — phase "up" of the from-scratch cycle: a fresh organization in local mode
# around the todo repository, ratified and filled, its api up through compose, the head serving its
# API for the legs, and the canned gateway the legs' model asks. Leaves everything running (dev, the
# head and the gateway as user units) and names the run in $CYCLE_DIR/current.
#
#   MODELS=canned|record|replay|live scripts/dna-cycle/up.sh
#
# MODELS: canned (default; FakeModel over canned/, no tokens), record (live models, every answer
# taped), replay (answers from the tape, no tokens), live. The legs always ask the canned gateway.
set -u
source "$(dirname "$0")/lib.sh"
MODELS=${MODELS:-canned}; TAPE_DIR=$CYCLE_DIR/tape; mkdir -p "$TAPE_DIR"
N=$(date +%s); RUN=$CYCLE_DIR/run-$N; mkdir -p "$RUN"; R=$RUN/report.md; LOG=$RUN/cycle.log; echo "$RUN" > "$CYCLE_DIR/current"
export XDG_STATE_HOME=$RUN/state XDG_CACHE_HOME=${XDG_CACHE_HOME:-$HOME/.cache}
units_env=(--setenv=PATH="$PATH" --setenv=HOME="$HOME" --setenv=HALE_BIN="$HALE_BIN" --setenv=HALE_SKIP_STALE_CHECK=1 --setenv=HALE_DNA_DISCOVER=off --setenv=GIT_CONFIG_NOSYSTEM=1 --setenv=XDG_STATE_HOME="$XDG_STATE_HOME")
free_ports; sleep 1
echo "# dna cycle UP $N (hale $(hale --version 2>/dev/null | head -1), $(git -C "$HW" rev-parse --short HEAD))" > "$R"

step "copy the repository" bash -c "cp -r '$HW/dna/tests/onboarding/todo' '$RUN/todo' && cd '$RUN/todo' && git init -q -b main && git config user.name riley && git config user.email riley@dna.cycle && git add -A && git commit -q -m 'todo: the repository' && rm -rf bin"
cd "$RUN/todo" || exit 1
step "hale dna init ." bash -c "hale dna init . > '$RUN/init.out' 2>&1; rc=\$?; cat '$RUN/init.out'; exit \$rc"
step "commit what init wrote" bash -c "git add -A && git commit -q -m 'the organism'"
case "$MODELS" in canned) mm="canned $HERE/canned";; live) mm=live;; *) mm=record;; esac
step "models: $MODELS" bash -c "python3 '$HERE/model-mode.py' $mm dna/org/models.hl && git add -A dna/org/models.hl && (git commit -q -m 'models: $MODELS' || true)"
step "the legs' model: the canned gateway" bash -c "python3 '$HERE/model-mode.py' gateway '$GATEWAY' dna/org/work.hl && git add -A dna/org/work.hl && git commit -q -m 'legs: the canned gateway'"
step "the gateway's key in the vault" bash -c "echo canned-gateway-key | hale dna secret set CANNED_GATEWAY_KEY"
note "models: $MODELS$( { [ "$MODELS" = record ] || [ "$MODELS" = replay ]; } && echo " (tape $TAPE_DIR)")"
note "init: $(grep -E '^graph ' "$RUN/init.out" | head -1 | cut -c1-160)"

systemd-run --user --quiet --collect --unit="dna-cycle-gateway-$N" "${units_env[@]}" python3 "$HERE/canned-gateway.py" 8796 "$RUN/gateway.jsonl"
systemd-run --user --quiet --collect --unit="dna-cycle-dev-$N" --working-directory="$RUN/todo" "${units_env[@]}" \
    --setenv=HALE_DNA_TAPE="$( [ "$MODELS" = replay ] && echo replay || echo record )" --setenv=HALE_DNA_TAPE_DIR="$TAPE_DIR" --setenv=HALE_DNA_FORGE=file \
    bash -c "hale dna dev . --no-iris > '$RUN/dev.log' 2>&1"
t0=$(date +%s); ready=0
while [ $(( $(date +%s) - t0 )) -lt 300 ]; do
    grep -q -E 'oversees its seeds|reads its facts from the nerves|"ready": true' "$RUN/dev.log" 2>/dev/null && { ready=1; break; }
    systemctl --user is-active --quiet "dna-cycle-dev-$N" || break; sleep 2
done
if [ $ready = 1 ]; then check "hale dna dev ready ($(( $(date +%s) - t0 ))s)" ok; else check "hale dna dev ready" no "$(tail -3 "$RUN/dev.log" | tr '\n' ' ' | cut -c1-300)"; fi

step "hale dna review (list)" bash -c "hale dna review > '$RUN/review.out' 2>&1; cat '$RUN/review.out'"; note "review: $(head -1 "$RUN/review.out" | cut -c1-120)"
for g in $(grep -o -E '^  [a-z]+ —' "$RUN/review.out" | awk '{print $1}' | sort -u); do decide "hale dna review $g approve --as riley" "$g" approve --as riley; done
step "hale dna review (after)" bash -c "hale dna review > '$RUN/review2.out' 2>&1; cat '$RUN/review2.out'"; note "after: $(head -1 "$RUN/review2.out" | cut -c1-120)"
for pos in board api/dev api/reviewer ui/reviewer compose/operator; do step "hale dna fill $pos riley" hale dna fill "$pos" riley --as riley; done
# a fill is a proposal its asker may not ratify; the Board, as a position no one holds yet, may
decide "hale dna review holds approve --as position:board" holds approve --as position:board
step "the head's DSN from memory migrate" bash -c "hale dna memory migrate . > '$RUN/migrate.out' 2>&1; cat '$RUN/migrate.out'; grep -q HEAD '$RUN/migrate.out'"
step "hale dna show org" bash -c "hale dna show org > '$RUN/org.out' 2>&1; cat '$RUN/org.out'"; note "org: $(grep -c '' "$RUN/org.out") lines"
step "hale build api -o bin/todo-api" hale build api -o bin/todo-api
step "docker compose up -d api (todo)" bash -c "TODO_TOKENS='ada=t-ada,bo:reader=t-bo' docker compose up -d api"
t0=$(date +%s); while [ $(( $(date +%s) - t0 )) -lt 60 ]; do curl -s -o /dev/null -m 2 http://127.0.0.1:8080/.description && break; sleep 2; done
note "api up after $(( $(date +%s) - t0 ))s via compose"
step "hale api call add" hale api call http://127.0.0.1:8080 Todos::add --title milk --bearer t-ada

# the head, source-built from the hale checkout by dna/face/start.sh
HEAD_DSN=$(grep -o 'HALE_DNA_MEMORY_DSN_HEAD=[^ ]*' "$RUN/migrate.out" | head -1 | cut -d= -f2-); note "head DSN: ${HEAD_DSN:+set}${HEAD_DSN:-MISSING}"
systemd-run --user --quiet --collect --unit="dna-cycle-head-$N" --working-directory="$HW" "${units_env[@]}" --setenv=HALE_DNA_MEMORY_DSN_HEAD="$HEAD_DSN" \
    bash -c "dna/face/start.sh '$RUN/todo' --port 8792 --api-port 8793 --commands-port 8795 > '$RUN/head.log' 2>&1"
t0=$(date +%s); up=0
while [ $(( $(date +%s) - t0 )) -lt 600 ]; do
    grep -q -E 'hale dna head: http://127.0.0.1:8792|commands http://127.0.0.1:8795' "$RUN/head.log" 2>/dev/null && { up=1; break; }
    systemctl --user is-active --quiet "dna-cycle-head-$N" || break; sleep 3
done
if [ $up = 1 ]; then check "the head serves ($(( $(date +%s) - t0 ))s)" ok; else check "the head serves" no "$(tail -4 "$RUN/head.log" | tr '\n' ' ' | cut -c1-400)"; fi
# a local-mode ID token for the legs (`hale dna work login`, the client secret from the vault, never printed)
step "hale dna work login --as riley" bash -c "hale dna work login --as riley --api '$HEAD_API' > '$RUN/login.out' 2>&1 && grep -o 'HALE_DNA_ID_TOKEN=.*' '$RUN/login.out' | head -1 | cut -d= -f2- | tr -d '\"' > '$RUN/id_token' && [ -s '$RUN/id_token' ]"
step "the head reads with the token" bash -c "curl -sf -m 5 -H \"Authorization: Bearer \$(cat '$RUN/id_token')\" '$HEAD_API/dna/tasks' | head -c 300"
step "the gateway answers" bash -c "curl -sf -m 5 -X POST -H 'Content-Type: application/json' -d '{\"model\":\"probe\",\"messages\":[{\"role\":\"user\",\"content\":\"probe\"}]}' '$GATEWAY/v1/chat/completions' | head -c 200"
echo "UP DONE $RUN"; cat "$R"
