# Execution Modes (`bulk`, `harmonic`, `resolution`)

A locus can declare up to three named **execution modes**—`bulk`, `harmonic`, and `resolution`—to express distinct hardware execution strategies (vectorized batch throughput, cache-tiled per-class passes, and single-element scalar evaluation) over the same underlying locus state and arena.

```hale,fragment
locus Pricer {
    params { base_rate: Float = 0.05; }

    mode bulk(count: Int) -> Float {
        // Vectorized pass across a batch of inputs
        return self.base_rate * to_float(count);
    }

    mode harmonic(bucket: Int) -> Float {
        // Cache-tiled pass scoped to a specific class or partition
        return self.base_rate * to_float(bucket);
    }

    mode resolution(spot: Float) -> Float {
        // Latency-sensitive single scalar decision
        return spot * self.base_rate;
    }
}
```

## The Three Execution Regimes

A locus declares only the modes it actually uses and invokes them with standard method call syntax (`self.bulk(...)`, `self.harmonic(...)`, `self.resolution(...)`):

| Mode | Hardware Regime | Computation Shape | Typical Use Case |
|---|---|---|---|
| `mode bulk(...)` | High-throughput SIMD / vector units | Applies the same kernel across a large batch of elements in one pass. | Batch pricing, array transforms, columnar scans. |
| `mode harmonic(...)` | Cache-resident / locality-tiled | Processes elements grouped by class or partition so per-class state stays in L1/L2 cache. | Bucketed simulations, multi-instrument order books. |
| `mode resolution(...)` | Low-latency scalar pipeline | Evaluates a single input to produce an immediate single decision. | Per-request admission, single-order risk checks. |

> [!NOTE]
> In the current compiler release, `mode` declarations lower identically to synchronous `fn` members on the locus; the mode keyword documents the target hardware regime in the program graph without yet emitting distinct codegen passes. Because modes are substrate-facing surfaces, they cannot be declared `fallible(E)`.

## Shared State & Arena Semantics

All modes declared on a locus operate directly on the same `params` fields and [locus arena](./memory.md):
- **Zero state duplication**: Switching between `bulk` warmup and per-event `resolution` requires no copying or synchronization between separate objects.
- **Synchronous execution**: Like ordinary locus methods, mode calls execute synchronously to completion, so two modes on the same locus instance never interleave concurrently over `self`.

## When to Declare Modes

Most application and service loci only need lifecycle hooks (`birth`, `run`, `dissolve`) and standard `fn` methods. Reach for `mode bulk`, `mode harmonic`, and `mode resolution` only when a single stateful kernel genuinely executes in multiple hardware cost regimes over the same data.

---

For the formal hypergraph specification underlying loci, projections, and modes, see [The Design](../the-design.md) and the [Language Reference](../reference.md).
