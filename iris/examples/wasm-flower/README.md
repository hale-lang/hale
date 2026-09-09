# wasm-flower — the render-loop spike (2026-07-27)

Answers the question that gated the browser render decision
(DESIGN §11): **can hale's wasm32 target host the flower's
render loop?** Yes, with ~100× frame-budget headroom.

What one Hale module proves end to end:

- `target wasm { }` + `@export fn frame(t_ms)` compiles via
  `hale build flower.hl --target wasm32` into a ~13KB module
  plus a generated `.mjs` loader.
- Entry-inversion: the loader calls `_hale_start` (persistent
  program arena), then the host drives `frame()` per
  `requestAnimationFrame`. State in the runtime state cell
  (`lotus_wasm_state_*`) survives across calls — a frame
  counter accumulated over 660 host calls.
- `@ffi("js")` host imports are the draw surface
  (`ctx_clear` / `ctx_petal` / `ctx_edge`); `std::math` trig
  maps to JS `Math` with no glue.

Measured (node harness, 600 timed frames after warmup):

| scene | wasm time / frame | 60fps budget used |
|---|---|---|
| 36 petals, 2 flowers | 4.8 µs | 0.03% |
| 3600 petals | 89.6 µs | 0.5% |

Linear scaling; canvas rasterization will bottleneck long
before the Hale side does.

## Run

```
hale build flower.hl --target wasm32   # emits flower.wasm + flower.mjs
node harness.mjs                       # validates + times the loop
python3 -m http.server 8971            # then open /flower.html
```

The animation here is synthetic (petal count and phases are
made up). The real M1 flower renders fused observed state —
see `consumer/fuse` serve mode and `render/web/`.
