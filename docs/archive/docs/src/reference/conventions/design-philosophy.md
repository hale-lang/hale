# Design philosophy

## Synopsis

> **Every named structural thing in Aperio source code is a
> [locus](../glossary.md#locus).**

A locus is not one construct among several. It is *the*
construct. `type` is the smallest growth stage of a locus
(shape only). A full locus with `params`, `capacity`, lifecycle
bodies, `closure` assertions, `on_failure` handlers, `bus`
blocks, and `perspective` projections is the largest. Every
structural decl in source code sits somewhere on that gradient.

This is the most radical reading of The Design's *form before
parameter*: form is the locus declaration; parameter is what
the compiler chooses given the form. The user describes what
*is*; the compiler decides how it's *built*.

The canonical version of this document — including detailed
rationale and the locked-in v1 decisions — lives at
`notes/agent-onboarding/aperio-design-philosophy.md` in the
source tree. This page is the user-facing summary.

## Not Ruby's "everything is an object"

The phrase "everything is X" can mean two different things:

| Model | Where the uniformity lives | Cost |
|---|---|---|
| Semantic uniformity (Ruby, Smalltalk) | At runtime. Every value is a heap object; every call is dynamic dispatch. | Significant runtime overhead in exchange for one mental model. |
| Structural uniformity (Aperio) | At source. Every named structural thing is a locus declaration. The compiler picks the lowering. | Zero runtime cost — `Int` is still `i64`, a `@form(vec)` locus lowers to a contiguous buffer, an empty `params { }` locus stack-allocates. |

Aperio's "everything is a locus" is structural. The source code
has one universal abstraction for naming structure; the
compiler is responsible for honoring it efficiently.

## The three axes of a locus

A locus declaration commits to three independent axes. Each
answers a different question.

| Axis | Direction | Question | Surface |
|---|---|---|---|
| Capacity | Inward | What does this locus hold inside? | `capacity { pool X of T; heap Y of U; }` |
| Projection | Upward | Where does this locus's own memory come from? | `: projection rich / chunked / recognition(...)` |
| Form | Sideways (to compiler) | How should the compiler implement this locus? | `@form(vec)` / `@form(hashmap)` / `@form(ring_buffer)` |

The three are orthogonal — any combination is valid. Most
loci use only capacity (default projection, no form).
Performance-critical loci add a form annotation. Loci that
live in dense parent-owned populations add a projection class.

### Capacity ↔ projection symmetry

What's a capacity slot from the parent's view is a projection
from the child's view. If `ParentL` has `pool kids of ChildL`,
then `ChildL` is projected into the parent's `kids` slot —
the parent is the source of `ChildL`'s memory.

### Form is independent

A form annotation never changes *what* the locus structurally
is. It only changes *how the compiler builds the bytes*. You
can add or remove a form annotation without changing the
locus's participation in the tower, its lifecycle behavior,
its failure routing, or its perspective surface.

## What's NOT a locus

The "everything is a locus" principle has clean exclusions:

- **Primitives.** `Int`, `Float`, `Bool`, `Decimal`, `Time`,
  `Duration`, `String`, `Bytes`. The atomic value layer.
- **Functions.** Pure mappings from inputs to outputs. No
  identity, no lifecycle, no tower position.
- **Generic parameters.** Placeholders bound at
  monomorphization.
- **Seeds.** Directory-level source-organization unit; a
  grouping over loci, not a super-locus.

These four exclusions are the complete list.

## The form annotation

A locus may carry an optional `@form(...)` annotation telling
the compiler "this locus's structural shape commits to a known
efficient implementation; lower me accordingly":

```aperio
@form(hashmap)
locus CmdRegistryL {
    capacity { pool entries of CmdEntry indexed_by name; }
    // get / set / has / remove / len synthesized by @form(hashmap)
}
```

The compiler:

1. **Verifies the locus shape matches the form's contract.**
   For `@form(hashmap)`: a `pool` capacity slot of structured
   entries with `indexed_by <field>`. Wrong shape → focused
   diagnostic and reject.
2. **Synthesizes the standard methods.** `push` / `get` / etc.
   come from the form; the user does not write them. User CAN
   add additional methods on top.
3. **Lowers to an efficient C-backed implementation.** The
   F.22 pool / heap on the locus struct is replaced with a
   form-specific layout (contiguous buffer for vec, real
   hashtable for hashmap).
4. **Preserves locus semantics.** Lifecycle, failure routing,
   perspectives, slot-sharing all still apply.

### v1 form library

| Form | Required capacity | Synthesized methods | Lowering |
|---|---|---|---|
| `@form(vec)` | `heap items of T` | `push(T) -> ()`, `get(Int) -> T`, `pop() -> T`, `len() -> Int` | `{ cap, len, T* buf }` with doubling realloc |
| `@form(hashmap)` | `pool entries of S indexed_by F` (F: String field on S) | `get(String) -> S`, `set(S) -> ()`, `has(String) -> Bool`, `remove(String) -> ()`, `len() -> Int` | Open-addressing hashtable keyed on F |
| `@form(ring_buffer)` | `pool items of T` with fixed cap | `push(T) -> Bool`, `pop() -> T`, `len() -> Int`, `is_full() -> Bool` | Fixed-size array + head/tail indices |

Future forms (`@form(tree)`, `@form(set)`, `@form(deque)`,
`@form(lru_cache)`, `@form(rope)`) are deferred until a
workload surfaces the need.

### Default behavior

A locus without `@form(...)` gets the literal F.22 default
lowering — chunked free-list for pools, doubly-linked live
list for heaps. Correct but unoptimized for specialized
workloads. The form annotation is the user's opt-in to a
specific efficient lowering.

### Performance commitment

> **A form-lowered locus must run within 10% of a hand-written
> equivalent in idiomatic C.**

This is the gate before adding more forms. If the hypothesis
fails for the first form (`@form(vec)`), the lowering is
redesigned before any subsequent form ships.

## Consequences for the language

Locking in everything-is-a-locus has direct consequences:

### No parametric collection types

Aperio source code **never** says `Map<K, V>`, `Vec<T>`,
`Option<T>`, `Result<T, E>` as parametric types. The collection
is the locus; the form annotation picks the lowering.

| Old idiom (Rust-shaped) | Aperio idiom |
|---|---|
| `Map<String, Int>` | `@form(hashmap) locus L { capacity { pool entries of Entry indexed_by key; } }` |
| `Vec<T>` | `@form(vec) locus L<T> { capacity { heap items of T; } }` |
| `Option<T>` | sentinel value + companion predicate |
| `Result<T, E>` | sentinel + predicate, OR locus-level `bubble` / `on_failure` |

### No value-level failure types

Failure is structural. The locus tower's `bubble` /
`on_failure` mechanism (F.9) handles every legitimate failure
path. Value-level error types (Result, Either) would duplicate
the structural failure mechanism at the parametric level —
exactly what The Design counsels against.

The complete failure surface:

1. **Closure violation** at audit time → `ClosureViolation`
   routed to parent's `on_failure(child, err)`.
2. **Sentinel-with-discriminator** for "couldn't compute"
   cases (`parse_int` returns 0; `can_parse_int` is the
   explicit predicate).
3. **Hard substrate failure** (OOM, divide-by-zero, null
   deref) terminates the process directly.

No `?` operator. No `panic(msg)`. No `assert(cond)`. No
`unwrap()`. The substrate covers it.

### Generics stay, scoped

Generic types and loci (m63) stay as the parametric mechanism
when type parameters are genuinely useful — `locus Box<T>`,
`type Pair<A, B>`. They become **orthogonal** to forms:

```aperio
@form(lru_cache, max = 100)
locus Cache<K, V> {
    capacity { pool entries of Entry<K, V> indexed_by key; }
}
```

The generic params parameterize the *shape*; the form
annotation parameterizes the *lowering*. They don't compete.

## Anti-patterns

### Reaching for parametric containers

```aperio
// WRONG — Aperio has no Map<K, V>.
let m: Map<String, Int> = Map::new();
m.insert("a", 1);
```

```aperio
// RIGHT — declare the registry locus.
@form(hashmap)
locus MyRegistryL {
    capacity { pool entries of Entry indexed_by key; }
}

let r = MyRegistryL { };
r.set(Entry { key: "a", value: 1 });
```

### Value-level error types

```aperio
// WRONG — no Result<T, E> in Aperio.
fn parse(s: String) -> Result<Int, String> { ... }
```

```aperio
// RIGHT — sentinel + predicate.
fn parse(s: String) -> Int {
    return std::str::parse_int(s);  // 0 on failure
}

fn main() {
    let s = std::env::arg(1);
    if std::str::can_parse_int(s) {
        let n = parse(s);
    }
}
```

### Value-level panic

```aperio
// WRONG — no panic() in Aperio.
fn divide(a: Int, b: Int) -> Int {
    if b == 0 { panic("divide by zero"); }
    return a / b;
}
```

```aperio
// RIGHT — closure assertion at the locus level.
locus DividerL {
    params { numerator: Int = 0; denominator: Int = 1; }
    closure denom_nonzero {
        self.denominator != 0;
    }
    fn divide() -> Int {
        return self.numerator / self.denominator;
    }
}
```

### Hand-rolled forms

```aperio
// WRONG — re-implementing hashmap mechanics when
// @form(hashmap) does it for you.
locus RegistryL {
    capacity { pool entries of CmdEntry; }
    fn get(name: String) -> CmdEntry {
        // 50 lines of hand-written hash + probe
    }
}
```

```aperio
// RIGHT — declare the form.
@form(hashmap)
locus RegistryL {
    capacity { pool entries of CmdEntry indexed_by name; }
    // get / set / has / remove / len synthesized.
}
```

## Locked-in decisions (v1)

For ready reference:

1. Every named structural thing is a locus. Types are
   loci-in-waiting.
2. `@form(<name>, <args>...)` is the form annotation syntax.
3. One form per locus. Composition deferred.
4. The form synthesizes methods. User adds extras on top.
5. `indexed_by` on slot; tuning knobs as annotation args.
6. v1 forms: `vec`, `hashmap`, `ring_buffer`. Others deferred
   until workload demands.
7. Default lowering (no form) is literal F.22 pool/heap.
8. Perf gate: form-lowered locus within 10% of hand-written C.
9. Perspectives reflect on structure, not lowering.
10. No `Map<K, V>` / `Vec<T>` / `Option<T>` / `Result<T, E>`
    as parametric types.
11. Generics (m63) stay as the orthogonal parametric mechanism.
12. No `panic()` / `assert()` / `?` / `unwrap()`. Closure-
    violation routing covers every legitimate failure surface.

## See Also

- [Types vs loci](./axiom.md) — the entry-point axiom this
  philosophy unpacks.
- [Pattern catalog](./patterns.md) — the six shapes loci take.
- [Anti-patterns](./anti-patterns.md) — what fighting the
  philosophy looks like in code.
- [Composition](./composition.md) — how to compose loci.
- [The Design](../../../../spec/design-rationale.md) — the
  primitives + mechanics this philosophy realizes.
