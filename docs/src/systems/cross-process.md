# Cross-process & hot-load

> **Status + terminology.** This chapter describes the
> *transport-driven* perspective — a serializable state bundle
> shipped between processes and hot-loaded from the wire. That path
> is **design/aspirational**: it is specified but not yet shipped.
> It reuses the `perspective` keyword but is distinct from the
> shipped, in-process perspective — a live-swappable *contract* + a
> program-global slot re-pointed with `reperspective` — covered in
> [Perspectives](../services/perspectives.md) (and normatively in
> `spec/semantics.md`). Both are the same idea (a stable, versioned
> boundary you can re-point at pointer-flip cost); today only the
> in-process contract/slot path runs.

> **Coming from Rust / C++?** This is typed, versioned state
> shipped between processes — but without a separate `.proto` and
> a codegen step. A `perspective` is a serializable parameter
> bundle; producer and consumer share its schema *because they
> compile from the same source*. No protobuf regen, no schema
> drift, no handshake.

## A perspective is a shippable view

Most of a locus's state is private to its region. A
`perspective` is the exception: a typed bundle a locus can
publish across a process boundary, with a compile-time guarantee
that the other side agrees on its shape.

```hale
perspective Kernel {
    params {
        scale_row:    [Decimal; 8];
        sigma_factor: Decimal;
        regime_id:    Int;
    }
    stable_when {
        return self.num_validated >= 3;
    }
    serialize_as KernelV1;
}
```

- **`params`** is the payload — the schema *is* this type.
- **`stable_when`** is a predicate the runtime checks before the
  perspective is allowed to ship — "is this data ready?" lives in
  the data's own declaration, not in a publisher flag.
- **`serialize_as`** names the wire format stably, so you can
  rename the identifier without breaking serialization.

A perspective is not a locus — no lifecycle, no bus block, no
methods beyond `stable_when`. It's a validated, serializable
bundle the substrate knows how to ship.

## The fitter / applier pattern

The canonical use: one process computes parameters slowly and
carefully; another applies them at high frequency. Both compile
from the same `Kernel` declaration, so the type is the protocol.

```hale
// fitter — publishes refined Kernels
locus Fitter {
    bus { publish KernelUpdates; }
    run() {
        let mut k = compute_kernel(observations);
        while !k.is_stable() { k = refine_kernel(k, more()); }
        KernelUpdates <- k;
    }
}

// applier — swaps in the latest, atomically
locus Applier {
    params { current: Kernel = default_kernel(); }
    bus { subscribe KernelUpdates as on_update; }
    fn on_update(k: Kernel) { self.current = k; }   // atomic swap; no torn read
}
```

The runtime guarantees the consumer-side swap is atomic — readers
see the old perspective or the new one, never a half-written
mix. This is also the **hot-load** mechanism: reconfigure a
long-running service by publishing a new perspective, with full
type-checking against the locally-compiled schema, no restart.

## Capability profiles and substrates

The same locus + bus + perspective triple runs on more than one
*substrate*. The native C-runtime is one; the browser runtime
([hale-js](https://github.com/hale-lang/hale-js)) is another. A
build target declares the capabilities a substrate offers:

```hale
target browser_js {
    arenas.epoch_view,
    time.monotonic, time.wallclock,
    random.csprng,
    gfx.canvas2d,
}
```

A program that reaches for a capability its target doesn't offer
fails at the *translation boundary* with a clear `CAP-MISSING`
diagnostic — at build, not at runtime. Substrate differences are
named and checked, not papered over. The locus you wrote doesn't
change between substrates; the capability profile and the
[transport binding](../services/multi-binary.md) do.

This is the long arc of the whole guide paying off: the same
shape you met as a small program in *the basics* runs across
processes, machines, and substrates because nothing in the shape
depended on where it ran.

Next, the most specialized tier feature — [Modes](./modes.md).
