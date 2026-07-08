# Perspectives as live redeploy + topology-aware placement

**Status: ✅ SHIPPED (2026-07-06).** Designed 2026-07-04; the core of both
coupled features landed over a 10-PR run. Hale is now a single-address-space
distributed system: *describe the machine, place components onto it,
live-redeploy them at pointer-flip cost.* This note is retained as the design
record; the two boxes below map each slice to its PR and flag the deferred tail.

**What shipped**

| Slice | What | PR |
| --- | --- | --- |
| Topology 1a | `pinned(cores = A..B / {…})` cpuset affinity | #168 |
| Topology 1b | `topology { }` + `pinned(node = N)` / `pinned(l3 = name)` | #173 |
| Topology (arena) | arena-on-node NUMA memory co-location (`mbind`) | #174 |
| Topology 1c | `replicas = K` single-threaded fan-out | #175 |
| Perspective 2a | contract + `serves` + global slot + sync dispatch | #172 |
| Perspective 2b | `reperspective` — the live redeploy (slot flip) | #176 |
| Perspective 2c-contract | bus surface in the contract + conformance | #177 |
| Perspective 3 | state-preserving swap (layout-identity zero-migration) | #179 |
| Perspective 2c-runtime | bus subscriptions swap on `reperspective` | #180 |

**Deferred (the note's aspirational tail — not blocking the core):**
- **Transport-driven redeploy from the wire** — a new impl *version* arriving as
  bytes over the bus → decode → `stable_when` gate → drain-in-flight → swap. The
  in-process swap ships; ingesting a redeploy *from the wire* does not.
- **Footprint-*changing* `migrate(old) -> Self`** — the state-preserving swap
  requires all impls of a perspective to share one footprint; a footprint change
  is a compile error pointing here. The `migrate` transform (in-scope native or
  versioned-wire) is future work.
- **Re-placing on swap** (Part 3's "redeploy onto a different NUMA node") and
  **cross-thread/pinned bus-perspective swap** — the swap is cooperative +
  same-placement today.

The tell the design held up: **nothing shipped was net-new machinery** — the
slot reused interface vtables, the bus swap reused `quarantine_self` +
`emit_bus_register`, the state-preserving swap fell out of the arena/vtable split
for free (it deleted more teardown code than it added).

---

## Thesis

- A **perspective** is a first-class, live-rebindable handle to a *contract*
  (an ABI/lens). You never hold a concrete implementation — you hold a
  perspective, call it through the contract, and the world can update what's
  behind it without your code changing.
- **Placement** is the *where* to the perspective's *what*: which NUMA node /
  cache domain / cores a component runs on, with its memory co-located.
- Composed: you `topology { }`-describe the host, `placement { }`-map
  perspectives onto it, and `reperspective`-live-redeploy them (optionally
  re-placing). Kubernetes-shaped, but in-process at nanosecond cost.

---

## Part 1 — perspective = the hot-swap unit

> **✅ Shipped (#172, #176, #177, #179, #180).** The contract + `serves` + global
> slot + sync dispatch (2a), the `reperspective` live redeploy (2b), the bus
> surface in the contract (2c-contract), the state-preserving swap (3), and the
> bus-subscription re-point on swap (2c-runtime) all landed. Shipped-syntax deltas
> from the throwaway sketches below: the bus surface is a `bus { subscribe …; }`
> block inside the contract (not bare `subscribe` lines); the swap is
> **state-preserving by default** (keep the slot's `data`, flip only the vtable) —
> there is no `drain V1` because the old code was never the state; a footprint
> change is rejected pending `migrate` (deferred). See `spec/semantics.md`
> §"Perspectives" and `docs/src/services/perspectives.md`.

The current `perspective` feature is inert (a flat type over a topic + helper
methods). Repurpose it: a perspective and a swappable-ABI slot are the same
shape — *a holder programs against a stable contract, reaches the real thing
only through an indirection, and that indirection can be re-pointed underneath
them.* They differ on one axis, **where the state lives**:

- **View perspective** — state lives *in front of* the indirection (a shared
  collection). Swapping the view is **free** (re-project the same data). This is
  exactly today's projection classes `rich/chunked/recognition` — subsumed as
  the stateless-swap end of the general construct.
- **Impl perspective** — state lives *behind* the indirection (the impl owns
  it). Swapping needs **state migration**.

The compiler already knows which regime applies (does the thing behind the
perspective declare its own `params`, or is it a view over someone else's
collection?), so it knows whether a rebind is free or needs a `migrate`.

### Surface (sketch — semantics matter, syntax throwaway)

```hale
perspective Router {                 // the stable contract = the ABI boundary
    fn route(r: Request) -> Response;
}

locus RouterV2 : serves Router {     // a swappable executable of that perspective
    params { table: RouteTable; cache: LruCache; }
    migrate(old: RouterV1) -> Self { ... }   // required iff footprint changed
    fn route(r: Request) -> Response { ... }
}

locus Gateway {
    params { router: perspective(Router); }   // holds the slot, not an impl
    fn on_req(r: Request) { self.router.route(r); }   // call through the indirection
}

// "get a new perspective on Router" — the live redeploy:
reperspective self.router as RouterV2;   // load V2, migrate (or free), flip slot, drain V1
```

### Dispatch — the slot (1-1, never 1-N)

A perspective is **1-1** (one impl behind the handle) — distinct from the bus's
1-N pub/sub. So it needs *one indirection everyone funnels through*, not a
registry:
- **sync** → the slot is a function pointer; the indirect call resolves to the
  current impl (a load + a predicted indirect branch — near-direct cost).
- **async** → the slot is the target *mailbox*; senders enqueue to "the current
  impl's mailbox."

Because the interops are closed-world and 1-1, the compiler knows **every** call
site and there is exactly one target, so **one atomic store redirects the whole
program** — soundness *and* O(1) swap. Three tiers: baked/frozen (inlined,
fastest, un-swappable) · single swappable slot (1-1, this) · dynamic registry
(1-N / open-world).

### The contract includes the bus surface, not just the sync ABI

The dispatch split above already makes async a first-class perspective mode — the
slot *is* a mailbox. That forces a completeness point on the **contract**: if a
perspective can be reached over the bus, its bus edges are part of the ABI a swap
is checked against, exactly like its `fn` signatures. The sync half checks
`RouterV2 : serves Router` implements `fn route`; symmetrically, if the
perspective subscribes/publishes, the new impl must present the **same** edges —
same subscribed topics (so publishers still reach it), same published topics (so
subscribers still hear it) — or the rebind silently drops wiring the "one atomic
store redirects the whole program" guarantee is supposed to cover. So the
contract grows an optional bus surface alongside its methods:

```hale
perspective OrderRouter {
    fn route(r: Request) -> Response;   // sync ABI  → function-pointer slot
    subscribe "orders" of Order;        // inbound edge → mailbox slot
    publish   "fills"  of Fill;         // outbound edge (publisher identity)
}

locus RouterV2 : serves OrderRouter { ... }   // must satisfy BOTH sets
reperspective self.router as RouterV2;         // re-points fn-ptr slot AND the
                                                // "orders" mailbox slot, atomically
```

Load-bearing subtleties:
- **Optional — keep the common case flat.** A pure sync `Router` you just call
  declares zero bus edges; the slot stays a function pointer. Bus-in-contract is
  an *impl-perspective* concern (state behind the slot), moot for *view*
  perspectives (the `rich`/`chunked`/`recognition` read-projection end).
- **1-1 is preserved.** A contract `subscribe "orders"` is a claim about the
  *handle→impl* subscription identity ("the impl behind this handle is the thing
  subscribed to orders"), not about the topic's global fanout. If other loci also
  subscribe `"orders"`, the topic stays genuinely 1-N; the perspective owns one
  edge into it, and the swap moves *that one edge*.
- **Not net-new machinery.** Re-pointing a mailbox slot is the same `m28b`
  cross-thread `lotus_mailbox_post` primitive already used for pinned/cross-pool
  delivery — and it composes with Part 3: a `reperspective` that also *re-places*
  the impl onto a different NUMA node just re-points the mailbox to a thread on
  that node.
- **It's a contract change, so it ripples — correctly.** Adding a subscribed
  topic recompiles holders (per below); impls still swap freely. Bus edges sit on
  the stable/versioned side of the wire boundary, exactly where the contract
  belongs.

### State migration

> **✅ Shipped: the identical-footprint zero-migration case (#179).** The swap
> keeps the slot's `data` and re-points only the vtable — code follows data on the
> *existing* arena, no data moves. The compiler enforces layout-identity
> structurally (all impls of a perspective must share params by name + type). The
> **changed-footprint + `migrate`** case is deferred (compile error today); the
> bytes/wire path over the versioned wire format is the aspirational tail.

Model it as *deploying an app over a running DB*: `params` (+ capacity/@form
slots — the full storage footprint) is the schema.
- **Identical footprint → zero migration.** State and code are already separate
  (arena vs methods); repoint the code at the *existing* arena. No data moves.
  Compiler proves layout-identity structurally.
- **Changed footprint + `migrate` provided → run it** (alloc new arena,
  `migrate(old)->new`, flip slot, drain old, dissolve). In-scope native
  migration, or bytes/wire migration over the versioned wire format when the old
  types are gone (a component redeploy) — literally a DB migration script over
  serialized rows.
- **Changed footprint + no `migrate` → compile error.** You cannot deploy a
  schema change without a migration — same gate-or-provide discipline as wasm
  rejection and the macOS `async_io` gate.

Caveats: layout-identity is *layout*-safe, not *semantics*-safe (unchanged
schema ≠ unchanged meaning — units-change needs an opt-in migrate). And it's the
full footprint, not just `params`.

### Cost — the pitch

- **Steady state:** one indirect call per call into a swappable perspective.
  The GC baseline to be "no worse than" is *zero* — Hale doesn't collect.
- **Swap event:** a pointer flip + at most one linear pass over *the single
  component you're replacing* (O(component state), or zero if footprint matches)
  + drain in-flight + wholesale-free the old arena.

So "no worse than a GC cycle" is *conservative*. The three things that make GC
hurt are all absent: it's **local** (one component; the other 63 cores never
pause), **voluntary + predictable** (fires when you deploy, not at allocation
pressure), and **O(one component), not O(heap)**. With double-buffering (new
version takes new traffic while old drains) there's *no global pause at all*.
Soft-unbounded piece: the **drain** waits for the component to quiesce (bound it
— cap the queue, swap in a quiet window). Cross-thread swap adds the
signal-and-join rendezvous. The `migrate` transform is user code (framework
guarantees the O(state) single pass).

### Wires onto existing machinery

slot = the perspective indirection · contract = the stable wire ABI · migration
= the state handoff · **ownership/supervision tree = rebind authority** (a
supervisor holds perspectives on its components; "deploy" = handing a component a
new perspective; holders only *call*) · bus = the async interop that follows the
same flip. A **contract** change ripples (recompile holders); *impls* swap
freely — the stable/mutable boundary the wire format already draws.

---

## Part 2 — placement DSL for full host topology

> **✅ Shipped (#168, #173, #174, #175).** `pinned(cores = A..B / {…})` cpuset
> affinity, `topology { }` with `pinned(node = N)` / `pinned(l3 = name)`,
> node-local arena allocation (`mbind`), and `replicas = K` single-threaded
> fan-out all landed. Affinity/NUMA are Linux-only and degrade gracefully
> elsewhere (advisory), as designed. Topology is declare-only (the discover mode
> below is not implemented). Note `bulk` is a reserved word, so an L3 domain can't
> be named `bulk` (the sketch uses `heavy`). See `spec/runtime.md` §"Schedule
> classes" / "Placement".

Today: `pinned` / `pinned(core=N)` (one thread, one core), `cooperative(pool=X)`
(share a pool's single thread), `where async_io`. Thread accounting: **1 OS
thread per pinned locus, 1 per distinct cooperative pool, +1 main.** Gaps for a
64-core box: no NUMA/cache awareness, no core *ranges/sets*, no way to co-locate
a locus's *memory* with its thread, no way to describe the machine.

### The hierarchy to model

`socket → NUMA node → CCD/CCX (shared L3) → core → SMT thread`. The payoff isn't
just thread affinity — it's **thread + memory co-location**: a NUMA-pinned locus
must allocate its *arena* from that node (cross-node memory access is what kills
big-box perf). Cache-domain co-location: cooperating loci on the same L3 domain
keep cross-locus bus traffic in L3.

### `topology { }` — describe / partition the machine

```hale
topology {
    reserve cores 0..3;                       // hands-off for OS / main
    node 0 {
        l3 hot  { cores 4..11; }              // one CCD, shared L3
        l3 warm { cores 12..19; }
    }
    node 1 {
        l3 heavy { cores 20..35; }   // (not `bulk` — a reserved word)
    }
}
```

Two modes: **declare** (reproducible deploy — bind logical domains to physical
at startup, fail if the machine doesn't match), or **discover** (query
hwloc/sysfs and fill in). Likely: declare the *logical* partition (which
subsystems get which domains), discover the *physical* mapping, bind at startup.

### Topology-targeted placement

```hale
placement {
    region_us: pinned(node = 0);                          // thread + arena on node 0
    matcher:   pinned(l3 = hot);                          // a core in the `hot` L3 domain; node-local arena
    ticker:    pinned(core = 5);                          // a specific core
    workers:   pinned(cores = 4..11, replicas = 8);       // 8 single-threaded loci, one per core
    io:        cooperative(pool = io, l3 = warm) where async_io;
    heavy:     cooperative(pool = h, cores = 20..35, replicas = 16);   // 16 workers on node 1
}
```

- `node = N` / `l3 = <name>` / `core = N` / `cores = A..B` — target any level;
  arena is **node-local** to wherever the thread lands (inferred from the core's
  node when only cores are given).
- **`replicas = K`** — the parallelism sugar. Rather than a multi-worker pool
  (which would break the single-consumer invariant the lock-free rings + bus
  devirt + single-threaded-method guarantee all rest on), fan one locus type
  into **K single-threaded instances**, one per core in the range. Parallelism =
  more units, each still single-threaded — every invariant survives.
- SMT: `core` = whole physical core; add `thread = N` (or `smt`) when you need a
  specific hardware thread vs the whole core.

### Portability

CPU affinity + NUMA binding are **Linux-only** (`pthread_setaffinity_np` /
`mbind` / libnuma; macOS was gated in the 2026-07 port). So topology placement is
a Linux *optimization*: on macOS/other it **degrades gracefully** — the OS
schedules freely, arenas allocate normally — exactly like `core = N` already
no-ops there. `topology { }` is advisory where unsupported.

---

## Part 3 — composition

> **◐ Partially shipped.** Both halves exist independently — you can `topology`/
> `placement`-map loci onto the machine, and you can `reperspective` a component
> live. The composition tail — a `reperspective` that instantiates the new impl at
> a *different* placement (live-rebalance across nodes) with `migrate` moving its
> arena — is deferred: today's swap keeps the same placement (and, being
> state-preserving, the same arena). The pieces are in place; wiring re-placement
> into the swap is future work.

A perspective's impl is a locus, so it *has* a placement — you place perspectives
onto the topology. `reperspective` instantiates the new impl at a placement,
which may **differ** from the old's: live-rebalance a component across nodes/core
ranges, with `migrate` moving its arena to the new node's memory. So the deploy
story is declarative and live:

- **`topology { }`** — the machine.
- **`placement { }`** — components (perspectives) mapped onto it, NUMA/cache-aware.
- **`reperspective`** — live redeploy, optionally re-placing, at pointer-flip +
  O(component) cost, local and pause-free for the rest of the box.

The tell that this is right: nothing here is net-new machinery invented for it.
The slot, the contract/wire ABI, the migration, the supervision tree, the arena
model, and the `@locality`/affinity plumbing are all things that already exist —
"perspective = live-rebindable handle to a contract" + "placement = topology-aware
where" is the naming that makes them one deployment feature.

---

## Open questions / hard edges

1. ✅ **Rebind authority vs call authority** — resolved as designed: `reperspective
   self.<field>` runs on the locus that owns the slot; holders only call. Enforced
   at typecheck (the field must be a `perspective(P)` param of the current locus).
2. ◐ **Cross-thread atomicity** — the swap is a single store visible to all
   holders. Full cross-thread signal-and-join / drain-before-dissolve is moot for
   the shipped state-preserving swap (nothing is dissolved); it returns with the
   deferred footprint-changing `migrate`.
3. ◐ **Contract-change ripple** — holds today (impls swap freely; the contract is
   the stable side). Versioning the *wire* contract belongs with the deferred
   transport-driven redeploy.
4. ✅ **Keep the common case flat** — held: a pure-sync perspective declares no bus
   edges and stays a function-pointer slot; non-perspective code emits zero
   perspective machinery (verified — no slot/vtable in a program without one).
5. ◐ **Topology declare-vs-discover + validation** — declare mode shipped;
   discover (query hwloc/sysfs) is not implemented. Unsupported platforms treat
   topology as advisory (graceful no-op).
6. ⬜ **State-migration mid-conversation** — still open, and now gated behind the
   deferred footprint-changing `migrate`: today a swap preserves state in place
   (no transform, no mid-exchange hazard); the transform semantics land with it.
