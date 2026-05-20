# Type system

This document specifies Aperio's type system: what types exist,
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
| `Bytes` (m89) | ptr → `[i64 len][u8 data[len]]` | Binary-safe. Single-pointer ABI like String, but the underlying blob carries an explicit length prefix so embedded NUL bytes don't truncate. `len(b)` reads the prefix. Distinct from `String` at the type level; the typechecker keeps them apart. Operations: `std::io::fs::read_bytes` (m89), `Stream.send_bytes` (m89), `Stream.recv_bytes` (Phase 2g), `std::bytes::at` / `std::bytes::slice` / `std::bytes::from_string` (Phase 2g), `std::str::from_bytes` for the inverse direction (Phase 2g). |
| `BytesView` (F.30, 2026-05-20) | ptr → `lotus_view_t { data: ptr, builder: ptr, stamped_epoch: i64 }` (24 bytes, F.30b layout) | Non-owning view over a `BytesBuilder`'s buffer. Returned by `BytesBuilder.view()` — the C primitive allocates a `lotus_view_t` in the caller arena and stamps the builder's current `mutation_epoch`. Coerces implicitly to `Bytes` at function-argument READ positions (e.g. `std::bytes::at(view, i)`, `len(view)`, user-defined fallible-fn args, self/external/interface method args, monomorphized-generic args); codegen emits a call to `lotus_bytes_view_data` which checks the stamped epoch against the builder's live epoch, panics on mismatch (F.30b mutation-while-view-live guard, 2026-05-20), and returns the underlying Bytes-shaped data ptr on the OK path. Rejected at `Bytes`-typed storage sites — callers wanting owned storage must `std::bytes::clone(view)` for a deep-copy into the caller's arena. Storage typed `BytesView` is allowed; a `String` / `Bytes` literal at a `BytesView`/`StringView` storage default is wrapped in a `lotus_view_t` with `builder=NULL` (the unpack helper's NULL branch returns the data pointer without an epoch check — the literal lives in the global string table at program-lifetime, so no source builder to check against). |
| `StringView` (F.30, 2026-05-20) | ptr → `lotus_view_t { data: ptr, builder: ptr, stamped_epoch: i64 }` (24 bytes, F.30b layout; data field points to a NUL-terminated C-string) | Non-owning view over a `BytesBuilder`'s NUL-terminated buffer. Returned by `BytesBuilder.text_view()`. Symmetric companion to `BytesView`: coerces to `String` at read sites via `lotus_str_view_data` (same epoch-stamped F.30b guard); rejected at `String`-typed storage; `std::str::clone(view)` upgrades to owned. The underlying buffer is the builder's `buf` with the maintained invariant `buf[len] == '\0'`, so unpacked reads see a well-formed C-string. |

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
| Struct | `type Foo { x: Int; y: Int = 0; }` | Named record. Each field can declare a default value (`= expr`); literals omitting a defaulted field fill it from the default at instantiation time. |
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
- Optional **capacity slots** (F.22 — `pool X of T;` / `heap Y
  of T;` declarations naming slots 1..N beyond the implicit
  slot 0 / Arena).
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

## Capacity-slot cell handles (F.22)

`Cell<T>` is the value type returned by `acquire()` (Pool slots)
and `alloc()` (Heap slots), and accepted by `release(c)` /
`free(c)`. It is **not** user-spellable in source — there is no
`let x: Cell<Int> = ...;` syntax. The type appears only at
typecheck and codegen.

| Aspect | v1 behavior |
|---|---|
| LLVM repr | `ptr` — a typed pointer to T's struct layout |
| Element type | The boxed inner type carries T from the slot's `of T` declaration; `Cell<Int>` and `Cell<Float>` typecheck distinctly. |
| Validity surface | Round-trip only: a value can flow through `let`-bindings, get re-supplied to `release` / `free`, and live inside the locus body. |
| Forbidden ops | println, arithmetic, comparison, fn-return-boundary crossing. Each rejects with a focused build-time diagnostic. |
| Field access (v1.x-2) | Struct cells support `cell.field` reads and `cell.field = v` writes; lowers to struct GEP + load/store. Primitive cells (`Cell<Int>` etc.) reject field access with a focused diagnostic. |
| Slot-of-origin tracking (v1.x-5) | `Cell<T>` carries both T AND the originating `(locus, slot)` pair. Releasing a cell into a different slot than it came from is a hard error at codegen and the interpreter, with a diagnostic naming the originating slot. |

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

Interface values are usable as fn parameters, fn returns, locus
param / field values, and `@form(vec)` cell elements. Method-call
receivers, polymorphic return through control flow, and
pass-through aliasing (the original instantiator's binding and
the returned binding refer to the same underlying locus) all
work end-to-end. The `std::text::Sink` stdlib migration (split
`Sink` into `StdoutSink` / `StringSink` / `FileSink`
loci behind one `Sink` interface) shipped 2026-05-11; see
`spec/stdlib.md` and `crates/aperio-codegen/tests/sink_polymorphism.rs`.

The implicit LocusRef → Interface coercion fires at the
following positions:

- **Free-fn arguments.** `fn render(s: Sink)` accepts any
  LocusRef of a locus satisfying `Sink`.
- **Locus-method arguments.** `fn add(t: Tower)` on a `Registry`
  locus accepts the same coerce (added 2026-05-18).
- **Returns.** `return r;` from a fn declared
  `-> Sink` builds the fat pointer at the return site.
- **`type` field initializers.** `TowerEntry { t: r }` where
  field `t` is interface-typed coerces the LocusRef `r`
  (added 2026-05-18).
- **Locus `params` / field initializers.** Same shape as
  above for locus param defaults and `locus L { params { t:
  Tower; } }` slots.
- **`@form(vec)` cell `push`.** A `Registry @form(vec) of
  Tower` accepts pushes of any satisfying LocusRef.
- **`or <substitute>` fallback expressions.** When a fallible
  has success type `Interface(I)`, an `or fallback`
  expression of LocusRef type satisfying `I` coerces
  (added 2026-05-18). E.g. `lookup(...) or Hello { }`
  where `lookup` returns `Greeter fallible(...)`.

The return path uses two cooperating mechanisms: at the return
site, an implicit locus → interface coercion builds the fat
pointer, and the locus-instantiation routing extension (the same
m90 shape that handles `-> LocusRef(L)` returns) routes any
instantiation of a satisfying locus inside an `-> Interface(I)`
fn body to the program-lifetime payload arena. The fat-pointer
struct itself is then deep-copied into the caller's arena by
`emit_return_value_deep_copy`. Single-element coverage is in
`crates/aperio-codegen/tests/interface_return.rs`.

Interface elements inside tuples and fixed arrays remain gated
on the same broader composite-construction coercion design that
governs tuple-of-`LocusRef` escape — recursive coercion at
composite-construction sites plus locus-routing across nested
return positions. Deferred.

F.11 child acceptance (`accept(c: ConcreteLocus) { ... }`) is
intentionally NOT in the coerce list above. The child-accept
mechanism keys dispatch by exact concrete locus name across
substrate sites — accept fn signature, the `self.children`
storage layout, and the parent's accept-dispatch table — so
`accept(c: Iface)` is a multi-system change, not a coercion
wire-up. Single-accept-type per parent is the v1 design.

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

### Numeric coercion: Int → Float (Phase 2c)

A single documented one-way widening fires at the following
surfaces:

- **let-binding type ascription:** `let nf: Float = self.n;`
  where `self.n: Int` widens `n` to a Float at the binding
  site (codegen `sitofp`).
- **fn-arg coercion:** when a parameter is typed `Float` and
  the call-site argument is typed `Int`, the argument widens
  at the call site. Same rule applies to user-declared fns
  and to stdlib path-calls (`std::math::sqrt(n)` with `n: Int`
  works without `2.0` literals).
- **binary-op promotion** (B13 / G30, 2026-05-17): when exactly
  one side of a numeric binop is `Int` and the other `Float`,
  the `Int` side widens to `Float` and the op produces `Float`.
  Same rule covers comparison ops (`<`, `>=`, `==`) so
  `i < 0.5` typechecks. Symmetric — either side can be the
  one that widens.
- **user-type field init:** assigning an `Int` value into a
  `Float`-typed struct field at literal-init time widens at the
  store, mirroring fn-arg coercion. Lets a config bundle
  declare `timeout: Float` and accept an `Int` from the caller
  without sprinkling `Float(n)` casts.

The widening is **strictly one-way**. `Float → Int` narrowing
remains explicit (round + cast). `Decimal` never participates
in implicit cross-type conversion. The rule was added 2026-05-11
as part of the float-surface-gaps friction-log resolution; see
F.23 in `spec/design-rationale.md` and the Phase 2c entry in
`spec/stdlib.md`.

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

Params must declare types explicitly. `params { x: Int = 0; }`
is the full form. (Inference of param types from defaults is
not supported in v0; explicit is preferred for the `inferred`-vs-
`= default` distinction.)

Three init shapes (2026-05-16):

- `name: T = expr;` — default. Used when the caller omits the
  field; evaluated in the caller's scope at instantiation time.
- `name: T;` — **required**. The caller MUST supply the field at
  the locus literal site; instantiation without it is a compile
  error. Use for fields where no sensible default exists (e.g.
  `Server { handler: ... }` where the handler is the whole
  reason the locus exists).

  **Exception (B7 / G19, 2026-05-17):** if `T` is a user-defined
  `type` (struct) and every field of `T` has a declared default,
  the compiler synthesizes `T { }` as the param's default. So
  `params { cfg: Cfg; }` against `type Cfg { host: String =
  "localhost"; port: Int = 8080; }` works without `= Cfg { }`
  spelled out. Required-shape is preserved when any field on
  `T` lacks a default.
- `name: T : inferred;` — F.3 inference path; compiler /
  runtime determines the value.

**T may be another locus (B10 / G24, 2026-05-17).** A param
typed as a locus name (`params { db: DB; }`) stores a `LocusRef`
— a single-pointer borrow. The param-holding locus does **not**
own the referenced locus; the caller keeps it alive. Cross-decl
declaration order doesn't matter: a forward reference
(`User { db: DB; }` declared above `locus DB { ... }`) resolves
via the codegen-side `pending_locus_names` pre-pass.

Reading through the borrow (`self.db.name`, `self.db.draining`)
goes through the same field-access lowering as any other
LocusRef receiver. Synthetic fields (`self.db.k_max`,
`self.db.draining`) work on non-self receivers too (B14 / G31).

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

## Fallible typing (v1.x-FORM-1)

A function declared `-> T fallible(E)` produces a value of
type `T fallible(E)` at every call site. This type cannot be
used where a plain `T` is expected — the caller MUST address
the error before the value is consumable. See
`notes/agent-onboarding/aperio-design-philosophy.md` § 2 for
the design rationale.

**Declaration sites are restricted by the two-channel rule
(see `spec/semantics.md` § "Fallible call semantics"
§ "Where each channel lives").** `fallible(E)` may be
declared on free fns and on stdlib-synthesized methods over
`@form(...)` containers; it is **rejected** on user-declared
locus methods, which communicate failure structurally via the
closure-violation channel. The typechecker emits the
diagnostic at the locus method's declaration site.

### `Ty::Fallible { success, payload }`

The checker represents fallible returns as a wrapper around
the underlying success type:

| Source                       | Inferred type                       |
|------------------------------|-------------------------------------|
| `fn f() -> T fallible(E)`    | `f()` has type `Ty::Fallible { success: T, payload: E }` |
| `match` / `or` on fallible   | unwraps to `T`                      |

A `Ty::Fallible` is **not assignable** to its success type. It
must be unwrapped at the immediate call site. The checker
emits `error: error not addressed` at:

- `let v = f();` with `f` fallible
- `let v: T = f();` with `f` fallible (typed binding)
- `f();` as an expression statement
- `g(f())` — fallible passed as a non-fallible-typed arg
- assignment, return, condition positions

### Disposition operators (`or`)

`<expr> or raise`
: Propagate the error one frame up the static call stack.
  Evaluates the inner; on `FallibleErr` payload, re-enters
  the fallible-return shape of the enclosing `fallible(E)`
  fn (the error climbs the call stack until a frame
  addresses it). The value-error channel is value-level
  and **orthogonal** to the closure-violation channel; the
  `bubble` / `on_failure` machinery is not entered by
  default. (An application may later promote a value error
  to a closure violation explicitly; no such syntax exists
  in v1.) On success, passes the inner value through. The
  resulting expression's type is the success type T.

  Past every enclosing `fallible(E)` frame — at the implicit
  main locus's root boundary — the runtime panics via
  `lotus_root_panic`. See `spec/semantics.md` § "Process
  exit" for the boundary semantics.

`<expr> or <fallback>`
: Substitute a fallback value of type T. On failure, evaluates
  the fallback expression with `err` implicitly bound to the
  payload (typed as E). On success, passes through. Fallback
  type must be assignable to T. The fallback may itself be
  a call (`or handler(err)`), making `err` a regular
  expression-position binding inside the fallback.

The `or` operator is right-associative: `a() or b() or raise`
parses as `a() or (b() or raise)`, so each level disposes one
fallible in turn until a non-fallible value remains.

### `fail` statement

`fail <expr>;` is only valid inside a fallible fn body. It
evaluates `<expr>`, requires the result type to match the fn's
declared payload type E, and exits via the error path (the
caller sees a `FallibleErr` value).

### Custom payload types

The payload E is an ordinary type expression — usually a small
user-defined record (`type ParseError { ... }`) or a stdlib-
synthesized type. The runtime / typechecker does NOT impose a
common base — there is no `Error` trait, no `impl Error for
ParseError`. Failure is a single anonymous fact; the payload is
just a value tagged onto the failure for diagnostic purposes.

### Synthesized stdlib payload types

The resolver injects four fallible-payload types into the top
scope so user code can name them in `fallible(...)` markers and
`or` substitute clauses. All are idempotent — a user-declared
type with the same name wins.

| Trigger | Type | Fields |
|---|---|---|
| `@form(vec)` | `IndexError` | `kind: String`, `index: Int`, `len: Int` |
| `@form(hashmap)` | `KeyError` | `kind: String` (also surfaces `IndexError` on `key_at` / `entry_at` — those are index-based, added 2026-05-16) |
| `@form(ring_buffer)` | `EmptyError` | `kind: String` |
| `std::io::fs::*` / `std::io::tcp::*` | `IoError` | `kind: String`, `errno: Int`, `path: String` |

The `IoError` payload (2026-05-16) is the unified shape for the
fallible I/O surface — see `spec/stdlib.md` § "IoError" for the
errno → kind tag taxonomy.

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

- **Trait system.** No `trait` keyword in v0 (reserved). The
  structural `interface` form (F.20) ships as the v1 interface
  mechanism — both Phase A (typecheck) and Phase B (codegen
  vtable dispatch) landed 2026-05-11. Full traits with `impl I
  for L` declarations and generic bounds remain deferred.
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
