#!/usr/bin/env bash
# Launch the native DNA API and bundled browser against an existing project.
# The shell owns only argument/build/process wiring, never domain operations.
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: iris/cockpit/start.sh PROJECT [--port PORT] [--api BINARY] [--source-drafts]

Starts Iris at http://127.0.0.1:8792 (or the chosen port).
Without --api/HALE_API_BIN, builds the checkout's native Hale API in temporary
storage using HALE_BIN (default: hale). Ctrl-C stops only this API process.

  --api BINARY       Use an existing/application-composed native API. It must
                     accept PROJECT PORT WEBROOT, as dna/api does.
  --port PORT        Loopback port, 1..65535 (default: 8792).
  --source-drafts    Enable Organization and ownership source preparation.
  --help            Show this help.

Existing service configuration is inherited:
  HALE_DNA_KNOWLEDGE_URL       Private state-service origin.
  HALE_DNA_KNOWLEDGE_READ_KEY  Private graph-read credential shared with it.
  HALE_IRIS_OBSERVER_ORIGIN   Optional native observer origin.
Authentication comes from the project's existing local/OIDC configuration.
This starts no body, database, broker, observer, migration or adoption command.
USAGE
}
fail() { printf 'Iris: %s\n' "$*" >&2; exit 2; }
valid_path() { [[ "$1" != *$'\n'* && "$1" != *$'\r'* ]]; }

project=
port=8792
api=${HALE_API_BIN:-}
while (($#)); do
  case "$1" in
    --help|-h) usage; exit 0 ;;
    --port|--api)
      (($# >= 2)) || fail "$1 requires a value"
      if [[ "$1" == --port ]]; then port=$2; else api=$2; fi
      shift 2 ;;
    --source-drafts) export HALE_IRIS_ORG_DRAFTS=1; shift ;;
    --) shift; (($# == 1)) && [[ -z "$project" ]] || fail 'expected one project path after --'; project=$1; shift ;;
    -*) fail "unknown option: $1" ;;
    *) [[ -z "$project" ]] || fail 'expected exactly one project'; project=$1; shift ;;
  esac
done
[[ -n "$project" ]] || { usage >&2; exit 2; }
[[ "$port" =~ ^[1-9][0-9]{0,4}$ ]] && ((port <= 65535)) || fail 'port must be 1..65535'
valid_path "$project" || fail 'project paths cannot contain newlines'
[[ -d "$project" ]] || fail 'project directory does not exist'
project=$(cd -- "$project" && pwd -P)
cockpit=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
checkout=$(cd -- "$cockpit/../.." && pwd -P)
valid_path "$checkout" || fail 'checkout paths cannot contain newlines'
command -v git >/dev/null || fail 'git is required to read the DNA Record'
# A caller's repository overrides must never redirect a named project.
unset GIT_DIR GIT_WORK_TREE GIT_COMMON_DIR GIT_INDEX_FILE GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES GIT_NAMESPACE
top=$(git -C "$project" rev-parse --show-toplevel 2>/dev/null) || fail 'project must be a Git worktree'
[[ "$(cd -- "$top" && pwd -P)" == "$project" ]] || fail 'pass the DNA project root, not a subdirectory'
git -C "$project" rev-parse --verify 'refs/dna/journal^{commit}' >/dev/null 2>&1 || fail 'project has no DNA Record; create or initialize it with hale dna first'
for asset in index.html app.js runtime.js application.js definition-draft.js organization-draft.js knowledge-draft.js task-administration.js task-create.js styles.css; do
  [[ -r "$cockpit/web/$asset" && -s "$cockpit/web/$asset" ]] || fail "missing browser asset: $asset"
done
if [[ -n "${HALE_DNA_KNOWLEDGE_URL:-}" || -n "${HALE_DNA_KNOWLEDGE_READ_KEY:-}" ]]; then
  [[ -n "${HALE_DNA_KNOWLEDGE_URL:-}" && -n "${HALE_DNA_KNOWLEDGE_READ_KEY:-}" ]] || fail 'Knowledge reads require both HALE_DNA_KNOWLEDGE_URL and HALE_DNA_KNOWLEDGE_READ_KEY'
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

if [[ -z "$api" ]]; then
  hale=${HALE_BIN:-hale}
  command -v -- "$hale" >/dev/null || fail 'Hale compiler unavailable; set HALE_BIN or supply --api'
  # Export the exact executable for the native source inspector as well.
  hale=$(command -v -- "$hale")
  [[ "$hale" == /* ]] || hale="$(pwd -P)/$hale"
  valid_path "$hale" || fail 'compiler paths cannot contain newlines'
  export HALE_BIN=$hale
  build_dir=$(mktemp -d "${TMPDIR:-/tmp}/hale-iris-api.XXXXXXXX")
  import_path=${checkout//\\/\\\\}; import_path=${import_path//\"/\\\"}
  printf 'import "%s/dna/api" as host;\nfn main() { host::main(); }\n' "$import_path" > "$build_dir/api.hl"
  printf 'Iris: building the native API from this checkout…\n' >&2
  # Observation belongs to the application. Do not attach the compiler or API.
  env -u LOTUS_OBS "$hale" build "$build_dir/api.hl"
  api="$build_dir/api"
else
  valid_path "$api" || fail 'API paths cannot contain newlines'
  [[ -f "$api" && -x "$api" ]] || fail 'API binary must be an executable file'
  api="$(cd -- "$(dirname -- "$api")" && pwd -P)/$(basename -- "$api")"
fi

printf 'Iris: starting http://127.0.0.1:%s/\n' "$port"
printf 'Iris: project %s\n' "$project"
if [[ -n "${HALE_DNA_KNOWLEDGE_URL:-}" ]]; then printf 'Iris: configured Knowledge service (credential stays on the server)\n'; fi
printf 'Iris: stop with Ctrl-C; existing application services keep running\n'
env -u LOTUS_OBS "$api" "$project" "$port" "$cockpit/web" <&0 &
child=$!
if wait "$child"; then result=0; else result=$?; fi
child=
exit "$result"
