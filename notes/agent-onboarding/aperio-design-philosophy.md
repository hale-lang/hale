# The Aperio design philosophy — everything is a locus

> Companion to `aperio-styleguide.md`. The styleguide is the
> **how** — "how do you write idiomatic Aperio." This document
> is the **why** — "what is Aperio committing to about the
> shape of source code, and why does the form behavior follow
> from those commitments."
>
> Where the styleguide is descriptive ("here is what good
> Aperio code looks like"), this document is *prescriptive at
> the design level* — it locks in the commitments that all
> future language decisions must honor. The form behavior is
> the immediate consequence; the underlying axiom is what
> makes that consequence non-arbitrary.

## The core axiom

> **Every named structural thing in Aperio source code is a locus.**

A locus is not one construct among several. It is *the* construct.
`type` is the smallest growth stage of a locus (shape only).
A full locus with `params`, `capacity`, lifecycle bodies, `closure`
assertions, `on_failure` handlers, `bus` blocks, and `perspective`
projections is the largest. Every structural decl in source code
sits somewhere on that gradient.

This is the most radical reading of The Design's *form before
parameter*: form is the locus declaration; parameter is what the
compiler chooses given the form. The user describes what *is*; the
compiler decides how it's *built*.

### Not Ruby's "everything is an object"

The phrase "everything is X" can mean two very different things:

| Model | Where the uniformity lives | Cost |
|---|---|---|
| **Semantic uniformity** (Ruby, Smalltalk) | At runtime. Every value is a heap object, every call is dynamic dispatch. | Significant runtime overhead in exchange for one mental model. |
| **Structural uniformity** (Aperio) | At source. Every named structural thing is a locus declaration. The compiler picks the lowering. | Zero runtime cost — Int is still an i64, a `@form(vec)` locus lowers to a contiguous buffer, an empty `params { }` locus stack-allocates. |

Aperio's "everything is a locus" is structural. The source code
has one universal abstraction for naming structure; the
compiler is responsible for honoring it efficiently.

### What this means for the user

When you write source code, you don't have two categories
("primitive" vs "composite", "value type" vs "reference type",
"data class" vs "service class"). You have one category — *the
locus* — at different growth stages. You add capacity when you
need to hold things; you add lifecycle when you need run-time
behavior; you add a form annotation when you want a specific
efficient lowering; you add a perspective when you need
cross-tower agreement.

The mental model never switches. You're always *naming a
structural thing* and *deciding which mechanics it grows into*.

## The locus gradient

A locus declaration accretes capabilities. Each stage strictly
extends the previous — you cannot have a higher stage's
mechanic without the lower stages' shape.

| Stage | Surface | What gets added |
|---|---|---|
| **Pure shape** | `type T { fields }` | Name + fields. No identity at runtime — just a layout. |
| **Parametric shape** | `type T<G> { fields }` | + generic params (m63 monomorphization). |
| **Tagged shape** | `type T { Variants }` | + enum / sum-type variants. |
| **Identity** | `locus L { params {} }` | + an arena, a name in the tower, defaultable params. |
| **Substrate** | `locus L { capacity { ... } }` | + F.22 storage discipline (pool / heap of T). |
| **Behavior** | `locus L { params {} run() {} }` | + lifecycle bodies (birth / run / drain / dissolve). |
| **Audit** | `locus L { closure {} }` | + closure assertions (F.10). |
| **Recovery** | `locus L { on_failure(c, err) {} }` | + F.9 failure routing. |
| **Cross-process** | `locus L { bus { ... } }` | + bus participation. |
| **Interop** | `perspective P of L { ... }` | + parametric reflection on L (multiple-DAG-projection). |

The keyword distinction (`type` vs `locus`) survives as
ergonomic sugar — the parser treats them as one construct with
different default starting points. There is no semantic
difference between "a type" and "a locus with only fields"; they
compile to the same thing.

## What is NOT a locus

The "everything is a locus" principle has clean exclusions.
Things that are NOT loci:

- **Primitives.** `Int`, `Float`, `Bool`, `Decimal`, `Time`,
  `Duration`, `String`, `Bytes`. These are the atomic value
  layer beneath locus. They have shape (a width, alignment, set
  of operations) but no identity, no lifecycle, no slot
  position. They compile to register / stack / contiguous-buffer
  values.

- **Functions.** A `fn name(args) -> ret { body }` is a pure
  mapping from inputs to outputs. Functions have no identity, no
  lifecycle, no position in the tower. They're orthogonal to the
  locus axis — the computational primitive beneath it.

- **Generic parameters.** The `T` in `locus L<T>` is a
  placeholder, not a locus. It gets bound at monomorphization.

- **Modules / seeds.** A seed is the unit of source-file
  organization (directory of `.ap` files compiled together).
  It's not a locus at runtime — there's no "seed instance" in
  the locus tower. Seeds are a *grouping* over loci, not a
  super-locus.

These four exclusions are the complete list. Every other named
structural construct in Aperio source is a locus.

## The three axes of a locus

A locus declaration commits to three independent axes. Each
axis answers a different question:

| Axis | Direction | Question | Surface |
|---|---|---|---|
| **Capacity** | Inward | What does this locus hold inside? | `capacity { pool X of T; heap Y of U; }` |
| **Projection** | Upward | Where does this locus's own memory come from? | `: projection rich / chunked / recognition(...)` |
| **Form** | Sideways (to compiler) | How should the compiler implement this locus? | `@form(vec)` / `@form(hashmap)` / `@form(ring_buffer)` |

The three are orthogonal — any combination is valid. Most loci
use only capacity (default projection, no form). Performance-
critical loci add a form annotation. Loci that live in dense
parent-owned populations add a projection class.

### Capacity ↔ projection symmetry

What's a capacity slot from the parent's view is a projection
from the child's view. If `ParentL` has `pool kids of ChildL`,
then `ChildL` is projected into the parent's `kids` slot — the
parent is the source of `ChildL`'s memory.

This is the locus tower mechanic expressed at the type-system
level: parent allocates, child runs, dissolution cascades.

### Form is independent

Form annotations never change *what* the locus structurally is.
They only change *how the compiler builds the bytes*. You can
add or remove a form annotation without changing the locus's
participation in the tower, its lifecycle behavior, its failure
routing, or its perspective surface.

A `@form(hashmap)` locus is still a locus. It still has birth /
run / dissolve. It still routes closure violations through
on_failure. It still appears in perspectives. The form just
tells the compiler "the pool slot's actual storage is a real
hashtable, not a free-list walk."

## The form behavior — locked-in design

This section is *prescriptive*. Decisions below are locked for
v1. Open questions are flagged as such; everything else is
committed.

### Form annotation syntax

```aperio
@form(vec)
locus ItemListL<T> {
    capacity { heap items of T; }
}
```

- **`@form(<name>)`** — the annotation. Decorator-shaped, sits
  above the locus declaration like the existing projection
  annotation.
- **`<name>`** — the form identifier. Lowercase, single word.
- **Optional arguments** — `@form(name, key = value, ...)`. Used
  for form-specific configuration that isn't a storage discipline.
- **Locked: one form per locus.** Composition (`@form(vec)
  @form(ordered)`) is deferred to v2; v1 rejects multiple form
  annotations on the same locus.

### Form contract

Each form specifies:

1. **Required capacity shape** — what slots the locus must
   declare, of what kinds, holding what cell types.
2. **Required method signatures** — names, parameter types,
   return types of the standard methods the form provides.
3. **Lowering strategy** — what C-runtime substrate the
   compiler emits in place of the literal F.22 pool / heap.

The compiler verifies (1) at typecheck time. If the locus shape
doesn't match the form's required capacity, the compiler emits
a focused diagnostic ("`@form(hashmap)` requires `indexed_by
<field>` on the pool slot; got `pool entries of CmdEntry`") and
rejects the program.

### Method synthesis

> **The form synthesizes the standard methods. The user does not
> declare them.**

```aperio
@form(vec)
locus ItemListL<T> {
    capacity { heap items of T; }
    // No method declarations needed.
    // push, get, len, pop come from @form(vec).
}

fn main() {
    let l = ItemListL_Int { };
    l.push(42);
    println(l.get(0));  // 42
    println(l.len());   // 1
}
```

The form annotation is the complete contract: shape + methods +
lowering, bundled. The compiler injects the synthesized methods
at typecheck time so call sites resolve normally.

**The user CAN add additional methods** on top of the
synthesized standard set:

```aperio
@form(vec)
locus ItemListL_Int {
    capacity { heap items of Int; }
    fn sum() -> Int {
        let mut s = 0;
        let mut i = 0;
        while i < self.len() {
            s = s + self.get(i);
            i = i + 1;
        }
        return s;
    }
}
```

The standard methods are synthesized; `sum` is the user's
addition. Naming a user method that collides with a synthesized
method (e.g. user writes their own `push`) is rejected at v1 —
override is deferred to v2.

### `indexed_by` placement

Form configuration splits between *slot clauses* and
*annotation arguments*. The dividing line:

- **Slot clause** — if the configuration changes how cells are
  laid out or accessed (a storage-discipline concern).
- **Annotation argument** — if the configuration is a policy /
  tuning knob the form's runtime cares about.

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
commitment — it changes the pool's layout (hash bucket array)
and access path (hash-and-probe). `max = 100` is an annotation
argument because it's a policy knob the cache runtime consults;
the underlying pool structure is the same regardless.

### v1 form library

The forms committed for v1:

| Form | Required capacity | Synthesized methods | Lowering |
|---|---|---|---|
| `@form(vec)` | `heap items of T` | `push(T) -> ()`, `get(Int) -> T`, `pop() -> T`, `len() -> Int` | `{ size_t cap, size_t len, T* buf }` with doubling realloc. |
| `@form(hashmap)` | `pool entries of S indexed_by F` where F is a String field on S | `get(String) -> S`, `set(S) -> ()`, `has(String) -> Bool`, `remove(String) -> ()`, `len() -> Int` | Open-addressing hashtable keyed on F's value. |
| `@form(ring_buffer)` | `pool items of T` with fixed cap | `push(T) -> Bool`, `pop() -> T`, `len() -> Int`, `is_full() -> Bool` | Fixed-size array + head/tail indices, no malloc. |

Future forms (`@form(tree)`, `@form(set)`, `@form(deque)`,
`@form(lru_cache)`, `@form(rope)`) are deferred until a
workload surfaces the need.

### Default behavior — no form annotation

> **A locus without `@form(...)` gets the literal F.22 default
> lowering.**

```aperio
// No form annotation.
locus CmdRegistryL {
    capacity { pool entries of CmdEntry; }
    fn get(name: String) -> CmdEntry {
        // user-written body: linear scan over the pool
    }
}
```

The compiler lowers the pool slot to the standard
`lotus_pool_t*` chunked free-list. The user's `get` body
runs as written — no synthesis, no shape verification beyond
the normal capacity-slot machinery. Correct but unoptimized for
lookup-heavy workloads.

The form annotation is the user's opt-in to a specific
efficient lowering. Without it, you get the predictable F.22
default.

### Performance commitment

> **A form-lowered locus must run within 10% of a hand-written
> equivalent in idiomatic C.**

This is the gate. The first form to ship (`@form(vec)`) is
benchmarked before any subsequent form is added:

- **Microbenchmark.** 1M append + 1M random-index read on the
  form-lowered Vec, compared against a hand-written C vec
  (malloc + doubling realloc). Wall-clock and RSS.
- **App benchmark.** A representative app (ferryman is the
  natural candidate given its scale and parsing-heavy
  workload) is rewritten to use form-lowered Vecs where it
  currently does explicit F.22 pool walks. Wall-clock + RSS
  compared before / after.

If the hypothesis fails, redesign the lowering before adding
more forms. The point of the form machinery is **not** to be
clever — it's to be roughly as fast as the C the user would
have written by hand, with all the locus tower's structural
benefits on top.

### Perspectives and forms

> **Perspectives reflect on structure, not on lowering.**

The form annotation changes how the compiler lays out memory
and synthesizes methods. It does not change:

- The locus's name or place in the tower.
- The set of fields declared in `params`.
- The capacity slot declarations (a `pool` is still a `pool`,
  even if it lowers to a hashtable).
- The closure / on_failure / bus blocks.

Perspectives that reflect on a form-lowered locus see the
*structural* view: the capacity slots, the params, the method
signatures (synthesized or user-written, treated uniformly).
They do not see the underlying C struct layout.

This means perspectives work uniformly across formed and
unformed loci. A `perspective Diagnostics of CmdRegistryL { ... }`
projects the same surface regardless of whether `CmdRegistryL`
is `@form(hashmap)`-lowered or default-lowered.

## Consequences for the language

Locking in everything-is-a-locus has direct consequences. The
following are decisions, not hypotheses:

### 1. No parametric collection types

Aperio source code **never** says `Map<K, V>`, `Vec<T>`,
`Option<T>`, `Result<T, E>` as parametric types. The collection
is the locus; the form annotation picks the lowering.

| Old idiom (Rust-shaped) | Aperio idiom |
|---|---|
| `Map<String, Int>` | `@form(hashmap) locus L { capacity { pool entries of Entry indexed_by key; } }` |
| `Vec<T>` | `@form(vec) locus L<T> { capacity { heap items of T; } }` |
| `Option<T>` | sentinel value + companion predicate (`parse_int` / `can_parse_int`) for "couldn't compute"; `fallible(T)` for "an error with diagnostic context" |
| `Result<T, E>` | `fn f() -> T fallible(E)` — value-level addressing, then `or raise` to bridge into the runtime's closure-violation mechanism |

### 2. One runtime failure mechanism, with a value-level addressing protocol

**The runtime observes exactly one form of failure: closure
violation, routed via `bubble` / `on_failure`.** Process
termination is the special case where a closure violation
bubbles past root with no `on_failure` catching it. That is the
*only* way the runtime ends a program against its will.

What the runtime *does not* observe as a separate category:
raw panics, uncaught traps, value-level error returns. Stdlib
never emits raw panics; hardware traps (div-by-zero, OOM, null
deref, segfault) are caught at the substrate layer and re-raised
as closure violations.

But the runtime is not the only layer. Above it, Aperio has a
**value-level addressing protocol** for "an error occurred":

> **An error has occurred.** Singular, anonymous. A function
> declares the *fact* of fallibility in its signature with a
> `fallible(T)` marker — where T is the payload type the
> function attaches for diagnostics. The type system tracks
> "this expression can error" as a property; payloads are not
> parametric polymorphism over E.

A caller of a fallible fn picks exactly one **motion**:

1. **Raise** — `or raise` — convert to a closure violation.
   The only bridge from value-level error → runtime mechanism.
2. **Substitute** — `or <expr>` — provide a fallback value,
   continue.
3. **Hand off** — `or handler(err)` — pass typed payload to a
   handler fn, use its return as the value.

Log isn't a separate motion — it's a substitute whose handler
logs as a side effect.

**The compiler rejects** any expression of fallible type that
isn't addressed by an `or` clause or a `match`. Bare
`let v = parse(s);` when `parse` is fallible is a compile
error: "error not addressed."

**The terminal binary.** Every error-handling path ends in
exactly one of two runtime states: **raised** or **not raised**.
Substitute and hand-off both collapse to "not raised." The
runtime never observes a third state.

**The corollary.** Errors do not propagate as values up the
call stack. If a caller wants its caller to know, it raises a
closure violation, which propagates structurally. Value-level
error handling is a local contract between immediate caller
and callee; structural propagation is the runtime's domain.

**The bridge sheds the type.** `or raise` produces a closure
violation that is uniform-opaque to `on_failure` handlers. The
payload attaches as diagnostic data, but `on_failure` doesn't
pattern-match on payload type — surgical recovery belongs at
the `or` site while the typed payload is in scope; closure-
violation handling is a structural-decision activity
(log / bubble / terminate).

What is still cut: **`?` operator, `panic(msg)`, `assert(cond)`,
`unwrap()`, and `Result<T, E>` / `Option<T>` as parametric
tagged-enum types**. The fallible-marker model covers the
same use case without re-introducing a parallel upward-
propagation mechanism at the value level.

Example:

```aperio
type ParseError { message: String; line: Int; col: Int; }

fn parse(s: String) -> Int fallible(ParseError) {
    if bad_shape(s) {
        fail ParseError { message: "expected digit", line: 0, col: 0 };
    }
    return computed;
}

// Caller MUST address it.
let n = parse(s) or raise;                       // → closure violation
let n = parse(s) or 0;                           // → substitute
let n = parse(s) or handle_parse_err(err);       // → hand off, err typed ParseError
```

The `fail <expr>` form is the emit surface inside a fallible
body — symmetric to `return` but exits via the error path.

The sentinel-with-discriminator pair (`parse_int` /
`can_parse_int`) remains as a separate, *infallible* idiom for
"couldn't compute" cases where 0-on-failure is the natural
default and a paired predicate suffices. Use sentinel-pair when
the error has no useful payload; use `fallible(T)` when callers
benefit from inspecting the failure.

### 3. Generics stay, scoped

Generic types and loci (m63) stay as the parametric mechanism
when type parameters are genuinely useful — `locus Box<T>`,
`type Pair<A, B>`. They become **orthogonal** to forms:

```aperio
@form(lru_cache, max = 100)
locus Cache<K, V> {
    capacity { pool entries of Entry<K, V> indexed_by key; }
}
```

The generic params parameterize the *shape*; the form annotation
parameterizes the *lowering*. They don't compete.

### 4. Types are loci-in-waiting

A `type T { fields }` declaration is the smallest growth stage
of a locus. The compiler treats it as such:

- A `type` can be promoted to a `locus` by adding `locus`
  blocks (lifecycle, capacity, etc.) without rewriting the
  field declarations.
- The CodegenTy distinction (`TypeRef` vs `LocusRef`) survives
  for backward compatibility and ergonomics, but the underlying
  construct is one.
- v1.x-8 (type records hold fn(...) fields) was the first step
  toward letting types accrete more locus-like capabilities;
  future v1.x work may extend types further along the gradient.

## What this DOESN'T mean

The "everything is a locus" framing is precise. It does not
mean:

### It's not Ruby

Aperio's uniformity is *structural*, paid at the source level.
Ruby's is *semantic*, paid at runtime. The compiler can lower a
locus to a stack-allocated value, a contiguous buffer, a real
hashtable, or a chunked free-list — whatever the form and shape
indicate. No object header, no vtable lookup per call, no GC.

### It's not Smalltalk

There's no message-passing-based dispatch. Method calls on a
locus go through the same calling convention as fn calls. The
"locus method" is just a fn with `self: *LocusL` as the implicit
first arg.

### It's not Rust traits / Swift protocols

Forms are not parametric polymorphism. The user doesn't write
`impl Map<K, V> for MyMap`. They write a locus with a specific
shape and tag it with a form annotation. The compiler does
shape recognition + reimplementation, not trait resolution.

### It's not C++ templates

Templates do source-level substitution. Forms do not — the
user writes a concrete locus with concrete types in the capacity
slot; the form's lowering is fixed C code parameterized only by
the cell type's size and layout.

### It's not a macro system

Forms are first-class compiler knowledge. The form library is
*part of the language*, not user-extensible at v1. A future
release may allow user-defined forms, but that's a separate
design pass.

## Concrete examples

### Example 1: a command registry, three lowerings

```aperio
type CmdEntry { name: String; help: String; run: fn(); }

// Default: literal F.22 pool with user-written linear scan.
locus DefaultRegistryL {
    capacity { pool entries of CmdEntry; }
    fn get(name: String) -> CmdEntry {
        let mut i = 0;
        while i < self.entries.len() {
            let e = self.entries.at(i);
            if e.name == name { return e; }
            i = i + 1;
        }
        return CmdEntry { name: "", help: "", run: noop };
    }
}

// Same shape; @form(hashmap) picks the efficient lowering.
@form(hashmap)
locus FastRegistryL {
    capacity { pool entries of CmdEntry indexed_by name; }
    // get / set / has / remove / len synthesized.
}

// Ring buffer for a recent-history list.
@form(ring_buffer)
locus RecentCmdsL {
    capacity { pool history of CmdEntry; }  // fixed cap from declaration
    // push / pop / len / is_full synthesized.
}
```

All three are loci. All three live in the tower. All three
have lifecycle and failure routing. The user picks the lowering
that matches the workload.

### Example 2: a growable buffer

```aperio
@form(vec)
locus ByteBufferL {
    capacity { heap bytes of Int; }
    // push / get / pop / len synthesized.
}

fn collect_chunks() -> ByteBufferL {
    let buf = ByteBufferL { };
    let mut i = 0;
    while i < 100 {
        buf.push(read_byte());
        i = i + 1;
    }
    return buf;
}
```

The locus surface gives you tower position, lifecycle, failure
routing. The form gives you amortized-O(N) appends, dense
storage, no per-cell allocator overhead.

### Example 3: a perspective on a formed locus

```aperio
@form(hashmap)
locus SessionCacheL {
    capacity { pool sessions of Session indexed_by id; }
}

perspective Diagnostics of SessionCacheL {
    // Reflects on structure, not lowering. Sees:
    //   - the params block
    //   - the capacity slot named `sessions`
    //   - the synthesized methods get / set / has / remove / len
    // Does NOT see the underlying hashtable C struct.
}
```

The perspective doesn't care that `SessionCacheL` is
hashmap-lowered. It sees the locus's structural API, same as it
would see for any other locus.

## Connection to The Design

This philosophy is a direct consequence of The Design's
mechanics, not an independent invention:

- **Form before parameter (F.0)** — the locus declaration is
  form; the compiler-chosen lowering is parameter. Aperio's
  surface is form-only; the compiler does the parametric work.
- **Capacity (F.22)** — the inward axis. Every locus declares
  what it holds.
- **Displacement** — the locus tower mechanic; parent →
  child memory sourcing.
- **Failure-propagation-upward (F.9)** — closure violations
  bubble through `on_failure` handlers. No parallel value-level
  *upward-propagation* mechanism. `fallible(T)` is the value-
  level addressing protocol for the immediate caller, but
  upward propagation crosses into the runtime via `or raise`
  and rides the same closure-violation track from there.
- **Root-as-boundary** — the top-level locus is the process
  boundary. Loci that bubble past root terminate the process.
- **Vertical-only flow (F.8)** — no sibling-to-sibling fn calls;
  cross-locus comms goes through the bus or through parent.
- **Multiple-DAG-projection** — perspectives expose multiple
  views of the same underlying locus. Form-lowering doesn't
  change the perspective surface.

Every locus instance is The Design realized in a particular
shape. Every form annotation is a commitment to a specific
implementation strategy that honors the structural mechanics.

## Anti-patterns

The locked-in philosophy rules out a set of recurring habits.
Catching these early avoids design drift.

### Anti-pattern: reaching for parametric containers

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

### Anti-pattern: parametric tagged-enum error types

```aperio
// WRONG — no Result<T, E> as a parametric tagged enum.
// This reintroduces upward-propagation-as-value, duplicating the
// closure-violation mechanism at the value level.
fn parse(s: String) -> Result<Int, String> {
    if !std::str::can_parse_int(s) {
        return Err(f"bad input: {s}");
    }
    return Ok(std::str::parse_int(s));
}
```

```aperio
// RIGHT — `fallible(T)` with a named payload type. The fact-of-
// fallibility is a marker on the signature; the payload is an
// ordinary user-defined type. Callers MUST address the error
// via `or raise | or <expr> | or handler(err)` or a `match`.
type ParseError { message: String; input: String; }

fn parse(s: String) -> Int fallible(ParseError) {
    if !std::str::can_parse_int(s) {
        fail ParseError { message: "bad input", input: s };
    }
    return std::str::parse_int(s);
}

fn main() {
    let s = std::env::arg(1);
    let n = parse(s) or raise;  // closure violation on failure
    println(n);
}
```

The sentinel-with-discriminator idiom is still valid for the
*simpler* case where no payload is useful:

```aperio
// RIGHT — sentinel + predicate when "0 on failure" is the
// natural default and no diagnostic context is needed.
fn main() {
    let s = std::env::arg(1);
    if std::str::can_parse_int(s) {
        let n = std::str::parse_int(s);
        println(n);
    } else {
        println("not a number");
    }
}
```

Rule of thumb: `fallible(T)` when callers benefit from inspecting
the failure; sentinel-pair when 0/empty/"" is a useful default
and the predicate question is all callers ask.

### Anti-pattern: value-level panic

```aperio
// WRONG — no panic() in Aperio.
fn divide(a: Int, b: Int) -> Int {
    if b == 0 { panic("divide by zero"); }
    return a / b;
}
```

```aperio
// RIGHT — locus closure asserts the invariant.
locus DividerL {
    params { numerator: Int = 0; denominator: Int = 1; }
    closure denom_nonzero {
        self.denominator != 0;
    }
    fn divide() -> Int {
        return self.numerator / self.denominator;
    }
}

// In the parent:
locus ParentL {
    on_failure(d: DividerL, err: ClosureViolation) {
        println("attempted divide by zero in ", err.closure);
        bubble(err);
    }
}
```

### Anti-pattern: hand-rolled forms

```aperio
// WRONG — re-implementing hashmap mechanics in user code
// when @form(hashmap) does it for you.
locus RegistryL {
    capacity {
        pool entries of CmdEntry;
    }
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

### Anti-pattern: form annotation without matching shape

```aperio
// WRONG — @form(hashmap) without indexed_by clause.
@form(hashmap)
locus RegistryL {
    capacity { pool entries of CmdEntry; }  // missing indexed_by
}
```

The compiler rejects this at typecheck. The form annotation is
a contract; if the locus's shape doesn't satisfy it, the
program is ill-formed.

## When the philosophy bends

A few legitimate cases where the strict everything-is-a-locus
framing flexes:

### Free functions

Pure transformations from inputs to outputs (`fn parse_int(s)`,
`fn sha1(b)`) are functions, not loci. They have no identity,
no lifecycle, no position in the tower. Use them for stateless
operations. When a *coherent vocabulary* of free fns forms,
consider the namespace-locus pattern from the styleguide.

### Primitives

Int, Float, Bool, Decimal, Time, Duration, String, Bytes are
the atomic value layer. They compile to register / stack /
contiguous-buffer values, not to loci. You can put them in a
locus's capacity slot, params block, or fields; you can't write
`locus Int { ... }`.

### Generic type parameters

`T` in `locus L<T>` is a placeholder, not a locus. The
monomorphizer binds it at use sites.

### Seeds

Seeds are the directory-level unit of source organization. They
don't have a runtime locus representation. Don't try to write
`locus MySeedL { ... }` thinking it represents the seed —
seeds are a grouping over loci, not a super-locus.

These four exclusions are the only ones. Every other named
structural construct in your source code should be a locus.

## Locked-in decisions summary

For ready reference, the v1 commitments:

1. **Every named structural thing is a locus.** Types are
   loci-in-waiting. Functions, primitives, seeds, generic
   parameters are the only exclusions.
2. **`@form(<name>, <args>...)`** is the form annotation
   syntax.
3. **One form per locus.** Composition deferred to v2.
4. **The form synthesizes methods.** User adds extras on top;
   override deferred to v2.
5. **`indexed_by` on slot; tuning knobs as annotation args.**
6. **v1 forms: `vec`, `hashmap`, `ring_buffer`.** Others
   deferred until workload demands.
7. **Default lowering (no form) is literal F.22 pool/heap.**
8. **Perf gate: form-lowered locus within 10% of hand-written
   C equivalent.** Verified via microbench + app bench before
   adding more forms.
9. **Perspectives reflect on structure, not lowering.** Work
   uniformly across formed and unformed loci.
10. **No `Map<K, V>` / `Vec<T>` / `Option<T>` / `Result<T, E>`
    as parametric tagged types.** Collections are loci with
    forms; failure has a value-level addressing protocol
    (`fallible(T)`) that bridges to one structural mechanism
    (closure violation).
11. **Generics (m63) stay** as the orthogonal parametric
    mechanism on the locus declaration itself.
12. **`fallible(T)` is the value-level error protocol.**
    Stdlib fns that can fail return `fallible(T)` with a named
    payload type. Callers MUST address with `or raise` /
    `or <expr>` / `or handler(err)` / `match`. The runtime
    still observes exactly one failure mechanism: closure
    violation. No `panic()` / `assert()` / `?` / `unwrap()`.

## Cross-references

- [The Aperio styleguide](./aperio-styleguide.md) — idiomatic
  code patterns within this philosophy.
- [App-dev brief](./app-dev-brief.md) — what Aperio is and how
  not to hallucinate Rust at it.
- [proto-locus design note](../proto-locus-design.md) — the
  pre-implementation design conversation that produced this
  philosophy. Captures the open questions and the perf-gate
  framing.
- [spec/design-rationale.md](../../spec/design-rationale.md) —
  The Design's primitives + mechanics that this philosophy
  realizes.
- [spec/types.md](../../spec/types.md), [spec/semantics.md](../../spec/semantics.md) —
  the formal language spec.

## Revision discipline

This document is the source of truth for the everything-is-a-
locus philosophy. Future design decisions that affect:

- The locus gradient
- The form annotation surface or contract
- The locked-in decisions above

…require updating this document **before** the implementation
changes land. The note is canonical; drift between this
document and the implementation must be flagged in PRs.

When the philosophy genuinely needs to evolve (e.g. user-
defined forms in a future release, or override semantics for
form-synthesized methods), update the relevant section here
first, then move the implementation.
