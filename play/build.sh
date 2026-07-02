#!/usr/bin/env bash
# Build the static "run Hale in your browser" tour.
#
# For each examples/<name>.hl it produces, in site/dist/:
#   <name>.wasm   <name>.mjs   — compiled with `hale build --target wasm32`
#   <name>.hl                  — the clean source the page displays
#   manifest.json              — the example index (copied from examples/)
#
# Each example is authored as a normal Hale program ending in `fn main()`.
# The wasm target has no `main` (the host drives `@export` entries), so the
# compiler's `--wrap-main` flag synthesizes the entry on the AST: it turns
# `fn main` into an `@export locus` and injects the `target wasm { }` gate,
# string/comment-safe and preserving source spans. The loader's
# `_hale_start` runs the synthesized birth() once on load; `println` is
# captured by the page.
#
# Examples already written as an `@export locus` (e.g. the frame()-driven
# sim, and ui.hl) have no `fn main`, so `--wrap-main` leaves them alone and
# does NOT inject the gate — those we still front with `target wasm { }`.
set -euo pipefail
cd "$(dirname "$0")"

# Locate the compiler: $HALE, else the in-tree release build, else PATH.
HALE="${HALE:-}"
if [ -z "$HALE" ]; then
  if [ -x ../target/release/hale ]; then HALE="$(cd .. && pwd)/target/release/hale";
  else HALE=hale; fi
fi
command -v "$HALE" >/dev/null 2>&1 || [ -x "$HALE" ] || {
  echo "error: no 'hale' compiler found (set \$HALE or run 'cargo build --release')"; exit 1; }

DIST=site/dist
rm -rf "$DIST"; mkdir -p "$DIST"
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT

shopt -s nullglob
for src in examples/*.hl; do
  name="$(basename "$src" .hl)"
  built="$tmp/$name.hl"
  if grep -q '@export' "$src"; then
    # already an @export locus (e.g. a frame()-driven sim): --wrap-main is
    # a no-op for it and won't add the gate, so front it with the target.
    { echo 'target wasm { }'; echo; cat "$src"; } > "$built"
  else
    # a normal `fn main` program: hand it over verbatim — --wrap-main
    # synthesizes the @export entry and the target gate on the AST.
    cp "$src" "$built"
  fi

  echo ">> building $name"
  "$HALE" build "$built" --target wasm32 --wrap-main
  mv "$tmp/$name.wasm" "$tmp/$name.mjs" "$DIST/"
  cp "$src" "$DIST/$name.hl"
done

cp examples/manifest.json "$DIST/manifest.json"

# The tour UI itself is a Hale @export locus — the page's interaction logic
# lives here, not in JS. Built like a no-main example (just set the target).
if [ -f ui.hl ]; then
  echo ">> building ui (Hale @export locus controller)"
  { echo 'target wasm { }'; echo; cat ui.hl; } > "$tmp/ui.hl"
  "$HALE" build "$tmp/ui.hl" --target wasm32 --wrap-main
  mv "$tmp/ui.wasm" "$tmp/ui.mjs" "$DIST/"
fi

echo ">> done -> $DIST"
echo "   serve it:  (cd site && python3 -m http.server 8000)  then open http://localhost:8000/"
