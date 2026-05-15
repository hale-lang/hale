# Brief: writing an Aperio app

You're an agent (or human) working on an Aperio program. This
brief tells you what you need to internalize about the language
to be productive without inventing features that don't exist.

## The mental model

A **locus** is the unit of structure. It declares typed state
(`params`), a lifecycle (`birth → accept → run → drain →
dissolve`, plus `on_failure`), what it publishes / subscribes
on the typed bus, and what methods it exposes. Apps, services,
handlers, caches, queues, namespaces — everything is a locus.
Loci nest inside loci all the way down.

A **type** is pure shape: fields, no flow, no lifecycle. Use it
when the thing you're modeling is a record (a `Point`, a
`Quote`). Use a locus the moment lifecycle or coordination
appears.

The **bus** is typed pub/sub. A `topic Foo { payload: T; }`
declaration names a channel; `subscribe Foo as on_foo;` binds a
handler; `Foo <- value;` publishes. Subscribers must be born
before publishers fire — instantiate them first in `main`.

`self` is valid only inside lifecycle / mode / closure / fn
member bodies. It refers to the enclosing locus's params and
exposed state.

## Things that are NOT in the language

If you find yourself reaching for one of these, don't — the
shape isn't there.

- No `import` / `use` / `from x` syntax. Stdlib modules are
  called inline through magic `std::*` paths
  (`std::io::tcp::listen(...)`); cross-seed user libraries use
  `import "lib/x" as alias;` (see `spec/projects.md`).
- No `pub fn` / `pub struct` / visibility modifiers. Every
  top-level decl in a seed is visible to every file in that
  seed. Decompose by *concern*, not by visibility.
- No `async` / `await` / `Future`. Concurrency comes from loci
  + the bus + schedule classes (`: schedule cooperative` /
  `: schedule pinned(core = N)`).
- No `trait` / `impl` blocks. There's `interface I { ... }`
  with structural satisfaction (any locus whose method set is
  a superset satisfies the interface — no `impl I for L`).
- No parametric `Vec<T>` / `Map<K, V>` / `Option<T>` /
  `Result<T, E>` as user-facing tagged enums. Use `@form(vec)`
  / `@form(hashmap)` on a locus, fixed arrays `[T; N]`, or
  sentinel + predicate. Errors flow through `fallible(T)`.
- No closures-as-values. Function pointers exist (typed `fn(T)
  -> U`); inline closure-with-capture does not. Reconstruct
  context inside the called fn or route through the bus.
- No method syntax on builtins. Use `len(s)`, `to_string(n)`,
  not `s.len()`. User-defined locus / type methods *do* use
  `obj.method()`.
- No printf-style format strings. `println(a, b, c)`
  concatenates its args.
- No `return` inside `birth` / `run` / `dissolve` lifecycle
  bodies. Factor short-circuit logic into helper free fns.
- No locus methods declaring `fallible(E)`. Free fns and
  `@form(...)`-synthesized methods are the only fallible
  surfaces. (See "two-channel rule" in
  `spec/design-rationale.md`.)

## Operational rules

- File extension `.ap`. ASCII-only outside string literals and
  comments.
- Statements end with `;`. Newlines are not terminators.
- Bindings are immutable by default: `let x = 1;` cannot be
  reassigned; `let mut x = 1;` can.
- Bare struct/locus literals at statement position run
  birth-through-dissolve immediately (`Pulse { iters: 4 };`).
  `let`-bound literals defer dissolve to the binding's scope
  exit (`let p = Pulse { ... };`).
- Bus send: `Foo <- value;`. Subscribe: declarative, paired
  with a handler fn.
- Build a directory: `aperio build mydir/` bundles every `.ap`
  in the directory as one program. Binary lands at
  `mydir/mydir`. Inside one seed, top-level scope is shared and
  resolution is order-free (alphabetical-by-filename merge).
- Don't edit `crates/`. That's compiler territory. If you hit
  a primitive that's missing, file friction in your own notes
  rather than reaching down into the compiler.

## Reading errors

The typechecker emits diagnostics with spans and rendered
source context. Most messages cite the rule that fired (e.g.,
"locus `X` already declared", "topic `T` is a topic reference;
`of type T` is forbidden"). Read the message verbatim — it's
usually directly actionable. If a diagnostic surprises you,
the most common causes are:

- Bus subscriber declared after publisher fired: instantiate
  subscribers first.
- Topic ref used as an expression value: topics aren't first-
  class values; they only address bus channels.
- `self` outside a method body: you're in a free fn or at
  top level — there is no enclosing locus.
- Lifecycle method declaring `fallible(E)`: convert to a free
  fn or move the fallible call onto a `@form`-synthesized
  method.

## Where the stdlib is

Bundled into every binary. Two shapes:

- Path-call (extern bridges): `std::env::*`, `std::time::*`,
  `std::str::*`, `std::io::fs::*`, `std::process::*`,
  `std::ts::*`. No `.ap` source — these route directly to the
  C runtime.
- Namespace lotus (Aperio-sourced): `std::cli::Resolver`,
  `std::iter::Lines`, `std::json::Builder`, `std::lang::*`,
  `std::log::*`, `std::yaml::*`, `std::text::Sink`,
  `std::io::tcp::*`. Source under
  `crates/aperio-codegen/runtime/stdlib/*.ap`.

A handful of free fns are always in scope without a path:
`print`, `println`, `eprint`, `eprintln`, `len`, `to_string`,
`min`, `max`, `abs`, `starts_with`, `contains`. Type primitives
(`Int`, `Uint`, `Float`, `Decimal`, `String`, `Bool`, `Time`,
`Duration`, `Bytes`) are valid only in type position.

## First-step protocol

1. Skim `spec/styleguide.md` — six pattern shapes, one short
   read.
2. Pick a small target. State it: app name, stdlib namespaces
   you'll need, what you're not sure about.
3. Read 2-3 relevant example fixtures under
   `crates/aperio-codegen/tests/fixtures/examples/`.
4. Write the smallest program that gets one thing working;
   `aperio run` it.
5. Iterate. If you hit a wall the language can't express,
   that's friction — don't paper over it; surface it.

## Naming

- Types: `PascalCase` (`Tick`, `Quote`, `OrderBook`).
- Loci: `PascalCase`, often with a role suffix that hints flow
  (`Counter`, `Pulse`, `OrderBookL` — the `L` is optional but
  conventional in code that mixes types and loci heavily).
- Methods, fields, params: `snake_case`.
- Internal stdlib free fns: `__name` prefix (the typechecker
  treats this prefix as private-by-convention).

`Aperio` is the language; `lotus` is the runtime substrate.
C-runtime symbols are `lotus_*` by design; don't rename them.
