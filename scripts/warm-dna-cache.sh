#!/usr/bin/env bash
# Warm the DNA toolchain cache for one hale binary before a test job runs:
# the host and the membrane client (built by the first host verb) and the
# observer (fuse-hl). Every DNA fixture pays for these builds inside its
# own deadline otherwise, beside the partition's other organisms, and on a
# loaded runner that is where "the membrane did not come up" comes from. The CLI tests keep their own cache directory
# (temp_dir()/hale-tests-iris-cache); it is pointed at the same build.
#
# HALE_WARM_SKIP_IRIS=1 skips the observer build: the face jobs' fixtures
# never run hale iris, while the cli partitions (iris_cli builds fuse-hl)
# and the dna partitions (hale dna run launches iris) still need it.
#
#   scripts/warm-dna-cache.sh <hale> [<cache dir>]
set -euo pipefail
hale=$(realpath -- "${1:?the hale binary}")
cache=${2:-${XDG_CACHE_HOME:-$HOME/.cache}}
tmp=$(mktemp -d "${TMPDIR:-/tmp}/hale-warm.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT
export XDG_CACHE_HOME=$cache HALE_DNA_DISCOVER=off HALE_BIN=$hale GIT_TERMINAL_PROMPT=0
"$hale" dna new "$tmp/warm" >/dev/null
if [[ "${HALE_WARM_SKIP_IRIS:-}" != 1 ]]; then "$hale" iris --build-only >/dev/null; fi
tests_cache=${TMPDIR:-/tmp}/hale-tests-iris-cache
mkdir -p "$tests_cache"
if [[ ! -e "$tests_cache/hale" ]]; then ln -s "$cache/hale" "$tests_cache/hale"; fi
echo "warm: host, membrane and ${HALE_WARM_SKIP_IRIS:+no }observer built under $cache; $tests_cache/hale points at it"
