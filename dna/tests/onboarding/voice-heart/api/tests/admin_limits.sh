#!/usr/bin/env bash
# api/tests/admin_limits.sh: a project's rate limit, from the Board that
# ratifies it to the 429 a caller gets, through a DNA organization's head
# (hale-lang/voice#1, #8). Run by admin_limits_test.hl with a hale on
# HALE_BIN (the toolchain carries the head, its API child and the stub
# provider), and docker for the organization's memory and nerves.
#
#   1. A fresh organization (`hale dna new`), under `hale dna dev`: its
#      memory (the record, GH #1590) and nerves from its seed compose.
#   2. `operating/limits/personal` proposed as a practice, its text stating
#      its grammar (governance.hl); the Board approves it, and the
#      organization ratifies it.
#   3. The head (`hale dna head`), its stub provider answering the
#      `client_credentials` grant for the service `voice-api`, its secret
#      in the vault, and the record mapping the service.
#   4. Voice's api given the head as a grafted one is (HALE_DNA_HEAD_URL,
#      HALE_DNA_APPLICATION_ID, HALE_DNA_OIDC_ISSUER, HALE_DNA_OIDC_CLIENT,
#      HALE_DNA_OIDC_CLIENT_VAULT), reading the limits as the service: the
#      admin plane shows the ratified limit and where it came from, refuses
#      to set it, and a project key's third request in the window is `429`
#      with `Retry-After`.
#   5. A superseding limit of 5, approved by the Board: the api reads it,
#      and serves three more in the same window.
#
# Everything lives under one directory made for the run (the vault too);
# it and every process whose command line names it, and the
# organization's compose project, go when the script ends, however it
# ends. VOICE_KEEP_SCRATCH=1 keeps them.
set -euo pipefail

hale=${HALE_BIN:?HALE_BIN names a hale toolchain}
voice=$(cd "$(dirname "$0")/../.." && pwd)
for tool in curl docker git python3; do command -v "$tool" >/dev/null || { echo "admin_limits: $tool is required" >&2; exit 2; }; done

root=$(mktemp -d "${TMPDIR:-/tmp}/voice-admin-limits.XXXXXXXX")
name=vl$$
app=$root/$name
export HALE_DNA_DISCOVER=off GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null
export HALE_VAULT_DIR=$root/vault
# the organization's one member, who proposes and, holding the Board,
# decides; the head signs them in as local-sub
member=fixture
export USER=$member
unset HALE_DNA_TRUSTED_LOCAL HALE_VAULT_ADDR LOTUS_OBS
# the organization's compose project is its own: the seed compose would
# otherwise be the project `dna`, every organization's on the box
export COMPOSE_PROJECT_NAME=$name
jobs_started=()

cleanup() {
  local code=$?
  local job
  for job in "${jobs_started[@]}"; do kill "$job" 2>/dev/null || true; done
  local pids
  pids=$(ps -eo pid=,args= | grep -F "$root" | grep -v grep | awk '{print $1}' | grep -vx "$$" || true)
  [[ -n "$pids" ]] && kill $pids 2>/dev/null || true
  sleep 1
  pids=$(ps -eo pid=,args= | grep -F "$root" | grep -v grep | awk '{print $1}' | grep -vx "$$" || true)
  [[ -n "$pids" ]] && kill -9 $pids 2>/dev/null || true
  if [[ "${VOICE_KEEP_SCRATCH:-}" == 1 ]]; then echo "admin_limits: kept $root and the compose project $name" >&2; else docker compose -p "$name" down -v >/dev/null 2>&1 || true; rm -rf "$root"; fi
  exit "$code"
}
trap cleanup EXIT

fail() {
  echo "admin_limits: $*" >&2
  for log in dev head voice reviews; do [[ -f "$root/$log.log" ]] && { echo "--- $log.log" >&2; tail -25 "$root/$log.log" >&2; }; done
  exit 1
}
# `until_ok <seconds> <what> <command…>`: the command succeeds within the bound
until_ok() {
  local secs=$1 what=$2; shift 2
  local until=$((SECONDS + secs))
  while ((SECONDS < until)); do "$@" && return 0; sleep 0.5; done
  fail "$what did not happen within ${secs}s"
}
step() { echo "admin_limits: $*" >&2; }
free_port() {
  local p
  for p in $(shuf -i 30000-46999 -n 64); do
    (exec 3<>"/dev/tcp/127.0.0.1/$p") 2>/dev/null || { echo "$p"; return; }
  done
  fail "no free port"
}

mkdir -m 700 "$HALE_VAULT_DIR"
"$hale" build "$voice/api" >/dev/null || fail "voice's api did not build"

# ---- 1. the organization, under dev
(cd "$root" && "$hale" dna new "$name" >"$root/new.log" 2>&1) || { cat "$root/new.log" >&2; fail "hale dna new"; }
git -C "$app" config user.name "$member" && git -C "$app" config user.email "$member@example.invalid"
git -C "$app" add -A && git -C "$app" commit -qm genome
(cd "$app" && exec "$hale" dna dev . --no-iris >"$root/dev.log" 2>&1) &
jobs_started+=($!)
until_ok 300 "the organization reading its nerves" grep -q "reads its facts from the nerves" "$root/dev.log"

# ---- 2. the limit, proposed, decided by the Board, ratified
grammar='The rate limit for the project personal: one name: value per line, from window_seconds (the window, in seconds), requests, input_tokens, output_tokens and concurrent, each a whole number alone after its name; a name left out is unlimited.'
limit() { printf '%s\nwindow_seconds: 600\nrequests: %s' "$grammar" "$1"; }
practices() { (cd "$app" && "$hale" dna practice 2>/dev/null); }
(cd "$app" && "$hale" dna practice propose operating/limits/personal --text "$(limit 2)" --because "the first limit" --as "$member" >/dev/null) || fail "the limit was not proposed"
pending() { (cd "$app" && timeout 60 "$hale" dna practice >"$root/reviews.log" 2>&1); grep 'operating/limits/personal' "$root/reviews.log" | grep -q 'awaiting the Board'; }
until_ok 60 "the limit's Review" pending
# the Board's verdict on the limit's Review, by its id
approve() {
  local review
  review=$(grep 'operating/limits/personal' "$root/reviews.log" | grep 'awaiting the Board' | grep -o 'k:[0-9a-f]*' | tail -1)
  [[ -n "$review" ]] || fail "no Review of the limit awaits the Board"
  step "the Board approves $review"
  (cd "$app" && timeout 120 "$hale" dna review "$review" approve --as "$member" --no-wait >/dev/null) || fail "the Board's verdict on $review was refused"
}
approve

step "the limit of 2 approved"
# ---- 3. the head, its stub provider answering for the service
head -c 16 /dev/urandom | od -An -tx1 | tr -d ' \n' >"$HALE_VAULT_DIR/oidc-service-voice-api"
chmod 600 "$HALE_VAULT_DIR/oidc-service-voice-api"
git -C "$app" config --local --add dna.oidc.service voice-api=voice
head_port=$(free_port); api_port=$(free_port); oidc_port=$(free_port); commands_port=$(free_port)
H=http://127.0.0.1:$head_port
issuer=http://127.0.0.1:$oidc_port
(cd "$app" && exec env HALE_DNA_HEAD_STATE="$root/state" HALE_DNA_OIDC_SERVICES=voice-api \
  "$hale" dna head . --port "$head_port" --api-port "$api_port" --oidc-port "$oidc_port" --commands-port "$commands_port" >"$root/head.log" 2>&1) &
head_job=$!
jobs_started+=("$head_job")
until_ok 180 "the head listening" grep -q "hale dna head: http://127.0.0.1:$head_port/" "$root/head.log"
until_ok 20 "the stub provider answering" curl -sf -o /dev/null "$issuer/.well-known/openid-configuration"

step "the head is up"
# the application's id at the head, as an operator reads it: the service's
# own token, and the head's applications
secret=$(cat "$HALE_VAULT_DIR/oidc-service-voice-api")
token_endpoint=$(curl -sf "$issuer/.well-known/openid-configuration" | python3 -c 'import json,sys; print(json.load(sys.stdin)["token_endpoint"])')
token=$(curl -sf --data-urlencode grant_type=client_credentials --data-urlencode client_id=voice-api --data-urlencode "client_secret=$secret" "$token_endpoint" | python3 -c 'import json,sys; print(json.load(sys.stdin)["access_token"])')
application=$(curl -sf -H "Authorization: Bearer $token" "$H/api/hale/v1/applications" | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["items"][0]["id"])')
[[ -n "$application" ]] || fail "the head names no application"
# ---- 4. voice's api under the head, as the service
voice_port=$(free_port); V=http://127.0.0.1:$voice_port
# as a grafted api is: the head and its service client handed over in the
# environment alone (the habitat's variables), no flag, the reading interval
# voice's own knob
VOICE_LIMITS_EVERY_S=1 HALE_DNA_HEAD_URL="$H" HALE_DNA_APPLICATION_ID="$application" HALE_DNA_OIDC_ISSUER="$issuer" HALE_DNA_OIDC_CLIENT=voice-api HALE_DNA_OIDC_CLIENT_VAULT=oidc-service-voice-api \
  "$voice/api/api" --port "$voice_port" --canned "$voice/api/canned" >"$root/voice.log" 2>&1 &
jobs_started+=($!)
until_ok 20 "voice's api answering" curl -sf -o /dev/null "$V/healthz"
curl -sf -X POST -H 'Content-Type: application/json' -d '{"id": "personal", "name": "Personal"}' "$V/admin/v1/projects" >/dev/null || fail "no project personal"
key=$(curl -sf -X POST -H 'Content-Type: application/json' -d '{"name": "fixture"}' "$V/admin/v1/projects/personal/keys" | python3 -c 'import json,sys; print(json.load(sys.stdin)["secret"])')
shown() { curl -s "$V/admin/v1/projects/personal/limits"; }
ratified() { shown | python3 -c 'import json,sys; d=json.load(sys.stdin); sys.exit(0 if d.get("source") == "practice" and d["practice"]["name"] == "operating/limits/personal" and [l.get("requests") for l in d["limits"]] == [int(sys.argv[1])] else 1)' "$1"; }
until_ok 90 "the ratified limit of 2 read through the head" ratified 2
code=$(curl -s -o /dev/null -w '%{http_code}' -X PUT -H 'Content-Type: application/json' -d '{"limits": []}' "$V/admin/v1/projects/personal/limits")
[[ $code == 409 ]] || fail "the admin plane set the organization's limit ($code)"
# no seat serves echo-1 here: a request the limit admits is 503
# no_eligible_seat, and one past it 429 before any seat is looked for
ask() { curl -s -o /dev/null -D "$root/last.headers" -w '%{http_code}' -X POST -H "Authorization: Bearer $key" -H 'Content-Type: application/json' --data '{"model":"echo-1","input":"hi"}' "$V/v1/responses"; }
first=$(ask); second=$(ask); third=$(ask)
[[ $first != 429 && $second != 429 ]] || fail "the first two requests were refused by the limit ($first, $second)"
[[ $third == 429 ]] || fail "the third request was not refused ($third)"
grep -qi '^Retry-After: [0-9]' "$root/last.headers" || fail "the 429 carries no Retry-After: $(cat "$root/last.headers")"

step "the limit of 2 refused the third request"
# ---- 5. a superseding limit, ratified the same way
# the full digest of the limit in force: proposed under its name, and
# ratified (the practice list shows it abbreviated)
active=$(cd "$app" && timeout 120 "$hale" dna record-rows 2>/dev/null | python3 -c '
import json, sys
named, ratified = set(), set()
for line in sys.stdin:
    try:
        r = json.loads(line)
        b = json.loads(r["body"]) if isinstance(r.get("body"), str) and r["body"].startswith("{") else {}
    except ValueError:
        continue
    if r["kind"] == "practice.proposed" and b.get("name") == "operating/limits/personal": named.add(b.get("digest"))
    if r["kind"] == "knowledge.ratified": ratified.add(r["entity"])
    if r["kind"] == "knowledge.retired": ratified.discard(r["entity"])
print(" ".join(sorted(named & ratified)))')
[[ -n "$active" ]] || fail "the ratified limit's digest was not found"
(cd "$app" && "$hale" dna practice propose operating/limits/personal --text "$(limit 5)" --because "more room" --supersedes "$active" --as "$member" >/dev/null) || fail "the superseding limit was not proposed"
until_ok 60 "the superseding limit's Review" pending
approve
until_ok 300 "the ratified limit of 5 read through the head" ratified 5
served=0
for _ in 1 2 3 4; do [[ $(ask) != 429 ]] && served=$((served + 1)) || break; done
[[ $served == 3 ]] || fail "under the limit of 5, $served more were admitted in the window (3 expected)"
step "the superseding limit of 5 admitted three more"
# ---- 6. a head that stops answering: the limits read last stand, and no
# answer waits on it (the head stopped, its socket still taking
# connections that nothing reads)
# the processes listening on the head's port and its API child's
listening() { ss -ltnpH "sport = :$1" | grep -oE 'pid=[0-9]+' | cut -d= -f2; }
head_pids=$( (listening "$head_port"; listening "$api_port") | sort -u | tr '\n' ' ')
[[ -n "${head_pids// /}" ]] || fail "no process found listening for the head"
step "the head stopped (pids $head_pids)"
unread() { grep -c 'the ratified rate limits were not read' "$root/voice.log" || true; }
unread_before=$(unread)
for p in $head_pids; do kill -STOP "$p" 2>/dev/null || true; done
sleep 3
t0=$(date +%s%N); code=$(ask); ms=$(( ($(date +%s%N) - t0) / 1000000 ))
[[ $code == 429 ]] || fail "with the head stopped, the window's sixth request was not refused by the limit read last ($code)"
(( ms < 1000 )) || fail "with the head stopped, an answer took ${ms} ms"
# the read under way ends at its deadline, and says so
# (a pinned locus's log line is written when the main thread next serves
# a request: FRICTION.md)
more_unread() { curl -s -o /dev/null "$V/healthz"; (( $(unread) > unread_before )); }
until_ok 60 "the read the stopped head did not answer logged" more_unread
for p in $head_pids; do kill -CONT "$p" 2>/dev/null || true; done
echo "admin_limits: the limit of 2 refused the third request; the superseding limit of 5 admitted three more; a stopped head held up no answer (${ms} ms)"
