# Brief: extending the stdlib or writing an Aperio library

You're an agent (or human) adding to Aperio's standard library
or writing a reusable Aperio library that other projects will
import. This brief tells you what shape your contribution
should take and what to avoid.

## Two stdlib shapes

The Aperio stdlib is two physically distinct things:

1. **Path-call dispatch.** Modules like `std::env::*`,
   `std::time::*`, `std::str::*`, `std::io::fs::*`,
   `std::process::*`, `std::ts::*` have no `.ap` source — the
   compiler routes their calls directly to a libcall (a libc
   function, the C runtime, or a Rust shim like
   `aperio-ts-shim`).
2. **Namespace lotus.** Modules like `std::cli::Resolver`,
   `std::iter::Lines`, `std::json::Builder`, `std::lang::*`,
   `std::log::*`, `std::yaml::*`, `std::text::Sink`,
   `std::io::tcp::*` are pure Aperio source under
   `crates/aperio-codegen/runtime/stdlib/*.ap`. Each module is
   a locus with empty (or config-only) `params` whose methods
   form the namespace's vocabulary.

## When to use each shape

- **Need to call out to C / Rust / libc?** Path-call dispatch.
  Add a route in the relevant `lower_std_*` block in
  `crates/aperio-codegen/src/codegen.rs`. If you're composing
  Aperio helpers around the extern, declare a thin extern
  signature in `core.ap` (or an existing namespace lotus) and
  build the higher-level surface in Aperio.
- **Pure Aperio?** Namespace lotus. Add a new
  `crates/aperio-codegen/runtime/stdlib/<name>.ap`, declare the
  locus per the namespace-lotus pattern in
  `spec/styleguide.md`, register an entry in
  `STDLIB_PATH_RENAMES` in `codegen.rs`, and append the file
  to `STDLIB_AP_SOURCE`'s `concat!(...)`.

## The friction discipline

Stdlib relieves *real friction* surfaced by working programs.
Speculative additions create dead surface area, increase the
maintenance burden, and crowd the docs. Wait for a concrete
case where the absence of a primitive forced two or more sites
to copy-paste the same workaround.

The corollary: don't add a `Map<K, V>` because Rust has one.
Don't add `Option<T>` because every other language does. Don't
add `Result<T, E>`. The forms `@form(vec)` and
`@form(hashmap)` exist for parametric collection lowerings;
`fallible(T)` is the value-error protocol. Both predate any
work you might do here. Use them.

## What's not in the stdlib (and why)

The list below is not exhaustive but flags the most common
"why doesn't X exist?" cases:

- Filesystem watch (inotify / fsevents) — no driver workload yet.
- Generics beyond `@form(...)` lowerings — `@form(vec)` and
  friends are the v1 answer for parametric collection shapes.
- Sum types in payloads / pattern-matching on enum variants —
  payloads are records today.
- Multiple distinct accept types in one locus.
- HTTP keep-alive, custom request headers, header maps, bodies
  > 8 KB — all out of scope for v1's std::http.
- Cross-seed module import / `use` — the `module` keyword is
  reserved with no semantics.
- Inline markdown formatting, graphics, UI, embedded shell.
- Compiler self-introspection (`std::aperio::parse(...)`).

If a workload genuinely needs one of these, that's worth
discussing — but the answer might be "build it as a user
library and see if the friction shape stabilizes" rather than
"add it to std."

## Form contracts (locked for v1)

The `@form(...)` annotations pick a lowering and synthesize a
canonical method set:

- `@form(vec)` — heap items, methods `push` / `get` / `pop` /
  `len` / `is_empty`.
- `@form(hashmap)` — pool entries indexed by a key field
  declared via `indexed_by`. Methods `set` / `get` / `has` /
  `remove` / `len` / `is_empty`.
- `@form(ring_buffer)` — fixed-cap pool with FIFO semantics.

User-extension methods declared *on top of* a formed locus
work; method overrides for the synthesized set are deferred to
v2. A formed locus must run within ~10% of hand-written C
equivalent (perf gate; benchmark before adding new forms).

See `spec/forms.md` for the full library.

## Spec discipline

If you change user-visible surface, the spec must change in
the same commit.

1. Land the implementation (parser + typechecker + codegen +
   runtime + tests).
2. Add an F-numbered commitment in `spec/design-rationale.md`
   for new design choices, OR update the relevant
   `spec/<topic>.md` for shipping new behavior under an
   existing commitment.
3. Update `spec/stdlib.md` if a stdlib path changes, gets
   added, or gets removed.
4. Resolve the corresponding entry in `notes/open-questions.md`
   if you closed a deferred question.

The spec is **not aspirational**. If it's in
`spec/design-rationale.md` as an F-commitment, it describes
shipped behavior. If a feature has been removed, the spec
entry must be removed too.

## Two-channel rule (locked design)

Locus methods cannot declare `fallible(E)`. The two surfaces
that *can* are free fns and `@form(...)`-synthesized methods.
This is permanent design, not a temporary limit — see
`spec/design-rationale.md` F-numbered commitment on the
two-channel rule.

If you need a stdlib operation to be fallible, add it as a
free fn (`std::io::fs::read_to_string(path) -> fallible(String,
IoError)`) or surface it through an `@form` whose synthesis
produces fallible accessors.

## Naming aliases for libraries

Library authors choose nothing at import time — the consumer
picks the alias (`import "lib/finance" as fin;`). What you
*can* control is what reads naturally under any short alias.
A few rules from `spec/styleguide.md`:

- Short, lowercase suggested aliases (`fin`, `helpers`, `log`).
- Don't embed the library name in your decl names. `fin::Quote`
  is clear; `fin::FinQuote` doubles the namespace.
- Top-level decls should read fluently under `alias::`.
