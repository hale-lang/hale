# The Aperio styleguide

> Companion to `app-dev-brief.md`. The brief is the **what** —
> "what is Aperio, and how do you not hallucinate Rust at it." This
> styleguide is the **how** — "how do you write idiomatic Aperio
> when you do sit down to compose a spell."
>
> Designed for both LLM agents (loaded into cold context before
> writing code) and humans (a reference for hand-written work).
> Every pattern carries one concrete example drawn from real
> shipped code so you can ground-truth it without hunting.

## The axiom this whole guide flows from

> **Types are for shapes. Loci are for flow.**

If a thing has lifecycle, contracts, bus participation, or
projection, it is a **locus**. If it is pure data (record,
returnable by value, no flow), it is a **type**. There is no
third category.

These are also the **only two things a seed exports** (see
`notes/aperio-seed.md`). Free fns are seed-internal
implementation; they do not cross seed boundaries. If a free fn
exists at the top level of an app or stdlib file, it is either
(a) a `return`-bearing helper called by a lifecycle method, (b) an
extension hook for a `fn`-pointer param, or (c) a smell that
wants extracting into a method on a namespace lotus.

## The recursive principle

Loci are the fundamental building block at every layer:

- **An app** is a locus. (Outer encapsulation; one per
  `apps/<name>/main.ap`.)
- **A namespace** of pure helpers is a locus. (Empty `params { }`,
  only methods.)
- **A long-running service** is a locus. (Birth/run/dissolve;
  often with `bus subscribe`.)
- **A spawned async worker** is a locus. (Child of its parent;
  cooperative schedule by default.)
- **A bus subscriber** is a locus. (HTTP route, message handler,
  event listener — all the same shape.)
- **A configured cache / pool / pipeline / queue** is a locus.

**Inside any locus, behavior is itself a locus tower one
layer down.** A cache's lookup flow has its own birth (acquire
lock), run (probe + return), dissolve (release lock). The
recursion bottoms at primitive operations — arithmetic, single
field reads, primitive calls. Everything above the floor is
loci nested in loci.

## The pattern catalog

Six patterns; one example each. If your code doesn't match one
of these, ask whether you're doing something the language wants
you to express differently.

### 1. App locus — outer encapsulation

Every app's `main.ap` defines an `<Name>L` locus that owns the
whole run. `main()` reads argv, instantiates the locus, exits.
The locus's `run()` body delegates to a free helper (m82
limitation: lifecycle bodies don't accept `return`, so short-
circuit logic factors out).

Example — `apps/onboard/main.ap:714-734`:

```aperio
locus OnboardL {
    params {
        dir: String = "apps/operational-graph/fixture";
        flavor: String = "go";
    }
    run() {
        __drive(self.dir, self.flavor);
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

Conventions to copy verbatim:

- Locus name is `<FileStem>L` with `L` suffix.
- `params` block holds argv-derived configuration with
  reasonable defaults (so the app self-demos with no flags).
- `run()` is the only lifecycle method needed for most apps.
  Drop the `fn` keyword for lifecycle methods.
- `main()` does the argv parsing, then a single statement-
  position locus literal kicks the run.
- Statement-position literals fire-and-forget: `OnboardL { ... };`
  starts the run and the locus dissolves at fn-return.

### 2. Namespace lotus — empty params, only methods

When a coherent set of pure helpers forms a vocabulary, wrap
them in a locus with empty `params { }` and method-only body.
Use sites instantiate once and dispatch through it.

Example — `crates/aperio-codegen/runtime/stdlib/lang.ap:403`
shows `__StdLangMorpheme`, exposed as `std::lang::Morpheme`:

```aperio
locus __StdLangMorpheme {
    params {
        flavor: String = "go";
        overrides: String = "";
    }
    fn lookup_morpheme(m: String) -> String { ... }
    fn suffix_rule(m: String) -> String {
        // Self-method calls compose within the namespace:
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

Notes:

- "Empty params" doesn't have to be literally empty —
  `Morpheme` carries `flavor` and `overrides` because they
  parameterize the lookup. The point is **no lifecycle
  state** that birth/run/dissolve would mutate.
- One alloc per instantiation. Negligible.
- Self-method calls (`self.X(...)`) work and are how methods
  compose internally.

### 3. Service locus — long-lived with lifecycle + bus

When the thing genuinely runs over time and participates in
the bus, write the full lifecycle.

Example — `crates/aperio-codegen/runtime/stdlib/io_tcp.ap:92` is
the canonical TCP listener:

```aperio
locus __StdIoTcpListener {
    params {
        host: String = "127.0.0.1";
        port: Int = 0;
        listen_fd: Int = -1;
        max_accepts: Int = 1;
        on_connection: fn(std::io::tcp::Stream) = __default_on_connection;
    }
    birth() {
        self.listen_fd = std::io::tcp::__listen_socket(self.host, self.port);
    }
    run() {
        let mut accepted = 0;
        while self.max_accepts < 0 || accepted < self.max_accepts {
            let conn = std::io::tcp::__accept_one(self.listen_fd);
            __handle_one_connection(conn, self.on_connection);
            accepted = accepted + 1;
        }
    }
    dissolve() {
        std::io::tcp::__close_fd(self.listen_fd);
    }
}
```

Conventions:

- `birth()` does setup that needs to run before any work.
  Mutate `self.field` to record acquired resources.
- `run()` does the long-lived work. Often a loop that consumes
  some bounded budget or runs forever depending on
  configuration.
- `dissolve()` releases what `birth()` acquired. Both are
  required if either is.
- Sentinel values in `params` (`-1` for "not yet bound") let
  `dissolve()` safely no-op on partially-constructed loci.

### 4. Spawned child locus — let-bound, scope-dissolves

When a parent locus's `run()` produces work that needs its
own lifecycle (a per-connection handler, a per-task worker),
let-bind a locus literal. m82's deferred-dissolve fires the
child's `dissolve()` at the parent fn's scope exit.

Example — same file, `__handle_one_connection` near line 67:

```aperio
fn __handle_one_connection(conn_fd: Int, on_conn: fn(std::io::tcp::Stream)) {
    let s = std::io::tcp::Stream { conn_fd: conn_fd };
    on_conn(s);
}
```

The `let s = ...` binds the Stream locus to the fn's scope; when
`__handle_one_connection` returns, `s.dissolve()` fires (which
closes `conn_fd`). No explicit `dissolve(s)` call needed.

Conventions:

- Use **let-binding** when the locus needs to live for a fn
  body's full duration. Statement-position literals fire and
  dissolve at the end of the *expression*, which is rarely
  what you want.
- Per-iteration cleanup uses a free helper fn whose return is
  the per-iteration boundary (as `__handle_one_connection`
  does for the listener loop). Block-level deferred-dissolve
  isn't shipped; this is the workaround.

### 5. Shape type — pure data, no flow

When a thing IS data, not flow, declare it as `type`. No
lifecycle, no contracts, no bus.

Example — `crates/aperio-codegen/runtime/stdlib/http.ap:32-50`:

```aperio
type __StdHttpRequest {
    method: String;
    path: String;
    version: String;
    body: String;
}

type __StdHttpResponse {
    status: Int;
    content_type: String;
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
- All fields named with snake_case.
- Returnable from fns by value (m84). No lifecycle implications.
- If you find yourself adding methods to the type, you've
  discovered it's actually a locus — change the keyword.

### 6. Free fn — sparingly

Free fns at the top of a file are appropriate in three cases
and only three:

1. **`return`-bearing helpers** called from lifecycle method
   bodies (which cannot themselves use `return` at v0). See
   the App locus pattern's `__drive`.
2. **Extension hooks** passed via fn-pointer params (e.g.,
   `on_connection: fn(Stream)`). The hook is named at the top
   level so a caller can pass it by name.
3. **Genuinely-isolated helpers** that don't fit any namespace.
   Rare. If three such helpers accumulate, they probably form
   a namespace lotus you haven't extracted yet.

Free fns are NOT exported across seed boundaries. Anything you
want another seed to call should live as a method on a locus.

## Naming conventions

| Construct                | Convention                              | Example                  |
|--------------------------|-----------------------------------------|--------------------------|
| Locus (any kind)         | `<Name>L` suffix                        | `OnboardL`, `MorphemeRewriterL` |
| Type (shape record)      | PascalCase, no suffix                   | `Request`, `Response`    |
| Stdlib mangled internal  | `__Std<Domain><Name>`                   | `__StdHttpRequest`       |
| Locus method / type field | snake_case                             | `name_to_motion`         |
| Lifecycle method         | drop `fn` keyword                       | `run() { ... }`          |
| Free helper fn           | `__name` (leading underscores)          | `__drive`, `__walk`      |
| Bus subject              | dot-separated, lowercase                | `log.app.db`             |
| Constants                | UPPER_SNAKE in stdlib; rare elsewhere   | `STDLIB_AP_SOURCE`       |

The leading-`__` on free helpers is doing two jobs: (1) marking
them as "implementation detail, don't call across seed
boundaries"; (2) avoiding name conflicts with stdlib path-call
dispatch in user code. Once user-defined seeds ship and the
manifest is the export source of truth, this convention may
relax for non-stdlib code.

## Composition patterns

- **Self-method calls** (`self.method(arg)`) compose within a
  locus. No special syntax, no virtual dispatch — the receiver
  is implicit because you're inside the locus body.
- **Cross-locus method calls** (`other.method(arg)`) work after
  m81. The receiver is a typed locus reference; methods
  resolve by the locus's declared name.
- **Let-bound locus literals** defer dissolve to scope-exit
  (m82). Use this when the locus's lifecycle should match a
  fn body's duration.
- **Statement-position literals** (`SomeL { ... };` with no
  `let`) fire and dissolve at end of expression. Use this
  when you want a one-shot run with no aftermath.
- **Cross-locus state via bus subjects**, not via field reads
  on a passed reference. The bus is the language-blessed
  channel for cross-locus coordination.

## Anti-patterns

The shape these violate is *almost always* "I'm avoiding the
language's primitive in favor of an old habit from another
language."

- **Bare `fn main()` with helpers and no outer locus.** The
  app's outer encapsulation must be a locus per the apps-are-
  loci rule. (Apps `ts-walk-demo` and `import-graph` were
  retrofitted from this antipattern early in the codebase-
  onboarder arc.)
- **Coherent helper vocabulary stranded as `__free_fns`** when
  it forms a namespace. The Morpheme rewriter started as
  `__split_camel`, `__lookup_morpheme`, `__suffix_rule`,
  `__name_to_motion` — these were lifted into
  `std::lang::Morpheme` once the coherence became visible.
- **`type` for things that have flow.** If the noun has a
  lifecycle implied (a Cache that's loaded/probed/evicted; a
  Server that starts/serves/stops), it should be a locus.
- **Methods on a `type` record.** Not supported at v0 — the
  language tells you "you wanted a locus." Use a locus with
  empty `params` instead.
- **Inappropriate `__` prefix** on something that's part of
  the user-facing API. The prefix marks "don't call this
  across the seed boundary"; if it IS the surface, drop the
  prefix.
- **"Util" namespaces of unrelated helpers.** Group by
  *vocabulary*, not by "everything that didn't fit elsewhere."
  A namespace lotus should answer to one question (e.g.,
  "noun-to-motion" or "tagged-accumulator parsing"), not many.

## v0 friction workarounds

Workarounds for current language gaps. Each will go away as the
language fills in; until then, an agent following this guide
won't repeatedly rediscover them.

- **Lifecycle bodies (`birth`/`run`/`dissolve`) reject `return`.**
  → Factor short-circuit logic into a free helper fn called
  from the lifecycle method.
- **No user-defined seeds yet** (only `std::*` exists). → Shared
  loci must live in the std seed (bundled at codegen) or get
  duplicated in apps. See `notes/aperio-seed.md` for the v1+
  plan.
- **No multi-file Aperio modules.** → An app is a single
  `apps/<name>/main.ap` file; cross-app shared code goes
  through the std seed.
- **No `List<T>` generic.** → Manual newline-string accumulators
  are the v0 idiom for "list of things" (see the
  tagged-accumulator pattern in `apps/onboard/main.ap` or
  `apps/tower-join/main.ap`). Generics are tracked but not
  shipped.
- **No methods on `type` records.** → Use a locus with empty
  `params { }` instead. The cost is one alloc per
  instantiation; negligible.
- **Empty `if` bodies parse-fail.** → Put a `// note` comment
  inside, or refactor to a positive condition.
- **`aperio run` rejects qualified-name literals**
  (`std::ts::*`, `std::lang::*`). → Use `aperio build` then run
  the resulting binary directly. Tracked friction.
- **No char-level access on String** (no `s[i]` for a char). →
  Use `s[i..i+1]` for a 1-char slice; compare via `==` or
  `std::str::index_of`.
- **No `std::str::trim` or `to_lower` builtins.** → Open-code
  in a namespace lotus method (see `__StdLangMorpheme.trim`
  and `to_lower` for the v0 idioms).
- **Fn-pointer callbacks can't capture state.** → Either route
  state through bus subjects, reconstruct state inside the
  callback, or factor into a locus method that has its own
  `self`.

## When something doesn't match the catalog

Stop and ask: am I expressing this with the right primitive?
Aperio's catalog is small on purpose. If you find yourself
needing a "module of free fns" or a "static class" or a
"singleton manager that's not really a service" — you're
probably hallucinating a primitive from another language. Look
again at the six patterns above; one of them almost certainly
fits, possibly with a workaround for a v0 gap.

If you genuinely think the catalog is missing a pattern, log
the case in `notes/aperio-friction.md` with the smallest
reproducible example. The catalog grows from real friction,
not from speculation.

## Cross-references

- `notes/aperio-types-vs-loci.md` — the source axiom.
- `notes/aperio-seed.md` — what a seed is and what it exports.
- `notes/onboarding-shape-rules.md` — the agent-driven model
  that informs how the codebase-onboarder treats foreign code.
- `notes/agent-onboarding/app-dev-brief.md` — the **what** brief
  this styleguide complements.
- `docs/grimoire/src/06-the-same-shape.md` — the perceptual
  primer for "every axis is the same shape, including
  inward."
- `notes/aperio-refactor-proposal.md` — concrete recommendations
  for applying this styleguide to existing code.
