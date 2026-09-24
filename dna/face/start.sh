#!/usr/bin/env bash
# Launch the face: the project service (the head) serving the browser
# shell, owning the project registry and proxying each attached project's
# native API. The shell owns only argument/build/process wiring, never domain
# operations.
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: dna/face/start.sh [PROJECT] [--port PORT] [--api-port PORT] [--api BINARY] [--head BINARY] [--source-drafts]

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
  --source-drafts    Enable Organization and ownership source preparation.
  --help            Show this help.

Existing service configuration is inherited:
  HALE_DNA_MEMORY_DSN_HEAD     Memory under the record's head role, as
                               `hale dna memory migrate` prints it. The API
                               reads Knowledge with it; without it Knowledge
                               is unsupported.
  HALE_DNA_MEMORY_DSN_OWNER, HALE_DNA_MEMORY_DSN_SPINE
                               Reach a body the head starts, never the API.
  HALE_DNA_HEAD_STATE          The head's state directory
                               (default: ${XDG_STATE_HOME:-~/.local/state}/hale/dna/head).
The head is trusted-local; a project configured for OIDC is refused at attach.
This starts no body, database, broker, migration or adoption command;
those are the head's operations, each a CLI verb run detached on request.
USAGE
}
fail() { printf 'face: %s\n' "$*" >&2; exit 2; }
valid_path() { [[ "$1" != *$'\n'* && "$1" != *$'\r'* ]]; }

project=
port=8792
api_port=8793
api=${HALE_API_BIN:-}
head=${HALE_HEAD_BIN:-}
while (($#)); do
  case "$1" in
    --help|-h) usage; exit 0 ;;
    --port|--api|--head|--api-port)
      (($# >= 2)) || fail "$1 requires a value"
      case "$1" in
        --port) port=$2 ;;
        --api-port) api_port=$2 ;;
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
((port != api_port)) || fail 'the head and the API child need different ports'
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
child=
cleanup() {
  local result=$?
  trap - EXIT INT TERM
  if [[ -n "$child" ]]; then
    kill -TERM "$child" 2>/dev/null || true
    wait "$child" 2>/dev/null || true
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

# Builds one seed of this checkout into the temporary build directory, the
# way the API was built here before: a one-line seed importing the checkout.
build_seed() {
  local seed=$1 name=$2
  local import_path=${checkout//\\/\\\\}; import_path=${import_path//\"/\\\"}
  printf 'import "%s/%s" as host;\nfn main() { host::main(); }\n' "$import_path" "$seed" > "$build_dir/$name.hl"
  printf 'face: building %s from this checkout…\n' "$seed" >&2
  # Observation belongs to the application. Do not attach the compiler or the head.
  env -u LOTUS_OBS "$hale" build "$build_dir/$name.hl"
  printf '%s\n' "$build_dir/$name"
}
absolute_executable() {
  valid_path "$1" || fail "$2 paths cannot contain newlines"
  [[ -f "$1" && -x "$1" ]] || fail "$2 binary must be an executable file"
  printf '%s\n' "$(cd -- "$(dirname -- "$1")" && pwd -P)/$(basename -- "$1")"
}
# One build directory for both seeds, made here rather than inside the
# command substitution that calls build_seed, so cleanup removes it.
if [[ -z "$api" || -z "$head" ]]; then build_dir=$(mktemp -d "${TMPDIR:-/tmp}/hale-dna-head.XXXXXXXX"); fi
if [[ -z "$api" ]]; then api=$(build_seed dna/api/practice_review practice_review); else api=$(absolute_executable "$api" API); fi
if [[ -z "$head" ]]; then head=$(build_seed dna/api/project_service project_service); else head=$(absolute_executable "$head" head); fi

printf 'face: starting http://127.0.0.1:%s/\n' "$port"
if [[ -n "$project" ]]; then printf 'face: project %s\n' "$project"; else printf 'face: no project attached; open the Projects workspace\n'; fi
if [[ -n "${HALE_DNA_MEMORY_DSN_HEAD:-}" ]]; then printf 'face: Knowledge reads memory as the head (the DSN stays on the server)\n'; fi
printf 'face: stop with Ctrl-C; the API child, a local body and running commands keep running and are re-adopted by the next head\n'
env -u LOTUS_OBS "$head" "$port" "$face/web" "$api" "$api_port" ${project:+"$project"} <&0 &
child=$!
if wait "$child"; then result=0; else result=$?; fi
child=
exit "$result"
