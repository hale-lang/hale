#!/usr/bin/env bash
# Warm the DNA toolchain cache for one hale binary before a test job runs:
# the host (built by the first host verb) and the observer (fuse-hl).
# Every DNA fixture pays for these builds inside its own deadline
# otherwise, beside the partition's other organisms, and on a loaded
# runner that is where an organism that never comes up comes from. The
# CLI tests keep their own cache directory
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
iris_dir=$("$hale" iris --where)
host_bin=$iris_dir/dna/host/host
host_ir=$iris_dir/dna/host/host.ll
host_ir_size=$iris_dir/dna/host/host.ir-size

# The host's IR size, a standing check between closes (F.40 phase 4):
# phase 3's build regression on dna/host showed first as IR growth
# (1.85 M lines to 2.13 M, the reclaim's guards emitted at every site),
# a number that does not depend on the machine. The build below is the
# one this cache makes of the host; LOTUS_DUMP_IR=1 has that same compile
# leave its pre-optimization IR beside the binary, which is counted into
# host.ir-size (cached with the binary: the cache key covers the
# compiler, its embedded sources and this script) and then removed. A
# host already cached without the record is rebuilt once, so the check
# always has its numbers.
if [[ -s "$host_bin" && ! -s "$host_ir_size" ]]; then rm -f -- "$host_bin"; fi
LOTUS_DUMP_IR=1 "$hale" dna new "$tmp/warm" >/dev/null
if [[ -f "$host_ir" ]]; then
    echo "lines=$(( $(wc -l < "$host_ir") )) defines=$(grep -c '^define ' "$host_ir")" \
         "calls=$(grep -cE '^[[:space:]]+(%[^ ]+ = )?((tail|musttail|notail) )?call ' "$host_ir")" > "$host_ir_size"
    rm -f -- "$host_ir"
fi

# Measured 2026-10-05 on F.40 phase 4's main; each number may move ±10%.
# Moving one on purpose is an edit here, with the reason in the commit
# (the same three numbers for three corpus programs are pinned in
# crates/hale-codegen/tests/ir_size_band.rs).
host_ir_pinned="lines=1549084 defines=6017 calls=214003"
if [[ ! -s "$host_ir_size" ]]; then
    echo "warm: FAILED — the host's IR size was not recorded at $host_ir_size by the build that produced $host_bin" >&2
    exit 1
fi
read -r measured < "$host_ir_size"
out_of_band=
for pin in $host_ir_pinned; do
    what=${pin%%=*} want=${pin#*=}
    got=$(tr ' ' '\n' <<< "$measured" | sed -n "s/^$what=//p")
    lo=$(( want * 9 / 10 )) hi=$(( (want * 11 + 9) / 10 ))
    if (( got < lo || got > hi )); then
        out_of_band+="  $what: measured $got, pinned $want, band $lo..$hi"$'\n'
    fi
done
if [[ -n "$out_of_band" ]]; then
    printf 'warm: FAILED — the IR of dna/host left its band:\n%smeasured %s\n' "$out_of_band" "$measured" >&2
    echo "If the change is meant to move it, edit host_ir_pinned in scripts/warm-dna-cache.sh to the measured value, with the reason in the commit." >&2
    exit 1
fi
if [[ "${HALE_WARM_SKIP_IRIS:-}" != 1 ]]; then "$hale" iris --build-only >/dev/null; fi
tests_cache=${TMPDIR:-/tmp}/hale-tests-iris-cache
mkdir -p "$tests_cache"
if [[ ! -e "$tests_cache/hale" ]]; then ln -s "$cache/hale" "$tests_cache/hale"; fi

# The toolchain cache is only as good as what actually landed on disk:
# a save that missed one of these (a caller's `path:` mismatch, a
# GitHub Actions cache quirk on a large binary) would go unnoticed
# until a fixture's own build budget paid for it later. Check — and
# say the size — right here, before the caller's save step trusts
# this directory.
sizes="host=$(stat -c%s "$host_bin" 2>/dev/null || stat -f%z "$host_bin" 2>/dev/null || echo MISSING)"
if [[ ! -s "$host_bin" ]]; then
    echo "warm: FAILED — $host_bin is missing or empty after the build that was supposed to produce it" >&2
    exit 1
fi
if [[ "${HALE_WARM_SKIP_IRIS:-}" != 1 ]]; then
    observer_bin=$iris_dir/iris/consumer/fuse-hl/fuse-hl
    sizes="$sizes observer=$(stat -c%s "$observer_bin" 2>/dev/null || stat -f%z "$observer_bin" 2>/dev/null || echo MISSING)"
    if [[ ! -s "$observer_bin" ]]; then
        echo "warm: FAILED — $observer_bin is missing or empty after the build that was supposed to produce it" >&2
        exit 1
    fi
fi
echo "warm: host and ${HALE_WARM_SKIP_IRIS:+no }observer built under $cache ($sizes bytes; host IR $measured); $tests_cache/hale points at it"
