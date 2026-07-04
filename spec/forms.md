# Forms

A **form** is a compiler-recognized annotation on a locus
declaration that picks an efficient lowering for the locus's
storage and synthesizes a standard method set. Forms are the
mechanism Hale uses in place of parametric collection types
(`Map<K, V>`, `Vec<T>`, etc.). See
`spec/design-rationale.md` for The Design's grounding (F.0
form-before-parameter, F.22 capacity).

This document specifies the form annotation system in general
(syntax, contract, verification) and the `@form(vec)` contract
in detail. Subsequent forms (`@form(hashmap)`,
`@form(ring_buffer)`, `@form(lru_cache)`) get their own sections
as they're committed.

## Annotation syntax

```
form_annotation = "@form" "(" form_name [ "," form_arg { "," form_arg } ] ")"
form_name       = LOWER_IDENT
form_arg        = IDENT "=" expression
```

A form annotation sits on the line above a `locus` declaration,
like the existing `@projection` annotation:

```hale
@form(vec)
locus ItemList<T> {
    capacity { heap items of T; }
}
```

- **`form_name`** — the form identifier. Lowercase, single word.
  The v1 form library is fixed (see "v1 form library" below);
  user-defined forms are deferred to a future release.
- **`form_arg`** — keyword arguments specific to the form. Used
  for tuning knobs that don't change storage discipline (e.g.
  `cap = 100` for `@form(lru_cache)` / `@form(ring_buffer)`).
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
   --> registry.hl:3:1
    |
  3 | @form(vec)
    | ^^^^^^^^^^
  4 | locus Registry { capacity { pool entries of CmdEntry; } }
    |                              ----------------------------
    |                              expected `heap items of T`
```

## Synthesized methods

The form *synthesizes* its standard method set. The user does
not declare them; call sites resolve as if they were declared.

```hale
@form(vec)
locus ItemList<T> {
    capacity { heap items of T; }
    // push, get, set, pop, len, is_empty come from @form(vec).
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

```hale
// Storage discipline — slot clause.
@form(hashmap)
locus CmdRegistry {
    capacity { pool entries of CmdEntry indexed_by name; }
    //                                   ^^^^^^^^^^^^^^^
    //                                   slot clause
}

// Policy / tuning — annotation argument.
@form(lru_cache, cap = 100)
locus SessionCache {
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

## Form-annotated loci as application-layer storage substrate

A `@form(...)` locus occupies a different position in The
Design's taxonomy than a user-declared locus, and the
distinction is load-bearing for the two-channel failure rule
(`spec/semantics.md` § "Fallible call semantics" § "Where
each channel lives"):

- **User-declared loci** are substrate-facing — they
  participate in the locus tower's lifecycle (bus
  subscriptions, modes, contract reads, lifecycle methods).
  Their methods communicate failure structurally via
  closure assertions + `on_failure` routing.
- **`@form(...)` loci** are application-layer storage
  substrate — they realize a substrate-honest *container*
  shape that application code uses to hold data. Their
  synthesized methods (`@form(vec).get`, `@form(vec).pop`,
  future `@form(...)` accessors) operate per-access and
  may be declared `fallible(E)`, addressing failure at the
  immediate caller's `or` clause.

This is why the synthesized `@form(vec)` `get` / `pop`
methods carry `fallible(IndexError)` while user-declared
locus methods cannot. The `@form(...)` annotation is the
declaration-site marker that "this locus is application-
layer storage substrate, not a substrate-structural
participant." The synthesized method surface gets the
application-layer failure channel; the underlying form-vec
locus still respects every other substrate invariant
(arena ownership, dissolve cascade, capacity slot
discipline).

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

The form machinery commits to three distinct perf shapes,
distinguished because they measure different things and have
different bands:

> **(a) Tight-loop primitive cost.** A form-lowered primitive
> (e.g. `@form(vec).push`) must run within **10% of a
> hand-written equivalent in idiomatic C** on a microbench that
> exercises the primitive in isolation.

> **(b) Amortized workload cost.** A form-lowered workload that
> mixes the form's primitives with real per-call work (the
> shape real apps exhibit) must run within **2× of an equivalent
> idiomatic C program**.

> **(c) Per-op fallible-method cost.** A form-lowered fallible
> primitive (e.g. `@form(vec).get` / `.pop` /
> `@form(hashmap).get` / `.remove`) measured in isolation pays
> the C-function-call boundary to the `lotus_*` primitive plus
> the fallible-ABI plumbing. **No 10% commitment at v1**;
> isolated-microbench numbers may show 10–50× behind C. The
> contract is that fallible primitives are correct, predictable,
> and competitive when amortized (the (b) band).
>
> **Update (2026-06-28): the isolated gap is largely closed.**
> `@form(vec)` `.get` / `.set` / `.pop` / `.push` are now inlined
> directly at codegen — bounds-check + typed GEP + load/store,
> no `lotus_*` C-call boundary. `.get` indexed by a counted-loop
> variable (`for i in 0..v.len()`, the vec unmutated in the body)
> additionally drops the bounds check entirely — it's provably
> in-bounds — so the read vectorizes. The remaining cross-boundary
> calls (arena allocation, etc.) inline under opt-in `LOTUS_LTO=1`
> (see `runtime.md`). The "IR-level inlining or LTO" this band
> once deferred to is shipped.

These bands track the same underlying performance reality at
different observer-perspectives (per The Design's
form/parameter cut): (a) measures primitive layout correctness,
(b) measures whether the substrate amortizes well at scale, (c)
measures the codegen-pattern overhead per primitive call.
`@form(vec)` is the canonical benchmark target (see "Bench
protocol" under the `@form(vec)` section below).

**Current standing (2026-06-28, vs Go at matched iteration counts):**

| Bench | Hale vs Go | Band | Status |
|---|---|---|---|
| `form_vec_push` (500k push) | 4.83× | (a) | beats Go ✓ |
| `vec_amortized` (push + fold, 200k) | 3.75× | (b) | beats Go ✓ |
| `form_vec_get` (200k get) | 2.60× | (c) | beats Go ✓ |
| `fn_scratch_work` (1k calls w/ work) | 7.05× | (b) | beats Go ✓ |

`Hale vs Go` = Go time ÷ Hale time (> 1 → Hale faster). The earlier
(2026-05-13) snapshot put `form_vec_get` at 0.026× and `vec_amortized`
at 0.42×; those were a benchmark **iteration-count mismatch** — the
Hale variants ran 20–25× more work than the `.go`/`.js`/`.py`
siblings (see the bench repo's N-audit) — compounded by
pre-optimization codegen. At matched N, with the `.get`/`.set`/`.pop`/
`.push` inlines + counted-loop bounds-check elimination + native-CPU/O3
defaults, Hale leads Go on all four. The formal within-10%/2×-of-**C**
verification for bands (a)/(b) still awaits the C twins noted in the
bench harness.

If a form fails its applicable band, the lowering is redesigned
before shipping more forms. The point of the form machinery is
not to
be clever — it's to be roughly as fast as the C the user would
have written by hand, with all the locus tower's structural
benefits on top.

---

# `@form(vec)`

A contiguous, growable buffer of `T`. The Hale analogue of
`Vec<T>` / `std::vector<T>` / Go slices. First form committed
for v1; canonical benchmark target for the 10% perf gate.

## Required capacity shape

The locus MUST declare exactly one `heap` slot. Its cell type
becomes the vec's element type `T`.

```hale
@form(vec)
locus ItemList<T> {
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
  `ItemList<T>`); monomorphization (m63) produces a concrete
  `@form(vec)` instance per binding.

The cell type MAY NOT be a locus reference — vecs hold values,
not loci. If you want a vec of child loci, use the F.22 `pool`
projection-class machinery instead; that's the structural shape
for parent-owns-children.

## Synthesized methods

```
fn push(x: T) -> ()                          # infallible
fn get(i: Int) -> T fallible(IndexError)
fn set(i: Int, x: T) -> () fallible(IndexError)
fn pop() -> T fallible(IndexError)
fn len() -> Int                              # infallible
fn is_empty() -> Bool                        # infallible
fn sort() -> ()                              # infallible; T in {Int, Float, String}
fn sort_by(cmp: fn(T, T) -> Bool) -> ()      # infallible
fn sort_desc_by(cmp: fn(T, T) -> Bool) -> () # infallible
```

The fallible methods return the locus-defined `IndexError`
payload type:

```hale
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

```hale
let head = vec.get(0) or raise;             # bubble on empty
let first = vec.get(0) or default_value;    # substitute
let nth = vec.get(i) or handle_oob(err);    # custom handler
```

### `set`

```
fn set(i: Int, x: T) -> () fallible(IndexError)
```

Overwrites the element at index `i` (0-based) with `x`. If
`i < 0` or `i >= len()`, fails with `IndexError { kind:
"out_of_bounds", index: i, len: self.len() }`. `set` does not
extend the vec — index must be inside the current length;
appending new elements uses `push`.

```hale
vec.set(0, new_first) or raise;
vec.set(i, x)         or noop(err);   # swallow OOB
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

### `sort`

```
fn sort() -> ()
```

Sorts the vec in place in ascending order. The cell type T MUST
be one of `Int`, `Float`, or `String`; any other T (struct, enum,
bytes, etc.) is a typecheck error suggesting `sort_by(cmp)`.
String comparison is lexicographic on the underlying byte
sequence (i.e., the C `strcmp` ordering); Float comparison
treats `NaN` as equal-to-anything to keep the ordering total
(no panic on NaN-bearing inputs).

`sort` is infallible. The substrate uses C `qsort` under the
hood — average O(N log N), worst-case O(N²) on pathological
inputs.

### `sort_by` and `sort_desc_by`

```
fn sort_by(cmp: fn(T, T) -> Bool) -> ()
fn sort_desc_by(cmp: fn(T, T) -> Bool) -> ()
```

Sort the vec in place under a user-supplied strict-less-than
comparator. The comparator's semantics: `cmp(a, b) == true`
means "a should come before b in the result." This is the
strict-`<` shape — `cmp(a, a)` SHOULD return `false` for any
`a`; a reflexive `<=` produces an unstable ordering but does
not panic.

`sort_desc_by(cmp)` is equivalent to `sort_by(|a, b| cmp(b, a))`
with the arg order swapped under the hood, so the same user
predicate produces the reverse ordering. Provided as a
convenience for the common "descending under the same key
extractor" pattern.

Both methods are infallible from the language surface. If `cmp`
itself faults (e.g., raises via `or raise` on a fallible call
inside the comparator body), the fault propagates through the
sort and the vec is left in an unspecified but valid state
(every element still present, ordering partially applied).

```hale
fn by_x(a: Point, b: Point) -> Bool { return a.x < b.x; }
points.sort_by(by_x);          # ascending by x
points.sort_desc_by(by_x);     # descending by x — same cmp
```

The cell type T may be any sortable shape, including structs and
enums. The substrate uses `qsort_r` with a per-(cell-type,
direction) trampoline synthesized at codegen time; the cookie
threads the caller's arena pointer through so the comparator's
body can use stdlib calls that allocate.

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

`@form(vec)` is the canonical benchmark target. The three perf
bands above map to three benches that ship under
`micro/` in the sibling `hale-lang/bench` repo:

1. **Tight-loop primitive (band (a), 10% gate).**
   `form_vec_push` — 1M `push` on a `@form(vec)` of `Int`,
   compared against an equivalent hand-written C program using
   `malloc` + doubling realloc and raw `int64_t[]` indexing.
   Wall-clock and peak RSS. **Status 2026-05-13:** 1.00× ratio
   vs Go (effectively at C parity); gate met after the
   `lotus_arena_create_subregion` elision for non-allocating
   fn bodies (`notes/form-perf-checkpoint.md` documents the
   path).
2. **Amortized workload (band (b), 2× gate).**
   `vec_amortized` — 200k push + 200k fold over the result,
   timed in one region. **Status 2026-05-13:** 0.42× ratio vs
   Go (2.4× behind) — outside the 2× band; investigation
   pending.
3. **Per-op fallible (band (c), advisory).** `form_vec_get` —
   200k indexed reads via `vec.get(j) or raise`. **Status
   2026-05-13:** 0.026× ratio vs Go (~38× behind isolated).
   Documented residual; advisory only at v1 — the gap is the
   C-function-call boundary to `lotus_vec_get` plus the
   fallible-ABI plumbing. Closing it requires either inlining
   the primitive's logic in IR at codegen time, or LTO. Both
   deferred until a workload measures the cost.
4. **App bench.** A representative app rewritten to use
   `@form(vec)` where it currently does explicit F.22 pool
   walks. Wall-clock and RSS compared before / after, with the
   form-lowered version targeted to be no worse than the F.22
   baseline. Not yet wired.

The microbench harness lives in the sibling `hale-lang/bench`
repo; its `run.sh` resolves the `hale` binary via
`$HALE_BIN` → `hale` on PATH → `../hale/target/release/hale`.
Bench sources are sibling `.hl` / `.go` / `.js` / `.py` files
under `micro/` and `app/`.

If a bench fails its applicable band, the lowering is
redesigned before further forms are added to the library.
`@form(hashmap)` shipped via v1.x-FORM-4 without a parallel
bench under this protocol (the band (a) win on
`form_vec_push` was held to license the form-machinery
extension); a `micro/form_hashmap_*` family in `hale-lang/bench`
is the natural follow-up if perf becomes load-bearing for
hashmap consumers.

## Anti-patterns

### Hand-rolling the contract on a form-annotated locus

```hale
// WRONG — @form(vec) synthesizes push; user declaration
// collides with the synthesized name.
@form(vec)
locus ItemList<T> {
    capacity { heap items of T; }
    fn push(x: T) -> () { /* ... */ }  // rejected
}
```

The compiler rejects this at typecheck with `error[FORM-COLLIDE]:
@form(vec) synthesizes `push`; user declaration shadows the
synthesized method (override is deferred to v2).`

### Ignoring the fallible return

```hale
// WRONG — `get` returns fallible(IndexError); the bare let
// binding drops the error.
let v = vec.get(i);  // compile error: error not addressed
```

```hale
// RIGHT — address the error with one of the three motions.
let v = vec.get(i) or raise;
```

### Treating the form annotation as syntactic sugar

```hale
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

## Open questions

Spec-level questions not blocking the current `@form(vec)`
contract; will be answered as workloads surface demand.

1. **Iteration surface — SHIPPED 2026-07-02.** `for x in
   v.items { ... }` iterates a `@form(vec)` (fully inline buf
   walk: len + buf loaded once, one GEP + load per element, no
   C calls — vectorizes for scalar cells) and `for e in
   m.entries { ... }` iterates a `@form(hashmap)` (cluster-aware
   slot-cursor walk via `lotus_hashmap_iter_next`: O(cap) for a
   full walk, where the index-based `key_at`/`entry_at` rescan
   from slot 0 per call — O(cap×len)). The loop variable is a
   per-iteration COPY for hashmap entries and a REFERENCE to the
   vec-owned cell for vec struct cells (scalars are copies by
   value). Mutating the form inside the body is unsupported (a
   grow rehashes/reallocs under the cursor). `break`/`continue`
   work. Ring-buffer iteration is still deferred (wrap-aware
   oldest-first walk).
2. **Bulk operations.** `extend(other: @form(vec))`, `clear()`,
   `truncate(n: Int)`. Useful but not foundational. Add after
   the core methods land.

---

# `@form(hashmap)`

A keyed associative store: each entry is a struct value `S` that
carries its own key as one of its fields. The Hale analogue of
`Map<K, V>` / `std::unordered_map` / Go `map[K]V` — but
*intrusive*: the value type S carries the key inside it rather
than the map storing separate (K, V) pairs. Shipped as the second
form in v1, following `@form(vec)`.

## Required capacity shape

The locus MUST declare exactly one `pool` slot, with an
`indexed_by <field>` clause naming a field of the cell type to
serve as the key:

```hale
type CmdEntry {
    name: String;
    handler: Int;
}
@form(hashmap)
locus CmdRegistry {
    capacity { pool entries of CmdEntry indexed_by name; }
}
```

Rules verified at typecheck:

- Exactly one slot. Zero slots, more than one slot, or a `heap`
  slot is rejected.
- The slot MUST be `pool`. (Hashmap recycles entry cells as
  inserts / removes flow — the `pool` discipline. `heap` is the
  growable-contiguous shape covered by `@form(vec)`.)
- The slot MUST declare `indexed_by <field>`. The named field
  must exist on the cell type.
- The cell type MUST be a user-declared `type` struct.
  Primitives, enums, type aliases, and qualified paths are
  rejected — the substrate needs the cell's field layout to
  GEP the key out at insert time, which only resolves cleanly
  for struct cells.
- The cell type MAY NOT be a locus reference. Cells are data;
  loci are managed entities. Storing an entity in a hashmap
  means the synthesized `.get(key)` materializes a stranger
  in the caller's scope — the same antipattern that
  `spec/semantics.md § Locus method dispatch` rejects at the
  user-declared layer. For keyed-children patterns, use the
  canonical alternatives:
  - **Parent-child**: declare `accept(c: ChildL)` on the
    parent locus. If name-based lookup is needed, pair with
    a parallel `@form(hashmap)` of cell type
    `type Index { key: String; child_idx: Int; }`.
  - **Bus topic**: publish commands keyed by name; the parent
    subscribes and dispatches to the right child.
  - **Delegation**: collapse the per-child operation onto the
    parent (`parent.inc_named(name)`).
- The slot name is user-chosen and is not part of the contract.
  The compiler finds the form's pool slot by *position*, not by
  name. Idiomatic spellings: `entries`, `bindings`, `routes`.
- `as_parent_for` on the slot is rejected — `@form(hashmap)`
  owns its slot's allocator, so the borrow mechanic from
  v1.x-4b doesn't compose.
- `@form(hashmap, ...)` accepts one optional kwarg:
  `sync = X` (F.32-1; see "Cross-pool sync disciplines"
  below). All other kwargs are rejected.

## Cross-pool sync disciplines

By default, `@form(hashmap)` is **single-pool only** — the
runtime has no synchronization on the hashmap entry points
(`lotus_hashmap_set` / `_grow` / etc), and cross-pool calls
into a plain `@form(hashmap)` receiver are typecheck-rejected
(F.32-0). The opt-in path is the `sync = ` kwarg:

| Annotation | Discipline | Status |
|---|---|---|
| `@form(hashmap)` | single-pool only | shipped |
| `@form(hashmap, sync = serialized)` | per-map `pthread_mutex_t` (F.32-1α) | shipped |
| `@form(hashmap, sync = striped)` | cell-level CAS + per-map `pthread_rwlock_t` for grow + cache-padded cells (F.32-1β2-v2) | shipped |
| `@form(hashmap, sync = lockfree)` (optional `cap = N` initial-size hint) | cell-level CAS, no rwlock or mutex on the steady-state path (F.32-1γ-v1); + `remove` via tombstones (F.32-1γ-v2 session 1); + lazy grow with brief writer/reader stall during migration (F.32-1γ-v2 session 3) | shipped |

**Discipline picker by workload:**

- **Single-pool only** (no cross-pool calls): plain
  `@form(hashmap)`. Densest layout, zero sync overhead;
  cross-pool calls are typecheck-rejected.
- **Cross-pool, write-heavy, cap unknown / dynamic**:
  `sync = serialized`. Per-map mutex; writers serialize but
  the path is short. Beats striped on 2-core / cheap-payload.
- **Cross-pool, read-heavy or per-op work is expensive**:
  `sync = striped`. Rwlock lets concurrent readers run in
  parallel; cache-padded cells avoid false-sharing between
  reader and writer cells. Slower than serialized on 2-core
  / cheap-payload writes (rwlock overhead > parallelism gain).
- **Cross-pool, write-heavy, cap known approximately**:
  `sync = lockfree` (optional `cap = N`). Pure CAS on the
  steady-state hot path — no kernel-mediated sync. Fastest of
  the four on the `form_hashmap_false_sharing` bench (~1.3×
  faster than α serialized at 2 cores). Trade-off: when the
  load factor exceeds 0.6, a grow event briefly stalls all
  lockfree ops (~ms for typical caps) while the migration
  runs. Steady state outside of grow remains fully lockfree.
  `cap = N` is an optional initial-size hint (omitting it
  starts at `LOTUS_HASHMAP_INITIAL_CAP = 8` and grows on
  demand); supply 2-4× the expected peak so grows are rare.

The `serialized` discipline wraps every public entry point in
a per-map mutex. Throughput is bounded by lock contention
(~5-10 M ops/s on a 4-writer workload); correctness is
trivial. The map's `lotus_hashmap_t` struct grows by 12 bytes
(int sync_mode + pointer mu); the pthread_mutex_t is
heap-allocated at init and destroyed at dissolve.

The `striped` discipline adds cache-line padding to the cell
stride (rounds up to `LOTUS_CACHE_LINE`, 64B default) and uses
a 3-state occupancy machine (EMPTY → CLAIMED → COMMITTED) with
`__atomic_compare_exchange_n` for slot claim. A
`pthread_rwlock_t` guards the grow path (set/get hold rdlock,
grow holds wrlock). On the 2-core / cheap-payload bench
striped measures ~1.87× slower than serialized — the rwlock
overhead per op (~150 ns) exceeds α's mutex+memcpy (~90 ns)
by more than the 2-core parallelism gain compensates.
Striped's win materializes on 4+ cores or with heavier per-op
work where the rwlock overhead amortizes.

The `lockfree` discipline (F.32-1γ) drops the rwlock
entirely. `cap = N` is an optional initial-size hint (was
required pre-γ-v2 session 3 before grow shipped; now grows
transparently when load factor crosses 0.6). Under γ-v1
`remove` was a no-op; under γ-v2 session 1 (2026-05-26)
`remove` is supported via tombstones (4-state cell machine:
EMPTY → CLAIMED → COMMITTED → TOMBSTONE). Pure CAS on the
occupancy byte; no kernel-mediated synchronization on the
hot path. The `form_hashmap_false_sharing` bench measures
lockfree at ~1.30× faster than serialized and ~2.54× faster
than striped on the 2-pool concurrent-write workload. Cap
should be sized to 2-4× the peak expected entry count to keep
linear-probe latency bounded and avoid early grows; the
runtime rounds the user's `cap = N` up to the next power of 2
(needed for the `& mask` probe). Omitting `cap` starts at
`LOTUS_HASHMAP_INITIAL_CAP = 8` and grows on demand.

Tombstones in γ-v2 session 1 are *not* reclaimed in place —
the probe advances past them, but inserts always land in the
next EMPTY slot rather than reusing a TOMBSTONE. Session 3's
grow path is where tombstones get compacted out: when the
load factor (`live + tombstones / cap`) crosses 0.6, the
table is rebuilt at double size and the new table omits
tombstones entirely. Consistency on remove follows the
lockfree model: a reader that observed COMMITTED before a
concurrent CAS to TOMBSTONE returns the (now-stale) value —
"the key was present at the moment we read."

Grow under γ-v2 session 3 uses a simpler design than the full
NBHM cooperative-helper migration: one writer wins a
grow_phase CAS, spin-waits for in-flight ops to drain via a
writers_in_flight counter, then runs the migration single-
threaded (no SENTINEL state, no cooperative helping). All
concurrent set/get/remove ops yield-spin during the migration
window. The hot-path cost in steady state is one atomic
load + branch-not-taken — measurably cheaper than the
NBHM-style 5-state CAS-on-every-probe. The trade-off is
tail latency on the writer that triggers grow (bounded by
the migration's O(cap) walk, ~ms for caps up to ~100k) and
brief stalls on all concurrent ops during that window.

The OLD slots buffer is freed eagerly at the end of the
migration (γ-v2 session 4). The drain-wait already guarantees
no in-flight op holds a stale pointer to OLD when grow
completes, so the use-after-free risk that the handoff doc's
QSBR design was meant to solve doesn't exist in this single-
grower variant — QSBR epoch tracking would be redundant. RSS
post-warmup is bounded by the current table size plus the
brief migration-window peak (OLD + NEW both alive for the
~ms duration of `lf_migrate`).

Cross-pool method calls into a `@form(hashmap, sync = ...)`
receiver are accepted without diagnostic — the chosen
discipline carries the substrate's safety contract. Inside a
single pool, all three sync modes pay only their respective
uncontended-fastpath costs (~30 ns for serialized,
~10 ns for lockfree's CAS).

See `notes/f32-cache-aware-delivery-plan.md` § F.32-1 for the
per-discipline implementation strategy + trade-off analysis,
and `spec/types.md` § "Single-threaded-method invariant
(F.31)" for how cross-pool calls into form-bearing receivers
interact with the placement system.

The key type `K` is derived from the resolved type of the
indexed-by field. At v1, K must be `Int` or `String`. Other
field types parse and synthesize methods but reject at codegen
with a focused diagnostic (the runtime ABI's `key_type_tag`
only enumerates these two).

## Synthesized methods

```
fn get(key: K) -> S fallible(KeyError)
fn set(value: S) -> ()                       # infallible
fn has(key: K) -> Bool                       # infallible
fn remove(key: K) -> () fallible(KeyError)
fn len() -> Int                              # infallible
fn is_empty() -> Bool                        # infallible
fn key_at(i: Int) -> K fallible(IndexError)
fn entry_at(i: Int) -> S fallible(IndexError)
fn bump(key: K) -> ()                        # infallible; S must be {key + Int counter}
```

The fallible methods return the synthesized `KeyError` payload:

```hale
type KeyError {
    kind: String;   # "missing_key" — only kind at v1
}
```

`KeyError` is injected into the bundle scope by the form
machinery alongside `IndexError`. The same type is shared across
all `@form(hashmap)` instantiations.

The key is not carried on the error because K varies per
hashmap. Users who want key context construct it through the
substitute motion (`or fallback(err)`), where `err: KeyError`
is in scope and any of the call's local bindings — including
the key arg — are available.

### `set`

```
fn set(value: S) -> ()
```

Inserts or replaces. `set(v)` GEPs the indexed-by field from
`v` to derive the key, then writes the whole struct at the
hashed slot. If a previous entry shared the key, it is
overwritten (`set` is unconditional — no error on duplicate).

`set` is **infallible**. OOM during the doubling realloc is a
substrate-level concern routed through the closure-violation
channel, not a `fallible(...)` return. Same shape as
`@form(vec)`'s `push`.

### `get`

```
fn get(key: K) -> S fallible(KeyError)
```

Returns the entry whose indexed-by field equals `key`. If no
such entry exists, fails with `KeyError { kind: "missing_key" }`.

```hale
let entry = registry.get(name) or raise;
let entry = registry.get(name) or default;
let entry = registry.get(name) or fallback(err);
```

### `has`

```
fn has(key: K) -> Bool
```

`true` iff an entry with this key is present. Equivalent to
"`get(key)` would succeed" but cheaper — no value copy.

### `remove`

```
fn remove(key: K) -> () fallible(KeyError)
```

Removes the entry whose indexed-by field equals `key`. If no
such entry exists, fails with `KeyError { kind: "missing_key" }`.
Idiomatic call shape:

```hale
registry.remove(name) or raise;        # bubble on missing
registry.remove(name) or ignore(err);  # swallow via Unit-returning handler
```

Hale doesn't surface `()` as a literal expression at v1, so
swallowing the error requires a Unit-returning handler call (or
guarding with `has` first):

```hale
fn ignore(_e: KeyError) { }
// later:
registry.remove(name) or ignore(err);
```

`remove` does not shrink the underlying buffer; capacity does
not decrease. Buffer release happens at locus dissolution.

### `len` and `is_empty`

```
fn len() -> Int
fn is_empty() -> Bool
```

`len()` returns the entry count; `is_empty()` is sugar for
`len() == 0`. Both infallible, O(1).

### `key_at` and `entry_at`

```
fn key_at(i: Int) -> K fallible(IndexError)
fn entry_at(i: Int) -> S fallible(IndexError)
```

Hash-table-order iteration (added 2026-05-16). `key_at(i)`
returns the i-th present key; `entry_at(i)` returns the i-th
present full entry value. Order is hash-table order (deterministic
for a given table state, but insertion-sensitive — adding entries
between iterations may rearrange the sequence as the table
rehashes). For a populate-then-iterate pattern the snapshot
order is reproducible.

`IndexError` payload matches the `@form(vec).get` shape (`kind:
String`, `index: Int`, `len: Int`). `out_of_bounds` is the only
`kind` produced; `index < 0` or `index >= len()` triggers it.

Per-call cost is O(cap) — the substrate walks the slots array
counting occupied entries until reaching the i-th. A full sweep
is O(cap²). Fine at small/medium scale; agents iterating
100k+-entry tables should populate a parallel `@form(vec)`
during inserts instead.

Closes the wordfreq-corpus reinvention pattern where every
program maintained a parallel keys vec to dodge the lack of
iteration.

### `bump`

```
fn bump(key: K) -> ()
```

Increment-or-init the Int counter field of the entry keyed by
`key`. Collapses the canonical 6-line pattern:

```hale
if m.has(k) {
    let prev = m.get(k) or raise;
    m.set(Entry { key: k, count: prev.count + 1 });
} else {
    m.set(Entry { key: k, count: 1 });
}
```

into:

```hale
m.bump(k);
```

**Cell-shape requirement.** The entry type `S` MUST have exactly
two fields: the `indexed_by` key field plus one `Int` field
(the counter). Any other shape (zero Int fields, two Int fields,
or extra non-Int fields) is a codegen error pointing at the
manual pattern. The Int field's name is detected at codegen —
"count", "n", "freq", "hits", etc. all work.

`bump` is infallible. Generalizing to arbitrary update fns
(`update_with(k, init, fn(S) -> S)`) is the natural follow-up
once a use case surfaces.

## Lowering strategy

`@form(hashmap)` lowers the pool slot to an inline six-field C
struct holding open-addressing hashtable state:

```c
typedef struct {
    size_t cap;          // power-of-two slot count
    size_t len;          // live entry count
    size_t key_size;     // sizeof(K), set at init
    size_t value_size;   // sizeof(S), set at init
    int    key_type_tag; // 0 = Int, 1 = String
    char  *slots;        // cap * (1 + key_size + value_size) bytes
} lotus_hashmap_t;
```

Each slot is `1 + key_size + value_size` bytes laid out as
`[occupied: u8][key: K][value: S]`. `occupied = 0` means empty;
the runtime uses **backward-shift deletion** (no tombstones) so
probes terminate as soon as an empty slot is seen.

- **Initial cap:** 8 slots, allocated at locus birth via
  `lotus_hashmap_init`. Power of two so hash → index folds to
  a single `& mask`.
- **Growth policy:** double `cap` when `(len + 1) > 0.7 * cap`.
  Rehash every live entry through the normal `set` path (the
  probe sequence changes with the new mask).
- **Shrink policy:** none. Capacity is monotonic in v1.
- **Hash functions:** 64-bit Knuth multiplicative for Int keys
  (`k * 0x9E3779B97F4A7C15`), FNV-1a over the bytes for String
  keys.
- **Probing:** linear with `& mask`. Backward-shift deletion
  walks the cluster forward, shifting any entry whose natural
  position is "before" the freed slot. Cluster boundary is the
  first empty slot.

### Key extraction at the codegen surface

At each `set(value: S)` call site, codegen GEPs the indexed-by
field offset on the value alloca to produce a pointer to the
key, then passes `(slot_ptr, key_ptr, value_ptr)` to
`lotus_hashmap_set`. The runtime memcpys `key_size` bytes from
`key_ptr` into the slot's key region and `value_size` bytes
from `value_ptr` into the value region.

At `get`, `has`, `remove` sites, codegen lowers the key arg
into an alloca matching `key_size` and passes its address.

## Arena ownership

`@form(hashmap)` is **not** a separate arena-allocated structure.
The `lotus_hashmap_t` struct lives inline in the locus's struct
layout, the same way the literal F.22 pool-slot declaration
would. The `slots` buffer is malloc'd from the *system
allocator*, not from the locus's arena — matching the existing
F.22 slot contract.

Dissolution: when the locus arena is destroyed, the hashmap's
`slots` buffer is freed via `lotus_hashmap_destroy` in the F.22
dissolve cascade.

For elements of pointer-shaped types (`String`, `Bytes`) in the
cell struct, the hashmap stores the pointer by value; the
pointed-to bytes live in whatever arena they were allocated
from. Hashmap dissolution frees the slots buffer but does not
free the pointed-to bytes — those follow their owning arena's
lifetime per the standard F.22 contract.

## Complexity

| Operation | Expected | Worst case |
|---|---|---|
| `set` (no resize) | O(1) | O(N) on probe cluster |
| `set` (with resize) | O(N) amortized over inserts | O(N) per resize |
| `get` / `has` | O(1) expected | O(N) on probe cluster |
| `remove` | O(1) expected | O(N) on shift |
| `len` / `is_empty` | O(1) | O(1) |

Load factor stays ≤ 0.7 by construction. Hash quality for Int
keys (Knuth multiplicative) handles dense sequences such as
consecutive IDs without all colliding on slot 0.

## Interaction with the locus tower

A `@form(hashmap)` locus is a locus in every other respect — it
can have `params`, lifecycle bodies (`birth` / `run` / `drain` /
`dissolve`), `closure` invariants, `on_failure` routing, bus
membership, and projection by `perspective` declarations. The
form annotation *replaces* the literal F.22 pool-slot lowering
and synthesizes the six methods; it does not replace any other
locus mechanic.

## Bench protocol (future FORM-N gate)

`@form(hashmap)` gets a bench family parallel to `@form(vec)`'s
three bands once `hale-lang/bench`'s `micro/form_hashmap_*`
is wired up:

1. **Tight-loop primitive (band (a)).** A `form_hashmap_set`
   microbench — 200k `set` calls on a hashmap of struct cells,
   compared against an equivalent hand-written C program using
   `malloc` + open-addressing tables of the same shape.
   Target: 10% of the C baseline. Expected to track
   `form_vec_push`'s shape closely since the subregion-elision
   work applies equally.
2. **Per-op fallible (band (c)).** A `form_hashmap_get`
   microbench — 200k `get(k) or raise` calls. Advisory; the
   same C-function-call-boundary residual `form_vec_get` shows
   will apply here. Closing it is the same work item.
3. **App bench.** A representative app rewritten to use
   `@form(hashmap)` where it currently does explicit registry
   walks; before / after comparison.

Not gated on FORM-4 shipping (FORM-4 was held to the band (a)
`form_vec_push` win for license); ships as a separate
milestone after a hashmap consumer surfaces concrete demand.

## Anti-patterns

### Treating `set` as keyed insert

```hale
// WRONG — set takes the whole value, not (key, value).
registry.set("foo", entry);   // type error: too many args
```

```hale
// RIGHT — value carries its key as a field; substrate extracts.
registry.set(CmdEntry { name: "foo", handler: 1 });
```

The intrusive shape means the type system catches this for you
(`set` is synthesized with the single-arg signature `set(value:
S) -> ()`), but the conceptual reflex from `HashMap<K, V>`
shaped languages is worth flagging.

### Ignoring the fallible return

```hale
// WRONG — get and remove return fallible(KeyError).
let v = registry.get(name);          // compile error: error not addressed
registry.remove(name);               // compile error: error not addressed
```

```hale
// RIGHT — address the error via one of the three motions.
let v = registry.get(name) or raise;
registry.remove(name) or ();
```

### Mutating the indexed-by field after `set`

The intrusive shape means the key is the field. If user code
keeps a reference to the value and mutates the indexed-by
field, the hashmap's invariant breaks (the cell sits in the
slot keyed by its *old* key, but `get` now looks up by its
*new* key). The v1 surface doesn't expose stored cells by
reference, so this isn't reachable from user code today.
Future iteration APIs that surface entry references will need
to gate against indexed-by-field mutation.

## Open questions deferred to a future milestone

These are spec-level questions that don't block FORM-4 because
the core surface above is independent of them.

1. **Iteration surface.** `for entry in registry { ... }` is
   natural but the loop construct's lowering depends on what
   the existing `for` over capacity slots does — and a hashmap
   iteration that visits each occupied slot once needs cluster-
   aware traversal. Deferred.
2. **Bulk operations.** `clear()`, `extend(other)`,
   `take(key) -> S fallible(KeyError)` (get + remove fused).
   Useful but not foundational. Add after a workload demands.
3. **Additional key types.** `Bytes`, custom structs with a
   hashable derivation, enum tags. Each adds a `key_type_tag`
   to the runtime ABI. Workload-driven.
4. **Capacity hints.** `@form(hashmap, cap = 64)` is rejected
   in v1; no tuning knobs. Add when a workload demonstrates
   the 0 → 8 → 16 → ... grow cascade is costing measurable
   time.
5. **Set type.** A `@form(set)` would be a hashmap-without-
   value variant (the cell IS the key). Not part of FORM-4;
   revisit if a workload needs it.

---

# `@form(ring_buffer)`

A fixed-capacity FIFO with push-back and pop-front semantics.
The Hale analogue of a bounded circular buffer — same shape as
a Go channel of capacity N, or a Java `ArrayBlockingQueue`, but
without the synchronization machinery (the cooperative scheduler
already serializes access). Shipped as the third form in v1
via v1.x-FORM-5.

## Required capacity shape

The locus MUST declare exactly one `pool` slot. The cell type is
the element type `T`; the capacity comes from the annotation arg
`cap = N`.

```hale
@form(ring_buffer, cap = 64)
locus RecentCmds {
    capacity { pool history of CmdEntry; }
}
```

Rules verified at typecheck:

- Exactly one slot. Zero slots, more than one slot, or a `heap`
  slot is rejected. (`heap` is the growable-contiguous shape
  covered by `@form(vec)`; ring buffer recycles fixed-capacity
  cells, which is the `pool` discipline.)
- The slot MUST NOT declare `as_parent_for` or `indexed_by` —
  those clauses belong to other forms.
- `@form(ring_buffer, cap = N)` requires `cap`, must be a
  positive integer literal. v1 doesn't const-evaluate
  expressions for form args.
- The cell type T may be a primitive, a user-defined `type`, or
  a generic parameter. It MAY NOT be a locus reference — same
  restriction as the other forms.

## Synthesized methods

```
fn push(x: T) -> Bool                        # false when full
fn pop() -> T fallible(EmptyError)
fn len() -> Int                              # infallible
fn is_full() -> Bool                         # infallible
```

The fallible `pop` returns the synthesized `EmptyError` payload:

```hale
type EmptyError {
    kind: String;   # "empty" — only kind at v1
}
```

`EmptyError` is injected alongside `IndexError` and `KeyError` by
the form machinery; user-declared `EmptyError` wins per the
existing idempotent-injection contract.

### `push`

```
fn push(x: T) -> Bool
```

Appends `x` after the last element. Returns `true` on success,
`false` when the buffer is at capacity. Callers decide
drop-vs-backpressure semantics by inspecting the result:

```hale
let accepted = recent.push(entry);
if !accepted {
    // backpressure: surface to caller, or drop, or evict-oldest
    // via pop()+push() in a separate path.
}
```

`push` is intentionally Bool-returning rather than
`fallible(FullError)`. The full-buffer state is a normal
operational condition (the caller chose a bounded capacity), not
a substrate failure — surfacing it as a Bool keeps the call
shape ergonomic and avoids forcing every caller through `or`.
This is the one place in the form library where infallible-but-
returning-a-status is the right idiom; vec's `push` is truly
infallible (OOM routes through the closure-violation channel),
and hashmap's `set` is unconditional (replace-on-collision). The
ring buffer's fixed cap makes "refused" a user-observable state.

### `pop`

```
fn pop() -> T fallible(EmptyError)
```

Removes and returns the oldest element (FIFO — the one inserted
earliest among those still present). Fails with
`EmptyError { kind: "empty" }` when the buffer is empty.

```hale
let cmd = recent.pop() or raise;       # bubble on empty
let cmd = recent.pop() or default_cmd; # substitute
let cmd = recent.pop() or fallback(err);
```

### `len` and `is_full`

```
fn len() -> Int
fn is_full() -> Bool
```

`len()` is the current element count, in `0..=cap`. `is_full()`
is sugar for `len() == cap`. Both infallible, O(1).

There is intentionally no `is_empty()` synthesized method on
`@form(ring_buffer)` — `pop` is the natural empty-detection
surface (the fallible return signals empty directly to the
caller addressing the error). Adding `is_empty()` would create
two redundant ways to ask the same question; defer until a
workload demonstrates a real need.

## Lowering strategy

`@form(ring_buffer)` lowers the pool slot to an inline five-field
C struct holding head/tail and a pre-allocated backing buffer:

```c
typedef struct {
    size_t cap;        // fixed at init; never changes
    size_t head;       // index of oldest element (next pop)
    size_t len;        // current count, 0..=cap
    size_t elem_size;  // bytes per element
    char  *buf;        // cap * elem_size bytes
} lotus_ring_buffer_t;
```

- **Birth.** `lotus_ring_buffer_init` mallocs `cap * elem_size`
  bytes and pins them for the locus's lifetime. The init takes
  `cap` and `elem_size` as args; `cap` flows in from the form
  annotation, `elem_size` from the cell type's LLVM `size_of`.
- **Push.** `lotus_ring_buffer_push` checks `len == cap`; if so
  returns 0. Otherwise computes the wrap index as
  `(head + len) % cap`, memcpys `elem_size` bytes from the
  caller-provided source into the slot, increments `len`,
  returns 1.
- **Pop.** `lotus_ring_buffer_pop` checks `len == 0`; if so
  returns 0. Otherwise memcpys from `buf + head * elem_size`
  into the out-pointer, advances `head` modulo cap, decrements
  `len`, returns 1.
- **No growth.** Once init runs, the backing buffer is fixed
  size. Push at capacity refuses; the spec contract is "fixed
  capacity" not "grows on demand."
- **Dissolution.** `lotus_ring_buffer_destroy` `free`s the
  backing buffer at locus arena destroy.

## Arena ownership

Same as `@form(vec)` and `@form(hashmap)`: the
`lotus_ring_buffer_t` struct lives inline in the locus struct
layout; the backing `buf` is malloc'd from the *system
allocator*, not from the locus's arena. Dissolution frees
`buf` via the F.22 dissolve cascade.

For pointer-shaped element types (`String`, `Bytes`), the ring
buffer stores the pointer by value; the pointed-to bytes live
in their owning arena per the standard F.22 contract.

## Complexity

| Operation | Cost |
|---|---|
| `push` (not full) | O(1) |
| `push` (full → refuse) | O(1) |
| `pop` (not empty) | O(1) |
| `pop` (empty → fail) | O(1) |
| `len` / `is_full` | O(1) |

All operations are constant-time and allocation-free after
locus birth. No realloc, no rehash, no compaction.

## Interaction with the locus tower

A `@form(ring_buffer)` locus is a locus in every other respect.
Same orthogonality as the other forms — `params`, lifecycle
bodies, closure invariants, `on_failure` routing, bus
membership, perspective projection all compose unchanged.

## Anti-patterns

### Forgetting to address `pop`'s fallible

```hale
// WRONG — pop returns fallible(EmptyError); the bare let drops it.
let v = recent.pop();  // compile error: error not addressed
```

```hale
// RIGHT — address with one of the three motions.
let v = recent.pop() or raise;
```

### Treating `push` as fallible

```hale
// WRONG — push returns Bool, not fallible(FullError).
let v = recent.push(x) or raise;  // type error: push isn't fallible
```

The full-buffer state surfaces as a Bool return, not a fallible
payload. Inspect the Bool directly.

### Resizing expectations

The ring buffer's cap is fixed at the annotation site. There is
no `grow` or `shrink_to_fit`. Apps that need a growable bounded
buffer should pick a generous cap up front, or use
`@form(vec)` if growth is the right semantic.

## Open questions deferred to a future milestone

1. **Iteration surface.** `for x in recent { ... }` is natural
   but iteration over a ring buffer must respect head/tail wrap
   — needs the `for` lowering to know about ring shapes.
   Deferred.
2. **Bulk operations.** `clear()`, `peek() -> T fallible`,
   evict-oldest-on-full mode (cyclic-overwrite as a tuning
   knob). Useful but not foundational; add when a workload
   demonstrates demand.
3. **Iteration in pop order without removing.** A "drain" or
   "iter_pop" that visits elements oldest-first as a one-shot.
4. **Bench protocol.** A `micro/form_ring_buffer_*` family
   in `hale-lang/bench`, parallel to vec's and hashmap's.
   Ships as a separate milestone after a consumer workload
   surfaces.

# `@form(lru_cache)`

A fixed-capacity keyed cache with least-recently-used eviction.
The keyed counterpart of `@form(ring_buffer)`: like
`@form(hashmap)` it is intrusively keyed (the cell carries its own
key via `indexed_by`), but like `@form(ring_buffer)` it is
capacity-bounded and NEVER grows. Inserting a new key over `cap`
silently evicts the least-recently-**used** entry to make room.
This is the "cap-bounded, never-flagged" keyed form — the
unbounded-allocation analysis (`spec/verification.md`) treats an
`@form(lru_cache)` locus as bounded, exactly like `ring_buffer`.
Shipped as the fourth form in v1 via v1.x-FORM-6.

## Required capacity shape

The locus MUST declare exactly one `pool` slot with an
`indexed_by <fieldname>` clause over a user-declared struct cell —
the same key surface as `@form(hashmap)` — AND the annotation arg
`cap = N`. `lru_cache` is the one form that needs BOTH a key and a
cap.

```hale
type SessionEntry { id: Int; token: String; }

@form(lru_cache, cap = 1000)
locus SessionCache {
    capacity { pool sessions of SessionEntry indexed_by id; }
}
```

Rules verified at typecheck:

- Exactly one slot, of kind `pool`. Zero slots, more than one
  slot, or a `heap` slot is rejected.
- The slot MUST declare `indexed_by <fieldname>`; the named field
  must exist on the cell struct and becomes the cache key type
  `K` (Int or String at v1). The cell struct is the value type
  `S`.
- `cap = N` is required and must be a positive integer literal.
  v1 doesn't const-evaluate expressions for form args.
- The slot MUST NOT declare `as_parent_for` — form-lowered slots
  own their own allocator.
- The cell type MAY NOT be a locus reference — same restriction
  as the other forms.

## Synthesized methods

```
fn put(x: S) -> ()                          # infallible; silent LRU evict
fn get(k: K) -> S fallible(KeyError)        # lookup + recency touch
fn contains(k: K) -> Bool                   # membership, NO recency touch
fn len() -> Int                             # infallible; count <= cap
```

`get`'s miss payload is the synthesized `KeyError` (shared with
`@form(hashmap)`); no new payload type is introduced.

### `put`

```
fn put(x: S) -> ()
```

Insert or update by the `indexed_by` key extracted from `x`. If
the key is already present, its value is overwritten and its
recency is refreshed. If the key is new and the cache is at
capacity, the least-recently-used entry is silently evicted first.
`put` is **infallible** — over-cap is a normal, silent operation
(eviction), not a failure. This matches the "never flagged /
bounded" contract: there is no `FullError`, unlike
`@form(ring_buffer).push` which returns a Bool.

Both insert and update mark the touched entry as recently used.

### `get`

```
fn get(k: K) -> S fallible(KeyError)
```

Looks up the value for key `k`. On a hit it returns the value
**and** marks the entry as recently used (a recency touch) — this
is what makes the policy LRU rather than FIFO: a `get` on an entry
saves it from an eviction that a purely-oldest-inserted policy
would take. On a miss it raises `KeyError`, addressed at the
call site's `or` clause.

### `contains`

```
fn contains(k: K) -> Bool
```

Membership test. Returns `true` if `k` is present. Unlike `get`,
`contains` does **NOT** touch recency — a `contains` on an entry
leaves it exactly as recently-used as it was. This distinction is
observable: after `contains(k)`, the entry `k` remains eligible
for eviction if it was the LRU entry.

### `len`

```
fn len() -> Int
```

Current live entry count, always `<= cap`. Infallible.

## Lowering strategy

`@form(lru_cache)` lowers the pool slot to an inline header struct
plus a single heap-allocated open-addressed table, managed by the
`lotus_lru_*` C runtime:

```
struct lotus_lru_t {
    size_t   cap;           // fixed live-entry cap (never grows)
    size_t   len;           // current live entries (<= cap)
    size_t   key_size;
    size_t   value_size;
    int      key_type_tag;  // Int / String key ABI (shared w/ hashmap)
    uint64_t tick;          // monotonic access counter
    size_t   table_cap;     // power-of-two slot count (>= 2*cap)
    char    *slots;         // table_cap * (occupied + tick + key + value)
}
```

It deliberately does **not** reuse the `lotus_hashmap_*` family:
that family auto-grows and has no recency notion, both of which
break the cap invariant. The table is sized to the next power of
two `>= 2*cap` (load factor `<= 0.5`, so a probe always meets an
empty terminator). Each slot carries an access tick; eviction
removes the occupied slot with the minimum tick (the LRU entry)
via backward-shift compaction (no tombstones), keeping the table
reusable under unbounded insert/evict churn. Ticks are globally
unique per access, so the LRU entry is unambiguous.

The backing table is pre-allocated at locus birth
(`lotus_lru_init`) and freed at dissolve (`lotus_lru_free`); the
inline header lives in the locus struct and dies with the arena —
the same inline-header / heap-buffer split as `@form(vec)`,
`@form(hashmap)`, and `@form(ring_buffer)`.

## Open questions deferred to a future milestone

1. **TTL eviction.** A `ttl = <duration>` annotation arg to
   expire entries by age in addition to LRU by capacity. Deferred
   — v1 evicts by capacity + recency only.
2. **`remove(k)` / `clear()`.** Explicit eviction of a named key
   and bulk clear. Add when a workload demonstrates demand.
3. **Iteration surface.** `key_at` / `entry_at`-style indexed
   iteration, parallel to `@form(hashmap)`. Deferred.
4. **Bench protocol.** A `micro/form_lru_cache_*` family in
   `hale-lang/bench`, parallel to the other forms'. Ships after a
   consumer workload surfaces.
