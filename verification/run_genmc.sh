#!/usr/bin/env bash
# Run every GenMC model in this directory under exhaustive
# interleaving. GH issue #18 item 2 (race-completeness).
#
# GenMC must be on PATH, or pointed at via $GENMC. Build it once with
# verification/build_genmc.sh (needs LLVM 18 + cmake). Exits non-zero
# if any model reports a race / UAF / assertion violation, so this is
# usable as a CI gate.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GENMC="${GENMC:-genmc}"

if ! command -v "$GENMC" >/dev/null 2>&1; then
    echo "error: genmc not found (set \$GENMC or build with verification/build_genmc.sh)" >&2
    exit 127
fi

echo "Using $("$GENMC" --version 2>&1 | head -2 | tail -1)"
fail=0
for model in "$here"/*_model.c; do
    [ -e "$model" ] || continue
    echo "── $(basename "$model") ───────────────────────────────"
    if "$GENMC" -- "$model"; then
        echo "  ✓ verified (no races / UAF / assertion violations)"
    else
        echo "  ✗ GenMC reported a violation in $(basename "$model")" >&2
        fail=1
    fi
done
exit "$fail"
