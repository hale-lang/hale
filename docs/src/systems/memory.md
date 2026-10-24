# Memory & Lifetime

Hale manages memory through hierarchical region allocation rather than garbage collection or compile-time borrow checking. Every locus owns an arena allocator: intermediate allocations inside methods use a per-call scratch region that resets on return, while values stored on `self` live in the locus's arena and free in one shot when the locus dissolves.

```hale
type Route { path: String; target: String; }
type Session { id: Int; user: String; }

locus Worker { }

locus Router : projection chunked {
    capacity {
        heap routes   of Route;
        pool sessions of Session;
    }
    accept(w: Worker) { }
}

fn main() {
    Router { };
}
```

## Region Hierarchy & Locus Arenas

Every locus instance owns an arena—a contiguous bump-allocated memory region. Strings, records, and collection buffers created and persisted by the locus allocate from this arena. Individual objects are not freed one by one; when the locus dissolves, its entire arena is reclaimed at once.

Arena regions nest alongside the locus tree. A child locus allocates from a sub-region of its parent:

```text
root
└── App arena
    └── Server arena
        ├── Conn A arena
        └── Conn B arena
```

When a parent locus dissolves, its entire subtree of child regions frees wholesale in depth-first order (see [Lifecycle & Shutdown](../services/lifecycle.md)).

### Flow-Child Reclamation (`release`)

When a long-running parent accepts child loci via `accept(c: Conn)`, child lifetime depends on whether any locus in the program declares `release(c: Conn)`:

- **Resident children (default)**: Without `release`, an accepted child's arena lives until the parent locus dissolves.
- **Flow children (`release(c: Conn)`)**: Declaring `release` marks `Conn` as a per-request or per-connection flow. As soon as the child's `run()` returns (or it executes `terminate;`), the runtime dissolves the child and reclaims its arena immediately (see [Parents & Children](../services/parents-children.md)).

An empty or omitted flow `run()` also completes and reclaims the child.
This includes children handed across a pool boundary to their accepting
owner: birth and the completion bookends execute on the owner's thread.
Only that accepting owner's `release` body runs, when it declares one.
The child leaves its owner's tracker and its struct becomes reusable.

A generic release uses its concrete type argument: specializing an owner
whose hook is `release(c: T)` with `T = Conn` makes `Conn` a flow.
Declaring the generic template alone names no concrete child type.

The compiler records the effective storage policy once in its allocation
summary, including recognition mode/cap and accept-hook presence. The
model and lowering view read that row, and generic instances inherit the
template's literal policy. This agreement supplies configuration facts for
resource analysis; it does not establish a retained-child or byte bound.

### Recognition reservations

`projection recognition(cap=N, fixed_cell)` and `shared_slab` reserve at most
`N` child arenas concurrently per owner, shared by its accepted child types.
An exhausted reservation fails with `RecognitionPoolUnavailable` before the
owner exposes the new child through its header, registration, `accept` and
`birth`. This is a hard runtime failure; the `accept` callback does not reject
children. Cross-domain create requests may have already evaluated and copied
params before this owner-side check.

A child's reclaim returns a reservation. Fixed cells become reusable; a shared
slab keeps its bytes and bump position until the owner dissolves. Reusing a
slab reservation does not replenish its storage budget. These caps alone do
not establish a total byte ceiling or account for pending create requests,
restart overlap, or temporary work.

## Structural Memory Safety

Garbage collectors and borrow checkers exist to determine when memory can be safely freed without creating dangling pointers. Hale enforces two structural invariants that make wholesale arena reclamation sound without runtime tracing or lifetime annotations:

1. **Vertical-only pointer flow**: References follow parent-to-child hierarchy (`H9`). A value inside one locus's arena is never referenced directly by a sibling locus, so freeing a locus region never leaves dangling pointers in peers.
2. **Copy-on-delivery messaging**: Bus payloads crossing a locus boundary are copied into a delivery-scoped buffer that is freed as soon as the subscriber handler returns. If a handler persists a field (`self.last = msg.body`), the runtime deep-copies that value into the receiver's own arena. Sender and receiver lifetimes remain completely independent.

## Per-Method Scratch Regions & Field Retirement

Not every allocation inside a locus belongs to its long-lived arena:

| Allocation Scope | Where It Allocates | When It Frees |
|---|---|---|
| Method / handler transient values | Per-activation scratch sub-region | Immediately when the method or handler returns |
| Scratch-local free `fn` | Function-local scratch region | Immediately when the free function returns |
| Non-scratch free `fn` | Caller's active region | When the caller's activation or scope ends |
| Persisted field (`self.field = val`) | Locus lifetime arena | When the locus dissolves (or on field replacement retirement) |
| Stack-local scalar array (`[T; N]`) | CPU stack frame (up to 8 KiB) | When the function frame returns |

### Whole-Field Replacement Retirement

When a method or bus handler overwrites a field on `self` (`self.latest = Thing { ... }` or `self.state = State::Ready(name)`), the struct bytes overwrite in place and any heap blocks owned by the displaced value—such as `String`s, `Bytes`, or nested records, tuples, and arrays—are retired at the activation boundary and recycled on subsequent stores into `self`.

For `String` and `Bytes` fields whose lengths vary across writes, retired blocks are retained at their size-class capacity and reused by future stores. As a result, repeated whole-field replacement inside handlers or methods keeps the locus arena flat at the high-water mark of the largest stored value.

> [!TIP]
> In-place scalar mutation (`self.latest.count = v`, `self.arr[i] = v`) avoids the clone-and-retire cycle altogether and is required inside tight loops in `run()` that do not cross a method or handler activation boundary.

## Capacity Slots (`heap` and `pool`)

While the default locus arena handles general locus-lifetime state, loci that manage structured collections declare explicit `capacity` slots:

```hale
type Route { path: String; target: String; }
type Session { id: Int; user: String; }

locus Router {
    capacity {
        heap routes   of Route;    // growable buffer, cells freed individually
        pool sessions of Session;  // bounded population of fixed-size recyclable cells
    }
}
```

| Slot Kind | Storage Discipline | Typical Use |
|---|---|---|
| `heap <name> of <T>` | Growable contiguous or node storage; elements can be resized and reclaimed during the locus's lifetime, and the entire slot frees at dissolve. | Backing slot for `@form(vec)` and `@form(heap)`. |
| `pool <name> of <T>` | Fixed-shape recyclable cell pool supporting fast acquire, release, and indexed lookup. | Backing slot for `@form(hashmap)`, `@form(set)`, `@form(ring_buffer)`, and `@form(lru_cache)`. |

Collection annotations such as `@form(vec)` and `@form(hashmap)` synthesize their typed method surfaces directly over these capacity slots (see [Forms Under the Hood](./forms.md)). For fixed-capacity lists embedded directly inside a value `type` rather than a locus, use `bounded[T; N]` (see [Collections](../everyday/collections.md)).

> [!IMPORTANT]
> Capacity slots hold value types (`type` records and primitives), never locus instances. Dynamic child loci are managed through `accept` and `release`, not storage slots.

## Projection Classes

When a parent locus accepts child loci, its **projection class** declares the observation resolution and population scale for those children, allowing the compiler to select an appropriate child-arena allocator:

| Projection Class | Population Scale | Observation Model | Allocator Strategy |
|---|---|---|---|
| `rich` | ~4–10 children | Individual named children, fully inspected | Dedicated per-child arenas, low churn |
| `chunked` *(default)* | ~10–30 children | Range-based observation and moderate churn | Per-child sub-regions with free-list reuse |
| `recognition(cap = N, fixed_cell)` | ~100–500+ children | Aggregate observation (counts, histograms) | Pre-allocated fixed-cell pool with hard runtime capacity `N` |

```hale
locus Worker { }

locus Fleet : projection recognition(cap = 200, fixed_cell) {
    accept(w: Worker) { }
    release(w: Worker) { }
}
```

Changing a locus's projection class alters the underlying region layout without changing parent or child method code.

## Sizing Hints vs. Lifetime Rules

Declared arena sizes are allocation hints: if a locus arena exceeds its initial chunk budget, the runtime attaches an additional chunk rather than panicking. The sole hard capacity limit is the `cap = N` parameter on `projection recognition`, which enforces a strict cell ceiling at runtime.

One allocation shape bypasses arenas entirely: a fixed-size scalar array literal bound to a local variable (`let mut counts: [Int; 16] = [0; 16];`) and accessed solely by index (`counts[i]`) is allocated directly on the CPU stack frame (up to 8 KiB per function). See [Performance](./performance.md) for details on keeping hot-path memory flat.
