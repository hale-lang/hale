# hello-world

The minimal Aperio program. One locus, one lifecycle method.

```
locus HelloL {
    params {
        greeting: string = "hello, world";
    }

    birth() {
        println(self.greeting);
    }
}

fn main() {
    HelloL { };
}
```

## What runs

1. The runtime invokes `main()` (the program entry point).
2. `HelloL { }` instantiates the locus:
   - The runtime allocates a region for the locus inside its
     parent's region (here: the runtime root region).
   - Default param values apply because no overrides are given.
   - The locus's `birth()` method runs synchronously.
3. `birth()` calls `println` with the greeting.
4. `birth()` returns. The locus has no `run` (no steady-state
   loop), no in-flight messages.
5. The `HelloL { }` expression's result is unbound, so the
   locus's lifetime ends at the enclosing statement boundary.
   The runtime invokes default `drain()` (no-op) and default
   `dissolve()` (frees the region).
6. `main()` returns. Process exits.

## Primitives this exercises

- **Locus declaration** — `locus HelloL { ... }`.
- **Params block with default value** — `params { greeting:
  string = "hello, world"; }`. The `greeting` param is
  declared with a hand-given prior; no inferred params here.
- **`birth()` lifecycle method** — invoked at instantiation.
- **`self.x` access to a locus param** — `self.greeting`
  references the locus's own state from inside a lifecycle
  block.
- **Locus instantiation as a struct-literal expression** —
  `HelloL { }` allocates the region and invokes `birth()`.
  Because `HelloL` is declared as a `locus` (not a `type`),
  the compiler emits the lotus-instantiation sequence
  (region allocation → birth → handle return).
- **`println` builtin** — top-level builtin function for
  stdout output. See `spec/tokens.md` "Built-in identifiers."
- **Top-level `main` function** — the program entry point.

## Primitives this does *not* exercise

These appear in subsequent examples (planned, not yet written):

- **`contract` block** — no coordination here, so no contract
  needed. Future example: a parent locus with a child locus
  that exposes fields.
- **`accept` / `run` / `drain` / `dissolve` lifecycles** —
  this program only uses `birth`. A future example shows a
  long-lived locus with a steady-state `run` loop.
- **Mode declarations** (`mode bulk`, `mode harmonic`, `mode
  resolution`) — no kernel projection is happening.
- **Closure tests** (`closure name { ... ~~ ... within ... }`)
  — no audit cycle to assert.
- **Bus interface** (`bus { subscribe ... ; publish ... ; }`)
  — output is via `println` builtin, not a typed bus subject.
- **Failure handling** (`on_failure(c, err) { ... }`) — no
  child to fail; runtime root handles `birth` panics with
  default exit.
- **Perspectives** — no analyst/executor split here.
- **Projection-class generics** (`Rich<T>`, `Chunked<T>`,
  `Recognition<T>`) — no parametric-projection-class data
  structures.
- **`accept(...)` / `restart(...)` / `bubble(...)` recovery
  primitives** — no parent-policy decisions.

## What writing this surfaced (for the spec / open-questions)

- **`self` keyword.** Not present in the v0.1 grammar; needed
  for lifecycle blocks to reference locus params. Added to
  `tokens.md` reserved-words list and noted in
  `spec/design-rationale.md` §A.
- **Locus instantiation as struct-literal expression.** The
  grammar's `struct_expr` production handles the syntactic
  form, but the *semantic* difference (locus instantiation
  triggers `birth()`, allocates a region, returns a handle)
  is not yet specified. Noted in `spec/design-rationale.md`
  §B.
- **Builtin functions.** `println` (and `print`) need
  somewhere to live. Added to `tokens.md` "Built-in
  identifiers" section.
- **Locus handle lifetime.** When the result of
  `HelloL { }` is unbound, when does the locus dissolve?
  Committed: at the enclosing statement boundary. Noted in
  `spec/design-rationale.md` §C.
- **Default lifecycle methods.** `drain()` and `dissolve()`
  must have compiler-supplied defaults (no-op drain;
  region-free dissolve). Already mentioned in §6 of
  design-rationale; reaffirmed.

## Next examples (planned)

- **01-locus-with-run** — single locus with a `run` loop that
  prints periodically until it receives a stop signal. Adds
  the `run` lifecycle.
- **02-parent-child** — a coordinator locus that accepts one
  greeter coordinatee and reads its `greeting` through the
  contract. Adds `accept`, `contract { expose ... ; }`, and
  contract-graded access.
- **03-closure-test** — a parent locus with two children
  whose outputs must satisfy a cyclic-closure test. Adds
  `closure name { ... ~~ ... within ... ; }`.
- **04-modes** — a locus that exposes the same kernel under
  bulk / harmonic / resolution modes. Adds `mode` declarations.
- **05-bus** — a locus whose input is a bus subscription
  (placeholder transport). Adds `bus { subscribe ... ; }`.

When all five are written and compile cleanly, the v0.1 spec
is empirically validated at the smallest scale. The trellis
trader/analyst pair is the next step beyond.
