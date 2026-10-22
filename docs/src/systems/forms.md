# Forms Under the Hood

A `@form(...)` annotation on a locus instructs the compiler to lower a `capacity` slot (`heap` or `pool`) into a monomorphized, type-specialized data structure and synthesize its access methods. Rather than wrapping a generic container, the compiler emits layout-specific code tailored to your exact element `type`.

```hale
type Hits { word: String; n: Int; }

@form(hashmap)
locus WordCounts {
    capacity { pool entries of Hits indexed_by word; }
}

fn main() {
    let wc = WordCounts { };
    wc.bump("hale");
    wc.bump("hale");
    let h = wc.get("hale") or Hits { word: "hale", n: 0 };
    println(h.n); // 2
}
```

## The Six Collection Forms

Every form requires a specific `capacity` slot declaration and synthesizes a fixed set of methods on the enclosing locus (maximum one `@form` per locus):

| Form | Backing Slot | Memory Layout | Synthesized Methods |
|---|---|---|---|
| `@form(vec)` | `heap <name> of <T>` | Doubling contiguous `{cap, len, buf}` buffer | `push`, `get`, `set`, `pop`, `len`, `is_empty`, `sort*`, `clear`, `truncate` |
| `@form(hashmap)` | `pool <name> of <T> indexed_by <key>` | Intrusive open-addressing hash table | `set`, `get`, `has`, `remove`, `len`, `is_empty`, `key_at`, `entry_at`, `bump` |
| `@form(set)` | `pool <name> of <T> indexed_by <key>` | Open-addressing hash table (value hidden) | `insert`, `contains -> Bool`, `remove`, `len`, `is_empty` |
| `@form(ring_buffer, cap = N)` | `pool <name> of <T>` | Fixed-capacity circular buffer | `push -> Bool`, `pop`, `len`, `is_full` |
| `@form(lru_cache, cap = N)` | `pool <name> of <T> indexed_by <key>` | Fixed-capacity keyed table with LRU eviction | `put`, `get`, `contains`, `len`, `remove` |
| `@form(heap)` | `heap <name> of <T> ordered_by min\|max <field>` | Binary heap over a doubling contiguous buffer | `push`, `pop`, `peek`, `len`, `is_empty`, `clear` |

### Choosing a Form

- **Ordered sequence with index access**: `@form(vec)`.
- **Key-value table where the key is a struct field**: `@form(hashmap)` with `indexed_by <field>`.
- **Key membership set**: `@form(set)` with `indexed_by <field>`.
- **Bounded FIFO queue with explicit full detection**: `@form(ring_buffer, cap = N)`.
- **Bounded keyed cache with automatic LRU eviction**: `@form(lru_cache, cap = N)` with `indexed_by <field>`.
- **Priority queue or timer frontier**: `@form(heap)` with `ordered_by min <field>` or `ordered_by max <field>`.

> [!IMPORTANT]
> Form cells must be primitives or value `type` records—never loci. To manage a keyed collection of live child loci, use `accept` and route messages over a keyed bus topic (see [Service Patterns](../services/patterns.md)).

## Method Semantics & Special Operations

### Fallibility and Return Types
- **Lookup and removal**: `get`, `pop`, `peek`, `remove`, and `vec.set` are `fallible` (raising on out-of-bounds, missing key, or empty container) and must be addressed with `or` at the call site.
- **Insertion**: `vec.push`, `hashmap.set`, `set.insert`, `lru_cache.put`, and `heap.push` are infallible. On `@form(ring_buffer, cap = N)`, `push` returns `Bool` (`false` when the ring is full).

### Counter Increment (`hashmap.bump`)
On a `@form(hashmap)` whose cell type contains exclusively the `indexed_by` key field and one `Int` field (such as `type Hits { word: String; n: Int; }`), `m.bump(key)` initializes a missing entry to `1` or increments the existing counter in place. `hale check` rejects `bump` on any other cell layout.

### LRU Eviction Hooks (`lru_cache` and `on_evict`)
On `@form(lru_cache, cap = N)`, `put` inserts or updates an entry and silently evicts the least-recently-used entry when the population exceeds `cap`. Calling `get` marks an entry as recently used; calling `contains` does not.

If cached cells own external resources (such as file descriptors or sockets), define `fn on_evict(k: K, v: V)` on the cache locus:
- The runtime invokes `on_evict` just before reclaiming each evicted cell.
- When the cache locus dissolves, `on_evict` runs for every remaining entry in least-recently-used order.
- `on_evict` may not call `put` on its own cache directly or transitively (`hale check` rejects recursive cache insertion).

### Batched Inline Iteration
Both `@form(vec)` and `@form(hashmap)` expose inline slot iteration via the capacity slot's name:
- `for x in v.items { ... }` walks a vector's contiguous buffer directly.
- `for e in m.entries { ... }` walks a hashmap's occupied buckets directly without per-element method overhead.

Do not mutate or resize the collection inside a batched iteration loop, as growing the backing buffer invalidates the active cursor.

## Memory Ownership & Copy-on-Write Reads

Every form owns the storage of its elements. Inserting or updating an element (`push`, `set`, `put`) deep-copies any heap-backed fields (`String`, `Bytes`, and nested records, tuples, arrays, or payload enums) into the form's storage. When an entry is overwritten, removed (`pop`, `remove`, `clear`, `truncate`), or evicted, all nested blocks are reclaimed and recycled at any depth, keeping arena memory flat across hours of churn.

### Value Isolation and In-Place Read-Modify-Write
Mutating a variable returned by `get` never silently mutates the stored cell unless you write it back with `set`. For single-pool `@form(hashmap)` and `@form(lru_cache)` instances, `get` returns a shallow copy of top-level scalar fields while sharing nested records, arrays, tuples, and payload enums copy-on-write.

If you read an entry, update a nested field or array element, and immediately write the entry back to the same key, the compiler elides the copy and updates the cell's backing block in place:

```hale,fragment
let mut e = m.get(k) or Cell { id: k };
e.levels[j] = Level { price: p, qty: q };
m.set(e);
```

This zero-allocation round-trip applies when `m.set(e)` executes immediately on all control-flow paths, no other read of `m` occurs in between, and `e` is not read afterward. Any other mutation copies the nested allocation on write; `hale check` flags that copy as a hot-path warning inside loops or bus handlers, and as a compile error inside `@hot` functions.

## Hashmap Concurrency (`sync` Disciplines)

By default, a `@form(hashmap)` uses an unsynchronized single-pool layout when accessed from only one worker pool. If the same hashmap instance is shared across multiple pools, the compiler either infers a synchronization discipline or enforces the explicit `sync` mode you declare:

| Annotation | Synchronization Strategy | Cross-Pool Access | Notes |
|---|---|---|---|
| `@form(hashmap)` *(default)* | Auto-inferred per instance (`none` if single-pool; `serialized` or `striped` if shared) | Allowed (compiler selects safe layout) | Two separate instances on two different pools each remain lock-free (`none`). |
| `@form(hashmap, sync = none)` | No locks or atomics; densest layout | Rejected at compile time | Safe inside `@no_block` methods and `@effects(depends: ...)` loci. |
| `@form(hashmap, sync = serialized)` | Per-map mutex | Allowed | Simple synchronization for low-to-moderate contention. |
| `@form(hashmap, sync = striped)` | Cell-level CAS with parallel writers | Allowed | High-throughput concurrent reads and writes; `get` copies the cell out for the reader. |
| `@form(hashmap, sync = lockfree)` | CAS-only steady state | Allowed | Lock-free concurrent access; `get` copies the cell out for the reader. |

## Composability with Projection Classes

A `@form(...)` annotation governs how a locus lays out its value `capacity` slot, whereas a `projection` class governs how a parent locus allocates arenas for its `accept`ed child loci (see [Memory & Lifetime](./memory.md#projection-classes)). The two mechanisms are orthogonal and compose on the same locus:

```hale
type Session { id: String; user: String; }
locus Worker { }

@form(hashmap)
locus SessionStore : projection chunked {
    capacity { pool sessions of Session indexed_by id; }
    accept(w: Worker) { }
}
```

## Performance Contract

Because forms lower to monomorphized, inlined code rather than virtual or boxed generics:
- **Tight-loop primitives** (`@form(vec)` `push`, `get`, `set`, `pop`): Inlined directly at codegen, executing within ~10% of `clang -O3` C.
- **Amortized table workloads**: Within ~2x of hand-written C, with contended `@form(hashmap)` writes outperforming hand-rolled C locks by ~1.3–1.4x.
- **Polymorphism across forms**: Because `@form(vec)` of `Player` is specialized to its locus, there is no generic `Vec<T>` type parameter. To write functions that operate over multiple form loci, define a structural `interface`.
