#!/usr/bin/env bash
# Regenerate .effects-baseline/corpus.effects — the committed
# behavioural fingerprint of the in-tree example corpus.
#
# The #265 CI gate (`--check-effects-manifest`) shipped with nothing
# to check: it was exercised on toy inputs in two CLI tests, and no
# baseline for this repo existed. A regression gate pointed at
# nothing is not a gate.
#
# Run this after an INTENDED effect change and commit the diff. The
# diff is the review artifact: it shows, per function, what the
# compiler thinks the program now does.
set -euo pipefail
cd "$(dirname "$0")/.."
HALE=${HALE_BIN:-./target/release/hale}
[ -x "$HALE" ] || { echo "no hale binary at $HALE (cargo build --release)" >&2; exit 1; }

out=.effects-baseline/corpus.effects
mkdir -p .effects-baseline
{
  echo "# Corpus effect baseline — the behavioural fingerprint of every"
  echo "# in-tree example, one section per program."
  echo "#"
  echo "# Regenerate with: scripts/effects-baseline.sh"
  echo "#"
  echo "# A diff here means some function's INFERRED effects changed."
  echo "# That is either intended (and the diff is the review artifact)"
  echo "# or a regression annotations could never catch — a handler that"
  echo "# quietly starts doing filesystem I/O changes no source you would"
  echo "# think to look at, but it changes this file."
  echo
  # Bytewise order, matching the Rust-side sort in
  # effects_baseline_gate.rs. Locale-dependent glob order would make
  # the two disagree on names like `03-closure` vs `03b-closure`.
  for d in $(LC_ALL=C ls -d crates/hale-codegen/tests/fixtures/examples/*/ | LC_ALL=C sort); do
    n=$(basename "$d"); [ -f "$d/main.hl" ] || continue
    echo "### $n"
    "$HALE" check "$d/main.hl" --dump-effects-manifest 2>/dev/null \
      | grep -v '^ok:' | grep -v '^# .hale.effects' || true
  done
} > "$out"
echo "wrote $out ($(wc -l < "$out") lines)"
