#!/usr/bin/env bash
# dna/kill-test/run.sh — the falsification test for Track D (GH #529).
#
# Produces the two views a reviewer would decide from — the git diff
# and the semantic diff — over the same change, and (optionally) opens
# the review view in iris. Read WALKTHROUGH.md first.
#
#   dna/kill-test/run.sh            # print both views
#   dna/kill-test/run.sh --iris     # …and open iris [4] on :8787
set -euo pipefail
cd "$(dirname "$0")"
HALE=${HALE_BIN:-hale}
out=${KT_OUT:-/tmp/hale-kill-test}
mkdir -p "$out"
# The candidate violates a law on purpose; a failing claim still cuts
# the artifact (the verdict is the thing being recorded).
"$HALE" check before --dump-topology="$out/before.topology" >/dev/null 2>&1 || true
"$HALE" check after  --dump-topology="$out/after.topology"  >/dev/null 2>&1 || true
test -s "$out/before.topology" && test -s "$out/after.topology"
echo "================ VIEW A: the git diff ================"
git --no-pager diff --no-index --stat before after || true
git --no-pager diff --no-index before after || true
echo
echo "================ VIEW B: the semantic diff (hale model diff) ================"
"$HALE" model diff "$out/before.topology" "$out/after.topology" --text
"$HALE" model diff "$out/before.topology" "$out/after.topology" > "$out/diff.json"
echo
echo "artifacts: $out/before.topology $out/after.topology  diff: $out/diff.json"
if [ "${1:-}" = "--iris" ]; then
  echo "iris: http://127.0.0.1:${PORT:-8787}/  press 4 for the review view, l for the law"
  exec "$HALE" iris "${PORT:-8787}" "$out/after.topology" --diff "$out/before.topology" "$out/after.topology"
fi
