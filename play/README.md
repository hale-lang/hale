# play — run Hale in your browser

A static, zero-backend tour: each example is real Hale compiled to
WebAssembly with `hale build --target wasm32` and run client-side. No
server, no sandbox — just static files you can host on GitHub Pages.

## Build & serve

```sh
./build.sh                              # compile examples/*.hl -> site/dist/
cd site && python3 -m http.server 8000  # then open http://localhost:8000/
```

`build.sh` finds the compiler at `../target/release/hale` (run
`cargo build --release` first), or set `HALE=/path/to/hale`.

## Adding an example

1. Write `examples/<name>.hl` as a normal Hale program ending in
   `fn main() { … }` using only the wasm-safe stdlib (`std::str`,
   `std::bytes`, `std::json`, `std::math`, the typed bus — **not**
   `std::io::tcp` / `std::process` / `std::http`, which the browser
   sandbox rejects).
2. Add an entry to `examples/manifest.json` (`name`, `title`, `blurb`).
3. `./build.sh`.

`build.sh` wraps each program for the wasm target — it prepends
`target wasm { }` and rewrites `fn main() { … }` into an
`@export locus __Tour { birth() { … } }`, which the generated `.mjs`
loader drives via `_hale_start`. `println` output is captured by the
page.

## Layout

```
examples/        authored .hl sources + manifest.json
build.sh         examples -> site/dist/ (wasm + loader + display source)
site/index.html  the frontend (example list, source, Run, output)
site/dist/       build output (gitignored; rebuilt by CI for deploy)
```
