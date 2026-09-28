#!/usr/bin/env bash
# Launch the face: the project service (the head) serving the browser
# shell, owning the project registry and proxying each attached project's
# native API. The shell owns only argument/build/process wiring, never domain
# operations.
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: dna/face/start.sh [PROJECT] [--port PORT] [--api-port PORT] [--oidc-port PORT] [--commands-port PORT] [--api BINARY] [--head BINARY] [--source-drafts]

Starts the face at http://127.0.0.1:8792 (or the chosen port). PROJECT is optional:
given, it is attached at startup; without it the head starts detached and the
browser's Projects workspace creates, initializes or attaches one.
Without --head/HALE_HEAD_BIN and --api/HALE_API_BIN, builds the checkout's
project service and its per-project native API (dna/api/practice_review) in
temporary storage using HALE_BIN (default: hale). Ctrl-C stops only the head:
the API child, a local body and every run it started are detached on
purpose and are re-adopted by the next head.

  --api BINARY       Use an existing/application-composed native API for the
                     attached project. It must accept PROJECT PORT, as
                     dna/api/practice_review does.
  --head BINARY      Use an already built project service.
  --port PORT        Loopback port of the head, 1..65535 (default: 8792).
  --api-port PORT    Loopback port of the API child, 1..65535 (default: 8793).
  --commands-port PORT
                     Loopback port of the API child's commands, its api binding's HTTP transport, 1..65535 (default: 8795).
  --oidc-port PORT   Loopback port of the stub OpenID provider, 1..65535 (default: 8794).
  --source-drafts    Enable Organization source preparation.
  --help            Show this help.

Existing service configuration is inherited:
  HALE_DNA_MEMORY_DSN_HEAD     Memory under the record's head role, as
                               `hale dna memory migrate` prints it. The API
                               reads Knowledge with it; without it Knowledge
                               is unsupported.
  HALE_DNA_MEMORY_DSN_OWNER, HALE_DNA_MEMORY_DSN_SPINE
                               Reach a body the head starts, never the API.
  HALE_DNA_NATS_URL_OWNER      The nerves' server a body the head starts runs
                               on. The head subscribes there, as the head's
                               user, to the rows the organism lands and tells
                               the browser; without it, on the project's
                               compose `nerves` while `hale dna dev` has it up.
  HALE_DNA_NATS_URL_HEAD       The head's own URL (subscribe only), as
                               `hale dna nerves migrate` prints it; wins over both.
  HALE_DNA_HEAD_STATE          The head's state directory
                               (default: ${XDG_STATE_HOME:-~/.local/state}/hale/dna/head).
The head serves under OIDC (GH #989): this starts the stub OpenID provider
(dna/oidc) on the loopback, and you sign in through it as yourself — the
subject local-sub, mapped to $USER. An attached project is configured for that
issuer (dna.principal, dna.oidc.issuer, dna.oidc.client, dna.oidc.member), and
the head forwards your ID token to its API child, which verifies it. A project
served under another issuer is refused at attach.
This starts no body, database, broker, migration or adoption command;
those are the head's operations, each a CLI verb run detached on request.
USAGE
}
fail() { printf 'face: %s\n' "$*" >&2; exit 2; }
valid_path() { [[ "$1" != *$'\n'* && "$1" != *$'\r'* ]]; }

project=
port=8792
api_port=8793
oidc_port=8794
commands_port=8795
api=${HALE_API_BIN:-}
head=${HALE_HEAD_BIN:-}
while (($#)); do
  case "$1" in
    --help|-h) usage; exit 0 ;;
    --port|--api|--head|--api-port|--oidc-port|--commands-port)
      (($# >= 2)) || fail "$1 requires a value"
      case "$1" in
        --port) port=$2 ;;
        --api-port) api_port=$2 ;;
        --oidc-port) oidc_port=$2 ;;
        --commands-port) commands_port=$2 ;;
        --api) api=$2 ;;
        --head) head=$2 ;;
      esac
      shift 2 ;;
    --source-drafts) export HALE_DNA_ORG_DRAFTS=1; shift ;;
    --) shift; (($# == 1)) && [[ -z "$project" ]] || fail 'expected one project path after --'; project=$1; shift ;;
    -*) fail "unknown option: $1" ;;
    *) [[ -z "$project" ]] || fail 'expected exactly one project'; project=$1; shift ;;
  esac
done
[[ "$port" =~ ^[1-9][0-9]{0,4}$ ]] && ((port <= 65535)) || fail 'port must be 1..65535'
[[ "$api_port" =~ ^[1-9][0-9]{0,4}$ ]] && ((api_port <= 65535)) || fail 'api-port must be 1..65535'
[[ "$oidc_port" =~ ^[1-9][0-9]{0,4}$ ]] && ((oidc_port <= 65535)) || fail 'oidc-port must be 1..65535'
[[ "$commands_port" =~ ^[1-9][0-9]{0,4}$ ]] && ((commands_port <= 65535)) || fail 'commands-port must be 1..65535'
((port != api_port && port != oidc_port && api_port != oidc_port)) || fail 'the head, the API child and the provider need different ports'
# the API child's api binding serves its HTTP transport there (GH #1135),
# and a port it cannot hold stops it at start
((commands_port != port && commands_port != api_port && commands_port != oidc_port)) || fail 'the API child'\''s commands need a port of their own'
face=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
checkout=$(cd -- "$face/../.." && pwd -P)
valid_path "$checkout" || fail 'checkout paths cannot contain newlines'
command -v git >/dev/null || fail 'git is required to read the DNA Record'
# A caller's repository overrides must never redirect a named project.
unset GIT_DIR GIT_WORK_TREE GIT_COMMON_DIR GIT_INDEX_FILE GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES GIT_NAMESPACE
if [[ -n "$project" ]]; then
  valid_path "$project" || fail 'project paths cannot contain newlines'
  [[ -d "$project" ]] || fail 'project directory does not exist'
  project=$(cd -- "$project" && pwd -P)
  top=$(git -C "$project" rev-parse --show-toplevel 2>/dev/null) || fail 'project must be a Git worktree'
  [[ "$(cd -- "$top" && pwd -P)" == "$project" ]] || fail 'pass the DNA project root, not a subdirectory'
  git -C "$project" rev-parse --verify 'refs/dna/journal^{commit}' >/dev/null 2>&1 || fail 'project has no DNA Record; create it from the Projects workspace, or with hale dna first'
fi
for asset in index.html app.js application.js definition-draft.js organization-draft.js knowledge-draft.js task-administration.js projects.js task-create.js styles.css; do
  [[ -r "$face/web/$asset" && -s "$face/web/$asset" ]] || fail "missing browser asset: $asset"
done
# The DSN carries the head role's password: refuse it without echoing it.
if [[ -n "${HALE_DNA_MEMORY_DSN_HEAD:-}" && ! "$HALE_DNA_MEMORY_DSN_HEAD" =~ ^postgres(ql)?://[^@/[:space:]]+@[^/[:space:]]+/[^[:space:]]+$ ]]; then
  fail 'Knowledge reads require HALE_DNA_MEMORY_DSN_HEAD to be a postgres:// DSN for the head role'
fi

build_dir=
builds=()
child=
provider=
reflexes=
cleanup() {
  local result=$?
  trap - EXIT INT TERM
  # A build still running when the launch is stopped: without job
  # control a background command ignores the terminal's SIGINT, so it is
  # stopped here, before its directory goes.
  if ((${#builds[@]})); then
    kill -TERM "${builds[@]}" 2>/dev/null || true
    wait "${builds[@]}" 2>/dev/null || true
  fi
  if [[ -n "$child" ]]; then
    kill -TERM "$child" 2>/dev/null || true
    wait "$child" 2>/dev/null || true
  fi
  if [[ -n "$provider" ]]; then
    kill -TERM "$provider" 2>/dev/null || true
    wait "$provider" 2>/dev/null || true
  fi
  if [[ -n "$reflexes" ]]; then
    kill -TERM "$reflexes" 2>/dev/null || true
    wait "$reflexes" 2>/dev/null || true
  fi
  if [[ -n "$build_dir" ]]; then rm -rf -- "$build_dir"; fi
  exit "$result"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# The head execs the compiler for every operation, so it is required even
# when both binaries are supplied.
hale=${HALE_BIN:-hale}
command -v -- "$hale" >/dev/null || fail 'Hale compiler unavailable; set HALE_BIN'
hale=$(command -v -- "$hale")
[[ "$hale" == /* ]] || hale="$(pwd -P)/$hale"
valid_path "$hale" || fail 'compiler paths cannot contain newlines'
export HALE_BIN=$hale

# Builds the seeds this launch was not handed into the temporary build
# directory, as CI builds them: each seed directory itself is the program.
# The checkout's Hale sources the seeds import (dna/, and the host's
# iris/process_identity) are copied there once, so the builds write
# nothing into the checkout. Not a one-line seed importing the checkout:
# an imported `main locus` is inert (its placement is the entry's to
# declare), and the head's server and watcher would run one after the
# other on the main thread.
#
# The builds run side by side, as scripts/warm-and-build.sh runs CI's:
# each `hale build` is one single-threaded process, and in turn the two
# took about 9 minutes on a CI runner (GH #1147). Every build is waited
# for even after one fails, each one's output is then printed as its own
# group, in the order the seeds were named, and the launch fails if
# either failed.
build_seeds() {
  local i seed status failed=()
  mkdir -p -- "$build_dir/src" "$build_dir/logs"
  (cd -- "$checkout" && find dna iris/process_identity -name '*.hl' -print0 | tar --null -T - -cf -) | tar -xf - -C "$build_dir/src" || fail 'cannot copy the checkout'\''s Hale sources'
  for i in "${!seeds[@]}"; do
    printf 'face: building %s from this checkout…\n' "${seeds[$i]}" >&2
    # Observation belongs to the application. Do not attach the compiler or the head.
    env -u LOTUS_OBS "$hale" build "$build_dir/src/${seeds[$i]}" > "$build_dir/logs/$i.log" 2>&1 &
    builds+=("$!")
  done
  for i in "${!seeds[@]}"; do
    seed=${seeds[$i]}
    if wait "${builds[$i]}"; then status=built; else status=FAILED; failed+=("$seed"); fi
    printf 'face: ── %s: %s\n' "$seed" "$status" >&2
    cat -- "$build_dir/logs/$i.log" >&2
  done
  builds=()
  ((${#failed[@]} == 0)) || fail "cannot build ${failed[*]} (its output is above)"
}
absolute_executable() {
  valid_path "$1" || fail "$2 paths cannot contain newlines"
  [[ -f "$1" && -x "$1" ]] || fail "$2 binary must be an executable file"
  printf '%s\n' "$(cd -- "$(dirname -- "$1")" && pwd -P)/$(basename -- "$1")"
}
# A binary handed in is checked before anything is built, so a wrong path
# fails at once rather than after the builds.
# GH #989: under OIDC the stub provider is a seed too, built beside the
# others; a fixture's trusted-local session (HALE_DNA_TRUSTED_LOCAL=1)
# starts none.
oidc_local=0
[[ "${HALE_DNA_TRUSTED_LOCAL:-}" == 1 ]] || oidc_local=1
if ((oidc_local)); then
  command -v curl >/dev/null || fail 'curl is required to wait for the OpenID provider'
  command -v openssl >/dev/null || fail 'openssl is required to make the OpenID provider its key'
fi
seeds=()
if [[ -z "$api" ]]; then seeds+=(dna/api/practice_review); else api=$(absolute_executable "$api" API); fi
if [[ -z "$head" ]]; then seeds+=(dna/api/project_service); else head=$(absolute_executable "$head" head); fi
if ((oidc_local)); then seeds+=(dna/oidc/serve); fi
# GH #988: the reflexes read a project's senses, so they are a seed when
# a project is attached, beside the stub provider (a fixture's
# trusted-local session starts neither)
if ((oidc_local)) && [[ -n "$project" ]]; then seeds+=(dna/reflexes); fi
if ((${#seeds[@]})); then
  build_dir=$(mktemp -d "${TMPDIR:-/tmp}/hale-dna-head.XXXXXXXX")
  build_seeds
  [[ -n "$api" ]] || api=$build_dir/src/dna/api/practice_review/practice_review
  [[ -n "$head" ]] || head=$build_dir/src/dna/api/project_service/project_service
fi

# GH #989: the head's principal path is OIDC. Once every seed is built,
# the stub provider serves on the loopback under a key and a secret made
# for this launch, and the person running this signs in through it as
# local-sub; it goes with the launcher (cleanup stops it on any exit).
if ((oidc_local)); then
  oidc=$build_dir/src/dna/oidc/serve/serve
  # the provider's key, made for this launch and readable by you alone; the
  # head pins its public half, since anyone on this machine could answer
  # on the loopback port
  key_file="$build_dir/oidc.key"
  (umask 077 && openssl ecparam -name prime256v1 -genkey -noout -out "$key_file" 2>/dev/null) || fail 'cannot make the provider its key'
  pub() { openssl ec -in "$key_file" -pubout -outform DER 2>/dev/null; }
  b64url() { base64 -w0 | tr '+/' '-_' | tr -d '='; }
  spki=$(pub | base64 -w0)
  key_x=$(pub | tail -c 64 | head -c 32 | b64url)
  key_y=$(pub | tail -c 32 | b64url)
  [[ ${#key_x} == 43 && ${#key_y} == 43 && -n "$spki" ]] || fail 'cannot read the provider key'\''s public half'
  # GH #989: the client's secret is the vault's `oidc-client-dna-local`,
  # which the organism's bootstrap provisions (`hale dna init`, `upgrade`);
  # the provider and the head each read it there, and nothing here draws,
  # holds or passes it. None in the vault: the provider refuses to start.
  env -u LOTUS_OBS HALE_DNA_OIDC_KEY_FILE="$key_file" HALE_DNA_OIDC_KEY_X="$key_x" HALE_DNA_OIDC_KEY_Y="$key_y" "$oidc" "$oidc_port" dna-local >&2 &
  provider=$!
  for _ in $(seq 1 100); do
    curl -sf "http://127.0.0.1:$oidc_port/.well-known/openid-configuration" >/dev/null 2>&1 && break
    kill -0 "$provider" 2>/dev/null || fail 'the OpenID provider exited before it listened (its client secret, oidc-client-dna-local, is provisioned by `hale dna init` or `hale dna upgrade` in a project)'
    sleep 0.1
  done
  # the provider answering is ours: it publishes the key made above
  curl -sf "http://127.0.0.1:$oidc_port/jwks" | grep -q "\"x\":\"$key_x\"" || fail "port $oidc_port answers, but not as this launch's provider"
  export HALE_DNA_OIDC_ISSUER="http://127.0.0.1:$oidc_port" HALE_DNA_OIDC_CLIENT=dna-local HALE_DNA_OIDC_KEY=$spki HALE_DNA_OIDC_MEMBER="local-sub=${USER:?USER must name you}"
  printf 'face: signing in through %s as local-sub (%s)\n' "$HALE_DNA_OIDC_ISSUER" "$USER"
fi

# GH #988: the reflexes, on the private-services tier beside the project's
# store: they hold the store's read URL and a publish-only credential on
# the project's nerves (the `reflexes` user), nothing else, and go with
# the launcher. A store or nerves that cannot come up leaves the face
# running without them.
if ((oidc_local)) && [[ -n "$project" ]]; then
  # either verb fails when its service cannot come up; under `set -e`
  # that must not end the launch, so each falls back to nothing
  senses_url=$("$hale" dna senses up "$project" 2>/dev/null | sed -n 's/^HALE_DNA_SENSES_URL=//p') || senses_url=
  roles=$("$hale" dna nerves migrate "$project" 2>/dev/null) || roles=
  reflexes_url=$(printf '%s\n' "$roles" | sed -n 's/^HALE_DNA_NATS_URL_REFLEXES=//p')
  reflexes_vault=$(printf '%s\n' "$roles" | sed -n 's/^HALE_DNA_NATS_VAULT_REFLEXES=//p')
  org=$(printf '%s\n' "$roles" | sed -n 's/^HALE_DNA_NATS_ORG=//p')
  if [[ -n "$senses_url" && -n "$reflexes_url" && -n "$reflexes_vault" && -n "$org" ]]; then
    env -u LOTUS_OBS HALE_DNA_SENSES_URL="$senses_url" HALE_DNA_NATS_URL_REFLEXES="$reflexes_url" HALE_DNA_NATS_VAULT_REFLEXES="$reflexes_vault" HALE_DNA_NATS_ORG="$org" "$build_dir/src/dna/reflexes/reflexes" >&2 &
    reflexes=$!
    printf 'face: reflexes reading %s\n' "$senses_url"
  else
    printf 'face: no reflexes: the project'\''s senses or nerves did not come up (`hale dna senses up`, `hale dna nerves migrate`)\n'
  fi
fi

printf 'face: starting http://127.0.0.1:%s/\n' "$port"
if [[ -n "$project" ]]; then printf 'face: project %s\n' "$project"; else printf 'face: no project attached; open the Projects workspace\n'; fi
if [[ -n "${HALE_DNA_MEMORY_DSN_HEAD:-}" ]]; then printf 'face: Knowledge reads memory as the head (the DSN stays on the server)\n'; fi
printf 'face: stop with Ctrl-C; the API child, a local body and running commands keep running and are re-adopted by the next head\n'
env -u LOTUS_OBS HALE_DNA_COMMANDS_PORT="$commands_port" "$head" "$port" "$face/web" "$api" "$api_port" ${project:+"$project"} <&0 &
child=$!
if wait "$child"; then result=0; else result=$?; fi
child=
exit "$result"
