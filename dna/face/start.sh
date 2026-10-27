#!/usr/bin/env bash
# Launch the face: the standard face (dna/face/standard) serving the
# browser shell and the project's ui/ on the public port, in front of the
# project service (the head), which serves surfaces only: it owns the
# project registry and proxies each attached project's native API. The shell
# owns only argument/build/process wiring, never domain operations.
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: dna/face/start.sh [PROJECT] [--port PORT] [--head-port PORT] [--api-port PORT] [--oidc-port PORT] [--commands-port PORT] [--api BINARY] [--head BINARY] [--source-drafts]

Starts the standard face at http://127.0.0.1:8792 (or the chosen port), the
head behind it on the loopback. PROJECT is optional:
given, it is attached at startup; without it the head starts detached and the
browser's Projects workspace creates, initializes or attaches one.
Without --head/HALE_HEAD_BIN, --face/HALE_FACE_BIN and --api/HALE_API_BIN, builds the checkout's
project service and its per-project native API (dna/api/practice_review) in
temporary storage using HALE_BIN (default: hale); the stub OpenID provider and
the reflexes likewise, without HALE_OIDC_BIN and HALE_REFLEXES_BIN. With an
installed toolchain and no checkout, `hale dna head` runs this launcher from
the toolchain's own copy, every binary handed in. Ctrl-C stops only the head:
the API child, a local body and every run it started are detached on
purpose and are re-adopted by the next head.

  --api BINARY       Use an existing/application-composed native API for the
                     attached project. It must accept PROJECT PORT, as
                     dna/api/practice_review does.
  --head BINARY      Use an already built project service.
  --face BINARY      Use an already built standard face.
  --port PORT        Loopback port of the face, the one a browser opens, 1..65535
                     (default: 8792).
  --head-port PORT   Loopback port of the head behind the face, 1..65535 (default: 8796).
  --api-port PORT    Loopback port of the API child, 1..65535 (default: 8793).
  --commands-port PORT
                     Loopback port of the API child's commands, its api binding's HTTP transport, 1..65535 (default: 8795).
  --oidc-port PORT   Loopback port of the stub OpenID provider, 1..65535 (default: 8794).
  --source-drafts    Enable Organization source preparation.
  --help            Show this help.

Existing service configuration is inherited:
  HALE_DNA_MEMORY_DSN_HEAD     Memory under the record's head role, as
                               `hale habitat migrate` keeps it. The API
                               reads Knowledge with it; without it Knowledge
                               is unsupported.
  HALE_DNA_MEMORY_DSN_OWNER, HALE_DNA_MEMORY_DSN_SPINE
                               Reach a body the head starts, never the API.
  HALE_DNA_NATS_URL_OWNER      The nerves' server a body the head starts runs
                               on. The head subscribes there, as the head's
                               user, to the rows the organism lands and tells
                               the browser; without it, on the organism's
                               nerves in the local habitat while they are up.
  HALE_DNA_NATS_URL_HEAD       The head's own URL (subscribe only), as
                               `hale dna nerves urls` prints it; wins over both.
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
face_bin=${HALE_FACE_BIN:-}
head_port=8796
oidc=${HALE_OIDC_BIN:-}
reflexes_bin=${HALE_REFLEXES_BIN:-}
while (($#)); do
  case "$1" in
    --help|-h) usage; exit 0 ;;
    --port|--head-port|--api|--head|--face|--api-port|--oidc-port|--commands-port)
      (($# >= 2)) || fail "$1 requires a value"
      case "$1" in
        --port) port=$2 ;;
        --api-port) api_port=$2 ;;
        --oidc-port) oidc_port=$2 ;;
        --commands-port) commands_port=$2 ;;
        --api) api=$2 ;;
        --head) head=$2 ;;
        --face) face_bin=$2 ;;
        --head-port) head_port=$2 ;;
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
[[ "$head_port" =~ ^[1-9][0-9]{0,4}$ ]] && ((head_port <= 65535)) || fail 'head-port must be 1..65535'
((port != api_port && port != oidc_port && api_port != oidc_port && head_port != port && head_port != api_port && head_port != oidc_port)) || fail 'the face, the head, the API child and the provider need different ports'
# the API child's api binding serves its HTTP transport there (GH #1135),
# and a port it cannot hold stops it at start
((commands_port != port && commands_port != head_port && commands_port != api_port && commands_port != oidc_port)) || fail 'the API child'\''s commands need a port of their own'
face=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
checkout=$(cd -- "$face/../.." && pwd -P)
valid_path "$checkout" || fail 'checkout paths cannot contain newlines'
command -v git >/dev/null || fail 'git is required to read the project'
# A caller's repository overrides must never redirect a named project.
unset GIT_DIR GIT_WORK_TREE GIT_COMMON_DIR GIT_INDEX_FILE GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES GIT_NAMESPACE
if [[ -n "$project" ]]; then
  valid_path "$project" || fail 'project paths cannot contain newlines'
  [[ -d "$project" ]] || fail 'project directory does not exist'
  project=$(cd -- "$project" && pwd -P)
  top=$(git -C "$project" rev-parse --show-toplevel 2>/dev/null) || fail 'project must be a Git worktree'
  [[ "$(cd -- "$top" && pwd -P)" == "$project" ]] || fail 'pass the DNA project root, not a subdirectory'
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
face_child=
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
  if [[ -n "$face_child" ]]; then
    kill -TERM "$face_child" 2>/dev/null || true
    wait "$face_child" 2>/dev/null || true
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
# the record is read through the toolchain (`hale dna record-rows`), never its storage
if [[ -n "$project" ]]; then
  rows=$("$hale" dna record-rows "$project" 2>/dev/null) || rows=
  [[ -n "$rows" ]] || fail 'project has no DNA Record; create it from the Projects workspace, or with hale dna first'
  unset rows
fi

# Builds the seeds this launch was not handed, straight from the checkout,
# each into the temporary build directory with `hale build -o`, so a build
# writes nothing into the checkout. (It used to copy the checkout's Hale
# sources into that directory first and build the copies, because the binary
# landed beside its seed.)
#
# The builds run side by side, as scripts/warm-and-build.sh runs CI's:
# each `hale build` is one single-threaded process, and in turn the two
# took about 9 minutes on a CI runner (GH #1147). Every build is waited
# for even after one fails, each one's output is then printed as its own
# group, in the order the seeds were named, and the launch fails if
# either failed.
build_seeds() {
  local i seed status failed=()
  mkdir -p -- "$build_dir/bin" "$build_dir/logs"
  for i in "${!seeds[@]}"; do
    printf 'face: building %s from this checkout…\n' "${seeds[$i]}" >&2
    # Observation belongs to the application. Do not attach the compiler or the head.
    env -u LOTUS_OBS "$hale" build "$checkout/${seeds[$i]}" -o "$build_dir/bin/${seeds[$i]##*/}" > "$build_dir/logs/$i.log" 2>&1 &
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
# A habitat shares one identity with the organisms it grafts: its own
# face's launch (HALE_DNA_IDENTITY=publish) keeps its provider's key in its
# state, so the key outlives a restart, and publishes the provider at
# <habitat>/identity.env; an organism's launch under a habitat that
# published one signs in through it and starts no provider of its own.
habitat_dir=${HALE_DNA_HABITAT:-${XDG_DATA_HOME:-$HOME/.local/share}/hale/habitat/local}
identity_env=$habitat_dir/identity.env
shared_identity=0
if ((oidc_local)) && [[ "${HALE_DNA_IDENTITY:-}" != publish && -f "$identity_env" ]]; then
  shared_identity=1
fi
if ((oidc_local)); then
  command -v curl >/dev/null || fail 'curl is required to wait for the OpenID provider'
  command -v openssl >/dev/null || fail 'openssl is required to make the OpenID provider its key'
fi
seeds=()
if [[ -z "$api" ]]; then seeds+=(dna/api/practice_review); else api=$(absolute_executable "$api" API); fi
if [[ -z "$head" ]]; then seeds+=(dna/api/project_service); else head=$(absolute_executable "$head" head); fi
if [[ -z "$face_bin" ]]; then seeds+=(dna/face/standard); else face_bin=$(absolute_executable "$face_bin" face); fi
if ((oidc_local && !shared_identity)); then
  if [[ -z "$oidc" ]]; then seeds+=(dna/oidc/serve); else oidc=$(absolute_executable "$oidc" 'OpenID provider'); fi
fi
# GH #988: the reflexes read a project's senses, so they are a seed when
# a project is attached, beside the stub provider (a fixture's
# trusted-local session starts neither)
if ((oidc_local)) && [[ -n "$project" ]]; then
  if [[ -z "$reflexes_bin" ]]; then seeds+=(dna/reflexes); else reflexes_bin=$(absolute_executable "$reflexes_bin" reflexes); fi
fi
# the launch's own directory: the builds, and the provider's key
if ((${#seeds[@]} || oidc_local)); then
  build_dir=$(mktemp -d "${TMPDIR:-/tmp}/hale-dna-head.XXXXXXXX")
fi
if ((${#seeds[@]})); then
  build_seeds
  [[ -n "$api" ]] || api=$build_dir/bin/practice_review
  [[ -n "$head" ]] || head=$build_dir/bin/project_service
  [[ -n "$face_bin" ]] || face_bin=$build_dir/bin/standard
  [[ -n "$oidc" ]] || oidc=$build_dir/bin/serve
  [[ -n "$reflexes_bin" ]] || reflexes_bin=$build_dir/bin/reflexes
fi

# GH #989: the head's principal path is OIDC. Once every seed is built,
# the stub provider serves on the loopback under a key and a secret made
# for this launch, and the person running this signs in through it as
# local-sub; it goes with the launcher (cleanup stops it on any exit).
if ((shared_identity)); then
  # the habitat's provider, as its face published it: its issuer, its
  # key's public half (which the head pins) and the client; this launch
  # starts no provider
  set -a
  # shellcheck disable=SC1090
  . "$identity_env"
  set +a
  [[ -n "${HALE_DNA_OIDC_ISSUER:-}" && -n "${HALE_DNA_OIDC_KEY:-}" && -n "${HALE_DNA_OIDC_DIAL:-}" ]] || fail "$identity_env does not name the habitat's provider"
  curl -sf "$HALE_DNA_OIDC_DIAL/.well-known/openid-configuration" >/dev/null 2>&1 || fail "the habitat's identity does not answer at $HALE_DNA_OIDC_ISSUER: its face serves it (\`hale habitat up\`)"
  export HALE_DNA_OIDC_MEMBER="local-sub=${USER:?USER must name you}"
  printf 'face: signing in through the habitat'\''s identity, %s, as local-sub (%s)\n' "$HALE_DNA_OIDC_ISSUER" "$USER"
elif ((oidc_local)); then
  # the provider's key, made for this launch and readable by you alone; the
  # head pins its public half, since anyone on this machine could answer
  # on the loopback port. The habitat's own (HALE_DNA_IDENTITY=publish) is
  # kept in its state instead, made once: the organisms that sign in
  # through it pin the same half across its restarts
  key_file="$build_dir/oidc.key"
  if [[ "${HALE_DNA_IDENTITY:-}" == publish && -n "${HALE_DNA_HEAD_STATE:-}" ]]; then
    mkdir -p "$HALE_DNA_HEAD_STATE"
    key_file="$HALE_DNA_HEAD_STATE/oidc.key"
  fi
  [[ -s "$key_file" ]] || (umask 077 && openssl ecparam -name prime256v1 -genkey -noout -out "$key_file" 2>/dev/null) || fail 'cannot make the provider its key'
  pub() { openssl ec -in "$key_file" -pubout -outform DER 2>/dev/null; }
  b64url() { base64 -w0 | tr '+/' '-_' | tr -d '='; }
  spki=$(pub | base64 -w0)
  key_x=$(pub | tail -c 64 | head -c 32 | b64url)
  key_y=$(pub | tail -c 32 | b64url)
  [[ ${#key_x} == 43 && ${#key_y} == 43 && -n "$spki" ]] || fail 'cannot read the provider key'\''s public half'
  # GH #989: the client's secret is the vault's `oidc-client-dna-local`,
  # which the organism's bootstrap provisions (`hale habitat graft`, `hale dna upgrade`);
  # the provider and the head each read it there, and nothing here draws,
  # holds or passes it. None in the vault: the provider refuses to start.
  # the provider binds the face's listen address and records its issuer
  # under the habitat's declared address; it is probed where it binds
  oidc_dial=${HALE_DNA_LISTEN_HOST:-127.0.0.1}
  [[ "$oidc_dial" == 0.0.0.0 ]] && oidc_dial=127.0.0.1
  services_file=
  [[ "${HALE_DNA_IDENTITY:-}" == publish ]] && services_file=$habitat_dir/identity.services
  env -u LOTUS_OBS HALE_DNA_OIDC_KEY_FILE="$key_file" HALE_DNA_OIDC_KEY_X="$key_x" HALE_DNA_OIDC_KEY_Y="$key_y" HALE_DNA_OIDC_SERVICES_FILE="$services_file" "$oidc" "$oidc_port" dna-local >&2 &
  provider=$!
  for _ in $(seq 1 100); do
    curl -sf "http://$oidc_dial:$oidc_port/.well-known/openid-configuration" >/dev/null 2>&1 && break
    kill -0 "$provider" 2>/dev/null || fail 'the OpenID provider exited before it listened (its client secret, oidc-client-dna-local, is provisioned by `hale habitat graft` or `hale dna upgrade` in a project)'
    sleep 0.1
  done
  # the provider answering is ours: it publishes the key made above
  curl -sf "http://$oidc_dial:$oidc_port/jwks" | grep -q "\"x\":\"$key_x\"" || fail "port $oidc_port answers, but not as this launch's provider"
  export HALE_DNA_OIDC_ISSUER="http://${HALE_DNA_HABITAT_ADDRESS:-127.0.0.1}:$oidc_port" HALE_DNA_OIDC_CLIENT=dna-local HALE_DNA_OIDC_KEY=$spki HALE_DNA_OIDC_MEMBER="local-sub=${USER:?USER must name you}"
  printf 'face: signing in through %s as local-sub (%s)\n' "$HALE_DNA_OIDC_ISSUER" "$USER"
  # the habitat's provider, published for the organisms it grafts
  if [[ "${HALE_DNA_IDENTITY:-}" == publish ]]; then
    mkdir -p "$habitat_dir"
    printf 'HALE_DNA_OIDC_ISSUER=%s\nHALE_DNA_OIDC_CLIENT=dna-local\nHALE_DNA_OIDC_KEY=%s\nHALE_DNA_OIDC_DIAL=http://%s:%s\n' "$HALE_DNA_OIDC_ISSUER" "$spki" "$oidc_dial" "$oidc_port" >"$identity_env.$$" && mv "$identity_env.$$" "$identity_env" || fail "cannot publish the habitat's identity at $identity_env"
  fi
fi

# GH #988: the reflexes, on the private-services tier beside the project's
# store: they hold the store's read URL and a publish-only credential on
# the project's nerves (the `reflexes` user), nothing else, and go with
# the launcher. A store or nerves that are not up leave the face running
# without them: the launcher brings nothing up (GH #1653).
if ((oidc_local)) && [[ -n "$project" ]]; then
  # either verb fails when its service is not up; under `set -e`
  # that must not end the launch, so each falls back to nothing
  senses_url=$("$hale" dna senses url "$project" 2>/dev/null | sed -n 's/^HALE_DNA_SENSES_URL=//p') || senses_url=
  roles=$("$hale" dna nerves urls "$project" 2>/dev/null) || roles=
  reflexes_url=$(printf '%s\n' "$roles" | sed -n 's/^HALE_DNA_NATS_URL_REFLEXES=//p')
  reflexes_vault=$(printf '%s\n' "$roles" | sed -n 's/^HALE_DNA_NATS_VAULT_REFLEXES=//p')
  org=$(printf '%s\n' "$roles" | sed -n 's/^HALE_DNA_NATS_ORG=//p')
  if [[ -n "$senses_url" && -n "$reflexes_url" && -n "$reflexes_vault" && -n "$org" ]]; then
    env -u LOTUS_OBS HALE_DNA_SENSES_URL="$senses_url" HALE_DNA_NATS_URL_REFLEXES="$reflexes_url" HALE_DNA_NATS_VAULT_REFLEXES="$reflexes_vault" HALE_DNA_NATS_ORG="$org" "$reflexes_bin" >&2 &
    reflexes=$!
    printf 'face: reflexes reading %s\n' "$senses_url"
  else
    printf 'face: no reflexes: the project'\''s senses or nerves are not up (`hale habitat migrate %s` brings them up)\n' "$project"
  fi
fi

printf 'face: starting http://127.0.0.1:%s/\n' "$port"
if [[ -n "$project" ]]; then printf 'face: project %s\n' "$project"; else printf 'face: no project attached; open the Projects workspace\n'; fi
if [[ -n "${HALE_DNA_MEMORY_DSN_HEAD:-}" ]]; then printf 'face: Knowledge reads memory as the head (the DSN stays on the server)\n'; fi
printf 'face: stop with Ctrl-C; the API child, a local body and running commands keep running and are re-adopted by the next head\n'
env -u LOTUS_OBS HALE_DNA_COMMANDS_PORT="$commands_port" HALE_DNA_PUBLIC_PORT="$port" "$head" "$head_port" "$api" "$api_port" ${project:+"$project"} <&0 &
child=$!
env -u LOTUS_OBS "$face_bin" "$port" "$face/web" "$head_port" &
face_child=$!
# either one ending ends the launch: a face with no head, or a head with no face, is no door
# (polled with kill -0: the system Bash of macOS is 3.2, which has no `wait -n`)
while kill -0 "$child" 2>/dev/null && kill -0 "$face_child" 2>/dev/null; do sleep 0.2; done
if ! kill -0 "$child" 2>/dev/null; then
  if wait "$child"; then result=0; else result=$?; fi
  child=
else
  if wait "$face_child"; then result=0; else result=$?; fi
  face_child=
fi
exit "$result"
