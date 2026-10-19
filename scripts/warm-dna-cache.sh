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
here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
# the root goes with whatever compose ran from under it: a habitat with no
# owner's servers brings its memory and nerves up as its own allocations,
# and the local habitat is the root's own here (XDG_DATA_HOME under it),
# never the developer's
# (a teardown that did not finish keeps the root, and fails the script)
trap 'if "$here/compose-down-under.sh" "$tmp"; then rm -rf -- "$tmp"; else exit 1; fi' EXIT
export XDG_CACHE_HOME=$cache XDG_DATA_HOME=$tmp/data HALE_DNA_HABITAT=$tmp/data/hale/habitat/local HALE_DNA_DISCOVER=off HALE_BIN=$hale GIT_TERMINAL_PROMPT=0
iris_dir=$("$hale" iris --where)
host_bin=$iris_dir/dna/host/host
# the host's verb program (GH #1490), built beside the body by the same
# first host verb
verb_bin=$iris_dir/dna/host/verb/verb

# The host's IR size, a standing check between closes (F.40 phase 4):
# phase 3's build regression on dna/host showed first as IR growth
# (1.85 M lines to 2.13 M, the reclaim's guards emitted at every site),
# a number that does not depend on the machine. The build below is the
# one this cache makes of the host's two programs, its body and its verb
# program; LOTUS_DUMP_IR=1 has each compile leave its pre-optimization IR
# beside its binary, which is counted into <binary>.ir-size (cached with
# the binary: the cache key covers the compiler, its embedded sources
# and this script) and then removed. A host already cached without the
# record is rebuilt once, so the check always has its numbers.
if [[ -s "$host_bin" && ! -s "$host_bin.ir-size" ]] || [[ -s "$verb_bin" && ! -s "$verb_bin.ir-size" ]]; then
    rm -f -- "$host_bin" "$verb_bin"
fi
# the habitat's birth runs the first host verb (GH #1653): on the owner's
# servers when they are named, else its own allocations
backends=own; [[ -n "${HALE_DNA_MEMORY_DSN_OWNER:-}" && -n "${HALE_DNA_NATS_URL_OWNER:-}" ]] && backends=operator
LOTUS_DUMP_IR=1 "$hale" habitat up --no-body --backends "$backends" >/dev/null
"$hale" habitat graft "$tmp/warm" --no-pack >/dev/null
for bin in "$host_bin" "$verb_bin"; do
    if [[ -f "$bin.ll" ]]; then
        echo "lines=$(( $(wc -l < "$bin.ll") )) defines=$(grep -c '^define ' "$bin.ll")" \
             "calls=$(grep -cE '^[[:space:]]+(%[^ ]+ = )?((tail|musttail|notail) )?call ' "$bin.ll")" > "$bin.ir-size"
        rm -f -- "$bin.ll"
    fi
done

# Measured 2026-10-11 after the host split into its body and its verb
# program (GH #1490): each still compiles the whole host seed, the body
# 0.2% and the verb program 0.3% under the single host it replaces
# (lines=1716679 defines=6262 calls=237696, structure as batches, #1493).
# Each number may move ±10%.
# Moving one on purpose is an edit here, with the reason in the commit
# (the same three numbers for three corpus programs are pinned in
# crates/hale-codegen/tests/ir_size_band.rs).
host_ir_pinned="lines=1999283 defines=6708 calls=265782"
verb_ir_pinned="lines=1998546 defines=6670 calls=265512"
measured_all=
for which in body verb; do
    if [[ $which == body ]]; then bin=$host_bin pinned=$host_ir_pinned seed=dna/host; else bin=$verb_bin pinned=$verb_ir_pinned seed=dna/host/verb; fi
    if [[ ! -s "$bin.ir-size" ]]; then
        echo "warm: FAILED — the IR size of $seed was not recorded at $bin.ir-size by the build that produced $bin" >&2
        exit 1
    fi
    read -r measured < "$bin.ir-size"
    measured_all+="$which: $measured; "
    out_of_band=
    for pin in $pinned; do
        what=${pin%%=*} want=${pin#*=}
        got=$(tr ' ' '\n' <<< "$measured" | sed -n "s/^$what=//p")
        lo=$(( want * 9 / 10 )) hi=$(( (want * 11 + 9) / 10 ))
        if (( got < lo || got > hi )); then
            out_of_band+="  $what: measured $got, pinned $want, band $lo..$hi"$'\n'
        fi
    done
    if [[ -n "$out_of_band" ]]; then
        printf 'warm: FAILED — the IR of %s left its band:\n%smeasured %s\n' "$seed" "$out_of_band" "$measured" >&2
        echo "If the change is meant to move it, edit host_ir_pinned or verb_ir_pinned in scripts/warm-dna-cache.sh to the measured value, with the reason in the commit." >&2
        exit 1
    fi
done
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
for bin in "$host_bin" "$verb_bin"; do
    if [[ ! -s "$bin" ]]; then
        echo "warm: FAILED — $bin is missing or empty after the build that was supposed to produce it" >&2
        exit 1
    fi
done
sizes="$sizes verb=$(stat -c%s "$verb_bin" 2>/dev/null || stat -f%z "$verb_bin" 2>/dev/null || echo MISSING)"
if [[ "${HALE_WARM_SKIP_IRIS:-}" != 1 ]]; then
    observer_bin=$iris_dir/iris/consumer/fuse-hl/fuse-hl
    sizes="$sizes observer=$(stat -c%s "$observer_bin" 2>/dev/null || stat -f%z "$observer_bin" 2>/dev/null || echo MISSING)"
    if [[ ! -s "$observer_bin" ]]; then
        echo "warm: FAILED — $observer_bin is missing or empty after the build that was supposed to produce it" >&2
        exit 1
    fi
fi
echo "warm: host and ${HALE_WARM_SKIP_IRIS:+no }observer built under $cache ($sizes bytes; IR ${measured_all%; }); $tests_cache/hale points at it"
