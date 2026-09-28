#!/usr/bin/env bash
# Pre-build the organism artifacts DNA's own test fixtures share (GH
# #1147 follow-up: the dna suite's critical path). `Host::build_seed`
# (dna/host/host.hl) already caches a seed's build by content
# fingerprint, keyed to be identical across two organisms with the
# same sources regardless of which temp root each one started in
# (fixed alongside this script — the fingerprint used to bake in the
# absolute seed path, so an organism at a fixture's own uniquely-named
# scratch root never matched any other organism's entry). Without a
# job-scoped warm, N fixtures on N parallel slice processes each race
# a COLD cache on their first organism: every one of them pays the
# ~4s build of dna/org (the vendored core) and the app seed at once,
# in parallel, none of them benefiting from the others' work. This
# script pays that cost exactly once, sequentially, before any slice
# starts, so the fixture that would have been first to miss instead
# finds both already there.
#
# Starts one throwaway organism just long enough for its two builds
# (dna/org, and the app seed `seed_dir()` names) to land in the shared
# cache, then stops it — the organism never has to fully come up (no
# lease held past this, no Board decision ratified) for the build
# side effect to count. Needs the same memory/nerves the dna job's
# fixtures already get from its services; skips itself, warning
# rather than failing the step, when they are not set — a job without
# them just runs every fixture's own build as before this script
# existed.
#
#   scripts/warm-dna-fixtures-cache.sh <hale> [<cache dir>]
set -euo pipefail
hale=$(realpath -- "${1:?the hale binary}")
cache=${2:-${XDG_CACHE_HOME:-$HOME/.cache}}

if [[ -z "${HALE_DNA_MEMORY_DSN_OWNER:-}" || -z "${HALE_DNA_NATS_URL_OWNER:-}" ]]; then
    echo "warm-dna-fixtures-cache: no memory/nerves in this job's environment; skipping (every fixture builds its own organism, as before this script)"
    exit 0
fi

build_cache="$cache/hale/dna-build"
mkdir -p "$build_cache"
before=$(find "$build_cache" -maxdepth 1 -type f 2>/dev/null | wc -l)

tmp=$(mktemp -d "${TMPDIR:-/tmp}/hale-warm-fixtures.XXXXXX")
pid=
cleanup() {
    if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
    rm -rf -- "$tmp"
}
trap cleanup EXIT

export XDG_CACHE_HOME=$cache HALE_DNA_DISCOVER=off GIT_TERMINAL_PROMPT=0 GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null
"$hale" dna new "$tmp/warm" >/dev/null
# --no-iris: this run exists for the build side effect alone. iris
# attached (the default) means fuse-hl, which grows without bound on
# a live organism's topology diff — confirmed directly (10 GB RSS
# within two minutes on an idle throwaway with nothing but this
# script driving it, GH #1147 follow-up measurement) — for something
# this script never reads.
"$hale" dna dev --no-iris "$tmp/warm" >"$tmp/dev.log" 2>&1 &
pid=$!

# Both the org and the app print "under LOTUS_OBS=1" once THEIR OWN
# build_seed has returned (Host::say, right after org_ready/app_ready
# is set) — true whether that build was fresh or already cached, so
# this is the fast path on an already-warm cache (a restored
# actions/cache entry) instead of waiting out the deadline below for
# cache-dir entries that a hit will never add.
deadline=$((SECONDS + 120))
while :; do
    ready=$(grep -c "under LOTUS_OBS=1" "$tmp/dev.log" 2>/dev/null ||:)
    if ((ready >= 2)); then break; fi
    if ! kill -0 "$pid" 2>/dev/null; then break; fi
    if ((SECONDS >= deadline)); then break; fi
    sleep 0.3
done
kill "$pid" 2>/dev/null || true
wait "$pid" 2>/dev/null || true
pid=

after=$(find "$build_cache" -maxdepth 1 -type f 2>/dev/null | wc -l)
ready=$(grep -c "under LOTUS_OBS=1" "$tmp/dev.log" 2>/dev/null ||:)
echo "warm-dna-fixtures-cache: dna-build entries $before -> $after; $ready/2 seeds reported ready"
if ((ready < 2)); then
    echo "warm-dna-fixtures-cache: WARNING — only $ready/2 seeds reported ready; whichever did not land falls back to a cold build in every fixture that needs it. dev.log:" >&2
    tail -n 40 "$tmp/dev.log" >&2
fi
