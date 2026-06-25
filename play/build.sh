#!/usr/bin/env bash
# Build the static "run Hale in your browser" tour.
#
# For each examples/<name>.hl it produces, in site/dist/:
#   <name>.wasm   <name>.mjs   — compiled with `hale build --target wasm32`
#   <name>.hl                  — the clean source the page displays
#   manifest.json              — the example index (copied from examples/)
#
# Each example is authored as a normal Hale program ending in `fn main()`.
# The wasm target has no `main` (the host drives `@export` entries), so we
# wrap it: prepend `target wasm { }` and rewrite `fn main() { … }` into an
# `@export locus __Tour { birth() { … } }`. The loader's `_hale_start` runs
# birth() once on load; `println` is captured by the page.
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
  if grep -q '^fn main() {$' "$src"; then
    # batch program: wrap `fn main() { … }` as an @export entry locus
    {
      echo 'target wasm { }'; echo
      sed 's/^fn main() {$/@export locus __Tour {\n    birth() {/' "$src"
      echo '}'
    } > "$built"
  else
    # already an @export locus (e.g. a frame()-driven sim): just set the target
    { echo 'target wasm { }'; echo; cat "$src"; } > "$built"
  fi

  echo ">> building $name"
  "$HALE" build "$built" --target wasm32
  mv "$tmp/$name.wasm" "$tmp/$name.mjs" "$DIST/"
  cp "$src" "$DIST/$name.hl"
done

cp examples/manifest.json "$DIST/manifest.json"

# The tour UI itself is a Hale @export locus — the page's interaction logic
# lives here, not in JS. Built like a no-main example (just set the target).
if [ -f ui.hl ]; then
  echo ">> building ui (Hale @export locus controller)"
  { echo 'target wasm { }'; echo; cat ui.hl; } > "$tmp/ui.hl"
  "$HALE" build "$tmp/ui.hl" --target wasm32
  mv "$tmp/ui.wasm" "$tmp/ui.mjs" "$DIST/"
fi

echo ">> done -> $DIST"
echo "   serve it:  (cd site && python3 -m http.server 8000)  then open http://localhost:8000/"
