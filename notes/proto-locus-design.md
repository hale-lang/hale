# Proto-locus design: everything-is-a-locus

**Status:** SHIPPED — `@form(vec)` / `@form(hashmap)` /
`@form(ring_buffer)` landed; see `spec/forms.md`. This note is
retained as the originating design pass (2026-05-12).

## Premise

**The locus is Hale's universal source-level structural
primitive.** Every named shape in a program — data record,
array, key→value lookup, ring buffer, growable list — is a
locus at the source level. The compiler picks the lowering:
some loci compile to literal F.22 pool walks, others to
optimized C-backed forms (contiguous buffer, hashmap, etc.).

This is the most radical reading of The Design's *form before
parameter* — form is the locus declaration; parameter is the
compiler-chosen implementation.

There is no source-level "value type vs locus" dichotomy.
`type T { ... }` is a locus that only declares fields — sugar
for the smallest growth stage. The gradient is:

| Growth stage | Surface | Has |
|---|---|---|
| Pure shape | `type T { fields }` | name, fields |
| Parametric shape | `type T<G> { fields }` | + generics |
| Enum shape | `type T { Variants }` | tagged-union shape |
| Identity | `locus L { params {} }` | + defaultable shape, owned + named in tower |
| Behavior | `locus L { params {} run() {} }` | + lifecycle bodies |
| Substrate | `locus L { capacity { pool of T; } }` | + F.22 storage discipline |
| Audit | `locus L { closure {} }` | + closure assertions |
| Recovery | `locus L { on_failure(c, err) {} }` | + F.9 failure routing |
| Cross-process | `locus L { bus { subscribe ... } }` | + bus participation |
| Interop | `perspective P of L { ... }` | + parametric reflection on L |

Each stage strictly extends the previous. `type` is "proto-
locus" — locus shape that hasn't yet accreted the higher
mechanics. The keyword distinction (`type` vs `locus`) survives
as ergonomic sugar; the underlying construct is one.

## The form annotation

A locus may carry an optional `@form(...)` annotation telling
the compiler "this locus's structural shape commits to a known
efficient implementation; lower me accordingly":

```hale
@form(hashmap)
locus CmdRegistryL {
    capacity { pool entries of CmdEntry indexed_by key; }
    fn get(name: String) -> CmdEntry { ... }
    fn set(entry: CmdEntry) { ... }
}
```

`@form(hashmap)` is a **commitment**, not a request. The
compiler:

1. **Verifies the locus shape matches the form's contract.**
   - Hashmap requires: a `pool` capacity slot of structured
     entries, an `indexed_by <field>` annotation on the slot,
     at least `get(K) -> V` / `set(V)` methods.
   - Vec requires: a `heap` capacity slot of T (or `pool` with
     order-preservation), at least `get(Int) -> T` / `push(T) -> ()` /
     `len() -> Int` methods.
   - Ring buffer requires: a `pool` capacity slot of T with a
     fixed cap, at least `push(T) -> ()` / `pop() -> T` /
     `len() -> Int` methods.

   If the locus doesn't satisfy the form's shape contract, the
   compiler emits a focused diagnostic naming the missing piece
   ("form(hashmap) requires `indexed_by <field>` on the pool
   slot; got `pool entries of CmdEntry`").

2. **Lowers to an optimized C-backed implementation.** The
   F.22 pool / heap on the locus struct is replaced with a
   form-specific layout (e.g. `{ size_t cap, size_t len,
   void *buf }` for vec, or a real hashtable struct for
   hashmap). The user's method bodies dispatch through the
   form's runtime primitives.

3. **Preserves locus semantics.** Lifecycle, failure routing,
   perspectives, slot-sharing all still apply. A
   `@form(hashmap)` locus is still a locus; it participates
   in the locus tower the same way any other locus does.

The annotation is the bridge between "user wrote a locus" and
"compiler picked the efficient lowering." Without the
annotation, the compiler defaults to the literal F.22 pool/heap
walk — correct but slower than a form-specialized lowering.

## v1 form library

Cut to the smallest set that covers the patterns recurring
across apps:

- `@form(vec)` — growable list of T. Backed by a doubling-
  realloc malloc buffer. Same shape as `lotus_str_builder` but
  generalized over T.
- `@form(hashmap)` — open-addressing hashmap keyed on a named
  field. Backed by a real hashtable C struct.
- `@form(ring_buffer)` — fixed-cap circular queue. Backed by
  a fixed-size array + head/tail indices.

Defer `tree`, `set`, `deque`, `bloomfilter`, etc. until a
workload surfaces the need.

## Performance hypothesis

> A `@form(vec)` / `@form(hashmap)` locus, when lowered by the
> compiler, runs within 10% of a hand-written equivalent in
> idiomatic C.

This hypothesis is the gate. Before shipping the form
machinery generally, the first form (`@form(vec)`) gets
benchmarked end-to-end:

- **Microbenchmark:** 1M append + 1M iterate, compared against
  `crates/hale-codegen/runtime/lotus_arena.c`-style C code.
- **App benchmark:** a representative parsing-heavy workload is
  rewritten to use form-lowered Vecs in place of F.22 pool
  walks. Wall-clock + RSS compared before / after.

If the hypothesis fails, redesign the lowering before adding
more forms. The point of the form machinery is *not* to be
clever — it's to be roughly as fast as the C the user would
have written by hand, with all the locus tower benefits on top.

## Implications for the v1.x list

- **v1.x-12 (Map stdlib)** is cut as a parametric stdlib type.
  Replaced by `@form(hashmap)` recognition + lowering.
- **v1.x-13 (Vec stdlib)** is cut as a parametric stdlib type.
  Replaced by `@form(vec)` recognition + lowering.
- **v1.x-14 (Rope / chunk-list)** is cut. `lotus_str_builder`
  already covers the friction case; if a future workload needs
  a rope-shaped collection, it becomes `@form(rope)` later.
- **Existing generics (m63) stay** for explicit parametric loci.
  They become orthogonal to forms — a generic locus
  `locus Cache<K, V>` can still carry `@form(lru)` to pick
  its lowering.
- **No `Map<K, V>` / `Vec<T>` keywords or stdlib types.**
  Hale source code never says `Map<K, V>` parametrically.
  It says "a locus shaped like a hashmap" via the annotation.

## Connection to existing design

- **F.22 capacity slots** stay as the substrate the forms build
  on. `@form(vec)` doesn't replace `heap of T`; it specializes
  the lowering of that exact shape.
- **Generics (m63)** stay as the parametric mechanism for
  explicit type parameters on loci. A form annotation is a
  *separate* axis from generic parameters.
- **The Design's "form before parameter"** is satisfied: the
  locus declaration is form (structural commitment), the
  compiler-chosen implementation is parameter (efficiency
  picked from a fixed menu of forms).
- **Failure-propagation-upward** applies uniformly. A
  `@form(hashmap)` locus's closure assertions and on_failure
  handlers work the same as any other locus's.

## Worked example (vec)

The first form to ship. Smallest shape contract; clearest perf
comparison point; smallest implementation surface.

```hale
@form(vec)
locus ItemListL<T> {
    capacity { heap items of T; }
    fn push(item: T) -> () { ... }
    fn get(i: Int) -> T { ... }
    fn len() -> Int { ... }
}

fn main() {
    let l = ItemListL_Int { };
    l.push(1);
    l.push(2);
    l.push(3);
    println(f"len={l.len()} first={l.get(0)}");
}
```

Verification checks:
1. Has exactly one capacity slot named `items` of kind `heap` of T.
2. Has `push(item: T) -> ()` method.
3. Has `get(i: Int) -> T` method.
4. Has `len() -> Int` method.

Lowering:
1. The `heap items of T` slot lowers to `{ size_t cap, size_t len, T *buf }`
   embedded in the locus struct (instead of `lotus_heap_t*`).
2. `push` body is replaced with a doubling-realloc append.
3. `get(i)` body is replaced with bounds-check + array index.
4. `len()` body is replaced with a field read.
5. `dissolve` runs `free(self.items.buf)` (and runs each T's
   dissolve first if T is a locus type).

The user's method bodies are *thrown away* in the lowered form
because the form's contract is precise enough that the bodies
are deterministic given the shape. Future variants could let
the user override individual methods for custom behavior;
v1 keeps it strict for simplicity.

## What this isn't

- **It's not Rust's traits or Swift's protocols.** Those are
  parametric polymorphism mechanisms. Forms are
  *implementation hints* — the user writes a concrete locus
  and the compiler picks an efficient lowering for shapes it
  recognizes.
- **It's not C++'s template specialization.** Templates do
  source-level substitution; forms do shape recognition +
  reimplementation.
- **It's not a macro system.** Forms are first-class compiler
  knowledge, not source-rewriting.
- **It's not optional.** A locus without a form annotation
  gets the default F.22 pool/heap lowering — correct but
  unoptimized. The annotation is the user *opting in* to a
  specific efficient lowering.

## Open design questions

1. **Naming.** `@form(vec)` vs `form vec` (decorator-style
   keyword) vs `locus L: vec { ... }` (trait-bound-style).
   First two are non-committal in syntax; the third would
   require parser work but reads cleanest. Bias toward
   `@form(...)` for v1 since it doesn't claim parser real
   estate.

2. **User overrides.** Can a `@form(vec)` locus override one
   of the synthesized methods? v1 says no — the form contract
   is total. v2 might allow overrides if the body type-checks
   compatibly.

3. **Form composition.** Can a locus carry multiple form
   annotations (`@form(vec) @form(ordered)`)? Defer — single
   form per locus at v1.

4. **`indexed_by` placement.** Does `indexed_by` live on the
   capacity slot (`pool entries of T indexed_by key`) or on
   the form annotation (`@form(hashmap, key = field_name)`)?
   The slot placement reads better (the indexing IS a
   storage-discipline concern), but the annotation placement
   keeps slots simpler. Tentative: slot.

5. **What does perspective do with form-lowered loci?**
   Perspectives reflect on locus *structure* — fields, methods,
   capacity slots. The form lowering changes the *implementation*
   but not the structure. So perspectives should work uniformly.
   Worth confirming with a worked example in step 2.

## Roadmap

| Step | What | Gate |
|---|---|---|
| 1 | This design note | User approval before any code |
| 2 | Spec the `@form(vec)` shape contract in detail | Worked example resolves open questions |
| 3 | Implement `@form(vec)` end-to-end | Annotation parsing + shape verification + C-runtime vec + codegen lowering |
| 4 | Benchmark `@form(vec)` against hand-written C | Perf hypothesis holds (within 10%) |
| 5 | Ship `@form(hashmap)` + `@form(ring_buffer)` | Pattern from step 3 generalizes |
| 6 | Cut v1.x-12 / 13 / 14 from the roadmap | Form library replaces them |

## What changes in the existing v1.x checkpoint

After this design note is accepted, the v1.x-checkpoint.md
"Cut from roadmap" section gains:

- v1.x-12 (Map stdlib) — replaced by `@form(hashmap)`.
- v1.x-13 (Vec stdlib) — replaced by `@form(vec)`.
- v1.x-14 (Rope) — covered by string-builder; future `@form(rope)`
  if a workload demands.

The "Remaining items" collapse to:

- v1.x-3 (recognition pool impl) — settled design, just needs
  implementation.
- v1.x-4b (as_parent_for runtime) — settled design, just needs
  implementation.
- v1.x-9 (closures with capture) — design-gated still.
- **NEW: proto-locus form machinery** — this note's roadmap.
