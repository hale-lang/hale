# Memory model

## Synopsis

Aperio's memory model is *per-region arenas*. Every locus owns
an arena; allocations made in the locus's body live in that
arena; when the locus dissolves, the arena is freed wholesale.
There is no garbage collector, no borrow checker, and no
runtime escape analysis — the locus boundary is the lifetime
boundary, by construction.

## Arenas

| Arena | Lifetime |
|---|---|
| **Per-locus arena** | Created at locus instantiation; freed at locus dissolve |
| **Free-fn subregion** | Created at fn entry as a child of the caller's arena; freed at fn return |
| **Static region** | Process-lifetime; holds string literals and other compile-time constants |
| **Lazy global payload arena** | Process-lifetime; holds bytes for cross-process bus payloads |

### Per-locus arena

Every locus gets its own arena. The arena is a contiguous
region of memory, sized to fit the locus's parameter struct
plus all allocations made during the locus's lifetime.

When the locus dissolves, the arena is freed in a single
operation. There is no per-allocation free; allocations are
released as a group, at the moment the locus's lifetime ends.

### Parent / child arena nesting

A child locus's arena is a *subregion* of its parent's:

- The child cannot allocate outside its arena.
- The parent cannot reach into the child's arena (except
  through the contract surface — see **F.14**).
- When the child dissolves, its sub-arena is freed; the parent
  retains the rest of its arena.
- When the parent dissolves, its arena (which contains all
  children's sub-arenas) is freed wholesale.

For loci with `: projection chunked`, the parent allocates a
fixed-size sub-region per child with free-list slot reuse on
dissolve. For `: projection rich`, each child gets an
independent arena (`lotus_arena_create`).

### Free-fn subregion

When a free fn (a top-level `fn`, not a locus method) is
called, the runtime allocates a *subregion* of the caller's
arena for the call. Allocations made inside the fn's body
land in the subregion. When the fn returns, the subregion is
freed.

For heap-typed return values (`String`, struct, enum with
payload, array), the runtime *deep-copies* the return value
back into the caller's arena before freeing the fn's
subregion. The caller sees a value that lives in its own
arena.

### Static region

String literals and other compile-time constants live in a
process-lifetime static region. They are not allocated and
never freed. Runtime concatenations and slicings produce
*new* strings in the current arena, not edits to literals.

### Lazy global payload arena

For cross-process bus subscribers, the deserializer needs to
allocate string bytes that survive the reader-thread →
`lotus_bus_local_dispatch` → drain → handler chain. A single
process-lifetime bump allocator
(`lotus_bus_payload_arena_alloc`) is created lazily on first
cross-process payload, used for the life of the process.

Memory grows unbounded for the program's lifetime; for v1
this is acceptable. A future revision may compact or recycle.

## Bus copy semantics

When `<-` runs, the payload is copied from the publisher's
arena into each subscriber's arena. The publisher's locus does
not block; the subscriber owns its copy.

For in-process dispatch, this is a memcpy-shaped operation
between two arenas in the same process.

For cross-process dispatch, the publisher serializes via the
m70 wire format; the subscriber deserializes into its own
arena (with String bytes routed through the lazy global
payload arena).

## Long-lived loci

A locus that subscribes to the bus is *long-lived*: its
lifecycle is not torn down at the construction-statement's
end. It remains alive — receiving bus messages — until the
enclosing scope's drain cascade reaches it.

This means a locus constructed in `main`'s body that subscribes
to a subject lives until `main` returns. Its arena lives as
long as it does.

## What cannot happen

The model rules out, by construction:

- **Allocations outliving their locus.** A child cannot leak a
  reference to its arena upward to a parent that outlives it;
  the *shape of escape* is not expressible.
- **Reading freed memory.** When a locus dissolves, its arena
  is gone; any reference into it is invalid. The substrate
  arranges that no such reference exists at the point of
  dissolve (the locus's own state is gone with the arena;
  subscribers received copies, not pointers).
- **Concurrent modification of a single arena.** Each arena
  belongs to exactly one thread (cooperative loci share the
  scheduler thread; pinned loci own their thread). The bus
  copies between arenas; arenas are never shared.

## See Also

- [Locus declarations](./loci/index.md)
- [Lifecycle methods](./loci/lifecycle.md)
- [Bus dispatch](./bus/index.md)
- [Runtime](./runtime.md)
