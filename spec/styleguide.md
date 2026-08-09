# Style guide

This document specifies idiomatic Hale in four layers: the
**shapes** a program is composed of, the **correctness rules**
that keep a long-lived program sound, the **speed rules** that
make a hot path allocation-free, and the **enforcement ladder**
the compiler backs them with. Where `grammar.ebnf` says what's
syntactically valid and `types.md` / `semantics.md` say what's
*meaningfully* valid, this document says what's *idiomatically*
valid — what a program should look like when an author has
applied the framework's primitives coherently rather than fought
them.

Every rule here is grounded in production Hale code — the
`pond` contrib libraries, the `causality` game engine, a
production market-data connector fleet, the `bench` performance
corpus, and a downstream market-data service whose leak hunts
drove much of the substrate's reclamation machinery. Rules cite
the shape that earned them.

The styleguide is normative for new code in `apps/`, the bundled
stdlib seed (`crates/hale-codegen/runtime/stdlib/`), and
`examples/`. It is descriptive for older code that predates a
given rule; refactors apply the rules opportunistically.

How to read it: **Foundations** and the **shape catalog** get a
program structured right. **Correctness** keeps it right over
days of uptime. **Fast** applies only where a path is genuinely
hot — the section says so explicitly. **Enforcement** is the
one-table summary of what the compiler checks for you.

---

## 1. Foundations

### The foundational axiom

> **Every named structural thing is a locus.** Types are
> loci-in-waiting — the smallest growth stage on the locus
> gradient.

If a thing has lifecycle, contracts, bus participation, modes,
closures, capacity slots, or projection class, it is a
fully-grown **locus**. If it is pure data (record, returnable by
value, no flow), it is a **type**. The two are points on one
gradient, not separate categories. There is no third primitive.
See `notes/hale-types-vs-loci.md` for the source axiom.

Loci recur at every layer: an app is a locus, a service is a
locus, a spawned worker is a locus, a cache is a locus. There is
no `module` / `class` / `package` keyword because none is needed
— anything one of those would do, a locus does, and the locus
carries lifecycle and contracts the other forms don't.

### Roles, not keywords

Only two declaration forms exist: `locus` and `main locus`.
"App", "service", "namespace", "child", "collection" are **roles**
a locus plays, realized through which members it declares —
`params` / lifecycle / `bus` / `accept` / `capacity` — not
through separate keywords. Don't hunt the grammar for a `service`
or `module` keyword; pick the role from the catalog below and
declare the members that realize it. (Production confirmation:
the connector fleet and `causality` together define every role
in the catalog using exactly the two forms.)

### The memory model in one page

This is the single most load-bearing page in the guide. A
developer who gets every *shape* right and skips this page will
still ship a leak — that is an observed outcome, not a
hypothetical.

**Arenas don't free per-allocation.** Every locus owns an arena
(`self.__arena`). Everything anchored there — struct clones,
String buffers, child structures — lives until the locus
*dissolves*. There is no per-object free.

**Method scratch reclaims at method exit.** Every allocating
method (and bus handler) gets a scratch region created at entry
and destroyed at return. Transients — parse results, temporaries,
locals — allocate into scratch and vanish per call. This is why
ordinary code is allocation-bounded *by construction*: allocate
freely in a method, and the method boundary cleans up.

**Escaping a value costs an anchor.** Storing a heap value into
`self` copies it out of scratch into the locus arena (an
"anchor"). The anchor is the *transfer of lifetime*: the value
now lives as long as the locus.

**Therefore: a long-lived locus means anything anchored
per-iteration accumulates until dissolve** — unless one of the
substrate's reclamation mechanisms applies. The mechanisms, in
the order you should reach for them:

1. **Don't escape.** Keep transients in method scratch. Free by
   construction.
2. **Mutate in place.** `self.f.x = v`, `self.arr[i] = v`,
   String/Bytes reassign that fits the existing buffer — all
   reuse existing storage, zero new bytes.
3. **Replace and let it retire.** `self.f = Struct { ... }`
   whole-value replace memcpys the struct bytes in place, and —
   since v0.11.3 — *retires* the replaced String clones at the
   method's activation boundary, recycling them on the next
   store. Steady-state replace of a scalar/String struct holds
   the arena flat. (Structs carrying `Bytes` / nested compound
   fields aren't fully retired yet — see §7.)
4. **Bound the container.** `capacity` slots, `bounded[T; N]`,
   `@form(ring_buffer)` / `@form(lru_cache)` are cap-bounded by
   type.
5. **Give it to a child.** An `accept`'d child locus owns its own
   arena, reclaimed when the *child* ends — the per-connection
   shape (see rule C4).
6. **Send it.** Bus payloads ride a per-dispatch arena, reclaimed
   after delivery.

One structural consequence to internalize: **a `run()` loop is a
single never-ending activation.** Nothing scratch-based reclaims
inside it, and pending retires never flush. Hot per-iteration
work belongs in a *method called from* the loop — the method
boundary is the reclaim point. (This is why every churn example
in this guide runs its body through a method.)

---

## 2. The shape catalog

Seven idiomatic shapes. Every locus or free fn in a well-written
program matches one; code that doesn't should be reconsidered
against the catalog before shipping.

### 2.1 App locus — outer encapsulation

Every app's `main.hl` defines a top-level locus that owns the
whole run; `fn main()` reads argv, instantiates it, exits.

```hale
locus Onboard {
    params { dir: String = "fixture"; flavor: String = "go"; }
    run() { drive(self.dir, self.flavor); }
}

fn main() {
    let mut dir = "fixture";
    if std::env::args_count() > 1 { dir = std::env::arg(1); }
    Onboard { dir: dir };
}
```

- `params` holds argv-derived configuration with defaults (the
  app self-demos with no flags).
- Statement-position literals fire-and-forget: the locus runs and
  dissolves at fn-return.
- Lifecycle bodies reject `return` — factor short-circuit logic
  into a free helper called from `run()`.

### 2.2 Namespace locus — empty params, methods only

A coherent vocabulary of pure helpers wrapped in a locus with
empty (or config-only) `params { }`. The language's substitute
for "module of functions" / "static class".

```hale,fragment
locus Morpheme {
    params { flavor: String = "go"; }
    fn lookup_morpheme(m: String) -> String { ... }
    fn name_to_motion(name: String) -> String {
        let hit = self.lookup_morpheme(name);
        ...
    }
}
```

Config-only params are fine; the point is **no lifecycle state**.
Promote three-plus related free fns into one of these when the
vocabulary becomes visible; leave unrelated helpers as free fns
(a "util" namespace of strangers is an anti-pattern).

### 2.3 Service locus — long-lived, lifecycle + bus

The full lifecycle for a thing that genuinely runs over time.

```hale,fragment
locus Listener {
    params {
        host: String = "127.0.0.1";
        port: Int = 0;
        listen_fd: Int = -1;
        on_connection: fn(std::io::tcp::Stream) = default_on_connection;
    }
    birth() {
        self.listen_fd =
            std::io::tcp::listen_socket(self.host, self.port) or raise;
    }
    run() { ...accept loop... }
    dissolve() { std::io::tcp::close_fd(self.listen_fd); }
}
```

- `birth()` acquires, `dissolve()` releases, `run()` loops.
- Sentinel params (`-1` for "not yet bound") let `dissolve()`
  no-op safely on partially-constructed loci; make close
  idempotent.
- A service that accepts children long-term needs the
  accept/release pairing — rule C4.

### 2.4 Spawned child locus — let-bound, scope-dissolves

A let-bound locus literal lives for the fn body and dissolves at
scope exit (the m82 dissolve-timing rule).

```hale
fn handle_one_connection(conn_fd: Int, on_conn: fn(std::io::tcp::Stream)) {
    let s = std::io::tcp::Stream { conn_fd: conn_fd, owns_fd: false };
    on_conn(s);
}
```

- Let-bind when the locus should live for the fn's duration;
  statement-position literals dissolve at end of expression.
- **Mind fd ownership**: `Stream` defaults `owns_fd: true` and
  closes its fd at dissolve. A transient Stream wrapped around a
  fd someone else owns (a pooled connection, an accepted raw fd)
  must say `owns_fd: false`, or the wrapper's scope exit closes
  the connection out from under the owner — a bug found in
  production twice (`pond` pq and keepalive both hit it).

### 2.5 Data collection — `@form` locus with a domain facade

Keyed or growable data lives in a `@form(hashmap)` /
`@form(vec)` locus. The idiomatic wrapper is thin **domain
methods over the form's synthesized surface** — a facade that
names the operations in the domain's vocabulary:

```hale
type Sig { key: Int = 0; px: Decimal = 0.0d; seq: Int = 0; }

@form(vec)
locus SigRows { capacity { heap rows of Sig; } }

locus SigList {
    params { rows: SigRows = SigRows { }; }
    fn append(s: Sig) { self.rows.push(s); }
    fn count() -> Int { return self.rows.len(); }
    fn at(i: Int) -> Sig { return self.rows.get(i) or Sig { }; }
}
```

The form locus carries the synthesized surface (`push` / `get` /
`set` / `len` / …); the facade holds it as a field and names the
domain operations. (Domain methods directly on the form locus
also work; the two-locus split keeps the form swappable and the
facade's methods flush-friendly.)

This shape recurs across every production codebase surveyed (a
downstream service defines it four times; `pond` and `bench`
pervasively). Conventions learned there:

- Cell types must be **unqualified in-seed structs** — not loci,
  not qualified paths (F.1 constraint; forces cell decls into
  the same seed).
- `@form(hashmap)` iterates in **bucket order** — add a
  `seq: Int` field if consumers need insertion order.
- `@form(hashmap)` has no delete — model removal with a
  tombstone field (`present: Bool`), the production idiom.
- Scalar `[T; N]` fixed arrays are the zero-alloc alternative
  when the population is fixed: `causality` runs its whole game
  state on SoA fixed arrays and uses **zero** `@form` loci.

### 2.5b Membership set — `@form(set)`

When the question is *is this present* rather than *what is stored
here*, use `@form(set)`. It is the hashmap slot and runtime with a
different surface:

```hale
type Item { key: String; }

@form(set)
locus Seen { capacity { pool items of Item indexed_by key; } }
```

`insert(v)` / `contains(k) -> Bool` / `remove(k)` /
`len()` / `is_empty()`.

The reason not to reach for `@form(hashmap)` with a throwaway value:
membership then reads `get(k) or false` at every call site — the
value plumbing leaking back out of a container whose value nobody
wants. Sync disciplines work identically (`@form(set, sync =
striped)`).

### 2.6 Shape type — pure data, no flow

```hale
type Request { method: String; path: String; body: String; }
```

Returnable by value; no lifecycle. Types may hold `fn(...)`
fields. If methods accumulate on a concept, it has flow — it
wanted to be a locus.

### 2.6b Secret-owning locus — `@sealed`, one classified operation

Key material, a token, anything the rest of the program must never
hold. The shape is not an analysis; it is the ownership model doing
its job with one word added.

```hale,fragment
@sealed locus Signer {
    params { env_var: String = "SIGNING_KEY"; key: Bytes = b""; }

    birth() { self.key = std::bytes::from_string(std::env::var(self.env_var)); }

    fn ready() -> Bool { return len(self.key) > 0; }

    @effects(is: { secret_use })
    fn sign(m: Bytes) -> Bytes {
        if !self.ready() { return b""; }
        return std::crypto::hmac_sha256(self.key, m);
    }
}
```

Four rules, each carrying its weight:

- **`@sealed`.** Loci are otherwise *not* field-encapsulated —
  `self.signer.key` typechecks from anywhere holding the locus — so
  without it "the key never leaves its owner" is something you check
  rather than something that is true. Sealing covers reads *and*
  writes; a locus that stops reads and permits writes lets a caller
  choose the key, which is worse.
- **Take the NAME of a source, not the bytes.** `env_var:` /
  `key_file:`, loaded in `birth`. Param initialization is deliberately
  not restricted — a parent writing `Signer { key: … }` already holds
  what it passes — so if the material arrives through a constructor
  argument, some line of the application held it. `birth` should be
  the only writer, including the no-source case, or a caller can seed
  it anyway.
- **Refuse when unavailable.** `ready()` is not enough on its own:
  every privileged method must consult it, or a misconfigured
  deployment signs under the empty key and anyone can forge the
  matching MAC.
- **Classify the one privileged method** with `secret_use`, so the
  law can find it.

Then the application states its own rule with claim forms that
already exist:

```hale,fragment
claims {
    no_plugin_secrets: forbid reaches(plugins, effects(secret_use));
    one_op_per_request: bound secret_use <= 1 on paths from handlers;
    vault_confined: require sealed(all vaults);
}
```

Prefer `std::secret::Signer` / `Credential` to writing your own —
they are this shape, reviewed once. Write your own when the operation
is not HMAC or credential comparison, and keep it small enough to
read: **the sealed locus's own body is trusted**, and that is the
residual assumption the whole shape rests on.

What this is: confinement. What it is not: information flow. A
signature *derived* from the key is not tracked, and a
constant-time compare still lets the verdict be published.

### 2.7 Error-check fn — value error → structural failure

A locus method that catches a `fallible` call's error and decides
between recovery (substitute a value) and escalation (drain and
notify the parent) pairs an `or self.method(err)` clause with an
`epoch inline` closure:

```hale,fragment
locus DbConnection {
    params { conn_fd: Int = -1; last_error: String = ""; }
    bus { subscribe ExecuteQuery as on_query; publish QueryResult; }

    closure fatal_io { captures: last_error; epoch inline; }

    fn handle_io(e: DbError) -> Row {
        self.last_error = e.detail;
        if e.kind == "send_failed" { violate fatal_io; }
        return Row { data: "" };
    }

    fn on_query(q: Query) {
        let r = send_query(self.conn_fd, q) or self.handle_io(err);
        if !self.draining { QueryResult <- r; }
    }
}
```

- The error-check fn is the **one named site** owning both the
  audit-state update and the escalation choice.
- `violate NAME;` diverges: the runtime synthesizes a
  `ClosureViolation`, sets `draining`, routes to the parent's
  `on_failure`. Guard downstream sends with `if !self.draining`.
- The two-channel rule: substrate-facing surfaces (lifecycle,
  modes, closure assertions, bus handlers) cannot declare
  `fallible(E)`; user-declared `fn` members and free fns can.
  See `spec/decisions.md` F.27.
- Loci whose methods can't return through the value channel use
  the **`last_error` scratch convention**: `last_error` /
  `last_kind` / `last_errno` fields the caller reads after a
  sentinel return (`pond` logfmt's file sink is the reference
  shape).

### Naming conventions

| Construct | Convention | Example |
|---|---|---|
| Locus / type | PascalCase | `Listener`, `Request` |
| Method / field | snake_case | `name_to_motion`, `listen_fd` |
| Lifecycle decl | drop the `fn` keyword | `run() { ... }` |
| Free fn | bare snake_case | `drive`, `handle_one_connection` |
| Bus subject | dot-separated, lowercase | `log.app.db` |
| Constants | UPPER_SNAKE_CASE | `STDLIB_AP_SOURCE` |

Library exports: pick short lowercase import aliases; name decls
to read naturally under the alias (`fin::Quote`, not
`fin::FinQuote`). See `spec/projects.md` for the seed model.

### This document is tested

`spec/styleguide.md` is the most prescriptive file in the repo, and
until 2026-08-03 nothing checked a line of it. That is how §7 came to
assert that a generic payload enum "compiles, constructs, and matches
today" when it does not construct — the claim was written once and
never executed again.

Two gates now run in the normal suite:

- **Snippets.** Every ```hale block must parse *and typecheck*.
  Partial sketches opt out with ```hale,fragment; deliberately-wrong
  anti-patterns use ```hale,counter, so a reader of the source can
  tell "incomplete" from "wrong on purpose".
  (`hale-types/tests/styleguide_snippets.rs`)
- **Claims.** §7's assertions about what the language does *not* do
  are pinned as tests. Absence is the thing nobody re-tests, so when a
  gap closes the prose keeps saying it is open. If one of these
  changes, the test fails and names the entry to update.
  (same file, plus `hale-cli/tests/styleguide_claims.rs` for the
  claims that are codegen limits rather than typecheck limits — a
  split worth knowing, because those are invisible to `hale check`.)

If you add a claim about the language here, add the test with it.

### Canonical form — `hale fmt` is the arbiter

Mechanical style is not a judgment call: `hale fmt` (v0.11.9,
spec/testing.md) defines the canonical form, zero config, and
`hale fmt --check` gates it in CI. Don't hand-enforce anything in
this list — run the tool:

- 4-space indentation, bracket-driven; a closer returns to its
  opener's line indent; continuation lines (a leading `&&`/`.`, a
  trailing operator) get one level.
- Binary operators spaced; unary `-`/`!` tight; `.`/`::`/`..`
  tight; nothing inside `(` `)` `[` `]`; literal braces spaced
  (`Rec { key: 1 }`, `{ }`); `:` tight-left except the spaced
  `locus X : serves P` conformance colon; generics tight
  (`Holder<Int>`); lifecycle parens tight (`run()`).
- Blank lines collapse to at most one; no alignment padding
  (`let x   = 1;` becomes single-spaced — don't build alignment
  columns, they churn every diff); exactly one trailing newline.
- Comments stay where you put them: own-line comments indent with
  the code, trailing comments sit one space after it.

What fmt deliberately does **not** decide stays yours, and is
where the rest of this guide applies: *where lines break* (fmt
preserves your line structure, gofmt-style — there is no
max-line-length rewrap), naming, comment content, and blank-line
placement (it collapses runs, it never inserts or removes the
single separators you chose). Write the shape; let the tool make
it canonical. For agents this is load-bearing: canonical form
means a model-written diff touches only the lines it changed.

### Rolling the design

The catalog is small on purpose. A new primitive must **roll
into** the seed: mirror an existing shape (a reader who knows one
knows it at a glance) *and* interlock in composition (its outputs
are valid inputs to existing consumers). A primitive that needs a
paragraph of "this one works differently" is a new category, not
a rolled one — log the friction, don't ship the invention.

---

## 3. Correctness rules

Each rule is tagged with its enforcement status:
**[error]** the compiler rejects it, **[warn]** the compiler
warns by default, **[@hot]** checked only inside `@hot` fns,
**[@effects]** checked when the fn declares the matching effect
assertion (`@effects(none: {…})` or its `@no_*` sugar),
**[convention]** nothing checks it — the guide is the enforcement.

### C1. Value ownership on assignment — trust it, know the edges

Assignment of heap values into self-storage has **value
semantics with single ownership**: every slot exclusively owns
its blobs. `self.g = self.f` copies; a struct literal embedding a
`self.<field>` read copies; replacing the source never corrupts
the destination. You don't defend against aliasing — the
compiler/runtime maintain the invariant (and anchor retirement
depends on it). **[structural — the substrate handles it]**

The edge that remains yours: **views**. `.view()` /
`.text_view()` are non-owning windows over a builder's buffer,
valid only until the next overwrite/recv. Persist one with
`std::str::clone` / `std::bytes::clone`; on reuse-buffer
protocols adopt the *deferred-clear discipline* — clear the
builder at the *start* of the next cycle, not the end of this
one, so views handed out stay valid between frames (`pond`
websocket's client is the reference). **[convention]**

### C2. Fd and resource ownership

- One owner per fd. Wrappers over borrowed fds declare
  `owns_fd: false` (see 2.4). **[convention]**
- `birth()` acquires / `dissolve()` releases, with sentinel
  defaults so partially-built loci dissolve safely; closes are
  idempotent. **[convention]**
- Injected loci (a locus passed in via params) are
  **caller-owned**: don't dissolve what you didn't instantiate.
  **[convention]**

### C3. Placement and starvation

- Blocking I/O belongs on `pinned` (its own thread) or a
  `where async_io` pool (parks on readiness). A blocking call on
  a classic cooperative pool stalls every co-scheduled locus —
  the compiler traces this **interprocedurally** and warns; a
  blocking cooperative *subscriber* is a dead receiver and is an
  error. **[warn / error]**
- **One non-returning `run()` per classic cooperative pool** —
  a second never starts. Give daemons their own pool or pin
  them. **[convention]**
- **Declare non-returning inline children last.** Params birth in
  declaration order and an inline-on-main child's `run()` runs
  synchronously during its own birth — so a keep-alive child
  declared early means every later param is never **born**: its
  `birth()` never runs, its subscriptions and sockets never
  exist, and the process looks like it booted and idles. Declare
  the blocker last, or move it off the main thread (`pinned` /
  a non-`main` pool) — only the blocker's placement matters. The
  compiler warns where it can prove the shape; absence of the
  warning is not a guarantee. **[warn]**
- A single-writer state locus read from other pools: pin it, and
  poll scalar fields across pools rather than sharing heap
  values. **[convention]**
- TLS I/O never goes on an async_io pool (its recv blocks the
  worker; no park integration yet — see §7). **[convention]**

### C4. accept / release — flows, not residents

Declaring `accept(c: Child)` without `release(c: Child)` makes
every accepted child a **resident**: it lives until the accepting
locus dissolves. On a daemon that's O(accepted-children) growth
until OOM — the canonical accept-loop leak. Declare
`release(c: Child)` to make children **flows**, reclaimed when
their `run()` completes; or `terminate` them from a handler.

```hale,fragment
locus Conn {
    params { fd: Int = -1; }
    run() { while true { let f = recv(...); if f.closed { return; } ... } }
}
locus Server {
    accept(c: Conn) { }
    release(c: Conn) { }   // ← Conn is a flow: reclaim on run() return
}
```

The compiler warns on accept-without-release when the accepting
locus's `run()` loops forever; run-to-exit programs accepting a
bounded batch stay silent. **[warn]**

### C5. Bus discipline

- Declare topics next to their publisher; `hale check` verifies
  the pub/sub graph — a subscription nothing publishes to warns
  (its handler can never fire), and a cooperative subscriber
  whose `run()` blocks is a dead receiver **error** (the
  blocking call starves the dispatch that would deliver to it).
  **[warn / error]**
- Payloads are declared types, not raw Strings — the payload
  type is the contract. **[convention]**
- Delivery is synchronous at the publish site (match + enqueue);
  handlers run when the target's queue drains. Don't assume a
  handler ran because the publish returned. **[convention]**
- **Single-writer collectors**: fan-in by having N publishers and
  one subscriber append to its own private field — never share a
  mutable collection across loci. **[convention]**
- **Keyed routing replaces handler-side filtering.** When N
  instances each want their own slice of a topic's traffic, key
  the topic and filter at the subscription:

```hale,fragment
  topic Posted { payload: Msg; keyed_by room; }
  locus Room {
      params { name: String = "lobby"; }
      bus { subscribe Posted as on_post where key == self.name; }
      fn on_post(m: Msg) { ... }   // only this room's traffic arrives
  }
  ```

  Keys may be `Int` / `Decimal` / `Time` / `Duration` / `Bool` /
  no-payload enum / `String` (String keys are hash-gated —
  non-matching traffic costs one integer compare per entry). An
  `if m.room == self.name` in the handler is the anti-pattern
  this exists to delete: it makes every instance pay for every
  message. Keys are **captured by value at registration** —
  re-keying means dissolve + re-instantiate. `on_unmatched:`
  picks the no-match policy (swallow / fail / fallback).
  **[convention; the routing itself is checked]**
- Inline the payload at the `<-` send site (`T <- Msg { ... }`
  publishes from a stack slot). Routing a payload through a free
  fn first anchors it in the caller's region — a per-publish leak
  `causality` hit in production. **[convention]**

### C6. Dispatch shape

`else if` chains and `match` (statements *and* expressions, with
String / enum-payload / guard arms) are all first-class. A
command router over a String field is a `match`:

```hale,fragment
fn on_command(f: Frame) {
    match std::json::find_string_field(f.json, "type") {
        "hello" -> self.on_hello(f),
        "join" -> self.on_join(f),
        "research" -> self.on_research(f),
        _ -> println("unknown command"),
    }
}
```

Deep `} else { if ... } }` ladders (a 16-deep one shipped in
`causality` before String-match landed) are a legacy shape —
rewrite on touch. F.18 exhaustiveness is checked in both
positions; expression-position arms must agree on one type.
**[error for non-exhaustive / mismatched arms]**

### C6b. Prefer `only:` to an enumerated `none:`

```hale,fragment
@effects(only: { alloc })          // closed — future-proof
@effects(none: { syscall, block, publish, time, entropy, env,
                 ffi, spawn, recursion })   // open — rots
```

`none:` lists what is forbidden and permits everything else, so a
contract meaning "this handler only allocates" has to enumerate every
other class — and the list is a snapshot. Add a class to the language,
or declare one of your own with `effect NAME;`, and every such
contract silently permits it. The annotation still reads "only alloc"
and no longer means it. Nothing fails; the certificate quietly
weakens.

`only:` states the permitted set and is checked against the complement
computed at check time from the classes that actually exist. Nothing
is written down that can go stale. **[convention — reach for `none:`
only when you genuinely care about one or two classes]**

### C7. `@form` constraints (the sharp edges of 2.5)

- Cell types: unqualified in-seed structs only. **[error]**
- Hashmap iteration is bucket-order → `seq` field for order;
  no delete → tombstone. **[convention]**
- A fixed-array *field* (`[Int; N]` on a struct) is out-of-line
  storage — it dangles across a zero-copy SHM boundary even
  though typecheck accepts it; SHM payloads need scalar fields.
  **[convention — see §7]**

---

## 4. Speed rules

**First ask: is this path hot?** Hot means a per-frame/per-message
handler, a tight parse loop, an ingest path — hundreds per second
and up. If not, stop here: the defaults are correct *and* fast
(method scratch reclaims per call; the compiler elides scratch
for non-allocating helpers entirely). Gold-plating a cold path
buys nothing and costs shape.

For genuinely hot paths, the idioms below are the difference
between flat RSS at 100k msg/s and a daemon that dies nightly.
Each is production-derived. Certify the path when done — S10.

### S1. Reuse buffers via params fields

The single highest-value idiom. A locus that processes frames
holds its buffers as fields and reuses them every frame:

```hale,fragment
locus WsConn {
    params {
        rx_buf: std::bytes::BytesBuilder = ...;   // frame reassembly
        tx_buf: std::bytes::BytesBuilder = ...;   // send assembly
        scratch: std::bytes::BytesBuilder = ...;  // unmask/inflate
    }
}
```

A locus or `BytesBuilder` instantiated *inside* a handler or loop
allocates a fresh arena/buffer every message (~4.5 KB/frame
measured in production) that reclaims only at method return — and
its chunk only at dissolve. The compiler warns on this shape by
default. `pond` websocket (three reused builders), the downstream
service's conn writer, and `causality`'s runner all converged on
the fields shape independently. **[warn; error under @hot]**

### S2. `recv_into`, not allocating recv

`recv_into(fd, buf, max)` appends into a reused builder's tail —
zero-alloc. The allocating `recv` family returns a fresh buffer
per call; in a loop that accumulates until the method returns.
**[warn in loops]**

### S3. Zero-copy views with the deferred-clear discipline

Read assembled data with `.view()` / `.text_view()` instead of
`snapshot()` / `finish()` — the latter copy the builder's whole
contents per call. Views are valid until the next overwrite;
C1's deferred-clear discipline makes them safe across frames.
**[@hot: snapshot-in-loop is flagged]**

### S4. Mutate scalars in place; replace structs judiciously

In-place scalar mutation (`self.state.px = v`) is
allocation-free. Whole-struct replace (`self.state = State{...}`)
is *correct* — the struct bytes memcpy in place and replaced
String clones retire (v0.11.3) — but each store still pays an
anchor-clone + retire per String field. On a certified-hot path
prefer in-place writes for the fields that changed; keep the
replace when most fields genuinely change (construction-position
init is legitimate now that retirement reclaims it).
**[@hot hint]**

One shape to know: re-anchoring a field only on *transition*
(e.g. a websocket message's `kind` re-anchors only on a
text/binary flip, not per frame) keeps replace costs off the
per-frame path entirely — the `pond` client does this.

### S5. Strings/Bytes that fit, stay in place

`self.f = <String/Bytes>` reuses the existing buffer when the new
value fits. Bounded-variance fields (timestamps, fixed headers,
checksums) hit the memcpy path forever. Grow-paths clone (the
abandoned String buffer retires; Bytes doesn't yet — §7), and
oscillating lengths degrade capacity — genuinely variable-length
hot fields belong in a reused `BytesBuilder` + view instead.
**[convention]**

### S6. Pre-render once, fan out many

When one event fans out to N consumers, render the common
payload once at ingest and let each consumer add only its
per-consumer delta (a sequence prefix, a connection id). N× the
delta beats N× the render — the downstream service renders its
update JSON once and its N connections each prepend a `seq`.
**[convention]**

### S7. Two-flavor emitters

Build human-path output with `std::json::Builder` (a locus —
fine in cold paths), and hot-path output with raw concat into a
reused buffer. Keeping both flavors side by side (`render` /
`render_pre`) documents which callers are hot. **[convention]**

### S8. Counted loops unlock BCE; helpers are free

`for j in 0..coll.len()` (exclusive, literal-0 lower bound, no
mutation of `coll` in the body) lets codegen elide the per-`get`
bounds check and vectorize — measured in `bench`'s form-vec
microbenches. And factor freely: non-allocating fns/methods cost
nothing (interprocedural scratch elision since 2026-06-28) — hot
code does not need to be monolithic code. **[convention]**

### S9. Flat payloads; batch drains

Transitively pointer-free payload types skip the serialize/
deserialize wire path entirely on cross-thread delivery — prefer
scalar payloads for high-rate topics. For high-rate consumers,
a `Drain<T>` handler runs once per queue drain with a zero-copy
`for` over the batch instead of once per message. **[convention]**

### S10. Certify the path

When a hot path is clean, pin it:

```hale,fragment
@hot @budget(alloc_per_call = 0) fn on_frame(m: Frame) { ... }
```

`@hot` turns this guide's warnings into hard errors inside the fn
and enables the stricter hints (S3, S4). `@budget(alloc_per_call
= N)` is the counted contract — the compiler proves the ceiling
transitively through callees and fails the build on violation;
`N = 0` is the zero-alloc certificate. The downstream service
carries `@budget(alloc_per_call = 0)` on its send path, its
gateway ingest, and its state facade — the contracts are also
the documentation. **[opt-in errors]**

### S11. Event-driven ingest, not polling

One reader locus per source, on a `where async_io` pool, parked
on readiness (`udp::Reader`, tcp recv) — no `set_recv_timeout`
poll loops. Measured: ~4 µs p50 wake latency, stable over hours.
Poll-scanner sleeps accumulate tail latency debt that looks like
a runtime problem but isn't. **[convention]**

### S12. Cache cross-locus handles at boot

A hot path that looks something up by string key in another locus
(a metrics counter, a symbol table) resolves the handle **once at
boot** and stores it as a field:

```hale,fragment
params { c_ticks: metrics::Counter; }      // resolved in main(), threaded in
fn dispatch(m: Msg) { self.c_ticks.inc(); }
```

Per-call keyed lookups walk a hashmap and allocate a search
temp in scratch — fine cold, waste hot. (Same idiom as Go/Rust
metrics code; not Hale-specific.) **[convention]**

### What's already free — don't optimize these

- Scalar assigns (`Int`/`Float`/`Bool`/`Decimal`/`Time`/
  `Duration`), `LocusRef` stores, view assigns, `Cell` fields.
- String fields holding **static literals** — `.rodata` pointers
  short-circuit every clone.
- The metrics RMW shape (`e = store.get(k); store.set(E { key:
  e.key, ... })`) — same-arena skips make it zero-alloc.
- Fresh literals in `return` position (sret routes them straight
  into the caller's arena). Returning via a `let` binding first
  forfeits that routing — inline the literal on hot paths.
- `hashmap.set` replacing an existing key — anchored in place,
  replaced clones retire.

### Raw-fd free fns vs `Stream`

Both work. The `Stream` locus methods are the default idiom —
they carry `owns_fd` semantics and fallible returns
(`fallible(IoError)`); the `__`-prefixed free fns
(`__send_bytes` / `__recv_bytes` / …) are the raw escape hatch
for pooled/borrowed fds where locus lifecycle would fight the
ownership (note their return convention: `0` = success, `-1` =
error — not a byte count). Historical reports of the free fns
crashing natively do not reproduce on ≥ v0.10.0 and the surface
is regression-tested. **[convention]**

---

### S13. Compose with chains, not hand-rolled loops

```hale,fragment
// idiomatic
let n = self.users.filter(it.active).count();
self.users.filter(it.active).into(self.actives);
let total = self.users.map(it.age).sum();
let oldest = self.users.max(it.age) or raise;
self.users.filter(it.active).each { self.greet(it); }

// not this
let mut i = 0;
let mut n = 0;
while i < self.users.len() {
    let u = self.users.get(i) or { break; };
    if u.active { n = n + 1; }
    i = i + 1;
}
```

A chain is not a value being built — it is rewritten to exactly the
loop on the right, so the two have identical cost. Prefer the chain
because it is shorter to read, and because stages fuse: two `filter`s
are one pass, where the hand-rolled version invites an intermediate.

Three properties make chains safe on a hot path, and they are worth
knowing rather than assuming:

- **Zero allocation.** A chain is legal under
  `@budget(alloc_per_call = 0)` and `@hot`. Nothing is materialized
  until a terminal that says so (`into`).
- **Eager.** A predicate's effects are attributed to the predicate's
  own line, not the terminal's. There is no deferred execution to
  mislead a witness path.
- **No lambdas.** `it` is bound by the rewrite; the predicate is
  syntax, not a value. Nothing captures, so nothing escapes.

Whole-set operations (`sort`, and later `reverse` / `group`) cannot
fuse — they need every element before producing any — so they
materialize into caller storage. That cost is visible in the budget,
which is the point.

---

## 5. The enforcement ladder

The compiler backs this guide in tiers. Everything default is
advisory or genuinely-broken-only; strictness is opt-in where you
certify a path — plus one tier you get for free because your
*placement* already declared the intent.

| Tier | Check | Status |
|---|---|---|
| **error** (default) | blocking `run()` on a cooperative *subscriber* (dead receiver); unowned subscriber in a handler; non-exhaustive / type-mismatched match; `@form` cell constraints | build fails |
| **warn** (default) | unbounded-alloc survey (self-escaping allocs in unbounded contexts; retirement-aware since v0.11.3); subscription nothing publishes to; locus/builder in a loop **or bus handler**; allocating recv in a loop; blocking call on a cooperative pool (interprocedural); accept-without-release on a daemon | advisory |
| **@hot** (opt-in) | all of the above as errors within the fn; `snapshot()`/`finish()` in a loop; whole-struct self-field replace | errors in certified fns |
| **@budget** (opt-in) | `alloc_per_call = N` counted transitively; `N=0` zero-alloc certificate | build fails on violation |
| **@effects** (opt-in) | `@effects(none: {syscall, block, time, entropy, env, ffi, publish, spawn, recursion})` / `@effects(publish: {T})`, and the `@no_*` / `@deterministic` sugar — checked transitively; diagnostics carry the call chain to the offending leaf | build fails on violation |
| **@sealed** (opt-in) | a sealed locus's `params` are reachable only from inside its own methods — reads and writes both. Loci are otherwise not field-encapsulated, so this is the only tier that makes state confinement structural rather than reviewed. `hale check --sealable` reports what taking it would cost | build fails on violation |
| **placement-implied** (automatic) | a handler on a `cooperative(pool = X) where async_io` locus that reaches a blocking call — the placement *is* the assertion, so no annotation is needed; writing `@no_block` upgrades it to an enforced error | advisory |
| **fmt** (CI gate) | `hale fmt --check` — canonical mechanical form (§2, "Canonical form"); exit 1 lists offenders | gate in CI; `hale fmt` fixes |
| escape hatches | `@unbounded` (fn or lifecycle hook) acknowledges intentional accumulation; `--allow-unowned-subscriber`; `--no-warn-unbounded-alloc` | |
| advisory tools | `hale check --sealable` (which loci could seal today); `hale check --strict-secret` (the fail-closed `@secret` walk — loud by design, because one body's reasoning is not a containment proof) | opt-in reports |

### Where law lives

The tiers above certify a *function*. Claims certify a *system*, and
the only real style question is which closed world owns each
sentence.

Two claim verbs quantify over the whole closed world rather than over
a path, which makes them the right shape for a baseline a team adopts
once: `require sealed(all G)` (every locus in the group keeps its
state to itself) and `require attributed(all C)` (every fn that
directly performs built-in class `C` names a user-declared purpose).
Both cover code nobody has written yet, which a group-scoped claim
does not — that is the reason to prefer them for policy, and to keep
`forbid reaches` for the specific edges a design forbids.

- **In the `main locus`** — law about this application. A claim is
  evaluated in a closed world, so it belongs where the world closes.
  Name it for the requirement (`orders_pass_risk`), never for the
  mechanism (`no_call_from_a_to_b`): the name is what a reviewer
  reads in a diff when someone weakens it.
- **In a `constitution`** — the same sentence needed by several
  entrypoints. Authorship moves; evaluation does not. Each adopting
  main still proves it in its own world, so `adopt Core;` is not a
  shortcut around closure. Composition is union-only: a stricter rule
  is a second clause, never a replacement, so a derived constitution
  can only add law.
- **In a library's `claims { }`** — obligations about the surface the
  library exports, re-evaluated in every importer.
- **In a fleet plan** — law no single binary can state, because it is
  about the arrangement. `count publishers == 1` inside one program
  is a different sentence from "one deployed publisher"; write the
  second where instances exist.

Two habits worth keeping. Declare the vocabulary a claim quantifies
over — an undeclared group is an error, and a group that is
legitimately empty says so with `may_be_empty` rather than being
allowed to hold vacuously. And read `uncertified` as work to do, not
as a pass: it means the model has a hole where the claim needed to
look.

Resource budgets (`--dump-resource-budget`,
`--check-resource-budget <toml>`, `--warn-resource-leak`) gate
thread / pool / subject / fd counts in CI — see
`spec/verification.md`.

---

## 6. Anti-patterns

- **Bare `fn main()` with helpers and no outer locus.** Apps are
  loci.
- **Handler-side topic filtering** (`if m.room == self.name`) on
  a fan-out topic — use `keyed_by` + `where key ==` (C5).
- **Loci or builders instantiated per-message** in a handler —
  hoist to fields (S1). The compiler warns.
- **`type` for things that have flow**; methods accumulating on a
  type — it wanted to be a locus.
- **"Util" namespaces of unrelated helpers** — group by
  vocabulary.
- **Fluent builders that mutate self, decorators, TOML/JSON
  config in a locus, singletons in disguise** — foreign patterns;
  find the seed shape that fits.
- **Spawning a bus subscriber unowned inside a bus handler** —
  it dissolves when the handler returns and can never fire.
  Compiler error; `accept` it instead. (In `run()` is fine.)
- **Churning heap state directly in a `run()` loop** — a run loop
  is one never-ending activation; nothing reclaims. Route
  per-iteration work through a method.
- **Deep `} else { if` ladders** — `else if` and `match` are
  first-class; String-match the command routers.
- **Floating quantities** — every named quantity has one locus
  owner. State that "lives between loci" is a modeling error.

---

## 7. Boundaries — deliberate absences, open gaps, sharp edges

Three different kinds of "the language doesn't do X" live here,
and they call for three different behaviors from you: absences
you should stop wanting (the design says no, and names the shape
to use instead), gaps you can expect to close (write the
workaround knowing it's temporary), and sharp edges you step
around (current limitations, no promise either way). Current as
of v0.11.3 (2026-07-17); shipped-and-gone entries are removed on
shipping — keyed String routing, match-as-expression, and
whole-struct replace reclamation all lived here once.

### Deliberate absences — the design says no

Not on any roadmap. Each has a blessed shape; reaching for the
absent thing means fighting the design, not waiting on it.

- **No parametric collection types** (`List<T>` / `Map<K,V>`).
  Collections are loci: `@form(vec)` / `@form(hashmap)` /
  `bounded[T; N]` with the facade shape (2.5). A builtin generic
  container would be a second primitive; the locus axiom says
  there is one.
- **No stdlib `Option<T>`.** Two separate reasons, previously
  conflated here. The *idiom* is deliberate: the blessed
  "couldn't compute" shapes are a sentinel + sibling predicate,
  or `fallible(E)` when diagnostic context matters (free fns,
  stdlib wrappers, and user-declared `fn` members all support
  it). An Option-threading style imports another language's
  error culture; the two-channel design is the native one.

  The *mechanism*, however, IS missing, and this section
  previously claimed otherwise. A generic payload enum
  declares and compiles, but does not construct:
  `type Opt<T> = enum { Some(T), None };` builds, and then
  `Opt::Some(1)` fails with "path call `Opt::Some` in
  expression position". Non-generic payload enums construct
  and match normally — genericity is the whole difference, and
  it is the same generic-instantiation gap behind
  `Vec<User>`. So `Option<T>` is not merely unblessed; it is
  not currently expressible.
- **Fn-pointer callbacks don't capture state.** Loci are the
  language's closures — state lives in a locus with its own
  `self`, or routes through bus subjects. A capturing lambda
  would be an anonymous locus without lifecycle or contracts;
  name it instead.

### Open gaps — expect these to close

Write the workaround knowing it's a placeholder.

- **Bytes / nested-compound fields of a replaced struct don't
  retire** (String leaves do, since v0.11.3). Until then:
  genuinely-churning Bytes fields in a reused `BytesBuilder`;
  nested compound fields in their own locus or flattened.
- **`striped` / `lockfree` `@form` maps don't retire** replaced
  cells — a churned String-bearing cell on those modes still
  accumulates. (`sync = serialized` retires since 2026-08-03:
  reads are owned snapshots cloned into the caller's arena, so
  churned shared maps no longer need to stay single-pool. The
  clone is per String field per read — a read-heavy hot path on
  a String-bearing cell now pays it, so weigh serialized against
  single-pool on read-dominated shapes.)
- **Dynamic per-instance bus subjects** (the conversation-per-
  topic shape). Keyed routing covers the *bounded/known* key-set
  case — including String keys — but an unbounded,
  runtime-created subject set still has no shape.
- **TLS has no async_io integration** — its recv blocks the
  thread, not the coro. Keep TLS off async_io pools (C3) until
  non-blocking TLS reads land.
- **`or fail E { ... }` payloads can't reference `err`.** Wrap
  the fallible call in a helper that catches-and-rebuilds the
  error (the `pond` subprocess wrapper is the reference).
- **Duration arithmetic in expression position** is limited —
  hold clock readings as Int ns from the start (`monotonic_ns`),
  which is also the fast path (no ASCII round-trip).

### Sharp edges — current limitations, step around them

Implementation constraints, not positions; no promise attached.
One is soundness-adjacent and worth knowing cold.

- **Fixed-array struct fields are out-of-line pointers, and
  typecheck accepts them in zero-copy SHM payloads** — where
  they dangle cross-process. This is the sharpest edge in the
  list: the compiler does not stop you. SHM payloads need scalar
  fields (the 512-hand-spelled-fields workaround in `bench`
  marks the pain; a flattening form is future work).
- **`@form` cell types can't be loci or qualified paths** — keep
  cell structs in-seed (C7).
- **Lifecycle bodies reject `return`** — factor short-circuit
  logic into a free helper. Related paper cuts: `-> ()` on a
  non-fallible method fails codegen (omit the return type);
  empty `if` bodies parse-fail (add a comment or invert the
  condition).
- **No char-level `s[i]`** — use `s[i..i+1]` slices,
  `std::str::index_of`, or the UTF-8 accessors
  `std::str::cp_at` / `cp_size` / `cp_count` when you need code
  points rather than bytes. There is still no `Char` type;
  `cp_at` yields the code point as an `Int`.

If the catalog seems to be missing a pattern, log a friction
entry with the smallest reproducible example — the catalog grows
from real friction, not speculation. (That's also how entries
move between the buckets above: friction against a *deliberate
absence* is a design conversation, not a feature request.)

---

## Appendix A. Leak-hunt diagnostics

The workflow that pins a leak, in escalation order:

```bash
# 1. Confirm + identify the growing arena (1 Hz heartbeat dump)
LOTUS_ARENA_RESIDENCY=1 ./binary
#    + call std::process::dump_arena_residency() from a hot path

# 2. Log every chunk attach with arena labels
LOTUS_ARENA_LOG_CHUNK_ATTACH=4096 LOTUS_ARENA_LOG_BIG_MAX_EVENTS=0 \
  LOTUS_ARENA_RESIDENCY=1 ./binary
#    grep 'kind=root label=<grower>' → the allocating call site

# 3. Pool stats at thread exit (atexit-only — long-running daemons
#    must exit early to capture; add a fixed-duration run mode)
LOTUS_CHUNK_POOL_STATS=1 ./binary

# 4. Static sweep — the model-side view of the same question
hale check app.hl                       # unbounded-alloc survey is default-on
hale check app.hl --dump-alloc-summary
```

Compose 1 → 2 → source grep; that exact chain pinned the May-2026
per-instance residuals. Full env-var reference:
`spec/runtime.md` § "Diagnostic env vars". The compiler-side
ladder (§5) is the static complement — prefer fixing the shape it
names over chasing the trace.

## Cross-references

- `spec/design-rationale.md` — the *why* behind shape decisions.
- `spec/semantics.md` — dissolve timing, capacity slots, fallible
  semantics, routing keys, failure traversal.
- `spec/memory.md` — the substrate reclamation contract (the
  normative version of §1's one-page model).
- `spec/verification.md` — the checks behind §5's ladder.
- `spec/forms.md` — the `@form(...)` library.
- `docs/src/systems/performance.md` — the pedagogical companion
  to §4.
- `AGENTS.md` — the agent workflow; `agents/compiler-dev.md` for
  compiler sessions.
- `notes/hale-types-vs-loci.md` — the source axiom.
