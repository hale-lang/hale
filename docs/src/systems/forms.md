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

The six forms and what they require:

| Form | Backing slot | Lowers to | Synthesized surface |
|---|---|---|---|
| `@form(vec)` | one `heap` | doubling contiguous buffer | `push`, `get`, `set`, `pop`, `len`, `is_empty`, `sort*`, `clear`, `truncate` |
| `@form(hashmap)` | one `pool` + `indexed_by` | intrusive open-addressing table | `set`, `get`, `has`, `remove`, `len`, `is_empty`, `key_at`, `entry_at`, `bump` |
| `@form(set)` | one `pool` + `indexed_by` | the hashmap table, value never surfaced | `insert`, `contains -> Bool`, `remove`, `len`, `is_empty` |
| `@form(ring_buffer, cap=N)` | one `pool` | fixed circular buffer | `push -> Bool`, `pop`, `len`, `is_full` |
| `@form(lru_cache, cap=N)` | one `pool` + `indexed_by` | fixed keyed table, LRU eviction | `put`, `get`, `contains`, `len`, `remove` |
| `@form(heap)` | one `heap` + `ordered_by min\|max <field>` | binary heap over a doubling buffer | `push`, `pop`, `peek`, `len`, `is_empty`, `clear` |

`get` / `pop` / `remove` (and a vec's `set`) are `fallible`
(bounds / missing-key / empty); `push` on `vec` is infallible, on `ring_buffer` returns
`Bool` (full is a normal condition, not an error). `lru_cache` is
the cap-bounded keyed form: `put` is infallible and silently
evicts the least-recently-**used** entry over `cap` (a `get`
counts as a use and saves an entry from eviction; `contains` does
not). Its `get` is `fallible(KeyError)` on a miss, and so is
`remove(k)`, which takes the entry out and hands you the value. A
cache whose cells own something outside memory — a socket, a file —
declares `fn on_evict(k: K, v: S)` on the locus: the cache calls it with
each entry it evicts, before reclaiming the cell, so the cell can be
closed.

Both a `vec` and a `hashmap` also expose **batched iteration** —
`for x in v.items { … }` walks the vec, and
`for e in m.entries { … }` walks the map. The loop is an inline
buffer/slot walk, not per-element method calls. (Don't mutate the
form inside the body — a grow would rehash under the cursor.)

A form owns what it holds. A `vec`'s `push` / `set` and a
`hashmap`'s `set` (and an `lru_cache`'s `put`) store the form's own copy of the value —
its Strings and Bytes, and any nested record, tuple, array or
payload-carrying enum with what that owns — and when a `set` replaces an element or cell, or a
`pop` / `remove` (or a vec's `clear` / `truncate`, or a cache's
eviction) drops one, all of it is reclaimed and reused, at
any depth, so a map or vec churned for hours keeps a flat arena,
under every sync discipline. A
value you read out stays whole: a vec's `get` hands you a copy, a
synced map's read (`serialized`, `striped` or `lockfree`) copies the
cell out for the reader, and a single-pool map's reclamation waits
for the end of the current activation. Reading a nested record out of a map cell and setting it
back under the same key costs nothing; storing it under another key
copies it.

By default a `@form(hashmap)` is single-pool: its densest layout
has no synchronization, and a *cross-pool* call into one instance
is rejected. "Cross-pool" is judged per instance, at the call
site: two loci that each hold their own form field on different
pools are two separate, single-threaded maps — each touched only
by its owner's pool — so neither is flagged and neither needs a
discipline. (You don't need byte-identical twin types to place
two same-type forms on two pools.) The rejection is for genuine
sharing: reaching one instance from a pool other than the one it
lives on. Opt into that with the `sync = …` parameter —
`@form(hashmap, sync = serialized)` (per-map mutex),
`sync = striped` (cell-level CAS, parallel writers), or `sync = lockfree`
(CAS-only steady state) — trading layout density for the sharing
discipline the workload needs.

A map you leave unconfigured is not always single-pool: when one
instance of it is called from more than one pool, the compiler
infers a discipline for it (`serialized`, or `striped` for several
writers on a hot path), lays the map out with it, and accepts the
cross-pool calls that discipline makes safe. The question is asked
per instance: two workers on two pools that each hold their own
map need nothing, and one map `fn main` hands to both does. Write `sync = none`
to keep a map single-pool regardless: inference leaves it alone,
and a cross-pool call into it is rejected. A `sync = none` map
takes no lock, so it is not one to the other checks either: a
`@no_block` method may read it, an `@effects(depends: …)` locus
may hold it, and one instance holding it, shared across two pools,
gets the unsynchronized-state warning.

## The performance contract

Each form commits to a performance band, measured by
microbenchmarks:

- **Tight-loop primitive** (`push`) — within ~10% of idiomatic
  C. `@form(vec).push` hits this.
- **Amortized workload** — within ~2× of the C equivalent.
- **Per-op fallible** (`get` through the fallible ABI) — no tight
  bound; advisory, because the fallible return shape costs real
  cycles. `@form(vec)`'s `get` / `set` / `pop` / `push` are inlined
  at codegen, so for a vec the gap is largely closed.

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
- **Bounded keyed cache, evict least-recently-used on full** →
  `@form(lru_cache, cap = N)` (`indexed_by` names the key field).
- **Always take the smallest (or largest) next** — a priority
  queue, a timer list, a search frontier → `@form(heap)`
  (`ordered_by min <field>` or `ordered_by max <field>`).

One form per locus — a locus is one container. Need two? That's
two loci, which is usually the cleaner decomposition anyway.

## Orthogonal to projection class

A form governs how a locus stores *cells of a value type*. A
[projection class](./memory.md) governs how a parent serves
observations of its *accepted child loci*. They operate on
different things and compose freely on the same locus:

```hale
type Session { id: String; user: String; }
locus Worker { }

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
