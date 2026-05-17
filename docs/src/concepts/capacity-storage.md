# Capacity & storage

> **α** — How does a locus declare what it holds, and how does
> that commitment shape its lowering?

A locus's `params` declare its baseline state — typed fields,
mutable from any of its methods, alive for the locus's
lifetime. That's enough for many loci. But once a locus needs
to hold a *collection* — a queue of pending work, a hashmap of
sessions, a recent-events ring buffer — `params` runs out. You
need bounded storage with a discipline.

This chapter covers three layered concepts: **capacity slots**
(the substrate-level storage primitives), **projection
classes** (how a locus declares the *resolution* at which it
serves observations of its children), and **forms** (the
application-layer storage discipline annotations: `@form(vec)`,
`@form(hashmap)`, `@form(ring_buffer)`).

## The implicit Arena

Every locus has an implicit *slot 0*: its **Arena**. The Arena
is a bump allocator for everything the locus's body
short-livedly allocates — string concatenations, struct
literals constructed inside a method, transient values.
Allocations into the Arena are freed *wholesale* when the
locus dissolves; nothing else needs to track them.

You never write the Arena down. It's there because it's
universal. When this chapter talks about *capacity slots* it
means slots 1..N, the storage commitments above the implicit
floor.

## Slot kinds: `pool` and `heap`

A `capacity { ... }` block declares 1..N storage slots:

```aperio
locus Matchmaker {
    capacity {
        heap waiting of Player;    // slot 1: growable, locus-bounded
        pool sessions of Cell;     // slot 2: fixed-shape, recyclable
    }
}
```

Two slot kinds, two commitments:

- **`heap X of T;`** — *growable storage bounded by my own
  lifetime*. Individual cells alloc and free during the
  locus's life; the whole region frees wholesale at dissolve.
  This is the right shape for things whose retained size
  isn't known at construction.
- **`pool Y of T;`** — *bounded recyclable cells of a fixed
  shape*. The population is bounded; individual values come
  and go, but the slot doesn't grow indefinitely. Right for
  map-style buckets, fixed-shape registries, per-handler
  scratch frames.

The slot name is yours; idiomatic names are `waiting`,
`entries`, `bindings`, `routes`, `bytes`. The cell type can be
any value-shape: a primitive, a `type` struct, a generic
parameter. **Slots cannot hold locus references.** Locus
membership goes through `accept(c: Child)`, not slots — slots
are for values.

At this layer the user-facing API is method-shaped. A `heap`
slot exposes `alloc()` and `free()`; a `pool` slot exposes
`acquire()` and `release()`:

```aperio
let cell = self.entries.acquire();
// ... mutate cell ...
self.entries.release(cell);
```

This is fine for some uses, but verbose for most. The **forms
layer** replaces it with method sets that match how you'd
normally think about the storage.

## Forms — the high-level annotation

A `@form(...)` annotation on a locus picks a high-level
lowering for one of its capacity slots and synthesizes a
matching method set. The user writes the locus once; the
compiler emits a tight, hand-rolled-C-class implementation.

Three forms ship in v1:

### `@form(vec)` — growable contiguous buffer

```aperio
@form(vec)
locus PlayerQueue {
    capacity { heap items of Player; }
    // synthesized: push, get, set, pop, len, is_empty,
    //              sort, sort_by, sort_desc_by
}

fn main() {
    let q = PlayerQueue { };
    q.push(Player { id: "p1", name: "Anna" });
    q.push(Player { id: "p2", name: "Bo" });
    let first = q.get(0) or raise;
}
```

The Aperio analogue of `Vec<T>` / `std::vector<T>` / Go slices.
Backed by a doubling-realloc buffer. `push` is amortized O(1).
`get` and `pop` are `fallible(IndexError)` — see the next
chapter on the failure channels for what `or raise` means.

`@form(vec)` requires exactly one `heap` slot. The slot's cell
type becomes the vec's element type.

### `@form(hashmap)` — intrusive open-addressing table

```aperio
type CmdEntry { name: String; handler: Int; }

@form(hashmap)
locus CmdRegistry {
    capacity { pool entries of CmdEntry indexed_by name; }
    // synthesized: set, get, has, remove, len, is_empty,
    //              key_at, entry_at, bump
}

fn main() {
    let r = CmdRegistry { };
    r.set(CmdEntry { name: "spawn", handler: 1 });
    let entry = r.get("spawn") or raise;
}
```

The Aperio analogue of `Map<K, V>` / `std::unordered_map`. The
key is *intrusive* — the cell type carries its own key as a
named field declared via `indexed_by`. `set(value)` takes the
whole value and extracts the key. This shape is structurally
different from `HashMap<K, V>` (no separate K and V slots) and
reflects how real keyed stores almost always look in practice:
the key is one of the fields.

`@form(hashmap)` requires exactly one `pool` slot with an
`indexed_by FIELD` clause. The slot's cell type must be a
user-declared struct; the indexed-by field must be `Int` or
`String`.

### `@form(ring_buffer, cap = N)` — fixed-capacity FIFO

```aperio
@form(ring_buffer, cap = 64)
locus RecentCmds {
    capacity { pool history of CmdEntry; }
    // synthesized: push -> Bool, pop -> fallible(EmptyError),
    //              len, is_full
}
```

A bounded circular buffer. `push` returns `Bool` — `true` on
success, `false` when the buffer is at capacity (so callers
choose drop-vs-backpressure). `pop` is fallible-on-empty.

`@form(ring_buffer)` requires a `pool` slot and the
annotation arg `cap = N` (positive integer literal).

## Why forms instead of `Vec<T>`?

Two reasons.

**The structural reason.** A growable buffer is a *storage
discipline*, not just a parameterized type. `Vec<T>` in Rust
glues "contiguous memory, dynamic length, owning the cells"
into one type. But in Aperio's substrate, every one of those
commitments is a separate decision: who owns the memory (the
locus does), where it lives (in the locus's slot), how it
grows (doubling realloc), what happens on dissolve (region
freed). The `@form(vec)` annotation makes those decisions
explicit at the declaration site.

**The pragmatic reason.** Each form has a single canonical
lowering tuned for the substrate. `@form(vec)`'s lowering is
within a few percent of hand-written C for push-heavy
workloads (verified by a microbench in `bench/micro/`). You
don't get a slow generic implementation that "works for any
type"; you get a tight implementation specialized for your
cell type via monomorphization.

The downside, in fairness: you can't pass a `@form(vec)` of
`Player` as an argument of type `Vec<Player>` to some library
function expecting a generic collection. The forms are
locus-shaped: each form is a locus type. If you want shared
APIs across forms, you write an interface (see
[The locus](./the-locus.md) on `interface I { ... }`).

## Projection classes

Forms are about *how a locus stores cells of a type*.
Projection classes are about something different: *how a
parent locus serves observations of its accepted children to
the observer above it.*

```aperio
locus Pool : projection chunked {
    accept(w: Worker) { /* ... */ }
}
```

Three projection classes:

- **`rich`** — fine-grained. The parent serves observations of
  *named individual children*. Typical N ≈ 4-10. Each child
  carries its own state worth observing in detail. Storage
  consequence: per-child arenas, low churn.
- **`chunked`** — mid-grained. The parent serves observations
  over chunks or ranges of its children. Typical N ≈ 10-30.
  Storage consequence: per-coordinatee sub-regions inside the
  parent's arena, freed on each child dissolution.
- **`recognition`** — aggregate. The parent serves
  *population-level* views ("represent as a histogram", "as a
  curve", "as a count"). Typical N ≈ 100-500. Individual
  children are not addressed by name. Storage consequence:
  pre-allocated fixed pool sized at parent birth; cell stride
  derived from the accept-method type union.

The projection class affects allocator strategy, sub-region
nesting, and the cost of iterating `self.children`. It does
*not* affect the surface methods on the parent or the
children — same code reads from a `rich` pool or a `chunked`
pool. The annotation is a *commitment about resolution*; the
compiler picks the allocator that makes that resolution cheap.

You rarely need to think about projection classes when writing
ordinary application code. They become load-bearing when
you're designing a parent that genuinely has many children
(workers, sessions, agents) and you want to commit to the
observation resolution upfront.

## Forms and projection classes are orthogonal

Both annotations can appear on the same locus:

```aperio
@form(hashmap)
locus SessionPool : projection chunked {
    capacity { pool sessions of Session indexed_by id; }
    accept(w: Worker) { /* ... */ }
}
```

`@form(hashmap)` controls how `sessions` slot's storage is
laid out and what methods get synthesized. `projection
chunked` controls how the parent serves observations of its
accepted `Worker` children. The two operate on different
slots of different shape and don't interfere.

## When to use what

| You need | Reach for |
|---|---|
| One value per field | `params` |
| Growable list of T | `@form(vec)` |
| Keyed store, key is a field of T | `@form(hashmap)` |
| Bounded FIFO, drop-on-full | `@form(ring_buffer)` |
| Parent holds many children, named | `accept` + `rich` projection |
| Parent holds many children, chunked | `accept` + `chunked` projection |
| Parent holds many children, aggregate | `accept` + `recognition` projection |
| Raw cell recycling with custom logic | `pool X of T` directly |

## Next

The next chapter, [Error handling](./error-handling.md),
covers the two orthogonal failure mechanisms — closures /
`on_failure` for structural failure, and `fallible(E)` /
`or`-disposition for value-level errors — and the rule for
which one to use where.
