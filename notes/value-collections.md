# Value collections — runtime-sized arrays (and the path to a growable Vec)

Status: **DECIDED — collections stay locus-only (2026-06-09).** Heap-owning
value collections (rungs 1/2 below) are **declined**. This note is kept as
the decision record: why value collections were considered, what they'd
have taken, why we said no, and — the practical upshot — why
`Type::from_json` has **no array struct-fields**, and what to do instead.

Written out of the JSON Tier-2 work (`Type::from_json`); the analysis
below is the reasoning behind the decision, not a build plan.

## Why this exists

`Type::from_json` (generated, schema-specialized) parses scalars and
nested structs today. It cannot parse a JSON **array** into a struct
field, because there is nowhere to put a variable-length sequence *as a
value*:

- `[T; N]` (`CodegenTy::Array(T, u64)`) is **compile-time fixed-size** —
  the length lives in the type. A declared-size field (`coords: [Float;
  3]`) works now (count the JSON array, fill by index, raise on length
  mismatch), but that only covers fixed-shape arrays — the minority of
  real JSON.
- `@form(vec)` is a **growable heap buffer, but locus-tied**: it is a
  locus cell with `push`/`pop`/`get`/`set`, its lifetime bound to the
  locus. It is not a value you can hold in a struct field or return from
  a function.

So the gap is a **heap-backed sequence that is a value** — sizeable at
runtime, storable in a struct, returnable from a parser.

## The key reframe (why this is smaller than "implement Vec")

A JSON array's length is **known before you fill it** — one cheap pass of
the existing array cursor counts the elements. So deserialization never
needs to *grow*: it can **count → allocate exactly N → fill by index**.
That collapses the requirement from "a growable Vec (push, capacity,
doubling realloc)" down to an **alloc-once, fill-once, then-immutable**
array. That is a strictly simpler primitive — and, for parsing, a faster
one (a single right-sized allocation, no realloc churn).

Call it **rung 1**: a runtime-sized, immutable-after-construction value
array.

## The crux: heap-owning value semantics (decide this FIRST)

The hard part of *any* value collection in Hale is not `push` or realloc.
It is: **a heap-owning value cuts against the language's memory model.**
Hale is arena- and locus-structured — collections live on loci with clear
lifecycles (`@form(vec)` exists precisely to keep ownership unambiguous).
A freely-passed `{ptr, len}` value re-opens the question Hale has so far
avoided:

- **Who frees the buffer, and when?**
- **What does copying / moving / returning the value mean** — deep copy,
  move-only, arena-scoped borrow, refcount?
- **Which arena does the buffer belong to** when a parser allocates it and
  returns it up the stack into a caller's struct?

This decision is shared by rung 1 and a future Vec, and it is the real
fork. Options, roughly in increasing departure from today's model:

1. **Arena-scoped, move-only.** The array is allocated in the current
   arena (the caller's frame / locus arena), is move-only (no implicit
   deep copy), and is freed when its arena unwinds. Closest to Hale's
   grain; the parser's result array lives in whoever called `from_json`.
   No refcount, no GC. **Recommended starting point** — it keeps "memory
   is structured by scope" intact and just lets a *value* name an
   arena-owned buffer.
2. **Deep-copy-on-assign (value semantics like a struct).** Simple mental
   model, but O(n) hidden copies — against Hale's explicit-cost ethos.
3. **Refcounted.** Flexible, but introduces runtime bookkeeping the
   language has deliberately not needed.

Had we proceeded, (1) **arena-scoped, move-only** was the approach, and
rung 1's immutability made it the safest case to prove (1) against. But
introducing *any* heap-owning value type was judged a departure from the
locus-owned memory model not worth making — see the decision below.

## Rung 1 — the runtime-sized value array

Representation: `{ T* ptr, i64 len }` (a new `CodegenTy`, distinct from
the inline `Array(T, u64)`). Allocated once at a runtime length.

Surface (sketch — names TBD):
- construction at a known size, filled by index, then read-only;
- `arr[i]` get (bounds-checked → `fallible(IndexError)`, reusing the pack
  error), `len(arr)`, and `for x in arr` iteration;
- element type generic over scalar / struct / nested.

The codegen reuses what exists: indexed get/set already works on inline
arrays; the new work is the heap representation + the runtime-sized alloc
+ the arena/ownership rules from the crux decision.

## Rung 2 — growable Vec value (the follow-on)

A growable value is **rung 1 + a `cap` field + `push`/`pop` + doubling
realloc**: `{ T* ptr, i64 len, i64 cap }`. The grow loop already exists in
`@form(vec)` — it is just bound to a locus today; rung 2 lifts that loop
onto rung 1's value representation. Because rung 1 has already settled the
heap-owning-value ownership rules, rung 2 is an *additive* step (add
mutation + growth on settled foundations) rather than deciding ownership
and mutation at once.

So: **rung 1 is most of the path to a growable value type** — it
establishes the representation, the runtime allocation, indexed access,
iteration, element-genericity, and (the crux) the heap-owning-value
memory model. Rung 2 adds only capacity + the already-proven grow logic.

## How the JSON generator consumes rung 1

For a field `items: [Inner]` (`Inner` itself a generated JSON struct):

1. On the matched key, take the array's raw text.
2. Pass 1 — walk it with the array cursor, **count** elements.
3. Allocate the rung-1 array at that length.
4. Pass 2 — walk again; for element `i`, fill `arr[i]` by parsing the
   element (scalar reader, or recurse into `__json_parse_Inner` for a
   nested element), propagating `JsonError` via `or raise`.
5. Construct the struct with the filled array.

No growth, one allocation, deterministic — the count-then-fill shape rung
1 is designed for.

## Decision: collections stay locus-only

The gating question — *does Hale want heap-owning value collections at
all?* — is answered **no.** A freely-passed, heap-owning `{ptr, len}`
value is a departure from the locus-owned/arena-structured memory model
that defines the language, and the JSON-array convenience does not justify
opening that door. Sequences are owned by loci, full stop; `@form(vec)` is
the answer for a growable, mutating, locus-owned list, and that is
deliberate, not a gap.

Consequently rungs 1 and 2 are **not pursued**, and the language gains no
value-collection type.

## What this means for `Type::from_json`

`from_json` supports scalar fields and nested `json:`-tagged structs. It
**does not** support array struct-fields, **by design** — there is no
value sequence to hold them, and we are not adding one. A fixed-size
`[T; N]` field could still be filled (the length is in the type), but
that covers little real JSON and isn't worth the special case on its own;
left out unless a concrete need appears.

To parse a JSON array, walk it with the existing **array cursor**
(`std::json::array_first` / `array_next`, or the `_span` variants) and
`push` each decoded element into a `@form(vec)` cell on a **locus** — the
sequence lives where Hale wants it. `from_json` stays the convenience for
the flat/nested *record* shape; arrays are an explicit, locus-owned step.

## If this is ever revisited

The fork is recorded, not erased. Were heap-owning value collections ever
reconsidered, the path is above: settle the arena-scoped move-only
ownership rule against rung 1 (immutable, alloc-once) first, then graft
`@form(vec)`'s grow loop for rung 2. But the current answer is locus-only,
and `from_json`'s array gap is a documented non-goal rather than a TODO.
