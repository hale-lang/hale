#!/usr/bin/env bash
# Warm the DNA toolchain cache (scripts/warm-dna-cache.sh) and build Hale
# seeds beside it, all at once. Each `hale build` is one single-threaded
# process, and so is the warm's host build: run one after the other on a
# four-core runner they left three cores idle for a quarter of an hour
# (the face jobs, 2026-09-26: 3.6 min of warm, then 12-13.5 min of
# builds). Side by side the step takes about as long as its longest
# build.
#
# The seeds' builds start in the background, the warm runs in the
# foreground, and then each build is waited for and its output printed
# as one group, so the log reads as if they had run in turn. Every build
# is waited for even after one fails; the step fails if any did.
#
#   scripts/warm-and-build.sh <hale> [--check] [--target-cpu <cpu>] [<seed dir>...]
#
#   --check             `hale check` each seed before building it
#   --target-cpu <cpu>  passed to every `hale build`
#
# The warm reads its own environment (HALE_WARM_SKIP_IRIS and the rest);
# the builds share the runtime object cache with it, which is
# content-addressed and safe to fill from several processes at once.
set -euo pipefail
hale=${1:?the hale binary}
shift
check=
build_flags=()
seeds=()
while (($#)); do
  case "$1" in
    --check) check=1; shift ;;
    --target-cpu) build_flags+=(--target-cpu "${2:?--target-cpu needs a value}"); shift 2 ;;
    -*) echo "warm-and-build: unknown option $1" >&2; exit 2 ;;
    *) seeds+=("$1"); shift ;;
  esac
done
logs=$(mktemp -d "${TMPDIR:-/tmp}/hale-seed-builds.XXXXXX")
trap 'rm -rf -- "$logs"' EXIT
pids=()
for i in "${!seeds[@]}"; do
  seed=${seeds[$i]}
  (
    set -e
    start=$SECONDS
    if [[ -n "$check" ]]; then "$hale" check "$seed"; fi
    "$hale" build "$seed" "${build_flags[@]}"
    echo "($seed: $((SECONDS - start)) s)"
  ) > "$logs/$i.log" 2>&1 &
  pids+=("$!")
done
start=$SECONDS
scripts_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
"$scripts_dir/warm-dna-cache.sh" "$hale"
echo "warm: $((SECONDS - start)) s"
failed=()
for i in "${!seeds[@]}"; do
  if wait "${pids[$i]}"; then status=built; else status=FAILED; failed+=("${seeds[$i]}"); fi
  echo "::group::${seeds[$i]}: $status"
  cat "$logs/$i.log"
  echo "::endgroup::"
done
echo "warm-and-build: every step done at $((SECONDS - start)) s"
if ((${#failed[@]})); then
  for seed in "${failed[@]}"; do echo "::error::hale build $seed failed (its group above has the output)"; done
  exit 1
fi
