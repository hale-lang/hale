#!/usr/bin/env bash
# The instructions `hale check` spends on a trivial seed, under callgrind.
#
#   scripts/check-cost/measure.sh <hale> [<callgrind output file>]
#
# Prints the total; the output file (default: a temporary one) is what
# `callgrind_annotate --inclusive=yes` reads to see where they went.
set -euo pipefail
HALE=$(readlink -f "$1")
OUT=${2:-$(mktemp -t hale-cg.XXXXXX)}
DIR=$(mktemp -d)
trap 'rm -rf "$DIR"' EXIT
printf 'fn main() {\n    println("hi");\n}\n' > "$DIR/main.hl"
cd "$DIR"
HALE_SKIP_STALE_CHECK=1 valgrind --tool=callgrind --callgrind-out-file="$OUT" "$HALE" check main.hl 2>&1 \
  | sed -n 's/.*I *refs: *//p'
echo "callgrind output: $OUT" >&2
