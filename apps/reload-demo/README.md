# reload-demo

The application-layer kernel + reload-lotus pattern, end-to-end
on currently shipped primitives.

## What it demonstrates

The user's framing:

> imagine two apps, each receiving raw market data. contain the
> same market data locus internally and consume the same data.
> they each have different goals, but operate on the same data.
> the idea with the configurable kernel is that if they both have
> the right contracts in their kernel, one app could be computing
> like, price targets using curve geometry. they could then ship,
> via the over-the-wire-dimention-hotswapping, like an overlay
> and mechanical lotus description that should allow other apps
> to take the output and overlay it internally back onto the same
> internal data

This demo wires that pattern up in a single binary (real two-binary
deployment via `std::bus::tcp` is a future arc). The locus
inventory:

- **`MarketStateL`** — the shared inner lotus. Both simulated
  apps instantiate it by the same declaration; that's the "same
  market data locus internally" point. Holds an 8-cap tick ring,
  fits curves via `SegmentL` on demand.
- **`ReloadL`** — the *reload-lotus*. Subscribes to
  `"model.curve"`. When a `ModelOverlay` arrives, it
  reparameterizes itself: installs `slope`, `intercept`, and
  the `source_coord` of the producer. The parent market reads
  the installed model through this locus.
- **`FitterAppL`** — the application-layer kernel for the
  curve-fitting role. Reads a market, fits a curve, publishes
  the overlay over `"model.curve"`. Different roles configure
  different fitter loci; the kernel (B/c/sigma/phi + coord +
  bus surface) is what differentiates them.
- **`SegmentL`** — least-squares regression accumulator,
  lifted from `examples/51-geom-segment`.
- **`Kernel`**, **`ModelOverlay`** — the kernel record + the
  portable lotus description that crosses the wire.

`main()` instantiates two markets (`fitter.market`,
`consumer.market`), one reload-lotus alongside each
(`fitter.reload`, `consumer.reload`), and the fitter app. It
feeds identical synthetic ticks to both markets, asks the fitter
to publish its overlay, and yields the cooperative bus queue so
both reload-lotus instances receive and install the overlay.
After install, each market can `extrapolate_via(reload, t)` —
the asserts verify the line round-trips (y = 2t + 1 → 11 at
t = 5).

## How to run

From the repo root:

```
cargo build --release -p aperio-cli
target/release/aperio build apps/reload-demo/
apps/reload-demo/reload-demo
```

Exit 0 silent (after the kernel rows + "all reload-demo
assertions passed") is the pass condition. Assertion failures
print `ASSERTION FAILED: <label>` with diagnostics and exit 1.

## v0 constraints, captured in code

- **Reload-lotus binds as a top-level sibling, not a child of
  the market.** The user's vision has the reload sitting *under*
  the market lotus; the v0 codegen path for nested-locus field
  reads through `for child in self.children` returns garbage for
  loci spawned via statement-position long-lived literals in
  `birth()`. The architectural intent is preserved in prose +
  comments and waits on the codegen fix. Logged in
  `notes/aperio-friction.md`.
- **`self.k_max` is interpreter-only.** The F.16 built-in
  computed field works in `aperio run` but errors at `aperio
  build`. The kernel rows display `B / c / sigma / phi` instead.
- **No `aperio run` path.** External method calls from `fn
  main()` (e.g., `market.feed_tick(...)`) trip the interpreter's
  empty-self-stack check. The build path works correctly.
- **Cached locus-method results.** Calling `s.count()` twice in
  sequence (where `s` is a let-bound locus returned from another
  locus's method) can read stale state on the second call. The
  fix is to bind the result once and reuse: `let n = s.count();
  if n >= 2 { ... }`.

## Cross-references

- `docs/src/std/roadmap.md` — "Future arc — Lotus harness for
  n-dim growth" (the roadmap entry this demo is a first concrete
  push toward; the arc name predates the user's clarification
  that this is the *application-layer kernel*, not the harness
  proper).
- `examples/51-geom-segment` through `examples/58-stat-correlate`
  — the library sketches this demo composes.
- `notes/aperio-friction.md` — the four pieces of friction that
  shaped the final demo structure.
