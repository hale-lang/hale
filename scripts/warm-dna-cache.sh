#!/usr/bin/env bash
# Warm the DNA toolchain cache for one hale binary before a test job runs:
# the host and the membrane client (built by the first host verb), the
# observer (fuse-hl), and the knowledge service (built by its first start).
# Every DNA fixture pays for these builds inside its own deadline otherwise,
# beside the partition's other organisms, and on a loaded runner that is
# where "the membrane did not come up" and a service that never answers
# come from. The CLI tests keep their own cache directory
# (temp_dir()/hale-tests-iris-cache); it is pointed at the same build.
#
# HALE_WARM_SKIP_IRIS=1 skips the observer build: the face job's fixtures
# never run hale iris, while the cli partitions (iris_cli builds fuse-hl)
# and the dna partitions (hale dna run launches iris) still need it.
#
#   scripts/warm-dna-cache.sh <hale> [<cache dir>]
set -euo pipefail
hale=$(realpath -- "${1:?the hale binary}") # the knowledge service starts from another directory
cache=${2:-${XDG_CACHE_HOME:-$HOME/.cache}}
tmp=$(mktemp -d "${TMPDIR:-/tmp}/hale-warm.XXXXXX")
trap 'if [[ -f "$tmp/knowledge.pid" ]]; then kill "$(cat "$tmp/knowledge.pid")" 2>/dev/null || true; fi; rm -rf -- "$tmp"' EXIT
export XDG_CACHE_HOME=$cache HALE_DNA_DISCOVER=off HALE_BIN=$hale GIT_TERMINAL_PROMPT=0
"$hale" dna new "$tmp/warm" >/dev/null
if [[ "${HALE_WARM_SKIP_IRIS:-}" != 1 ]]; then "$hale" iris --build-only >/dev/null; fi
port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
( cd "$tmp/warm" && HALE_DNA_KNOWLEDGE_DSN=memory exec "$hale" dna knowledge . --port "$port" >"$tmp/knowledge.log" 2>&1 ) &
echo $! > "$tmp/knowledge.pid"
for _ in $(seq 1 900); do
  if curl -fsS --max-time 2 "http://127.0.0.1:$port/" >/dev/null 2>&1; then break; fi
  if ! kill -0 "$(cat "$tmp/knowledge.pid")" 2>/dev/null; then echo "warm: the knowledge service exited before answering:" >&2; tail -20 "$tmp/knowledge.log" >&2; exit 1; fi
  sleep 1
done
curl -fsS --max-time 2 "http://127.0.0.1:$port/" >/dev/null || { echo "warm: the knowledge service never answered" >&2; exit 1; }
tests_cache=${TMPDIR:-/tmp}/hale-tests-iris-cache
mkdir -p "$tests_cache"
if [[ ! -e "$tests_cache/hale" ]]; then ln -s "$cache/hale" "$tests_cache/hale"; fi
echo "warm: host, membrane, ${HALE_WARM_SKIP_IRIS:+no }observer and knowledge service built under $cache; $tests_cache/hale points at it"
