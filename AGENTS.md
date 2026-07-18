# AGENTS.md — Hale for agents writing `.hl` programs

You are writing programs in Hale. This file is the load-bearing
prompt: encoding first, then operational. Read it once before
you write any `.hl` code.

## The Design (recursive hypergraph)

Hale operationalizes **The Design** — a recursive hypergraph — at
the language substrate. Every rule about how Hale code is shaped
traces back here. Quote node / hyperedge / invariant IDs when
citing a rule (e.g. `H8`, `I4`).

```
%DESIGN v1

nodes:
  α   axiom                ; declared, irreducible
  Σ   system               ; α with 1→N decomposition
  Δ   dimension            ; projection of Σ's 1→N
  Π   perspective          ; choice of Δ
  K   capacity             ; bound on Σ
  D   displacement         ; K-full + new → drop-least
  ↑   failure-up           ; D insufficient → Σ.parent
  ⊥   root-as-boundary     ; recursion stops at horizon
  ∥   vertical-only        ; edges ⊆ {parent↔child}
  ⋈   multi-DAG-projection ; substrate-DAGs join via form

hyperedges:                              ; arity ≥ 2
  H1  unfold     (α, Σ, [α₁..αₙ])        ; α-as-Σ has children
  H2  recurse    (Σ, αᵢ)                 ; each child read-as-α
  H3  compose-Δ  (Δ₁, Δ₂) → Δ₃           ; ⊗-closed
  H4  observe    (Π, Δ)                  ; perspective picks Δ
  H5  depth-Δ    (Π, Δd)                 ; cross-depth Π IS a Δ
  H6  bound      (Σ, K)
  H7  displace   (Σ, K-full, new) → kept ; drop reveals priority
  H8  bubble     (Σ, ↑) → Σ.parent.on_fail
  H9  vertical   ∀ edge ∈ Σ-tree : edge ∈ ∥
  H10 lateral-Δ  compose-Δ is licit-lateral-at-Δ-layer only
  H11 root       (Σ.root, ⊥) = current-observable-horizon
  H12 DAG-join   (⋈, DAGᵢ) via form

invariants:
  I1 form         : portable across deployment substrates
  I2 params       : substrate-local
  I3 pyramid      : ∀ d ∈ depths(Σ-tree), ∃ stability-tuple_d
  I4 MS2          : ∀ q ∈ model, q ∈ exactly-one-Σ-tower
                    floating-q ⟹ modeling-error
  I5 form-content : form claims are perspective-invariant;
                    content reduction claims are perspective-conditioned

hale ≜ operationalization(DESIGN, substrate=language)
map:
  locus               ↔ Σ
  type                ↔ Σ-proto (no lifecycle/flow)
  contract            ↔ Π@depth-edge
  expose | consume    ↔ Π↑ | Π↓
  capacity{pool,heap} ↔ K-tuple, slot-0 implicit Arena
  on_failure          ↔ ↑
  drain (cascade)     ↔ ∥ depth-first
  projection class    ↔ K-conditioned Π-resolution (rich|chunked|recognition)
  bus                 ↔ Δ-composed channel, ⋈ when bound to transport
  closure-test        ↔ I3 local check at Σ
  perspective T       ↔ Π serialized across processes
  fallible(E)         ↔ value-channel Π↑ (orthogonal to ↑)
  @form(...)          ↔ K-discipline lowering, application-layer Σ

hale.root: ⊥(language-graph) = DESIGN itself
```

## The locus axiom

Everything named and structural is a **locus** (Σ). If it has
lifecycle, contracts, bus participation, modes, closures,
capacity slots, or projection class, it is a fully-grown
locus. If it is pure data — record, returnable by value, no
flow — it is a **type**, a locus still in proto-form. There is
no third primitive at the structural layer. By `I4` (MS2),
every named quantity must be assignable to exactly one locus
in one locus tower; floating quantities signal modeling error,
not framework gap.

## The pattern catalog

Seven shapes. Every well-written `.hl` program matches one. If
something doesn't fit, reconsider against the catalog before
inventing. (`spec/styleguide.md` is the unified guide — shapes,
correctness rules, hot-path speed rules, and the compiler's
enforcement ladder in one place; this section is the working
summary.)

1. **App locus** — outer encapsulation; one per app. PascalCase
   name; `params` from argv defaults; `run()` delegates to a
   free helper; `main()` reads argv and statement-instantiates
   the locus.
2. **Namespace lotus** — empty (or config-only) `params { }`,
   methods only. The language's substitute for "module of
   functions" / "static class". Instantiate once, dispatch
   through it.
3. **Service locus** — full lifecycle (`birth → run → drain →
   dissolve`) for things that genuinely run over time. Sentinel
   params (e.g. `-1` for "not yet bound") let `dissolve` no-op
   on partially-constructed loci.
4. **Spawned child** — `let s = SomeLocus { ... };` defers
   dissolve to the enclosing fn's scope exit. The binding stays
   valid for method calls between construction and dissolve.
   Statement-position literals (`SomeLocus { };` with no `let`)
   fire-and-forget at the end of the statement.
   - **Accept'd flow child** (connection / per-request shape):
     a long-lived parent `accept(c: Conn)`s one child per
     connection; the child's `run()` IS its lifetime (a recv
     loop that returns on close). Declare `release(c: Conn)` on
     the parent to mark `Conn` a *flow* — then run-completion
     reclaims the child (its arena freed as the connection ends,
     not at parent dissolve). Or end it explicitly with
     `terminate;`. Without `release`/`terminate` an accept'd
     child is a *resident* and lives until the parent dissolves
     — on a daemon, forever (unbounded growth). See
     `spec/semantics.md § release(c)`.
5. **Data collection** — a `@form(vec)` / `@form(hashmap)` locus
   holding the storage, wrapped by a thin facade locus whose
   methods name the domain operations (`append` / `count` / `at`
   over `push` / `len` / `get`). Cell types must be unqualified
   in-seed structs; hashmap iteration is bucket-order (add a
   `seq` field for order) and has no delete (tombstone with a
   `present: Bool`). Fixed scalar `[T; N]` arrays are the
   zero-alloc alternative for fixed populations.
6. **Shape type** — `type Foo { a: Int; b: String; }`. Pure
   data, no flow. PascalCase, snake_case fields. Construct via
   struct literal.
7. **Free fn** — first-class seed member. Use when the operation
   has no flow and isn't naturally a method on an existing
   locus. When 3+ free fns form a coherent vocabulary, promote
   them to a namespace lotus (pattern 2).

**Composition patterns** (one layer up — how the shapes combine in
real services; full treatment in `docs/src/services/patterns.md`):

- **Three-locus gateway** — pinned reader → cooperative manager that
  `accept()`s → keyed per-entity children. The topic declares
  `keyed_by <field>` and each child subscribes `where key ==
  self.<field>` (scalar or String keys), so the bus delivers only
  that entity's traffic — never filter in the handler. The answer
  to "N dynamic keyed children with lifecycle"; the bus *is* the
  keyed routing table, so you never need a map of loci. Declare
  `release(c)` so children reclaim.
- **Demand-driven discovery** — a subscription triggers the
  `accept()`; topology grows from the data, zero hardcoded children.
- **Hot-path counters/gauges** — locus methods returning locus
  values are rejected (GH #18.6 "CQRS"). Migration: pre-allocate the
  counter/gauge loci as `params` at boot and mutate fields in place,
  or publish `MetricUpdate` to a single-writer collector. Never a
  method that returns a locus.
- **Publish-policy gate** — accumulate in place, publish behind a
  `tick()` with a time/volume trigger, so publish volume is bounded
  independently of input volume.
- **View lifetime** — `StringView` / `BytesView` / `*_span` results
  are valid only until the next recv/overwrite; `std::str::clone`
  out to persist. Stale access is now panic-guarded.

## What's NOT in the language

Filter these reflexes before they cost you time.

- **No `import` / `use` syntax for stdlib.** Stdlib is called
  inline through `std::*` paths (`std::io::fs::read_file(p)`).
  Cross-seed user libraries use `import "lib/x" as alias;`.
- **No visibility modifiers** (`pub`, `private`). Every
  top-level decl in a seed (one directory) is visible to every
  file in that seed. Decompose by concern, not visibility.
- **No `async` / `await`.** Concurrency comes from loci + the
  bus + per-locus thread *placement* declared in `main`'s
  `placement { }` block (F.31): `placement { gateway:
  pinned(core = 1); metrics: cooperative(pool = io); }`.
  Placement is a deployment seam on `main` only — never on
  the locus declaration. Unspecified main-locus params default
  to `cooperative(pool = main)`.
- **No `trait` / `impl` blocks.** There's `interface I { ... }`
  with structural satisfaction — any locus whose method set is
  a superset satisfies `I`. No `impl I for L`.
- **No parametric collection types** — no `Vec<T>` / `Map<K,V>`
  / `Option<T>` / `Result<T,E>`. Use `@form(vec)` /
  `@form(hashmap)` / `@form(ring_buffer)` on a locus. Errors
  flow through `fallible(E)` with required addressing at the
  call site.
- **No closures-as-values.** Function pointers exist (typed
  `fn(T) -> U`); inline closure-with-capture does not.
  Reconstruct context in the callee or route through the bus.
- **No method syntax on builtins.** `len(s)`, `to_string(n)` —
  not `s.len()`. User-defined locus / type methods use
  `obj.method()` normally.
- **No printf-style format strings.** `println(a, b, c)`
  concatenates its args. F-strings `f"hello {name}"` interpolate.
- **No `return` inside `birth` / `run` / `dissolve` bodies.**
  Factor short-circuit logic into helper free fns.
- **`fallible(E)` is rejected on substrate-facing surfaces:**
  lifecycle methods (`birth` / `run` / `dissolve`), mode bodies
  (`bulk` / `harmonic` / `resolution`), closure-assertion
  bodies, and bus-subscribed handlers. Those have no caller
  frame to address the error channel, so a `fallible(E)`
  declaration would describe a contract that can't be
  satisfied. User-declared `fn` members on a locus and free
  fns DO carry `fallible(E)` — they have a real caller. The
  narrowed two-channel rule (2026-05-25) keeps `↑` and `fallible`
  separate at the substrate boundary; everywhere else they
  compose. See `spec/semantics.md § fallible-on-locus`.
- **No `panic(msg)` / `assert(cond)`.** Failure is structural,
  routed through closure-tests + `on_failure` (the `↑` channel)
  or value-level via `fallible(E)`.

## Operational rules

- File extension `.hl`. ASCII-only outside string literals and
  comments. Statements end with `;`. Newlines are not
  terminators.
- `let x = 1;` is immutable; `let mut x = 1;` is reassignable.
- Bare struct/locus literals at statement position run
  birth-through-dissolve immediately. `let`-bound literals defer
  dissolve to the binding's scope exit.
- Bus send: `Foo <- payload;`. Subscribe is declarative
  (`subscribe Foo as handler;`). Subscribers must be born
  before publishers fire — instantiate them first in `main`.
- `self` is valid only inside lifecycle / mode / closure / `fn`
  member bodies of a locus.
- Build a directory: `hale build mydir/` bundles every `.hl`
  in the directory as one program; binary lands at
  `mydir/mydir`. Inside one seed, top-level scope is shared and
  resolution is order-free.
- Don't edit `crates/`. That's compiler territory. If a
  primitive you need is missing, work within the existing
  surface; don't reach into the compiler.

## Naming

- Locus, type, perspective, interface: `PascalCase`.
- Method, field, param, free fn: `snake_case`.
- Constant: `SCREAMING_SNAKE_CASE`.
- Bus subject (literal form): dot-separated lowercase
  (`log.app.db`). Topic name (preferred form): PascalCase
  identifier.

## Reading errors

Diagnostics cite the rule that fired. Read verbatim. Common
surprises:

- Bus subscriber declared after publisher fired → instantiate
  subscribers first.
- Topic ref used as expression value → topics aren't values;
  they address bus channels only.
- `self` outside a method body → you're in a free fn or top
  level; no enclosing Σ.
- Lifecycle / mode / closure-assertion / bus-handler method
  declared `fallible(E)` → the substrate orchestrates these,
  so the error channel has no caller to address. Drop the
  `fallible(E)` and route failure through `↑` (closure-test
  + `on_failure`), OR factor the body into a user-declared
  `fn` member that the lifecycle method calls with `or` to
  bridge the channels.
- "Error not addressed" on a `fallible` call → add `or raise` /
  `or default` / `or handler(err)`.

## First step

1. Skim `spec/styleguide.md` if you haven't (the seven patterns
   above are condensed from it — its §1 memory model and §4
   speed rules are the parts most sessions under-read).
2. Pick the smallest target. State it out loud: app name,
   stdlib paths you'll need, what you're not sure about.
3. Read 2-3 programs close to your target shape. The richest
   in-tree sources are in
   `crates/hale-codegen/tests/fixtures/examples/` (small
   per-feature anchors, numbered — the broadest acceptance
   surface).
4. Write the smallest program that gets one thing working.
   `hale run <file-or-dir>` for fast feedback; `hale build`
   for the native binary.
5. Iterate. Before you finish, run `hale fmt <file-or-dir>` —
   canonical form is tool-defined (zero config), so your diff
   touches only the lines you actually changed. Never
   hand-align columns; fmt collapses them.

## Hot-path memory patterns

Before writing code that runs many times per second (per-frame
handlers, tight loops, bus dispatch hot paths), read
[`spec/styleguide.md`](./spec/styleguide.md) §1 "The memory model
in one page" and §4 "Speed rules" (which absorbed the old
`agents/memory-patterns.md`). They catalog which assignment /
return / lookup shapes the substrate makes allocation-free
automatically and which require care from the author. The arena
allocator doesn't free per-allocation, so patterns that look
innocent at the call site can leak into a locus's lifetime arena
— but the substrate closes more of those shapes than you'd
expect, and the "What's already free" list preempts overcautious
code. The compiler backs the rules with a layered enforcement
ladder (default warnings → `@hot` → `@budget`); see styleguide
§5.

## Binding an external C library

Hale binds to non-stdlib C libraries (raylib, sqlite, curl, ...)
via user-extensible `@ffi("c")` declarations — no stdlib
expansion or compiler change needed. If you're writing a binding
library (typically landing in pond), read
[`agents/binding-packages.md`](./agents/binding-packages.md) for
the recommended file layout, the wrap-vs-leave-bare conventions,
the C-glue skeleton, and the testing pattern. The substrate
contract for the FFI surface itself is in
[`spec/ffi.md`](./spec/ffi.md).

## Inline structural failure

For "catch a value error in a locus method and shut this locus
down," use the four-piece pattern. Spec reference:
`spec/design-rationale.md` § F.27.

- **`closure NAME { captures: f1, f2; epoch inline; }`** —
  assertion-less closure shape that fires only via explicit
  `violate`. Snapshots the listed fields into the violation
  payload.
- **`violate NAME;` / `violate NAME with expr;`** —
  statement-level, divergent (`Never` type, same shape as
  `fail` in fallible fns and `bubble` in `on_failure`).
  Valid only inside a locus member fn; resolves NAME against
  the current locus's `epoch inline` closures.
- **Error-check fn pattern** — a locus member fn with
  signature `fn(ErrType) -> SuccessType`, used as
  `or self.handler(err)` at fallible call sites. Body either
  returns the substitute value or `violate`s. The bridge
  between the value channel and the structural channel.
- **`self.draining`** — synthetic Bool field on every locus,
  readable inside method bodies. True from the moment
  `violate` (or any drain trigger) fires until dissolve
  completes. Useful for "don't publish further after we
  decided to wind down."

The canonical pattern for "catch error and shut this locus
down" is a closure declaration + a member fn + one `violate`
statement. Don't reach for a `should_exit: Bool` flag plus a
`while !should_exit { yield; }` loop — the primitives above
are the supported form.

## Pointers

- Spec (canonical contract): `spec/`. Start with
  `spec/styleguide.md`, then `spec/semantics.md`, then
  `spec/grammar.ebnf`.
- Stdlib surface: `spec/stdlib.md`.
- Form library (`@form(vec)`, `@form(hashmap)`,
  `@form(ring_buffer)`): `spec/forms.md`.
- Memory / capacity slots / projection classes:
  `spec/memory.md`.
- Per-feature anchor programs (in-tree):
  `crates/hale-codegen/tests/fixtures/examples/`.
- Contrib libraries — protocols / parsers / shapes that don't
  belong in stdlib but are too useful to rewrite per-project:
  <https://github.com/hale-lang/pond>. Vendor via
  `hale.toml` → `hale fetch`; import as
  `import "vendor/pond/<lib>" as <alias>;`.
- Sibling repos: <https://github.com/hale-lang/examples>,
  <https://github.com/hale-lang/bench>.

---

**Hale** is the language. **lotus** is the runtime substrate.
C-runtime symbols stay `lotus_*` by design.
