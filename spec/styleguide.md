# Style guide

This document specifies idiomatic Aperio: the shape primitives an
Aperio program is composed of, the naming conventions, the
composition rules, and the anti-patterns the substrate makes
expensive. Where `grammar.ebnf` says what's syntactically valid
and `types.md` / `semantics.md` say what's *meaningfully* valid,
this document says what's *idiomatically* valid — what an Aperio
program should look like when an author has applied the
framework's primitives coherently rather than fought them.

The styleguide is normative for new code in `apps/`, in the
bundled stdlib seed (`crates/aperio-codegen/runtime/stdlib/`),
and in `examples/`. It is descriptive for older code that
predates a given rule; refactors apply the rules opportunistically.

## The foundational axiom

> **Every named structural thing is a locus.** Types are loci-
> in-waiting — the smallest growth stage on the locus gradient.

If a thing has lifecycle, contracts, bus participation, modes,
closures, capacity slots, or projection class, it is a fully-
grown **locus**. If it is pure data (record, returnable by value,
no flow), it is a **type** — a locus that's still proto-form. The
two are points on one gradient, not separate categories. There is
no third primitive. See `notes/aperio-types-vs-loci.md` for the
source axiom and `spec/design-rationale.md` § "Foundational
axiom" for the design treatment.

A seed (one directory under `apps/` or the bundled `runtime/stdlib/`)
exports loci, types, and free fns at the top level. Free fns are
first-class seed members; use them when the operation has no flow
(no lifecycle, no contracts, no mutable state) and isn't naturally
a method on an existing locus. When a coherent vocabulary of three
or more free fns forms, the namespace-lotus form (below) often
reads better — it makes the vocabulary nameable and self-composing.

## The recursive principle

Loci are the fundamental building block at every layer of an
Aperio program:

- **An app** is a locus (outer encapsulation; one per `apps/<name>/`).
- **A namespace of pure helpers** is a locus (empty `params { }`,
  methods only).
- **A long-running service** is a locus (birth/run/dissolve;
  often with `bus subscribe`).
- **A spawned async worker** is a locus (child of its parent;
  cooperative schedule by default).
- **A bus subscriber** is a locus (HTTP route, message handler,
  event listener — all the same shape).
- **A configured cache / pool / pipeline / queue** is a locus.

**Inside any locus, behavior is itself a locus tower one layer
down.** A cache's lookup flow has its own birth (acquire lock),
run (probe + return), dissolve (release lock). The recursion
bottoms at primitive operations — arithmetic, single field reads,
primitive calls. Everything above the floor is loci nested in loci.

The recursion has practical consequences for code organization:
there is no `module`, `class`, `package`, or `namespace` keyword
because none is needed. Anything one of those would do, a locus
does — and the locus carries lifecycle and contracts the other
forms don't.

## The pattern catalog

Six idiomatic shapes. Every locus or free fn in a well-written
Aperio program matches one of these. Code that doesn't match one
of these should be reconsidered against the patterns before
shipping.

### 1. App locus — outer encapsulation

Every app's `main.ap` defines an `<Name>L` locus that owns the
whole run. `fn main()` reads argv, instantiates the locus, exits.
The locus's `run()` body delegates to a free helper because
lifecycle bodies reject `return` at v0 — short-circuit logic
factors out.

```aperio
locus OnboardL {
    params {
        dir: String = "apps/operational-graph/fixture";
        flavor: String = "go";
    }
    run() {
        drive(self.dir, self.flavor);
    }
}

fn main() {
    let mut dir = "apps/operational-graph/fixture";
    let mut flavor = "go";
    if std::env::args_count() > 1 {
        dir = std::env::arg(1);
    }
    if std::env::args_count() > 2 {
        flavor = std::env::arg(2);
    }
    OnboardL { dir: dir, flavor: flavor };
}
```

Conventions:

- Locus name is `<FileStem>L` with `L` suffix.
- `params` block holds argv-derived configuration with reasonable
  defaults (so the app self-demos with no flags).
- `run()` is the only lifecycle method needed for most apps.
- `main()` does the argv parsing, then a single statement-position
  locus literal kicks the run.
- Statement-position literals fire-and-forget: `OnboardL { ... };`
  starts the run and the locus dissolves at fn-return.

### 2. Namespace lotus — empty params, methods only

When a coherent vocabulary of pure helpers forms, wrap them in a
locus with empty (or config-only) `params { }` and a method-only
body. Use sites instantiate once and dispatch through it. The
pattern is the language's substitute for "module of functions" /
"static class" / "stateless service object."

```aperio
locus Morpheme {
    params {
        flavor: String = "go";
        overrides: String = "";
    }
    fn lookup_morpheme(m: String) -> String { ... }
    fn suffix_rule(m: String) -> String {
        if self.ends_with(m, "er") { ... }
    }
    fn name_to_motion(name: String) -> String {
        let hit = self.lookup_morpheme(name);
        ...
    }
}

// Use site:
let r = std::lang::Morpheme { flavor: "go" };
let motion = r.name_to_motion("OrderProcessor");
```

Conventions:

- "Empty params" doesn't have to be literally empty — config-only
  params (e.g. `flavor`, `overrides`) parameterize the lookup.
  The point is **no lifecycle state** that birth/run/dissolve
  would mutate.
- Self-method calls (`self.X(...)`) compose within the namespace.
- One alloc per instantiation; negligible.

### 3. Service locus — long-lived with lifecycle + bus

When the thing genuinely runs over time and participates in the
bus, write the full lifecycle.

```aperio
locus Listener {
    params {
        host: String = "127.0.0.1";
        port: Int = 0;
        listen_fd: Int = -1;
        max_accepts: Int = 1;
        on_connection: fn(std::io::tcp::Stream) = default_on_connection;
    }
    birth() {
        self.listen_fd = std::io::tcp::listen_socket(self.host, self.port);
    }
    run() {
        let mut accepted = 0;
        while self.max_accepts < 0 || accepted < self.max_accepts {
            let conn = std::io::tcp::accept_one(self.listen_fd);
            handle_one_connection(conn, self.on_connection);
            accepted = accepted + 1;
        }
    }
    dissolve() {
        std::io::tcp::close_fd(self.listen_fd);
    }
}
```

Conventions:

- `birth()` does setup that must run before any work. Mutates
  `self.field` to record acquired resources.
- `run()` does the long-lived work. Often a loop bounded by
  configuration.
- `dissolve()` releases what `birth()` acquired.
- Sentinel values in `params` (`-1` for "not yet bound") let
  `dissolve()` safely no-op on partially-constructed loci.

### 4. Spawned child locus — let-bound, scope-dissolves

When a parent's `run()` produces work that needs its own
lifecycle, let-bind a locus literal. Per the m82 dissolve-timing
rule (see `spec/semantics.md` § "Dissolve timing rules"), the
deferred-dissolve mechanism fires the child's `dissolve()` at the
parent fn's scope exit; the user-visible binding stays valid for
method calls in between.

```aperio
fn handle_one_connection(conn_fd: Int, on_conn: fn(std::io::tcp::Stream)) {
    let s = std::io::tcp::Stream { conn_fd: conn_fd };
    on_conn(s);
}
```

The `let s = ...` binds the Stream locus to the fn's scope; when
`handle_one_connection` returns, `s.dissolve()` fires (which
closes `conn_fd`). No explicit `dissolve(s)` call needed.

Conventions:

- Use **let-binding** when the locus needs to live for a fn body's
  full duration. Statement-position literals dissolve at end of
  expression — rarely what's wanted for a usable handle.
- Per-iteration cleanup uses a free helper fn whose return is the
  per-iteration boundary; block-level deferred-dissolve isn't
  shipped (`handle_one_connection` is the workaround shape).

### 5. Shape type — pure data, no flow

When a thing IS data, not flow, declare it as `type`. No
lifecycle, no contracts, no bus, no mutable methods.

```aperio
type Request {
    method: String;
    path: String;
    version: String;
    body: String;
}
```

Use sites construct via struct literal:

```aperio
let req = std::http::Request {
    method: "GET", path: "/", version: "HTTP/1.1", body: ""
};
```

Conventions:

- PascalCase, no `L` suffix (the suffix is reserved for loci).
- Fields named with snake_case.
- Returnable from fns by value. No lifecycle implications.
- Types may hold `fn(...)` fields (v1.x-8); dispatch via
  `record.field(args)`.
- If methods accumulate, it has flow — change `type` to `locus`.

### 6. Free fn — first-class seed member

Free fns are first-class seed members. Every top-level decl in a
seed is visible to every file in the seed. Use a free fn when the
operation has no flow and isn't naturally a method on an existing
locus.

Common shapes:

1. **`return`-bearing helpers** called from lifecycle method
   bodies (which cannot themselves use `return` at v0).
2. **Extension hooks** passed via fn-pointer params (e.g.,
   `on_connection: fn(Stream)`). The hook is named at the top
   level so a caller can pass it by name.
3. **Standalone helpers** that compose with the rest of the seed:
   format / parse / convert / classify utilities that don't carry
   state.

When a coherent vocabulary of three or more free fns forms, the
namespace-lotus form often reads better. The pattern catalog's
`std::lang::Morpheme` and `std::cli::Resolver` are vocabularies
that earned promotion. Helpers that don't form a coherent
vocabulary stay as free fns.

## Naming conventions

| Construct | Convention | Example |
|---|---|---|
| Locus (any kind) | `<Name>L` suffix | `OnboardL`, `RecognitionCoord` |
| Type (shape record) | PascalCase, no suffix | `Request`, `Response`, `Point` |
| Locus method / type field | snake_case | `name_to_motion`, `listen_fd` |
| Lifecycle method declaration | drop the `fn` keyword | `run() { ... }`, `birth() { ... }` |
| Free fn | bare snake_case | `drive`, `handle_one_connection`, `say` |
| Bus subject | dot-separated, lowercase | `log.app.db`, `agent.intent.camera` |
| Constants | UPPER_SNAKE_CASE | `STDLIB_AP_SOURCE` |

The `<Name>L` suffix on loci is the load-bearing convention that
makes loci-vs-types instantly recognizable at the call site. A
new locus declaration without the suffix is a style violation
even if it parses cleanly.

## Composition patterns

- **Self-method calls** (`self.method(arg)`) compose within a
  locus. No special syntax, no virtual dispatch — the receiver is
  implicit because the call is inside the locus body.
- **Cross-locus method calls** (`other.method(arg)`) work on
  typed locus references. Methods resolve by the locus's declared
  name.
- **Let-bound locus literals** defer dissolve to scope-exit per
  the m82 dissolve-timing rule. Use when the locus's lifecycle
  should match a fn body's duration.
- **Statement-position literals** (`SomeL { ... };` with no
  `let`) fire and dissolve at end of expression. Use for one-shot
  runs with no aftermath.
- **Cross-locus state via bus subjects**, not via field reads on
  a passed reference. The bus is the language-blessed channel for
  cross-locus coordination; vertical-only flow (see
  `spec/design-rationale.md` F.6 / F.11) makes lateral
  field-reads non-typeable.
- **Fn-pointer callbacks** (e.g., `on_connection: fn(Stream)`)
  cannot capture surrounding state. Either route state through
  bus subjects, reconstruct state inside the callback, or factor
  into a locus method that has its own `self`.

## Rolling the design

The pattern catalog is small on purpose. New primitives must
**roll into** the existing seed, not sit beside it. Rolling means
two conditions held simultaneously:

- **Continuity in shape.** A new locus mirrors the shape of an
  existing locus — same params/methods/lifecycle pattern,
  different domain. A reader who knows one knows the new one at a
  glance.
- **Interlock in composition.** A new locus's outputs are valid
  inputs to existing primitives. The seed forms a graph; each new
  primitive slots into that graph or it doesn't roll.

Both conditions matter. A primitive that mirrors an existing
shape but produces an isolated output is recognizable but
useless. A primitive that interlocks but invents a new shape is
useful but foreign.

### The test for a new primitive

When proposing one — a new namespace lotus, a new shape type, a
new free fn that wants to graduate — ask:

1. **Which existing pattern does this mirror?** Params +
   self-composing methods → namespace lotus per
   `std::lang::Morpheme`. Birth/run/dissolve + `on_X` callback →
   service locus per `std::io::tcp::Listener`. If nothing existing
   mirrors what's proposed, the proposer may be inventing a
   category — pause.
2. **What consumes the output?** New outputs should slot into
   existing consumers without per-call adaptation. If nothing
   existing reads what the primitive produces, it has created an
   island.
3. **Could a reader who knows the catalog recognize this
   immediately?** If the primitive needs a paragraph of "this
   works differently from the others," the proposer is adding a
   category, not rolling one.

If all three answers are clean, the primitive rolls. If any is
"I don't know" or "this one is special," reconsider against the
catalog before proceeding.

### Good in the code AND good in the machine

The frame is the same lens applied twice:

- **In the code** (reader-side): patterns repeat, so the reader's
  mental model doesn't fragment per-feature. Cognitive load
  amortizes across the catalog instead of compounding.
- **In the machine** (composition-side): outputs interlock, so
  each primitive's results flow into the next without glue. The
  medium is shared (newline-separated String, tagged-row String,
  tree-sitter Int node, struct-literal value). A primitive that
  speaks a foreign medium would force every consumer to bridge —
  glue code grows quadratically with the number of primitives.

Break either condition and the primitive doesn't fit. A primitive
that needs glue at runtime also needs explanation at read time.

## Anti-patterns

The shapes these violate are *almost always* "an old habit from
another language smuggled in past the substrate."

- **Bare `fn main()` with helpers and no outer locus.** The app's
  outer encapsulation must be a locus per the apps-are-loci rule.
- **Coherent helper vocabulary stranded as `free_fns`** when it
  forms a namespace. Lift into a namespace lotus once the
  coherence is visible.
- **`type` for things that have flow.** If the noun has a
  lifecycle implied (a Cache that's loaded/probed/evicted; a
  Server that starts/serves/stops), it is a locus.
- **Methods on a `type` record.** Not supported at v0 — the
  language is telling the author "this wanted to be a locus."
  Use a locus with empty `params` instead.
- **"Util" namespaces of unrelated helpers.** Group by
  *vocabulary*, not by "everything that didn't fit elsewhere." A
  namespace lotus should answer one question (e.g.,
  "noun-to-motion" or "tagged-accumulator parsing"), not many.
- **Floating quantities.** Per the MS2 invariant
  (`spec/design-rationale.md` references The Design's
  one-locus-tower commitment): every named quantity should be
  assignable to one locus. State that "lives between loci" is
  modeling error; find the right owner.
- **Reaching for foreign patterns.** TOML/JSON inside a locus,
  fluent-builder chains that mutate self, decorators, singletons
  in disguise. The right move is almost always to find the
  existing seed shape that fits.

## When something doesn't match the catalog

The catalog is small by design. If a primitive seems to need a
"module of free fns" / "static class" / "singleton manager
that's not really a service," the author is probably
hallucinating a primitive from another language. The six patterns
above almost always fit — possibly with a workaround for a v1
language gap (next section).

If the author genuinely thinks the catalog is missing a pattern,
the canonical move is to log a friction entry in
`notes/aperio-friction.md` or the app's `FRICTION.md`, with the
smallest reproducible example. The catalog grows from real
friction, not from speculation. See "The friction-log contract"
in `notes/agent-onboarding/app-dev-brief.md` for the format.

## Current language gaps (and idiomatic workarounds)

These are gaps the language is filling in incrementally. The
workarounds below are the idiomatic v1 shape; they go away as the
underlying surface lands.

- **Lifecycle bodies (`birth` / `run` / `dissolve`) reject
  `return`.** Factor short-circuit logic into a free helper fn
  called from the lifecycle method body.
- **No user-defined seeds yet** (only `std::*` and per-app dir
  seeds). Shared code across apps must live in the bundled
  stdlib seed or be duplicated. See `notes/aperio-seed.md` for
  the v1+ plan.
- **No cross-seed imports.** Within one seed (one directory),
  all `.ap` files share a top-level scope; multi-file
  decomposition works (`aperio build apps/<name>/`). Cross-seed
  sharing routes through the stdlib seed or duplicates.
- **No parametric collection types** (no `List<T>`, no
  `Map<K, V>`). Use `@form(vec)` on a locus for contiguous
  growable buffers; future `@form(hashmap)` will cover keyed
  storage. For "list of things" results from helpers, the
  newline-string accumulator pattern or the index-API pair
  (`count` + `at(i)` path-calls, per
  `std::io::fs::list_dir_count` / `list_dir_at`) is the v1
  idiom.
- **No methods on `type` records.** Use a locus with empty
  `params { }` instead. The cost is one alloc per instantiation;
  negligible.
- **No `Option<T>` / `Result<T, E>` value-level error types.**
  For "couldn't compute" cases, return a sentinel
  (`0` / `""` / `-1` / `false` / `nil`) paired with a sibling
  bool predicate (`parse_int` + `can_parse_int`). For true error
  paths where diagnostic context matters, declare the function
  `fallible(E)` (v1.x-FORM-1; free fns and stdlib-synthesized
  `@form(...)` methods only, per the two-channel rule in
  `spec/semantics.md` § "Fallible call semantics").
- **Empty `if` bodies parse-fail.** Put a `// note` comment
  inside, or refactor to a positive condition.
- **`aperio run` rejects qualified-name struct/locus literals**
  (`std::http::Request { ... }`, `std::log::Logger { ... }`,
  etc.). Use `aperio build` and run the resulting binary
  directly for programs that use path-qualified stdlib types.
- **No char-level access on String** (no `s[i]` for a single
  char). Use `s[i..i+1]` for a 1-char slice; compare via `==` or
  `std::str::index_of`.
- **Fn-pointer callbacks can't capture state.** Route state
  through bus subjects, reconstruct state inside the callback,
  or factor into a locus method that has its own `self`.

## Cross-references

- `spec/design-rationale.md` — the *why* behind each shape
  decision; F.0–F.24 numbered commitments.
- `spec/semantics.md` — operational semantics; dissolve timing
  rules, capacity slot lifecycle, fallible call semantics,
  failure traversal.
- `spec/types.md` — type system; interface F.20 surface,
  fallible typing, projection-class types.
- `spec/forms.md` — `@form(...)` annotation system; the v1 form
  library.
- `spec/memory.md` — region-based allocation; per-projection-class
  arena strategies; recognition sub-modes.
- `spec/stdlib.md` — shipped stdlib surface; phase-by-phase
  history.
- `notes/agent-onboarding/app-dev-brief.md` — agent-workflow
  brief for sessions building apps in `apps/<name>/`. The
  styleguide here is normative for code; the brief covers the
  workflow conventions (friction-log contract, hard guardrails,
  first-step protocol).
- `notes/agent-onboarding/compiler-session-brief.md` —
  agent-workflow brief for sessions modifying the compiler
  itself (`crates/`, `spec/`, `docs/`).
- `notes/aperio-types-vs-loci.md` — the source axiom.
- `notes/aperio-seed.md` — what a seed is and what it exports.
