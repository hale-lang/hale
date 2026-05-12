# Forms

A **form** is a compiler-recognized annotation on a locus
declaration that picks an efficient lowering for the locus's
storage and synthesizes a standard method set. Forms are the
mechanism Aperio uses in place of parametric collection types
(`Map<K, V>`, `Vec<T>`, etc.). See
[`notes/agent-onboarding/aperio-design-philosophy.md`](../notes/agent-onboarding/aperio-design-philosophy.md)
for the design philosophy and `spec/design-rationale.md` for The
Design's grounding (F.0 form-before-parameter, F.22 capacity).

This document specifies the form annotation system in general
(syntax, contract, verification) and the `@form(vec)` contract
in detail. Subsequent forms (`@form(hashmap)`, `@form(ring_buffer)`)
get their own sections as they're committed.

## Annotation syntax

```
form_annotation = "@form" "(" form_name [ "," form_arg { "," form_arg } ] ")"
form_name       = LOWER_IDENT
form_arg        = IDENT "=" expression
```

A form annotation sits on the line above a `locus` declaration,
like the existing `@projection` annotation:

```aperio
@form(vec)
locus ItemListL<T> {
    capacity { heap items of T; }
}
```

- **`form_name`** — the form identifier. Lowercase, single word.
  The v1 form library is fixed (see "v1 form library" below);
  user-defined forms are deferred to a future release.
- **`form_arg`** — keyword arguments specific to the form. Used
  for tuning knobs that don't change storage discipline (e.g.
  `max = 100` for `@form(lru_cache)`).
- **One form per locus.** Composition (`@form(vec) @form(ordered)`)
  is rejected in v1.

Form-specific configuration that *does* change storage
discipline goes on the capacity slot, not in the annotation
arguments — see "`indexed_by` and slot clauses" below.

## Form contract

Each form specifies three things the compiler verifies and
implements:

1. **Required capacity shape.** What slots the locus must
   declare, of what kinds, holding what cell types. Verified at
   typecheck.
2. **Synthesized method set.** Names, parameter types, return
   types of methods the form provides. Injected at typecheck so
   call sites resolve normally.
3. **Lowering strategy.** What C-runtime substrate the compiler
   emits in place of the literal F.22 pool / heap lowering.

If the locus's shape doesn't match the form's required capacity,
the compiler emits a focused diagnostic and rejects the program.
Example:

```
error[FORM-SHAPE]: @form(vec) requires exactly one `heap` slot;
                   found `pool entries of CmdEntry` instead.
   --> registry.ap:3:1
    |
  3 | @form(vec)
    | ^^^^^^^^^^
  4 | locus RegistryL { capacity { pool entries of CmdEntry; } }
    |                              ----------------------------
    |                              expected `heap items of T`
```

## Synthesized methods

The form *synthesizes* its standard method set. The user does
not declare them; call sites resolve as if they were declared.

```aperio
@form(vec)
locus ItemListL<T> {
    capacity { heap items of T; }
    // push, get, pop, len, is_empty come from @form(vec).
}

fn main() {
    let l = ItemListL_Int { };
    l.push(42);
    let head = l.get(0) or raise;
    println(head);  // 42
}
```

**The user CAN add additional methods** on top of the
synthesized standard set. Naming a user method that collides
with a synthesized method (e.g. user writes their own `push`)
is rejected at v1 — override is deferred to v2.

## `indexed_by` and slot clauses

Form configuration splits between *slot clauses* and
*annotation arguments*. The dividing line:

- **Slot clause** — if the configuration changes how cells are
  laid out or accessed. A storage-discipline concern.
- **Annotation argument** — if the configuration is a policy /
  tuning knob the form's runtime consults; the underlying
  storage shape is the same regardless.

```aperio
// Storage discipline — slot clause.
@form(hashmap)
locus CmdRegistryL {
    capacity { pool entries of CmdEntry indexed_by name; }
    //                                   ^^^^^^^^^^^^^^^
    //                                   slot clause
}

// Policy / tuning — annotation argument.
@form(lru_cache, max = 100, ttl = 60s)
locus SessionCacheL {
    capacity { pool sessions of SessionEntry indexed_by id; }
}
```

`indexed_by` is a slot clause because indexing IS a storage
commitment — it changes the pool's layout and access path.

## Default lowering (no form annotation)

A locus without `@form(...)` gets the **literal F.22 default
lowering**: pool slots become `lotus_pool_t*` chunked free-list;
heap slots become `lotus_heap_t*` doubling buffer. The user's
own methods run as written; no synthesis, no shape verification
beyond the normal capacity-slot machinery.

The form annotation is the user's opt-in to a specific efficient
lowering. Without it, you get the predictable F.22 default.

## Perspectives and forms

> **Perspectives reflect on structure, not on lowering.**

The form annotation changes how the compiler lays out memory
and synthesizes methods. It does not change:

- The locus's name or place in the tower.
- The set of fields declared in `params`.
- The capacity slot declarations.
- The `closure` / `on_failure` / `bus` blocks.

Perspectives that reflect on a form-lowered locus see the
*structural* view: the capacity slots, the params, the method
signatures (synthesized or user-written, treated uniformly).
They do not see the underlying C struct layout.

## Performance commitment

> **A form-lowered locus must run within 10% of a hand-written
> equivalent in idiomatic C.**

The 10% gate is verified before any new form is added to the
library. `@form(vec)` is the first form to ship and is the
canonical benchmark target (see "Bench protocol" under the
`@form(vec)` section below).

If a form fails the gate, the lowering is redesigned before
shipping more forms. The point of the form machinery is not to
be clever — it's to be roughly as fast as the C the user would
have written by hand, with all the locus tower's structural
benefits on top.

---

# `@form(vec)`

A contiguous, growable buffer of `T`. The Aperio analogue of
`Vec<T>` / `std::vector<T>` / Go slices. First form committed
for v1; canonical benchmark target for the 10% perf gate.

## Required capacity shape

The locus MUST declare exactly one `heap` slot. Its cell type
becomes the vec's element type `T`.

```aperio
@form(vec)
locus ItemListL<T> {
    capacity { heap items of T; }
}
```

Rules verified at typecheck:

- Exactly one slot. Zero slots, more than one slot, or any
  `pool` slot is rejected.
- The slot MUST be a `heap` slot. (`pool` is the unordered free-
  list shape; `vec` is the contiguous shape — they're different
  storage disciplines, so a `pool` declaration with `@form(vec)`
  is a contradiction.)
- The slot name is user-chosen and is not part of the contract.
  The compiler finds the form's heap slot by *position*, not by
  name. Idiomatic spellings: `items`, `entries`, `bytes`, `xs`.

The cell type `T` may be:

- A primitive (`Int`, `Float`, `Bool`, `Decimal`, `Time`,
  `Duration`, `String`, `Bytes`).
- A user-defined `type` (struct or enum).
- A generic parameter (`heap items of T` inside a generic locus
  `ItemListL<T>`); monomorphization (m63) produces a concrete
  `@form(vec)` instance per binding.

The cell type MAY NOT be a locus reference — vecs hold values,
not loci. If you want a vec of child loci, use the F.22 `pool`
projection-class machinery instead; that's the structural shape
for parent-owns-children.

## Synthesized methods

```
fn push(x: T) -> ()                          # infallible
fn get(i: Int) -> T fallible(IndexError)
fn pop() -> T fallible(IndexError)
fn len() -> Int                              # infallible
fn is_empty() -> Bool                        # infallible
```

The fallible methods return the locus-defined `IndexError`
payload type:

```aperio
type IndexError {
    kind: String;   # "out_of_bounds" or "empty"
    index: Int;     # the requested index (0 for empty-pop)
    len: Int;       # the vec's len at fail time
}
```

`IndexError` is defined in the synthesized form preamble; the
user does not declare it. The same type is shared across all
`@form(vec)` instantiations (it's a flat record, not parametric
over T).

### `push`

```
fn push(x: T) -> ()
```

Appends `x` after the last element. Amortized O(1). The
synthesized lowering grows the underlying buffer by doubling
when capacity is exhausted; the realloc cost amortizes across
N appends to O(N) total.

`push` is **infallible**. OOM during the doubling realloc is a
substrate-level concern: the C runtime traps malloc failure and
re-raises as a closure violation, not as a `fallible(...)`
return on `push`. From the language surface, `push` never
errors. (See `spec/runtime.md` for the OOM trap convention.)

### `get`

```
fn get(i: Int) -> T fallible(IndexError)
```

Returns the element at index `i` (0-based). If `i < 0` or
`i >= len()`, fails with `IndexError { kind: "out_of_bounds",
index: i, len: self.len() }`.

Idiomatic call sites:

```aperio
let head = vec.get(0) or raise;             # bubble on empty
let first = vec.get(0) or default_value;    # substitute
let nth = vec.get(i) or handle_oob(err);    # custom handler
```

### `pop`

```
fn pop() -> T fallible(IndexError)
```

Removes and returns the last element. If `len() == 0`, fails
with `IndexError { kind: "empty", index: 0, len: 0 }`.

`pop` does not free the underlying buffer — capacity does not
shrink. Buffer release happens at locus dissolution.

### `len` and `is_empty`

```
fn len() -> Int
fn is_empty() -> Bool
```

`len()` returns the number of elements currently in the vec.
`is_empty()` is sugar for `len() == 0`. Both are infallible and
O(1).

## Lowering strategy

`@form(vec)` lowers the heap slot to a three-field C struct:

```c
typedef struct {
    size_t cap;     // allocated capacity (elements)
    size_t len;     // number of valid elements
    T*     buf;     // contiguous element array
} lotus_vec_<T>_t;
```

- Initial capacity: `0` (no allocation at birth). First `push`
  allocates the initial buffer.
- Initial buffer size on first push: `4` elements. Chosen as a
  small constant that avoids the malloc-per-element shape
  without over-allocating for short-lived vecs.
- Growth policy: double `cap` on overflow. New buffer is
  malloc'd; old elements are `memcpy`'d; old buffer is freed.
- Shrink policy: none. Capacity is monotonic in v1. (A
  `shrink_to_fit` method may be added later if a workload
  surfaces the need.)

Element storage is by-value: a `@form(vec)` of `Int` is a
contiguous `int64_t[]`; a `@form(vec)` of `type Pair { x: Int;
y: Int; }` is a contiguous array of `{int64_t, int64_t}`
records. No per-element heap allocation, no per-element header.

For elements of pointer-shaped types (`String`, `Bytes`), the
vec stores the pointer by value; the pointed-to bytes live in
whatever arena they were allocated from. Dissolution of the vec
frees the buffer but does not free the pointed-to bytes — those
follow their owning arena's lifetime per the standard F.22
contract.

## Arena ownership

`@form(vec)` is **not** a separate arena-allocated structure.
The three-field `lotus_vec_*` struct lives inline in the locus's
struct layout, the same way the literal `heap items of T`
declaration would. The growable buffer (the `buf` field) is
malloc'd from the *system allocator*, not from the locus's
arena — this is the existing F.22 heap-slot contract, unchanged.

Dissolution: when the locus arena is destroyed, the vec's
`buf` is freed via the F.22 dissolve cascade (the synthesized
destructor emits `free(self.items.buf)` for each formed heap
slot).

## Interaction with the locus tower

A `@form(vec)` locus is a locus in every other respect. It can:

- Have `params { ... }` with defaults.
- Have `birth`, `run`, `drain`, `dissolve` lifecycle bodies.
- Declare `closure { ... }` invariants.
- Route failures via `on_failure(child, err)`.
- Participate in a `bus`.
- Be projected by `perspective` declarations.

These are orthogonal to the form annotation. The form
*replaces* the literal F.22 heap-slot lowering; it does not
replace any other locus mechanic.

## Bench protocol (FORM-3 gate)

`@form(vec)` is the canonical benchmark target. Before any
additional form ships, `@form(vec)` must demonstrate:

1. **Microbench.** 1M `push` followed by 1M random-index `get`
   on a `@form(vec)` of `Int`, compared against an equivalent
   hand-written C program using `malloc` + doubling realloc and
   raw `int64_t[]` indexing. Wall-clock and peak RSS. The
   `@form(vec)` lowering must come within 10% of the C baseline
   on both metrics.
2. **App bench.** A representative app (ferryman is the
   tentative candidate, given its parsing-heavy workload) is
   rewritten to use `@form(vec)` where it currently does
   explicit F.22 pool walks. Wall-clock and RSS compared
   before / after, with the form-lowered version targeted to be
   no worse than the F.22 baseline.

Both benches live under `bench/forms/vec/` (path to be created
when FORM-3 starts). The microbench harness is a fresh binary
target; the app bench reuses ferryman's existing harness.

If either bench fails the 10% gate, the lowering is redesigned
before further forms are added to the library.

## Anti-patterns

### Hand-rolling the contract on a form-annotated locus

```aperio
// WRONG — @form(vec) synthesizes push; user declaration
// collides with the synthesized name.
@form(vec)
locus ItemListL<T> {
    capacity { heap items of T; }
    fn push(x: T) -> () { /* ... */ }  // rejected
}
```

The compiler rejects this at typecheck with `error[FORM-COLLIDE]:
@form(vec) synthesizes `push`; user declaration shadows the
synthesized method (override is deferred to v2).`

### Ignoring the fallible return

```aperio
// WRONG — `get` returns fallible(IndexError); the bare let
// binding drops the error.
let v = vec.get(i);  // compile error: error not addressed
```

```aperio
// RIGHT — address the error with one of the three motions.
let v = vec.get(i) or raise;
```

### Treating the form annotation as syntactic sugar

```aperio
// WRONG — assumes @form(vec) is "just like" hand-writing the
// methods over a literal F.22 heap. The lowering is different;
// the storage layout is different; the perf characteristics
// are different.
```

The form is a *contract*, not sugar. It commits to a specific
lowering and performance shape. Code that depends on
implementation details of the literal F.22 heap-slot lowering
(e.g. memory addresses of individual cells across pushes) will
not behave the same under `@form(vec)`.

## Open questions deferred to FORM-2 / FORM-3

These are spec-level questions the FORM-2 implementation work
will answer concretely. They don't block FORM-1 because the
contract above is independent of them.

1. **Iteration surface.** A `for x in vec.items { ... }` form
   is natural, but the loop construct's lowering depends on
   what the existing `for` over F.22 heap slots does. Deferred
   until the implementation pass.
2. **Bulk operations.** `extend(other: @form(vec))`, `clear()`,
   `truncate(n: Int)`. Useful but not foundational. Add after
   the core five methods land.
3. **Mutation in place.** `set(i: Int, x: T) -> () fallible(IndexError)`.
   Mirrors `get` for write. Likely added in FORM-2; left out of
   the v1 core only because the bench workloads don't require
   it.

---

# `@form(hashmap)` — pending FORM-4

Spec to be written when FORM-4 starts. Surface preview:

```aperio
@form(hashmap)
locus CmdRegistryL {
    capacity { pool entries of CmdEntry indexed_by name; }
}
```

Synthesized methods:

```
fn get(key: K) -> S fallible(KeyError)
fn set(value: S) -> ()
fn has(key: K) -> Bool
fn remove(key: K) -> () fallible(KeyError)
fn len() -> Int
```

Where `K` is the type of the field named in `indexed_by` and
`S` is the slot's cell type. Lowering: open-addressing
hashtable keyed on the indexed field's value.

# `@form(ring_buffer)` — pending FORM-4

Spec to be written when FORM-4 starts. Surface preview:

```aperio
@form(ring_buffer, cap = 64)
locus RecentCmdsL {
    capacity { pool history of CmdEntry; }
}
```

Synthesized methods:

```
fn push(x: T) -> Bool          # returns false when full
fn pop() -> T fallible(EmptyError)
fn len() -> Int
fn is_full() -> Bool
```

Lowering: fixed-size array of size `cap` + head/tail indices,
no malloc after birth.
