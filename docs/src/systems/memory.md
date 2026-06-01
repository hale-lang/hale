# Memory & lifetime

> **Coming from Rust / C++?** No garbage collector, and no borrow
> checker either. Memory is *region-based*: every locus owns an
> arena, allocations inside it are bump-pointer cheap, and the
> whole region frees in one shot when the locus dissolves. The
> locus tree *is* the ownership graph — so lifetimes are
> structural, not annotated. You never write `free`, and you never
> fight a borrow checker, because no pointer ever crosses sideways.

You've used loci for pages without thinking about memory, because
the model is automatic. Here's what's underneath.

## A locus owns a region

Every locus has an arena — a region of memory. Everything the
locus allocates (strings it builds, records it constructs,
collection storage) comes from that arena. When the locus
dissolves, the entire region is freed at once. There is no
per-object deallocation, ever.

Regions nest exactly like loci do. A child's region is a
sub-region of its parent's:

```
  root
  └── App's region
      └── Server's region
          ├── Conn A's region
          └── Conn B's region
```

When a locus dissolves, its whole subtree of regions frees
wholesale. This is why [shutdown](../services/lifecycle.md)
cascades cleanly and why [flow children](../services/parents-children.md)
reclaim per connection: freeing is structural, not traced.

## Why no GC and no borrow checker

Both exist to answer one question — *when is it safe to free
this?* Hale answers it structurally instead:

- **No pointer crosses sideways.** [Vertical-only
  flow](../services/parents-children.md) means a value in one
  locus's region is never referenced by a sibling. So when a
  region frees, nothing dangles into it.
- **Messages are copies, not pointers.** A payload crossing a
  locus boundary is copied into the receiver's arena. Sender and
  receiver have independent lifetimes; the sender can dissolve
  while the receiver still holds its copy.

With those two invariants, wholesale-free-at-dissolve is sound
with no tracing and no aliasing analysis. The discipline the
borrow checker enforces with annotations, Hale enforces with
structure — you got it for free by building a locus tree.

## Bounded storage: capacity slots

The arena is for transient, locus-lifetime allocation. When a
locus needs *bounded, disciplined* storage — a recycling pool, a
growable buffer — it declares **capacity slots**:

```hale
locus Router {
    capacity {
        heap routes  of Route;     // growable, individually freed
        pool sessions of Session;  // fixed-shape, recyclable cells
    }
}
```

- **`heap X of T`** — growable storage, cells allocated and freed
  individually during the locus's life, the whole slot reclaimed
  at dissolve.
- **`pool Y of T`** — a bounded population of fixed-shape,
  recyclable cells (acquire / release).

The [forms](./forms.md) you've been using — `@form(vec)`,
`@form(hashmap)` — are built on exactly these slots; the form
annotation just synthesizes the method surface over them. Slots
hold *values*, never locus references: locus membership goes
through [`accept`](../services/parents-children.md), not storage.

## Projection classes: committing to resolution

When a parent has *many* children, you can commit up front to the
resolution at which it observes them — which lets the compiler
pick the allocator that makes that resolution cheap:

```hale
locus WorkerPool : projection chunked {
    accept(w: Worker) { }
}
```

- **`rich`** — a handful of named children (≈4–10), each fully
  observed. Per-child arenas, low churn.
- **`chunked`** — moderate counts (≈10–30), observed in ranges.
  Per-child sub-regions with free-list reuse — the default when a
  locus accepts children.
- **`recognition`** — large populations (≈100–500), observed in
  aggregate (a count, a histogram). Pre-allocated fixed pools.

The projection class changes the allocator strategy, not your
code: the same parent and child methods read from a `rich` pool
or a `recognition` pool unchanged. It's a commitment about
*observation resolution*; the compiler turns that into a layout.

## Sizing is hints, lifetime is law

Declared sizes are hints — an arena that out-allocates its budget
just adds another chunk; it doesn't panic. The load-bearing
property is *lifetime*: wholesale free at dissolve. That's the
contract every other guarantee leans on.

Next: keeping a long-running program's memory flat —
[Performance](./performance.md).
