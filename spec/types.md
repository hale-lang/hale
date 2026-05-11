# Type system

This document specifies lotus's type system: what types exist,
how they relate, what the compiler verifies. Where the grammar
(`grammar.ebnf`) tells you what's syntactically valid, this
document tells you what's *meaningfully* valid.

## Primitive types

| Type | Repr (v0 codegen) | Notes |
|---|---|---|
| `Int` | i64 | Signed; default for integer literals |
| `Uint` | i64 | Unsigned at the type level; codegen lowers as i64. Parser-recognized; full lowering pending a workload that exercises the unsigned-arithmetic distinction. |
| `Float` | f64 | IEEE 754 double; default for float literals |
| `Decimal` | i128 | Mantissa with implicit scale 9 (`mantissa × 10^-9`). Distinct from `Float` at the type level; same-shape arithmetic with scale-adjusted mul/div. Suffix `d` on literals (`1.50d`). Real arbitrary-precision deferred. |
| `String` | ptr (NUL-terminated) | UTF-8 bytes, C-style NUL-terminated. Single-pointer ABI to fit return-by-value through the m49 calling convention. Embedded NUL truncates — use `Bytes` for binary content. Allocated in the caller's arena (or the lazy global payload arena for stdlib returns whose lifetime needs to outlive the call). |
| `Bool` | i1 | `true` / `false` |
| `Time` | ptr (string-shaped, v0) | v0 codegen stores `Time` as a pointer to the literal's source-spelling String — a placeholder shape that the typechecker keeps distinct from `String`. Real `i64`-since-epoch lowering deferred. |
| `Duration` | i64 | Nanoseconds. Suffix literals (`5s`, `100ms`). |
| `Bytes` (m89) | ptr → `[i64 len][u8 data[len]]` | Binary-safe. Single-pointer ABI like String, but the underlying blob carries an explicit length prefix so embedded NUL bytes don't truncate. `len(b)` reads the prefix. Operations: `std::io::fs::read_bytes`, `Stream.send_bytes`. Distinct from `String` at the type level; the typechecker keeps them apart. |

**FnPtr (m80):** First-class function values, type-spelled
`fn(T1, T2) -> R` (or `fn(T1, T2)` for void-returning). LLVM
lowering is `ptr` (raw fn pointer); calls go through
`build_indirect_call` with an LLVM `FunctionType` synthesized
from the FnPtr's args/ret at the call site. The implicit
`__caller_arena: ptr` first param of every user fn (m49 calling
convention) is also expected on the FnPtr's call ABI —
indirect calls prepend it before user-visible args. See
`stdlib/io_tcp.ap` for the canonical use:
`Listener.on_connection: fn(std::io::tcp::Stream)`.

## Compound types

| Construct | Form | Notes |
|---|---|---|
| Slice / array | `[T]` or `[T; N]` | Dynamic or fixed-size |
| Tuple | `(A, B, C)` | Fixed-size heterogeneous |
| Struct | `type Foo { x: int; y: int; }` | Named record |
| Enum | `type Foo = enum { A, B(int) };` | Tagged union (sum type) |
| Function | `fn(A, B) -> C` | First-class function values |
| Generic | `Foo<T>` | Parametric over type T |

## Projection-class types

`Rich<T>`, `Chunked<T>`, `Recognition<T>` are **language-native
generic constructors**. The compiler recognizes them and selects
allocator + implementation strategy based on which projection-
class wrapper a value carries (per `memory.md`).

The constraint `<T: ProjectionClass>` (per F.2) is a built-in
"any-of-three" constraint: T must instantiate to `Rich<U>`,
`Chunked<U>`, or `Recognition<U>` for some U. No trait system
required.

```
fn process<P: ProjectionClass, T>(input: P<T>) -> P<U> { ... }
```

The compiler monomorphizes per concrete `P` instantiation.

## Locus types

A `locus L { ... }` declaration introduces a *locus type* L.
Locus types have:

- A set of **params** (name, type, default value or
  `: inferred`); these are also the locus's mutable state (per
  F.3 / §3 in design-rationale).
- Optional **contract** (expose / consume entries).
- Optional **lifecycle methods** (`birth`, `accept`, `run`,
  `drain`, `dissolve`, `on_failure`).
- Optional **mode declarations** (`bulk`, `harmonic`,
  `resolution`).
- Optional **bus interface** (subscribe, publish).
- Optional **closure tests**.
- Optional **member fns**.

Instantiating a locus type produces a **locus handle** of that
type, allocated as a region within the enclosing scope (per
`memory.md`).

## Perspective types

A `perspective P { ... }` declaration introduces a *perspective
type* P — a serializable parameter bundle within a shared
compiled-in schema. Used for fitter↔applier communication
(among other things). Has:

- Params (the parameter bundle)
- A `stable_when { ... }` block (commit predicate)
- Optional `serialize_as TypeV1` annotation

## Interface types (F.20)

An `interface I { fn ...; ... }` declaration introduces a
**structural interface type** I — a named set of method
signatures. A locus L satisfies I iff for every method in I, L
declares a method with the same name, the same arity, compatible
param types, and a compatible return type. Satisfaction is
**implicit**: there is no `impl I for L` declaration.

Interface types appear in fn parameter positions:

```
fn render(sink: Sink) {
    sink.line("hello");
}
```

The structural-impl check fires at every call site where a fn
declares an interface-typed param: missing-method, arity-
mismatch, param-type, or return-type mismatches all produce
typed diagnostics at typecheck time.

**v0.1 scope (Phase A + Phase B).** Interface declarations
parse, register, and the typechecker enforces the structural
rule (Phase A, shipped 2026-05-10). **Codegen vtable dispatch
(Phase B) is shipped 2026-05-11.** Interface values are fat
pointers `{data, vtable}` allocated in the current arena; the
data slot holds the underlying locus pointer (same single-ptr
ABI as `LocusRef`) and the vtable slot holds a per-(locus,
interface) static global of fn pointers indexed by interface-
method declaration order. A locus flowing into an interface
slot coerces at the call site; method calls on an interface
value lower as indirect calls through `vtable[i]` with the
data pointer passed as the implicit self arg.

Interface values are usable as fn parameters and as receivers
for method calls. The `std::text::Sink` migration (separate
StdoutSink / StringSink / FileSink loci behind one Sink
interface) is a stdlib follow-up: now unblocked, but ships in
a later milestone. Returning an interface value from a fn,
storing one in a locus param/field, or putting interfaces in
arrays/tuples is not yet supported — deep-copy across arena
boundaries for the fat pointer is a Phase B follow-up.

Interfaces have no default methods at v0; the body is signature-
only. No interface inheritance, no multi-interface bounds on
generics, no interface equality. F.21 sketches a paired
substrate-aware (cascading-dimension) interface form for the
n-dim growth case; not implemented at v0.

## Type compatibility

### Subtyping

Lotus is **invariant** at the type level — no implicit subtyping.
A `Rich<int>` is not assignable to a `Chunked<int>` even though
both wrap `int`; explicit conversion required.

Exception: contract-graded subtyping (next section).

### Contract compatibility

When parent declares `consume X: T` and child declares
`expose X: T`, the compiler requires:

- `X` is the same name in both.
- The child's type for `X` is a *subtype* of (or equal to) the
  parent's expected type.

For v0, "subtype" is just type equality. Future versions may
admit covariant / contravariant relationships; v0 is invariant.

This is the F.8 commitment expressed as a typing rule.

### Three-way interface (F.14)

Per F.14: any function injected by L into its arena that
satisfies a contract entry must return the contract's typed
surface. The compiler verifies, for each contract `expose X: T`
declaration:

- L has either (a) a param named `X` of type `T`, OR (b) a fn
  returning `T` named `X` (or an annotated impl), OR both.
- Multiple impls (the projection-class-specific case) are
  permitted; all must return T.

Default-implementation rule: if no fn is annotated, the param
named X is the default implementation (read field directly).

## Mutability

Per F.E (design-rationale): bindings are **immutable by
default**. `let x = 0;` produces an immutable binding.
`let mut x = 0;` produces a mutable binding; reassignment
permitted.

Mutability is a per-binding property, not a per-type property.
There's no `Mut<T>`; the binding either is or isn't `mut`.

For `params` / locus state, the implicit rule is that fields
are mutable through `self.x = ...` (per F.3). The locus's
state is the locus's mutable bundle.

## k_max as a typing rule

Per F.1 / F.3: the compiler computes
`k_max = B / [(1 - phi) * c + phi * sigma]` from the locus's
declared params. This determines the maximum coordinatees an
`accept()` can attach.

If params are constants (compile-time-known), k_max is a
compile-time integer. The compiler may reject `accept` call
sites that statically exceed k_max. (For dynamic params, the
runtime checks at each accept; exceeding k_max raises a
typed `KMaxExceeded` failure handled by the parent's
`on_failure`.)

## Generics

Generic params are declared with angle brackets:

```
fn map<T, U>(xs: [T], f: fn(T) -> U) -> [U] { ... }
type Stack<T> { items: [T]; }
```

The constraint syntax `<T: Constraint>` admits:

- `ProjectionClass` (built-in any-of-three; F.2)
- A specific projection class: `Rich`, `Chunked`, `Recognition`
- Concrete types: `<T: int>` is illegal (use the type directly);
  `<T: SomeStruct>` is also illegal (no trait system in v0)

V0 supports only projection-class constraints. Future versions
may add traits.

Monomorphization: the compiler emits one machine-code instance
per concrete generic instantiation (per F.1 commitment to
runtime perf over compile-time perf). Compile times grow with
generic surface; runtime is full-speed.

## Type inference

### `let` bindings

The type of `let x = expr;` is inferred from `expr`. Explicit
annotation `let x: T = expr;` overrides inference; if `expr`'s
type is incompatible with `T`, compile error.

### Function return types

If a fn omits `-> T`, the return type defaults to `()` (unit).
Explicit `-> T` is required for any non-unit return.

### Locus params

Params must declare types explicitly. `params { x: int = 0; }`
is the full form. (Inference of param types from defaults is
not supported in v0; explicit is preferred for the `inferred`-vs-
`= default` distinction.)

## `inferred` params

Per F.3: a param declared `: inferred` (instead of `= value`)
indicates the compiler / runtime determines the value, not the
author. The compiler treats:

- Hand-declared `= value` → prior, fixed at compile time.
- `: inferred` → unknown; resolved at compile time if possible
  (constant propagation), at runtime otherwise (via the
  perspective-stability machinery).

Typing-wise, inferred params have the declared type; they're
just not bound to a value at declaration time.

## Function types

Functions are first-class values. `fn(A, B) -> C` is a type;
function literals can be assigned, passed, returned.

A locus's `fn` member can be a method (takes `self`) or a free
function within the locus's scope. Lifecycle methods (`birth`,
`accept`, etc.) are not regular `fn`s — they have their own
syntax and don't take `self` (it's implicit).

## Contract subsumption

For two contracts `C1` and `C2`, `C1 ⊆ C2` iff every entry in
`C1` has a matching (compatible) entry in `C2`. This is used
for:

- Parent-child compatibility: parent's `consume` ⊆ child's
  `expose`.
- Locus-type substitutability: when a child locus type is
  expected, any locus type with a compatible expose-surface
  may substitute.

## Vertical-only flow as a typing rule

Per F.6 / F.11 / `memory.md`: cross-locus references at the
type level are limited to the contract's typed surface.
Specifically:

- A reference to a coordinatee accessible only via
  `self.children[i].x` where `x` is in the contract.
- Sibling references: not typeable. No syntax exists for
  reaching from one sibling to another; if attempted via
  manual pointer construction, the compiler rejects on
  region-lifetime grounds.
- Grandparent references from a child: not typeable. Failure
  flows through `bubble`; intent flows through the parent.

This makes the framework's vertical-only commitment a
type-system invariant, not just a convention.

## Closure-test typing

A `closure name { left ~~ right within tolerance; ... }`
declaration types as:

- `left` and `right` must be expressions of compatible
  numeric types (integer or numeric).
- `tolerance` must be a non-negative numeric expression.
- The compiler verifies left/right resolve in the closure's
  scope (which is the locus's scope; `self.x` is permitted,
  member fns may be called).
- For `epoch tick` / `epoch duration(d)` / `epoch dissolve`
  / `epoch birth` / `epoch explicit`, the runtime evaluates
  the closure at the appropriate event boundary.

A closure failure at evaluation produces a typed
`ClosureViolation` event (per F.9), not a generic error.

## Recovery-primitive typing

Recovery primitives (`restart`, `restart_in_place`,
`quarantine`, `reorganize`, `bubble`, `dissolve`, `drain`)
are statement-level keywords (per `precedence.md`); they
don't have types in the value sense. They take a locus handle
or error value as argument:

```
restart(child);
quarantine(child) for 30s;
bubble(err);
```

The compiler verifies the argument is a valid handle / error
in the current scope.

## What's deferred

Per `notes/open-questions.md` and design-rationale §16:

- **Trait system.** No `trait` keyword in v0 (reserved). Generic
  constraints limited to projection class + `Numeric`. The
  structural `interface` form (F.20) ships as the v0 interface
  mechanism; full traits with `impl I for L` declarations and
  generic bounds remain deferred.
- **Refinement types** (e.g., `int where x > 0`). Deferred.
- **Effect / capability system.** Substrate-derivation tracking
  is currently runtime-enforced via closure tests; future
  version may move into type system as effects.
- **Async / await.** Reserved keywords; no v0 typing.
- **Macros.** Reserved keyword; no v0 typing.
- **Sum-type-typed `self.children`** for multi-accept-type loci.
  v0 is single-accept-type only (F.11).
- **Projection-class-annotated translation impls** (per F.14
  follow-on). Deferred until forced by an example.

## Verification responsibilities

Where each typing rule lives in the compiler pipeline:

- **Parse + AST construction**: grammar.ebnf rules.
- **Name resolution**: identifier scopes, qualified-name lookup.
- **Type inference**: let-binding types, fn return types when
  explicit `-> T` omitted.
- **Type checking**: assignment compatibility, function call
  signature, generic instantiation, projection-class
  constraints.
- **Locus-shape checking**: contract compatibility (F.8 / F.14),
  k_max bounds (F.1 / runtime-checked when dynamic), mode
  signature consistency, lifecycle method signatures.
- **Region-lifetime checking** (compile-time): no escape from
  shorter-lived to longer-lived scope; no sibling references.
- **Closure-cycle existence check**: closure assertions
  reference defined values in the closure's scope.

The Phase 1 compiler in Rust implements these checks; the
Phase 6 self-hosted compiler ports them.
