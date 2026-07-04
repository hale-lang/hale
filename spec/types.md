# Type system

This document specifies Hale's type system: what types exist,
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
| `BytesView` (F.30, 2026-05-20) | `lotus_view_t { src: ptr, epoch: i64 }` (16 bytes, by-value; SysV AMD64 returns in `rax`/`rdx`) | Non-owning view over a `BytesBuilder`'s buffer. Returned by `BytesBuilder.view()`. `src` is the builder pointer; `epoch` snapshots the builder's `mutation_epoch`. The underlying Bytes-shaped data pointer (`buf - 8`) is *recomputed* at read time by `lotus_bytes_view_data`, so the view itself doesn't allocate or carry the data pointer. Coerces implicitly to `Bytes` at function-argument READ positions (e.g. `std::bytes::at(view, i)`, `len(view)`, user-defined fallible-fn args, self/external/interface method args, monomorphized-generic args); codegen emits a call to `lotus_bytes_view_data` which checks the stamped epoch against the builder's live epoch, panics on mismatch (F.30b mutation-while-view-live guard), and returns the recomputed data ptr on the OK path. Rejected at `Bytes`-typed storage sites — callers wanting owned storage must `std::bytes::clone(view)` for a deep-copy into the caller's arena. Storage typed `BytesView` is allowed; a `String` / `Bytes` literal at a `BytesView`/`StringView` storage default is wrapped via `lotus_view_from_static_data` with `epoch == LOTUS_VIEW_EPOCH_STATIC = -1` — the unpack helper sees the static sentinel and returns `src` directly without an epoch check (the literal lives in the global string table at program-lifetime, so there's no source builder to check against). 2026-05-22 PM: ABI compacted from a 24-byte heap-allocated struct to this 16-byte by-value shape; no arena allocation per `view()` call. |
| `StringView` (F.30, 2026-05-20) | `lotus_view_t { src: ptr, epoch: i64 }` (16 bytes, by-value) | Non-owning view over a `BytesBuilder`'s NUL-terminated buffer. Returned by `BytesBuilder.text_view()`. Symmetric companion to `BytesView`: same layout, same epoch guard, same static sentinel. Coerces to `String` at read sites via `lotus_str_view_data` (which recomputes the C-string pointer as `b->buf` — the `buf[len] == '\0'` invariant maintained by every mutating op makes this well-formed); rejected at `String`-typed storage; `std::str::clone(view)` upgrades to owned. |
| `BytesMut` (#3, 2026-06-13) | raw `{ptr, len}` window (by-value; no `[i64 len]` prefix) | A non-owning **raw writable/readable window** — distinct from `Bytes` (which carries a length prefix the handle points *into*) because `BytesMut` is a bare `{ptr, len}` pair over memory owned elsewhere. Handed out by the zero-copy ring producer (`Topic.write(max) { w => … }` binds `w: BytesMut` over the reserved slot) and by `std::io::MirrorRing` (`readable()` / `writable()` return a `BytesMut` over the live / free region). Read it zero-copy with the `_raw` siblings of the binary-pack family (`std::bytes::read_*` / `at` / `find_byte` accept a `BytesMut` directly — length is the window length, not a prefix); write into it with the binary-pack writers (`std::bytes::write_*`). The window is valid only until the next ring commit / mirror advance — no epoch guard, so the lifetime discipline is the caller's. |

**FnPtr (m80):** First-class function values, type-spelled
`fn(T1, T2) -> R` (or `fn(T1, T2)` for void-returning). LLVM
lowering is `ptr` (raw fn pointer); calls go through
`build_indirect_call` with an LLVM `FunctionType` synthesized
from the FnPtr's args/ret at the call site. The implicit
`__caller_arena: ptr` first param of every user fn (m49 calling
convention) is also expected on the FnPtr's call ABI —
indirect calls prepend it before user-visible args. See
`stdlib/io_tcp.hl` for the canonical use:
`Listener.on_connection: fn(std::io::tcp::Stream)`.

**FFI-portable subset (Stage-1 FFI, 2026-05-22):** the primitive
type set above carries an additional axis of distinction at the
`@ffi("c")` boundary: which types have a stable C-ABI mapping.
`Int` / `Float` / `Bool` / `Duration` / `Time` / `String` /
`Bytes` / `BytesView` / `StringView` may appear in `@ffi`
parameter and return positions; `Decimal` / `Uint` are
typecheck-rejected (platform-variable ABI or Hale-internal).
See [`spec/ffi.md`](./ffi.md) for the full marshalling table and
the lifetime contract.

## Compound types

| Construct | Form | Notes |
|---|---|---|
| Slice / array | `[T]` or `[T; N]` | Dynamic or fixed-size |
| Bounded collection | `bounded[T; N]` | Fixed-capacity counted list, INLINE in its containing type/params (`{ i64 len, [N x T] }`). See § "bounded[T; N]" below. |
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
| Slot-of-origin tracking (v1.x-5) | `Cell<T>` carries both T AND the originating `(locus, slot)` pair. Releasing a cell into a different slot than it came from is a hard error at codegen, with a diagnostic naming the originating slot. |

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
`spec/stdlib.md` and `crates/hale-codegen/tests/sink_polymorphism.rs`.

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
- **let-binding ascription with composite type (G20,
  2026-05-23).** `let arr: [Greeter; 2] = [Hi {}, Hey {}];`
  coerces each element through the ascription's element type;
  same shape for `let pair: (Greeter, Greeter) = (Hi {}, Hey
  {});` and `let arr: [Greeter; 3] = [Hi {}; 3];`. The codegen
  routes the RHS through `lower_expr_into(expr, hint)` so the
  composite-element coerce fires per position. The single-
  element let-ascription case is still permissive-via-Unknown
  (interface names resolve to `Ty::Unknown` at typecheck per
  `collect_known_names`); composite-element coercion picks up
  where the single case's implicit handling leaves off.

The return path uses two cooperating mechanisms: at the return
site, an implicit locus → interface coercion builds the fat
pointer, and the locus-instantiation routing extension (the same
m90 shape that handles `-> LocusRef(L)` returns) routes any
instantiation of a satisfying locus inside an `-> Interface(I)`
fn body to the program-lifetime payload arena. The fat-pointer
struct itself is then deep-copied into the caller's arena by
`emit_return_value_deep_copy`. Single-element coverage is in
`crates/hale-codegen/tests/interface_return.rs`.

**Composite-construction coercion (G20, 2026-05-23).** Interface
elements inside fixed-size arrays, array-repeat literals, and
tuples are now coerced at the construction site when the
destination type is known. The let-RHS with a composite
ascription is the wired entry point:

```hale
let arr:  [Greeter; 2]    = [Hi { }, Hey { }];
let arr3: [Greeter; 3]    = [Hi { }; 3];
let pair: (Greeter, Greeter) = (Hi { }, Hey { });
```

The codegen propagates the ascription's element type through
`lower_expr_into(expr, hint)` so per-position
`coerce_to_interface` fires before the array's "mixes element
types" check would otherwise reject heterogeneous LocusRefs.
Tests live in
`crates/hale-codegen/tests/interface_in_composites.rs`.

Once the let-RHS coerces, the array's static type is already
`Array<Interface, N>` (or the tuple's positional types are
`Interface`), so flowing it to fn-args, struct fields, or
return positions of the same type goes through plain type
matching — no further coercion is needed at the consumer
side.

**Still deferred:** locus-routing across nested return positions
— a fn declared `-> [Greeter; N]` instantiating loci inline in
its return expression still aliases the fn's stack frame.
Closing that needs the m90 routing extension to fire on nested
locus instantiations inside composite returns, the same gap that
governs tuple-of-`LocusRef` escape today.

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

#### Explicit numeric conversions

Where the implicit widening above does not apply — most often a
`Float → Int` narrowing, or an `Int → Float` conversion needed in
the middle of an expression rather than at one of the coercion
surfaces — there are two explicit forms, both round-toward-zero
for the narrowing direction (LLVM `fptosi` / `sitofp`):

- **The `Int(x)` / `Float(x)` casts** — the idiomatic in-language
  form. `Int(f)` narrows a `Float` to an `Int` (truncates toward
  zero); the cast is opt-in, so there is no silent `Float → Int`.
- **`std::math::int_to_float(i: Int) -> Float` and
  `std::math::float_to_int(f: Float) -> Int`** (WS3.1, 2026-06-11)
  — the named-function spelling, callable in any expression
  position. Semantically identical to the casts (`sitofp` /
  `fptosi`, round-toward-zero); provided so numeric code does not
  have to round-trip through ASCII (`to_string` + `parse_*`) and
  so the conversions sit alongside the other `std::math` numeric
  primitives. An already-correct-typed argument passes through
  unchanged.
- **`std::math::round(f: Float) -> Int` and
  `std::math::trunc(f: Float) -> Int`** — the Float→Int
  conversions that round at a chosen mode. `trunc` is round-
  toward-zero (an alias of `float_to_int`); `round` is round-
  half-away-from-zero (`3.7 → 4`, `2.5 → 3`, `-2.5 → -3`),
  computed as `fptosi(f + copysign(0.5, f))` via a compare/select
  half-shift (no `llvm.round` intrinsic, so the path needs no
  libm libcall — it lowers host-free on `wasm32`). `round` is the
  spelling numeric code wants when building an integer field from
  a Float quantity; `Int(f)` / `float_to_int` / `trunc` all
  truncate.

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

**Reassigning a locus-typed field is a lifecycle transition.**
A field that holds a child locus (`params { conn: WsClient =
WsClient { … }; }`) can be whole-value reassigned from a member
fn (`self.conn = WsClient { … }`). Because a locus is not a plain
value — it owns a region and possibly `@ffi`-acquired resources —
this is lowered as **dissolve-the-old + construct-the-new**, not a
pointer store: the previous instance is reclaimed (its `drain` /
`dissolve` run, releasing its resources) and the new instance is
constructed into the owning locus's arena, owned by the field (so
the parent's dissolve cascade reclaims it). The field always
points at a fully-live instance. (Before this rule the new
instance was a scope-bound temporary dissolved at the method's
exit — a use-after-free; see WS1#4.) For "same instance,
reconfigure," prefer **in-place mutation** (`self.conn.url = …`),
which keeps the locus's identity and resources and is cheaper.
v1 limitation: the reassigned instance inherits the owner's pool;
reassigning a field with an explicit non-default `placement` does
not re-apply that placement.

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

## bounded[T; N] — fixed-capacity collections in types (2026-07-02)

Types are pure data, so they cannot hold a `@form(vec)` (a locus).
`bounded[T; N]` is the type-level collection: a fixed-capacity
counted buffer laid out inline as `{ i64 len, [N x T] }` — the
capacity is part of the type (K made value-level, the F.22
philosophy). Works in `type` fields and locus `params`.

Operations are GRAMMAR INTRINSICS (like `len(s)`), not methods, so
the types-have-no-methods axiom holds:

```hale
push(f, x)       -> ()  fallible(CapacityError)  // full = error;
                                                 // displacement policy
                                                 // lives in the or-arm
at(f, i)         -> T   fallible(IndexError)
set(f, i, x)     -> ()  fallible(IndexError)     // overwrite live slot
count(f)         -> Int
clear(f)                                          // len = 0
truncate(f, n)   -> Int                           // len = clamp; returns it
for x in f { }                                    // iterate live slots
```

Semantics:
- Fields auto-initialize EMPTY. Literal init and whole-field
  assignment are rejected — the intrinsics are the only mutation
  surface. Whole-STRUCT copies carry elements + count by
  construction (the storage is inline).
- Scalar elements (Int/Float/Bool/Decimal/Duration) are flat under
  `zero_copy` and travel the bus as raw bytes. Pointer-shaped
  elements (String/Bytes/struct) work in-process — push/set
  arena-anchor the element into the receiver's owning arena — but
  are rejected in cross-process bus payloads (post-v1 polish).
- Drop-front/FIFO is the shift-left idiom: `set` live slots down,
  then `truncate`.
- `CapacityError { cap: Int; count: Int }` and the shared
  `IndexError` are the injected error shapes.
- The unbounded-alloc analysis treats bounded fields as bounded by
  construction. `@form(vec)` remains the unbounded, locus-owned
  collection: unbounded data lives on a locus; bounded data can
  live in a type.

## Generics

**Generic type-expr ↔ monomorph unification (2026-07-02):** a
generic instantiation type-expr (`Box<Int>`) resolves at typecheck
to its mangled monomorph name (`Box_Int`) — the same name codegen
synthesizes and that `Box_Int { ... }` literals produce — so
declarations and literals unify, and a `Box_String` literal in a
`Box<Int>` slot is a caught mismatch. Monomorph literal fields
validate against the template with the type args substituted, and
field reads on monomorph values type as the substituted field.

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

## Single-threaded-method invariant (F.31)

A locus's methods may be invoked only on the OS thread that
owns the locus's placement's pool. Cross-pool direct calls
are typecheck errors; cross-pool coordination goes through
the bus.

The invariant is enforced via a static call-graph walk seeded
from the `main locus`'s `placement { }` entries:

1. Each main-locus `params` field has a pool — explicit
   (`placement { field: cooperative(pool = X); }` or
   `placement { field: pinned; }`) or default
   (`cooperative(pool = main)`).
2. Each nested locus inherits its containing tower's pool
   (see `spec/semantics.md` § "Nested instantiation"). Methods
   on a nested locus run on the parent's pool's thread.
3. For each method-call expression `recv.foo(args)`, the
   typechecker determines `recv`'s pool from its static type
   and the surrounding pool context.
4. If `recv`'s pool differs from the caller's pool, the call
   is rejected with a diagnostic naming both pools and
   pointing at the `placement { }` entries that picked them.
5. Bus sends (`Topic <- v;` / `"subj" <- v;`) are unrestricted
   — the runtime's cross-thread dispatch (m28b condvar+memcpy)
   handles the boundary safely.

The invariant is the substrate enforcement that makes M:N
cooperative pools safe. Without it, multi-pool deployments
would silently race on locus arenas (unsynchronized bump
allocators by design). The typecheck happens once per main
locus (the placement-bearing locus is unique per binary), so
the rule applies at binary compile time rather than at every
library typecheck.

**Interaction with `LocusRef` borrows.** A locus param of
type `LocusRef(L)` carries a borrow of an `L` — but the
borrow's pool is the locus's own placement, not the borrower's
placement. So `self.db.query(...)` where `self.db: DB` and
the `DB` instance is on pool `db_pool` is a cross-pool call
from any non-`db_pool` thread, and routed through the bus.
This is the "vertical-only flow" rule generalized to
cooperative pools: cross-pool access is the same shape as
sibling access — bus only.

**Interaction with builtins / stdlib.** Free functions and
stdlib path-calls (`std::io::fs::read_file`, `std::str::*`,
etc.) are pool-neutral — they run on whichever thread calls
them. Their arena routing through `lotus_current_caller_arena`
TLS handles the per-thread isolation. The single-threaded-
method invariant applies only to locus member functions,
which are what carry per-locus arena state.

**Interaction with `@form(...)` loci.** A locus declared
with a `@form(...)` annotation (`@form(hashmap)`,
`@form(vec)`, `@form(ring_buffer)`) is **single-pool by
default** — its methods participate in the same single-
threaded-method invariant as any other locus. Plain
`@form(...)` cells have no runtime synchronization;
concurrent writers from different pools corrupt the
underlying structure.

Cross-pool access is opt-in via the `sync = ` kwarg on
the form annotation:

| Annotation | Discipline | Trade-off |
|---|---|---|
| `@form(hashmap)` | single-pool only | densest layout, no sync overhead, cross-pool calls rejected |
| `@form(hashmap, sync = serialized)` | per-map mutex (F.32-1α) | correct cross-pool; throughput bounded by lock contention |
| `@form(hashmap, sync = striped)` | cell-level CAS + per-map rwlock for grow + cache-padded cells (F.32-1β2-v2) | parallel writers; grow path serializes; rwlock overhead can outweigh parallelism on cheap-payload workloads |
| `@form(hashmap, sync = lockfree, cap = N)` | fixed-cap, cell-level CAS, no rwlock or mutex (F.32-1γ-v1) | highest measured throughput on the false-sharing bench; no grow, no remove in v1 |

When a locus carries a recognized sync discipline, cross-
pool method calls into it are accepted without diagnostic —
the substrate's chosen discipline carries the safety
contract. Plain `@form(...)` (no sync kwarg) gets the same
cross-pool diagnostic as any other locus, extended with an
upgrade-path hint naming the sync kwargs.

Concrete shape: a `Registry @form(hashmap, sync = striped)
of Counter indexed_by name` shared across producer pools
(gateway loci incrementing counters) and a consumer pool
(`MetricsEndpoint` rendering Prometheus text) typechecks
clean. Without the `sync = striped`, every cross-pool
`self.registry.counter(...)` would be rejected.

Non-form receiver loci have no sync discipline available;
cross-pool coordination must go through the bus.

**Inference.** F.32-1∞ adds a closed-world inference pass
that picks a `sync` default per form-bearing locus type
from the pool-propagation graph. The explicit annotation
always overrides; the inferred pick is shown via a
compile-time diagnostic at the decl site. See
`notes/f32-cache-aware-delivery-plan.md` § F.32-1∞.

**History.** Commit `3ec6391` (2026-05-24, first cut)
admitted any `@form(...)` locus into the cross-pool-safe
set unconditionally, on the strength of a not-yet-shipped
"form ABI serializes" claim. Bench-prep for F.32-1
surfaced that the runtime had no synchronization on the
form paths; F.32-0 (this section's current state) scopes
the exemption to explicit opt-in via `sync = X`. See
`notes/f32-cache-aware-delivery-plan.md` § F.32-0.

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
`spec/design-rationale.md` for the design rationale.

**Declaration sites are restricted by the two-channel rule
(see `spec/semantics.md` § "Fallible call semantics"
§ "Where each channel lives").** `fallible(E)` may be
declared on:

- Free fns.
- Stdlib-synthesized methods over `@form(...)` containers
  (`@form(vec).get` / `.pop`, `@form(hashmap).get` / `.remove` /
  `.key_at` / `.entry_at`, `@form(ring_buffer).pop`).
- **User-declared `fn` member fns on a locus** (open-question
  #24, shipped 2026-05-25). Heap-bearing success and err
  payload types are supported via the same TLS caller-arena
  snapshot non-fallible heap-returning locus methods use.

`fallible(E)` is **rejected** on substrate-facing surfaces
that have no caller frame to address the error channel:

- **Lifecycle methods** (`birth` / `run` / `accept` / `drain` /
  `dissolve` / `on_failure`). Physically rejected at the AST
  level — `LifecycleDecl` doesn't carry a `fallible` field.
- **Mode methods** (`bulk` / `harmonic` / `resolution`).
  Same shape as lifecycle: AST doesn't carry a `fallible`
  field.
- **Closure assertions.** Substrate evaluates the assertion at
  the epoch boundary; no caller frame exists in the expression.
- **Bus-subscribed handlers.** A fn that's declared
  `fallible(E)` may not also be referenced by a `bus
  subscribe ... as <fn>` declaration. Bus dispatch has no
  return path. Rejected at the subscribe site, not the fn
  decl (one fn may be referenced by zero subscriptions; the
  subscription is what fails to typecheck).

The narrowing from "no fallible on locus methods" to
"substrate-facing surfaces only" preserves the two-channel
separation (structural failures still flow vertically via
closure violations + `on_failure`) while removing the
friction that made devs extract free fns just to get a value-
error channel back. The typechecker emits the diagnostic at
the offending site (locus member decl for lifecycle / mode;
subscribe site for bus-handler-fallible conflict).

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

  **Fallible handlers (2026-07-02):** the handler may itself be
  `fallible(E2)`. Its success value substitutes; its FAILURE
  propagates through the ENCLOSING fn's error path — implicit
  `or raise`, sugar for the already-legal nested spelling
  `call() or (handler(err) or raise)`. E2 must be assignable to
  the enclosing fn's declared payload ("handler's failure has
  nowhere to go" / "propagated payload must match" otherwise).
  User free fns, imported-path fns, and locus member fns are
  classified; `@form`-synthesized methods and stdlib path-calls
  still need the explicit nested spelling. In statement position
  the substituted value is discarded, so the handler's success
  type needn't match the call's.

  The fallback may be a **`{ block }`** — `or { … }`, with `err`
  in scope — for multi-statement recovery. Two cases:

  - A block that **always diverges** (`return` / `fail` on every
    path) produces no substitute value, so it imposes no
    constraint on T and is accepted for **any** success type —
    `let s = read_file(p) or { return "fallback"; };` where the
    fallible's success type is `String`, `Bytes`, a struct, etc.
    (It disposes like `or raise`: the err branch is closed and
    only the success value reaches the continuation.)
  - A non-diverging block substitutes its **tail expression** as
    the fallback value, whose type must be assignable to T:
    `let s = read_file(p) or { log(err); "default" };`.

  On a **Unit-success** fallible (`() fallible(E)`, e.g.
  `std::io::fs::write_file`), `or { block }` runs the block for
  effect — including in **statement position**:
  `write_file(p, s) or { println("failed"); };` — the same as
  `or raise` / `or discard` there.

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

## Working-set estimator (F.32-2)

F.32-2 ships a compile-time working-set estimator that
projects each user-declared locus's approximate byte cost
and compares against a cache-tier budget. The estimator runs
post-typecheck, pre-codegen; it doesn't change codegen
output and emits diagnostics rather than changing program
behavior.

**Estimator formula** (per-locus, in bytes):

```
working_set(L) =
    sizeof(L's struct)                              [arena + user fields with alignment padding]
  + sum(slot in L's capacity slots) cap × cell_stride
  + sum(child in L's params if locus-typed) working_set(child)
```

Cache-tier budgets are read from
`/sys/devices/system/cpu/cpu0/cache/index{0,2,3}/size` on
Linux at first probe (cached for the build's lifetime);
static fallbacks 32 KB / 512 KB / 8 MB apply on non-Linux or
when sysfs is unavailable. See `hale_types::working_set` for
the engine.

**Per-locus annotation** (F.32-2 v0.2, 2026-05-25):

`@locality(L1)` / `@locality(L2)` / `@locality(L3)` declare
a per-locus cache-tier expectation. `@locality(any)`
explicitly opts the locus out of any global gate. The
annotation stacks with `@form(...)` in either order:

```hale
@form(hashmap, sync = lockfree, cap = 64)
@locality(L2)
locus Registry {
    capacity { pool entries of Entry indexed_by k; }
}
```

The grammar surface is in `grammar.ebnf`
§ `locality_annotation`.

**Build-flag surface** (CLI, on `hale build`):

| Flag | Effect |
|---|---|
| `--locality-report` | Emit a per-locus stderr report listing each locus's estimated bytes, smallest-fitting tier, and a struct / capacity / children byte decomposition. Build proceeds. |
| `--target-cache l1\|l2\|l3` | Evaluate each locus against the named tier's budget. Over-budget loci surface as a stderr warning by default. |
| `--strict` | Convert the warning into a build error (exit 1 before codegen). Only meaningful in combination with `--target-cache` or a program that carries `@locality(...)` annotations. |

**Effective budget precedence** (per locus):

1. `@locality(L1|L2|L3)` annotation → that tier (hard
   contract; evaluated regardless of CLI flag).
2. `@locality(any)` annotation → no budget (opts out even
   under `--target-cache`).
3. No annotation → falls through to `--target-cache` global
   tier (or no budget when the flag isn't set).

`--strict` controls warnings vs errors uniformly across both
sources. The diagnostic names which source applied (per the
`BudgetSource::label()` "@locality" / "--target-cache"
attribution).

**Estimator approximations** (deliberately imprecise; see
the working_set module doc for the full list):

- `params { }` field layout uses alignment-correct
  accumulation (each field rounded up to its natural
  alignment; struct rounded up to max field alignment). Was
  packed-layout in v0.1; v0.2 adds the padding.
- Arena overhead modeled as a flat 64-byte budget for
  synthetic headers.
- Heap-managed primitives (`String` / `Bytes` / `*View`)
  count as 16 bytes (ptr + len); the heap buffer's contents
  are not counted (lives in the locus's arena).
- Per-method scratch high-water mark — not modeled. The
  scratch arena is destroyed at method exit, so contents
  are transient, not resident.
- Unbounded-cap capacity slots (no `cap = N` on the form)
  surface in `WorkingSetEstimate::unbounded_slots` rather
  than contributing zero silently.

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
