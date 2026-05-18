# Modeling — how to think in Aperio

> **α** — Given the primitives, how do you actually *use* them
> to model a real system?

This is the synthesis chapter. The previous seven cover what
the primitives *are*. This one is about how to compose them
into idiomatic programs — and, just as importantly, what to
do when the language seems to resist your design.

## The one-tower rule

The deepest commitment Aperio makes about modeling is this:

> Every named quantity in your model must be assignable to
> exactly one locus in one locus tower.

This isn't a style guideline. It's structural: every other
guarantee Aperio makes (wholesale-free at dissolve,
vertical-only flow, the closure-violation channel,
deterministic cleanup cascade) depends on each piece of state
having exactly one owning locus. When state floats — when
some buffer is "shared" between two loci, or there's a
global registry nobody owns, or a configuration value "lives
in the environment" — the guarantees unravel at the floating
point.

When the language seems to resist where you want to put a
piece of state, the productive move is not to invent a
workaround. It's to ask: **which locus should own this?**
That question almost always has a structural answer; the
answer is the productive move.

(The [`pond`](https://github.com/aperio-lang/pond) contrib
monorepo ships a *memory-owner-architecture* (MOA) library
under `pond/moa/` that develops this rule into concrete
patterns and helpers for declaring ownership and verifying
the assignment. It's likely to spin out into its own repo
at some point; treat the link as the current pointer rather
than a permanent address.)

## The seven idiomatic patterns

Every well-shaped Aperio program is composed of seven recurring
patterns. If your code doesn't fit one of these, reconsider
before inventing — the catalog is small on purpose, and most
"I need an eighth pattern" instincts turn out to be one of
the seven in a foreign shape.

### 1. App locus — outer encapsulation

Every app's `main.ap` defines a top-level locus that owns the
whole run. `fn main()` reads argv, instantiates the locus,
exits.

```aperio
locus Onboard {
    params {
        dir:    String = "fixture";
        flavor: String = "go";
    }
    run() {
        drive(self.dir, self.flavor);
    }
}

fn main() {
    let mut dir    = "fixture";
    let mut flavor = "go";
    if std::env::args_count() > 1 { dir    = std::env::arg(1); }
    if std::env::args_count() > 2 { flavor = std::env::arg(2); }
    Onboard { dir: dir, flavor: flavor };
}
```

Conventions:

- Locus name is the file stem in PascalCase.
- `params` holds argv-derived config with reasonable defaults
  (so the app self-demos with no flags).
- `run()` is the only lifecycle method needed for most apps.
- `main()` does argv parsing, then a single statement-position
  locus literal kicks the run.

### 2. Namespace lotus — empty params, methods only

When a coherent vocabulary of pure helpers forms, wrap them
in a locus with empty (or config-only) `params { }` and
methods only. Instantiate once, dispatch through it. The
language's substitute for "module of functions" / "static
class" / "stateless service object."

```aperio
locus Morpheme {
    params {
        flavor:    String = "go";
        overrides: String = "";
    }
    fn lookup_morpheme(m: String) -> String { ... }
    fn name_to_motion(name: String) -> String {
        let hit = self.lookup_morpheme(name);
        // ...
    }
}

fn main() {
    let r = std::lang::Morpheme { flavor: "go" };
    let motion = r.name_to_motion("OrderProcessor");
}
```

The point isn't that `params` is literally empty — *no
lifecycle state mutated by birth/run/dissolve.* Config params
are fine. Self-method calls compose within the namespace.
One alloc per instantiation; negligible.

### 3. Service locus — long-lived with lifecycle + bus

When the thing genuinely runs over time and participates in
the bus, write the full lifecycle.

```aperio
locus Listener {
    params {
        host:          String = "127.0.0.1";
        port:          Int    = 0;
        listen_fd:     Int    = -1;
        max_accepts:   Int    = 1;
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

- `birth()` acquires resources; mutates `self.field`.
- `run()` does the long-lived work; often a loop bounded by
  config.
- `dissolve()` releases what `birth()` acquired.
- Sentinel values (`-1` for "not yet bound") let
  `dissolve()` safely no-op on partially-constructed loci.

### 4. Spawned child — let-bound, scope-dissolves

When a parent's work produces children that need their own
lifecycles, let-bind. The let-bound locus's dissolve fires at
the enclosing function's scope exit; the binding stays valid
for method calls in between.

```aperio
fn handle_one_connection(conn_fd: Int, on_conn: fn(std::io::tcp::Stream)) {
    let s = std::io::tcp::Stream { conn_fd: conn_fd };
    on_conn(s);
}
```

The `let s = ...` binds the Stream locus to the fn's scope;
when `handle_one_connection` returns, `s.dissolve()` fires
(which closes `conn_fd`). No explicit cleanup call needed.

Conventions:

- Use **let-binding** when the locus needs to live for a fn
  body's full duration. Statement-position literals dissolve
  at end of expression — rarely what's wanted for a usable
  handle.
- Per-iteration cleanup uses a free helper fn whose return is
  the per-iteration boundary (the example above is exactly
  this pattern).

### 5. Shape type — pure data, no flow

When a thing IS data, not flow, declare it as `type`.

```aperio
type Request {
    method:  String;
    path:    String;
    version: String;
    body:    String;
}
```

Construct via struct literal:

```aperio
let req = std::http::Request {
    method: "GET", path: "/", version: "HTTP/1.1", body: ""
};
```

Conventions: PascalCase, snake_case fields, returnable by
value, no lifecycle implications. Types may hold `fn(...)`
fields — dispatch via `record.field(args)`. If methods
accumulate, the thing has flow — promote `type` to `locus`.

### 6. Free fn — first-class seed member

Free fns are first-class seed members. Every top-level decl in
a seed is visible to every file in the seed. Use a free fn
when the operation has no flow and isn't naturally a method
on an existing locus.

Common shapes:

1. **Return-bearing helpers** called from lifecycle method
   bodies (which reject `return` at v0).
2. **Extension hooks** passed via fn-pointer params (e.g.,
   `on_connection: fn(Stream)`). The hook is named at the top
   level so a caller can pass it by name.
3. **Standalone helpers** that compose with the rest of the
   seed: format / parse / convert / classify utilities that
   don't carry state.

When a coherent vocabulary of three or more free fns forms,
the namespace-lotus form (pattern 2) often reads better.

### 7. Error-check function — bridging the channels

A locus member fn whose signature is `fn(ErrType) -> SuccessType`,
used as the fallback in an `or self.handler(err)` clause at a
fallible call site. Internally, it examines the error and
chooses: return a value (substitute, continue) or
`violate NAME` (escalate to the structural channel).

```aperio
locus DbConnection {
    params { conn_fd: Int = -1; last_error: String = ""; /* ... */ }
    bus { subscribe ExecuteQuery as on_query; publish QueryResult; }

    closure fatal_io { captures: last_error; epoch inline; }

    fn handle_io(e: DbError) -> Row {
        self.last_error = e.detail;
        if e.kind == "send_failed" || e.kind == "recv_empty" {
            violate fatal_io;
        }
        return Row { data: "" };
    }

    fn on_query(q: Query) {
        let r = send_query(self.conn_fd, q) or self.handle_io(err);
        if !self.draining { QueryResult <- r; }
    }
}
```

This is the canonical bridge between the value channel and
the structural channel — see
[Error handling](./error-handling.md) §"Bridging the
channels" for the full treatment.

Conventions:

- **Naming.** snake_case. Name for *what is being handled*,
  not what's being done: `handle_io`, `handle_parse`,
  `handle_timeout` — not `recover_or_die`.
- **Signature.** The return type is the success type of the
  call sites that use this handler. One handler per
  `(ErrType, SuccessType)` pair on a given locus.
- **Body shape.** `if`-chain or `match` on the error kind;
  each arm either `violate`s a named closure or `return`s a
  substitute value. The two motions are exhaustive — the
  typechecker ensures every path either returns the success
  type or diverges via `violate`.
- **Closure references.** `violate NAME` is locus-scoped —
  the named closure must be declared on the same locus. This
  is why the handler is a *member fn*, not a free fn.

## Anti-patterns

The shapes below are *almost always* "an old habit from
another language smuggled past the substrate." When you
catch yourself reaching for one, reconsider.

- **Bare `fn main()` with helpers and no outer locus.** The
  app's outer encapsulation must be a locus per pattern 1.
- **Coherent helper vocabulary stranded as free fns** when it
  forms a namespace. Lift into a namespace lotus once the
  coherence is visible (pattern 2).
- **`type` for things that have flow.** If the noun has a
  lifecycle implied (a Cache that's loaded/probed/evicted, a
  Server that starts/serves/stops), it is a locus, not a
  type.
- **Methods on a `type` record.** Not supported at v0 — the
  language is telling you "this wanted to be a locus."
- **"Util" namespaces of unrelated helpers.** Group by
  *vocabulary*, not by "everything that didn't fit
  elsewhere." A namespace lotus should answer one question
  ("noun-to-motion", "tagged-accumulator parsing"), not many.
- **Floating quantities.** Per the one-tower rule: every
  named quantity should be assignable to one locus. State
  that "lives between" loci is modeling error.
- **Tagged-locus dispatch.** A single locus with a `kind:
  String` param branching on every method, instead of an
  interface and multiple loci. The structural-interface
  primitive (F.20) is the right tool.
- **Fluent-builder chains that mutate self.** If you're
  writing `obj.with(x).with(y).build()`, the thing wanted to
  be a locus with proper `params` and lifecycle.

## A worked example: choosing the model

To make the modeling rules concrete, here's a small system
walked through pattern-by-pattern:

> *"I need a rate-limiter that bounds a downstream service's
> request rate. Requests come in over the bus. When the
> downstream is overloaded, the limiter should emit a
> backpressure signal upstream."*

**Step 1: identify the loci.**

The rate-limiter is a service locus (pattern 3): it has
state (the recent-request window), lifecycle (birth → run →
dissolve), and bus participation. One locus.

What about the downstream service? Probably a separate
locus, also pattern 3. The two coordinate through the bus,
not through direct reference.

The backpressure signal: not a locus, it's an event. A topic
(`Backpressure { payload: ... }`).

The "request": same — a topic (`Request { payload: ... }`).

The "recent-request window": held by the rate-limiter, in a
capacity slot. `@form(ring_buffer)` is the right shape — we
want a bounded window with drop-on-full.

**Step 2: sketch the locus.**

```aperio
type Req     { id: String; ts: Time; }
topic Request      { payload: Req; }
topic Backpressure { payload: Req; }

@form(ring_buffer, cap = 100)
locus RateLimiter {
    params { window_ms: Int = 1000; threshold: Int = 50; }
    capacity { pool recent of Req; }
    bus {
        subscribe Request as on_request;
        publish   Backpressure;
    }
    fn on_request(r: Req) {
        self.recent.push(r);
        if self.over_threshold() {
            Backpressure <- r;
        }
    }
    fn over_threshold(self) -> Bool {
        // ...
    }
}
```

**Step 3: check against the patterns.**

- Pattern 3 (service locus): ✓
- Capacity slot for the window: ✓ (`pool recent of Req` with
  `@form(ring_buffer)`)
- Bus subscribe / publish: ✓
- One-tower: `recent`, `window_ms`, `threshold` all owned by
  `RateLimiter`. No floating quantities.
- Anti-patterns: none.

**Step 4: where would friction surface?**

- If the rate-limiter needs to *track which client* was
  rate-limited, we'd add per-client state — maybe a
  `@form(hashmap)` keyed by client ID. That's a second
  capacity slot, still one-tower.
- If multiple rate-limiters need to coordinate (one per
  service, sharing a global cap), they'd coordinate
  through a parent locus that holds the global budget. Bus
  topic `GlobalBudget` between them.
- If we wanted to deploy the limiter as a separate binary
  from the downstream, we'd add a `bindings` block in main
  to route the Request topic through a Unix socket.

Notice how each "what if" stays inside the pattern catalog.
You don't reach for a new primitive; you compose what you
have.

## A reading order, going forward

You've finished Concepts. The two natural next steps:

1. **Read the [Reference](../reference/language.md) section**
   for the canonical formal definitions of every construct.
   The spec corpus is the source of truth.
2. **Read working examples.** The
   [`apps/`](https://github.com/aperio-lang/aperio/tree/main/apps)
   directory has 11 real programs exercising every pattern
   in this chapter. Pick one close to what you want to
   build and read it end-to-end.

If you're building a multiplayer game, the matchmaker
example from the introduction grows into a complete system —
matchmaker locus, per-match game session loci, terminal
client loci, all composed through the bus. The "Build a real
app" tutorial walks through that build (forthcoming).
