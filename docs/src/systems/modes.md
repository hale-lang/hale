# Modes

> **Coming from Rust / C++?** Think of modes as asking the
> compiler to emit a different execution strategy for the *same*
> computation over the *same* state — vectorized throughput,
> cache-tiled per-class work, or a single scalar decision —
> without you maintaining three copies. It's the most specialized
> feature in the language; most loci never declare one.

## Three named projections of one kernel

A locus can declare up to three **modes**, each a named
projection of the same underlying computation, operating on the
same locus state:

```hale,fragment
locus Pricer {
    params { /* shared state */ }

    mode bulk(...)       -> ... { /* vectorized over many inputs */ }
    mode harmonic(...)   -> ... { /* per-class / cache-tiled */ }
    mode resolution(...) -> ... { /* one decision, scalar */ }
}
```

You invoke a mode like a method — `self.bulk(...)`,
`self.resolution(...)` — and declare only the subset you actually
operate in. They map to genuinely different hardware execution
regimes:

- **`bulk`** — vectorized throughput: the same operation across
  many elements at once.
- **`harmonic`** — cache-tiled, per-class projection: work
  organized so each class's data stays resident.
- **`resolution`** — a single scalar decision: the
  one-input-one-answer path.

The compiler emits a strategy tuned to each regime, rather than
running one general implementation everywhere.

## They share the arena

All three modes read and write the *same* locus state through the
same [arena](./memory.md) — there's no duplicate allocation and
no copy between them. Because they can touch the same fields, the
compiler verifies the modes don't *write-conflict*: a
`resolution`-mode write to state that `bulk` mode also writes
during overlapping evaluation is a compile-time error. You get
three execution strategies over one piece of state, with the
aliasing hazard checked for you.

## Why three, and no fourth

The count isn't arbitrary minimalism — it's that vectorized,
cache-tiled, and scalar are three distinct cost regimes on real
hardware (high-throughput SIMD, locality-bound per-class, and
latency-bound single-decision). There's no fourth regime the
hardware rewards, so there's no fourth mode. The same commit-hard
discipline as the [three projection
classes](./memory.md) for memory.

## When you'll reach for this

Rarely, and only at this tier — when a locus has a kernel
computation that genuinely runs in more than one of those
regimes (a numeric model evaluated both in batch and
per-decision, say) and you want each path lowered well from one
declaration. For ordinary application and service code, you'll
never declare a mode; the lifecycle methods and `fn` members
cover everything.

---

That's the systems tier — and the bottom of the descent. You
started with variables and functions; you've now seen the memory
model, the allocation disciplines, zero-copy transport, the C
boundary, cross-process state, and hardware execution regimes.
Every one of them is the *same locus* you met in *the basics*,
observed at greater and greater resolution.

To see why one shape holds across all four tiers — and across
human, LLM, and machine — read [The design](../the-design.md).
For exact rules, the [reference](../reference.md) points into the
canonical spec.
