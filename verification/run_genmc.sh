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
# A model may pin the memory model it is checked under with a
# `GENMC-FLAGS:` line in its header comment, e.g.
#
#     /* GENMC-FLAGS: --sc */
#
# Default is GenMC's release-acquire model, which is the faithful one
# for the runtime's `__ATOMIC_ACQUIRE` / `__ATOMIC_RELEASE`. A model
# that pins `--sc` MUST say in its header why, and what the RA result
# was — silently weakening the checker is how a model stops meaning
# anything.
for model in "$here"/*_model.c; do
    [ -e "$model" ] || continue
    flags="$(sed -n 's|.*GENMC-FLAGS:[[:space:]]*||p' "$model" | head -1 \
                | sed -e 's|\*/.*||' -e 's|[[:space:]]*$||')"
    echo "── $(basename "$model") ${flags:+[$flags] }───────────────────────────────"
    if "$GENMC" $flags -- "$model"; then
        echo "  ✓ verified (no races / UAF / assertion violations)"
    else
        echo "  ✗ GenMC reported a violation in $(basename "$model")" >&2
        fail=1
    fi
done
exit "$fail"
