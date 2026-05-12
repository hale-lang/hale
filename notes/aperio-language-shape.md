# Aperio language shape: how params, capacity, projection, form, and schedule compose

The four (really five) commitments a locus declaration carries
are at *different abstraction levels* and *orthogonal axes*.
They look adjacent in the source because they appear close
together inside a `locus { ... }` block, but each answers a
different question. Once you separate the questions, the
combinations make sense.

This note is the orientation for new readers who see something
like

```aperio
@form(vec)
locus FooL : projection chunked, schedule cooperative {
    params { ... }
    capacity { ... }
    ...
}
```

…and ask: which of those does what, and how do they relate?

---

## The five axes, in one sentence each

| Axis | Question it answers | Where it appears |
|---|---|---|
| **`params`** | What named knobs does this locus carry, with defaults? | `params { ... }` block |
| **`capacity`** | What storage substrate does this locus own inside itself? | `capacity { ... }` block (F.22) |
| **`projection`** | How are this locus's *children* placed in memory? | `: projection X` annotation |
| **`schedule`** | How does the runtime schedule this locus's work? | `: schedule X` annotation |
| **`@form(...)`** | Is this locus an application-layer container with a known shape? | `@form(X)` prefix on the locus |

Every locus has params (possibly empty), capacity (at minimum
slot 0 — its own arena), and a default projection / schedule.
`@form(...)` is opt-in and only applies to specific stdlib-
recognized shapes (`@form(vec)` ships in v1).

---

## What each axis actually commits to

### `params { ... }` — the locus's tunable knobs

A flat declaration of named values with types and defaults,
provided per-instance at construction time:

```aperio
locus AggregatorL {
    params {
        B: Int = 100;        // budget
        c: Int = 1;          // attach cost
        sigma: Int = 1;      // summary cost
        phi: Float = 1.0;    // formality
    }
}

fn main() {
    AggregatorL { B: 200, phi: 0.5 };   // override two; sigma + c take defaults
}
```

This is the (B, c, σ, φ) shape from paper 1, generalized: every
locus has parameters that quantify its commitments. Defaults
ship with the locus type; instantiation may override any subset.
Within method bodies, params are read as `self.B`, `self.phi`,
etc.

Params are **values**, not storage. They don't allocate
anything beyond the field in the locus struct. They're the
quantitative dimension of the locus's identity.

### `capacity { ... }` — the locus's storage substrate (F.22)

A flat declaration of *allocator slots* — substrate-honest
commitments about how the locus stores cell data internally:

```aperio
locus DataStoreL {
    capacity {
        pool entries of Record;       // slot 1: pool of Record cells
        heap log of Bytes;            // slot 2: heap of Bytes blobs
    }
}
```

Each slot has:
- A **kind**: `pool` (chunked free-list, fixed-size cells) or
  `heap` (doubling buffer, variable-size cells).
- A **cell type**: `of T` — the type of values stored per cell.
- An implicit **allocator pointer** on the locus struct that
  points at the C-runtime allocator (`lotus_pool_t*` or
  `lotus_heap_t*`).

Slot 0 is implicit: every locus has its own `lotus_arena_t`
arena, used for ad-hoc allocations during its lifetime
(composite literals, bus payloads, etc.).

Cells acquired from a slot live until released back to the
same slot, or until the locus dissolves (freeing the slot's
allocator wholesale). Released cells are recycled.

```aperio
locus DataStoreL {
    capacity {
        pool entries of Record;
    }
    run() {
        let cell = self.entries.acquire();   // allocate a Record cell
        cell.id = 42;
        cell.body = "hello";
        // ... use cell ...
        self.entries.release(cell);           // back to the pool
    }
}
```

Capacity is **storage**, not values. Each slot is an allocator;
cells are the storage units it dispenses. The locus owns the
slot's underlying allocator for its lifetime.

### `: projection X` — how this locus decomposes into children

A *structural commitment* about the 1→N relationship between
this locus and its accepted children. Three classes in v1:

| Class | Children layout | Use when |
|---|---|---|
| `rich` (default) | Each child gets its own fresh arena allocated at accept time. | Heterogeneous children with diverging lifetimes. |
| `chunked` | Parent carves a sub-region of its own arena per child, registered in a parent-side slot table. | Many similar children, lifetimes bounded by parent's. |
| `recognition` | Parent maintains a per-sub-mode pool/slab of fixed-size cells; each child fits in a cell. | Many small recognized children with bounded per-instance state. |

```aperio
locus CoordinatorL : projection chunked {
    accept(c: WorkerL) { ... }     // each WorkerL gets a sub-region of CoordinatorL's arena
    run() {
        WorkerL { };
        WorkerL { };
        WorkerL { };
    }
}
```

Recognition (post-v1.x-3) requires an explicit sub-mode:

```aperio
locus ManyLeavesL : projection recognition(cap=16, fixed_cell(bytes=64)) {
    accept(c: LeafL) { ... }
}
```

Sub-modes (per the v1.x-3 design): `fixed_cell(bytes=N)` —
each child must fit in N bytes; `spillover(bytes=N)` — same
with malloc fallback (v1.x ship); `summary_only` — children
carry no per-instance state (v1.x ship); `shared_slab(bytes=N)`
— one bump arena shared across all children. See
`notes/v1.x-3-handoff.md`.

Projection is about *children*, not the locus itself. A locus
with no children (a leaf) has a projection class declared but
the choice doesn't materially affect anything — the default
`rich` is fine.

### `: schedule X` — runtime scheduling discipline

Names how the runtime schedules this locus's work:

- `schedule cooperative` (default) — runs on the cooperative
  scheduler. Yields at substrate-cell boundaries (handler
  exit, lifecycle transition, bus dispatch).
- `schedule pinned(core=N)` — runs on a dedicated OS thread
  pinned to a specific CPU core. For latency-critical work
  that can't afford the cooperative scheduler's batching.

```aperio
locus FastPathL : schedule pinned(core=2) {
    run() { ... }
}
```

Schedule is orthogonal to projection and capacity — it changes
*when* the locus's methods execute, not what they store or
how their children lay out.

### `@form(...)` — application-layer container lowering

A *lowering directive* that picks a specific efficient layout
for the locus AND synthesizes a standard method set. v1 ships
`@form(vec)`; future v1.x will add `@form(hashmap)`,
`@form(ring_buffer)`, etc.

```aperio
@form(vec)
locus ItemListL {
    capacity {
        heap items of Item;     // required: exactly one heap slot
    }
    // push, get, pop, len, is_empty are synthesized — don't write them.
}

fn main() {
    let l = ItemListL { };
    l.push(Item { id: 1 });
    let head = l.get(0) or raise;
    println(head.id);
}
```

The form annotation:
- Validates the locus's capacity shape against the form's
  contract (e.g., `@form(vec)` requires exactly one `heap`
  slot of any cell type T).
- Lowers the heap slot from a `lotus_heap_t*` allocator to an
  inline `{ cap, len, buf }` struct managed by `lotus_vec_*`
  C-runtime fns.
- Synthesizes a standard method set (`push`, `get`, `pop`,
  `len`, `is_empty`).

`@form(...)` loci are **application-layer storage substrate**
— they realize substrate-honest container shapes that
application code uses to hold data. This is load-bearing for
the two-channel failure rule (see `spec/semantics.md`):
synthesized methods on `@form(...)` containers may be
declared `fallible(E)` (because the container is application-
layer); user-declared methods on user-declared loci may not
(because those are substrate-structural).

---

## How they compose: an example walkthrough

A real-ish program might layer all five:

```aperio
@form(vec)                                          ← LOWERING:
                                                       application-layer
                                                       vec container
locus OrderBookL : projection rich,                 ← CHILDREN: each Trade
                   schedule pinned(core=1) {          gets own arena;
                                                       PINNED to core 1
    params {                                        ← KNOBS:
        max_depth: Int = 1000;
    }
    capacity {                                      ← STORAGE:
        heap orders of Order;                          one heap slot —
                                                       @form(vec) requires
                                                       it.
    }

    accept(t: TradeL) { ... }                       ← ACCEPT a Trade child
    run() { ... }
}
```

What each axis contributes:
- **`@form(vec)`** says "this is an application-layer
  container of Orders" → synthesizes push / get / pop / len /
  is_empty over `orders`.
- **`: projection rich`** says "each TradeL child gets its
  own arena at accept" → at runtime, accepting a TradeL
  allocates a fresh sub-arena.
- **`: schedule pinned(core=1)`** says "run this locus on a
  pinned thread" → the runtime spawns a dedicated thread for
  it.
- **`params`** says "max_depth is a tunable knob with default
  1000" → `OrderBookL { max_depth: 5000 }` overrides it.
- **`capacity`** says "the locus stores orders in a heap-slot
  named `orders` over the Order type" → the storage substrate
  for `push` / `get` / etc.

None of these constrain each other — they're orthogonal
commitments. A locus can have any combination (subject to
form-specific shape requirements like `@form(vec)`'s
"exactly one heap slot").

---

## The mental model that helps

Think of the five axes as orthogonal *dimensions* of the
locus's identity, each answering a different question:

```
            params (knobs)
              ↑
              |
capacity ←────┼────→ projection
(storage)     |     (children)
              ↓
            schedule (runtime)

              @form(...) — the lowering hint
              that re-projects the whole thing
              for a known container shape.
```

When reading a locus declaration:
1. **What does it tune?** → params.
2. **What does it store?** → capacity.
3. **What does it contain?** → projection (for children) +
   accept method.
4. **When does it run?** → schedule + lifecycle methods.
5. **Is it a known container shape?** → form annotation.

Each question is independently answerable. Confusion sets in
when readers conflate "storage" (capacity) with "containment"
(projection) — because both feel like "what's inside the
locus." The distinction is:
- **Capacity = the locus's substrate-honest allocators** for
  cells *it* owns and uses internally.
- **Projection = how the locus places its children** — child
  loci accepted via `accept(...)`.

A cell in a capacity slot is data the locus owns and
manipulates. A child locus is a *separate locus instance* that
runs its own lifecycle inside the parent. They're at different
levels of the recursion.

---

## Quick decision tree for "where does this go?"

Reader's mental shorthand when designing a locus:

> "I need to remember a counter that's bumped on each event."
→ `params` (tunable initial value) OR a regular field on the
  locus struct (mutable state). Not capacity, not projection.

> "I need to store hundreds of small records that the locus
  reads and writes throughout its life."
→ `capacity { pool records of Record; }`. Cells acquired +
  released as needed.

> "I need a contiguous, growable list of items."
→ `@form(vec)` + `capacity { heap items of T; }`.
  Synthesized methods give you push / get / pop / etc.

> "I need to accept many short-lived sub-loci of the same type."
→ `: projection chunked` (carved sub-region per child) or
  `: projection recognition(...)` (pooled small children).
  `accept(c: ChildL) { ... }` is where you wire each one.

> "This locus's work needs predictable latency, not
  cooperative-scheduler batching."
→ `: schedule pinned(core=N)`.

The four-then-five questions, asked in order, take you from
an empty `locus FooL { }` to a complete declaration without
the axes blurring together.

---

## Cross-references

- `spec/forms.md` — full contract for `@form(...)` annotations;
  detailed `@form(vec)` spec.
- `spec/semantics.md` § "Fallible call semantics" — the two-
  channel rule, including why `@form(...)` synth methods can
  be fallible but user-declared locus methods can't.
- `spec/design-rationale.md` § F.22 — capacity-tuple substrate
  design.
- `spec/runtime.md` — scheduler classes; runtime fn ABI.
- `examples/02-parent-child/main.ap` — basic accept + child
  pattern.
- `examples/14-projection-classes/main.ap` — explicit
  projection-class annotations (rich, chunked, recognition).
- `notes/v1.x-3-handoff.md` — recognition class sub-modes
  (forthcoming work).
