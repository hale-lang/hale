# Brief: extending the stdlib or writing an Hale library

You're an agent (or human) adding to Hale's standard library
or writing a reusable Hale library that other projects will
import. This brief tells you what shape your contribution
should take and what to avoid.

## Two stdlib shapes

The Hale stdlib is two physically distinct things:

1. **Path-call dispatch.** Modules like `std::env::*`,
   `std::time::*`, `std::str::*`, `std::io::fs::*`,
   `std::process::*`, `std::ts::*` have no `.hl` source — the
   compiler routes their calls directly to a libcall (a libc
   function, the C runtime, or a Rust shim like
   `hale-ts-shim`).
2. **Namespace lotus.** Modules like `std::cli::Resolver`,
   `std::iter::Lines`, `std::json::Builder`, `std::lang::*`,
   `std::log::*`, `std::yaml::*`, `std::text::Sink`,
   `std::io::tcp::*` are pure Hale source under
   `crates/hale-codegen/runtime/stdlib/*.hl`. Each module is
   a locus with empty (or config-only) `params` whose methods
   form the namespace's vocabulary.

## When to use each shape

- **Need to call out to C / Rust / libc?** Path-call dispatch.
  Add a route in the relevant `lower_std_*` block in
  `crates/hale-codegen/src/codegen.rs`. If you're composing
  Hale helpers around the extern, declare a thin extern
  signature in `core.hl` (or an existing namespace lotus) and
  build the higher-level surface in Hale.
- **Pure Hale?** Namespace lotus. Add a new
  `crates/hale-codegen/runtime/stdlib/<name>.hl`, declare the
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
  > 8 KB — all out of scope for v1's std::http. (The
  `std::http::Server` locus shipped 2026-05-16 wraps accept +
  parse + dispatch + write; route table is the user's
  `handler: fn(Request) -> Response` callback.)
- Nested JSON trees (tagged-union JsonValue with recursive
  Object/Array) — Hale lacks payload-bearing enums + Box, so
  v1 ships flat-shape helpers only: `std::json::escape_string`
  / `unescape_string`, `find_*_field` on flat objects,
  `ArrayIter` for top-level array elements.
- Cross-seed module import / `use` — the `module` keyword is
  reserved with no semantics.
- Inline markdown formatting, graphics, UI, embedded shell.
- Compiler self-introspection (`std::hale::parse(...)`).

If a workload genuinely needs one of these, that's worth
discussing — but the answer might be "build it as a user
library and see if the friction shape stabilizes" rather than
"add it to std."

## Form contracts (locked for v1)

The `@form(...)` annotations pick a lowering and synthesize a
canonical method set:

- `@form(vec)` — heap items, methods `push` / `get` / `set` /
  `pop` / `len` / `is_empty` / `sort` / `sort_by` /
  `sort_desc_by`.
- `@form(hashmap)` — pool entries indexed by a key field
  declared via `indexed_by`. Methods `set` / `get` / `has` /
  `remove` / `len` / `is_empty` / `key_at` / `entry_at` /
  `bump`. `key_at` / `entry_at` iterate in hash-table order
  (no parallel keys vec needed); `bump` is increment-or-init
  for cells shaped `{key + one Int counter}`.
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

## Two-channel rule (narrowed 2026-05-25)

`fallible(E)` is rejected on **substrate-facing surfaces**:
lifecycle methods (`birth` / `run` / `dissolve`), mode bodies
(`bulk` / `harmonic` / `resolution`), closure-assertion
bodies, and bus-subscribed handlers. The substrate
orchestrates those — there's no caller frame to address
the error channel, so a `fallible(E)` declaration would
describe a contract that can't be satisfied. Surface failure
there through `↑` (closure-test + `on_failure`).

`fallible(E)` IS allowed on user-declared `fn` members and
on free fns — both have a real caller that can `or raise` /
`or default` / `or handler(err)` the result. Library code
that wants to expose a fallible operation as a method on a
locus type can declare it as a user `fn` member:

```hale
locus Reader {
    fn parse(b: Bytes) -> Message fallible(ParseError) {
        if bad { fail ParseError { msg: "bad header" }; }
        return Message { ... };
    }
    run() {
        let m = self.parse(b) or default_message();
    }
}
```

The earlier blanket rule ("locus methods can't be fallible
at all") was narrowed because the friction signal across
multiple libraries showed devs extracting free fns just to
get a value-error channel back — losing `self` ergonomics
and splitting closely-related code across two top-level
decls. See `spec/semantics.md § fallible-on-locus` for the
canonical statement and `notes/open-questions.md § #24` for
the rationale + rejected alternatives.

`@form(...)`-synthesized accessors (`get` / `set` /
`array_at`) remain fallible like before — that's
orthogonal to the narrowing.

## Naming aliases for libraries

Library authors choose nothing at import time — the consumer
picks the alias (`import "lib/finance" as fin;`). What you
*can* control is what reads naturally under any short alias.
A few rules from `spec/styleguide.md`:

- Short, lowercase suggested aliases (`fin`, `helpers`, `log`).
- Don't embed the library name in your decl names. `fin::Quote`
  is clear; `fin::FinQuote` doubles the namespace.
- Top-level decls should read fluently under `alias::`.

## Shipping a library via git

A consumer who wants your library declares it in their
`hale.toml`:

```toml
[deps]
mylib = { git = "https://github.com/you/mylib", tag = "v0.1.0" }
```

Then `hale fetch` clones your repo into their `vendor/mylib/`
and writes a SHA to `hale.lock`. They reference it as
`import "vendor/mylib" as ml;`. The directory itself becomes
a single Hale seed (every `.hl` at the repo root is one
library; nested directories are NOT crawled). What this means
for your repo layout:

- Source goes at the repo root, not under `src/`. Naming files
  is your choice — alphabetical merge order is the only
  ordering the compiler imposes.
- Tag releases with semver-ish strings if you want consumers to
  pin via `tag = "v0.1.0"`. The compiler doesn't enforce
  semver; the tag is just a git ref.
- Your library has no manifest of its own (no transitive deps
  in v1). If your code calls into another git library, the
  consumer of *your* library needs to vendor that other library
  too. Document that requirement in your README.

This shape favors small libraries with shallow dep trees. It's
the same trade Go made before modules and still works well at
single-developer scale.
