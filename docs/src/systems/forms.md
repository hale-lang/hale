# Forms under the hood

> **Coming from Rust / C++?** A form is closer to a monomorphized
> template than to a generic collection object. `@form(vec)`
> doesn't wrap a one-size-fits-all container — the compiler emits
> a tight, type-specialized implementation per cell type, sized
> and laid out for *your* element. You declared the access
> discipline at [the everyday level](../everyday/collections.md);
> here's what it lowers to and how to choose.

## A form is a lowering, not a type

When you write:

```hale
@form(vec)
locus Names {
    capacity { heap items of String; }
}
```

the compiler doesn't reach for a library `Vec`. It synthesizes,
for *this* locus and *this* cell type, a contiguous growable
buffer and the methods over it — `push`, `get`, `set`, `pop`,
`len`, `is_empty`, and the sort family. The storage is the
`heap` [capacity slot](./memory.md); the form decides the layout
(here: a `{cap, len, buf}` struct with doubling realloc) and the
method surface.

The three forms and what they require:

| Form | Backing slot | Lowers to | Synthesized surface |
|---|---|---|---|
| `@form(vec)` | one `heap` | doubling contiguous buffer | `push`, `get`, `set`, `pop`, `len`, `is_empty`, `sort*` |
| `@form(hashmap)` | one `pool` + `indexed_by` | intrusive open-addressing table | `set`, `get`, `has`, `remove`, `len`, `is_empty` |
| `@form(ring_buffer, cap=N)` | one `pool` | fixed circular buffer | `push -> Bool`, `pop`, `len`, `is_full` |

`get` / `pop` / `remove` are `fallible` (bounds / missing-key /
empty); `push` on `vec` is infallible, on `ring_buffer` returns
`Bool` (full is a normal condition, not an error).

## The performance contract

Each form commits to a performance band, verified by
microbenchmarks in the tree:

- **Tight-loop primitive** (`push`) — within ~10% of idiomatic
  C. `@form(vec).push` hits this.
- **Amortized workload** — within ~2× of the C equivalent.
- **Per-op fallible** (`get` through the fallible ABI) — no tight
  bound; advisory, because the fallible return shape and the
  function-call boundary cost real cycles.

The point: a form isn't a slow generic that "works for any type."
It's a specialized implementation monomorphized to your cell
type. The cost is that a `@form(vec)` of `Player` isn't
interchangeable with some library's `Vec<Player>` — there's no
such shared generic. If you want a shared API across forms, you
declare an `interface`.

## Choosing a form

- **Growable, ordered, index access** → `@form(vec)`.
- **Keyed lookup, key is a field of the value** → `@form(hashmap)`
  (`indexed_by` names the key field).
- **Bounded window, drop-or-backpressure on full** →
  `@form(ring_buffer, cap = N)`.

One form per locus — a locus is one container. Need two? That's
two loci, which is usually the cleaner decomposition anyway.

## Orthogonal to projection class

A form governs how a locus stores *cells of a value type*. A
[projection class](./memory.md) governs how a parent serves
observations of its *accepted child loci*. They operate on
different things and compose freely on the same locus:

```hale
@form(hashmap)
locus SessionStore : projection chunked {
    capacity { pool sessions of Session indexed_by id; }
    accept(w: Worker) { }
}
```

`@form(hashmap)` lays out the `sessions` value store;
`projection chunked` sizes the allocator for the accepted
`Worker` children. Different slots, no interference.

## Cells are data

A form cell can be a primitive or a `type` record — never a
locus. Storing a locus in a map would mean `get(key)` hands a
live entity to a stranger, the same antipattern the language
rejects for [methods returning
loci](../everyday/locus-gently.md). For keyed *entities*, make
them accepted children and key a parallel index by name. Cells
are values; entities are children.

Next: the fastest same-machine transport — [Zero-copy & the
high-frequency bus](./zero-copy-bus.md).
