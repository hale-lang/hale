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
   whole-value replace memcpys the struct bytes in place, and
   *retires* what only the replaced value reached at the method's
   activation boundary, recycling it on the next store: its String
   clones (since v0.11.3), its `Bytes`, and its nested records,
   tuples, arrays and payload enums with everything under them.
   Steady-state replace of any struct holds the arena flat, and so
   does a tuple, array or payload-enum field replaced whole. A nested block the new value
   carries over from the field itself (`self.f = F { n: i, inner:
   self.f.inner }`) is kept, not copied; one it takes from anywhere
   else is copied, so a field never shares a block it will retire.
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

Eleven idiomatic shapes. Every locus, type or free fn in a
well-written program matches one; code that doesn't should be
reconsidered against the catalog before shipping.

### 2.1 App locus — outer encapsulation

Every app's `main.hl` defines a top-level locus that owns the
whole run; `fn main()` reads argv, instantiates it, exits.

```hale
locus Onboard {
    params { dir: String = "fixture"; flavor: String = "go"; }
    run() { drive(self.dir, self.flavor); }
}

fn drive(dir: String, flavor: String) {
    println("onboarding ", dir, " ", flavor);
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
- A lifecycle body short-circuits with `return;` as a method does
  (`birth`, `run` and `dissolve` alike).

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

- Cell types are **data, never loci** — a locus cell is refused
  (§7, deliberate absences). A struct from an imported seed or the
  stdlib is a cell type like an in-seed one (`shared::Quote`,
  `std::http::Request`).
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

Four rules, each load-bearing:

- **`@sealed`.** Loci are otherwise *not* field-encapsulated —
  `self.signer.key` typechecks from anywhere holding the locus — so
  this is what makes "the key never leaves its owner" a fact the
  compiler knows rather than one you verify by reading. It covers
  reads *and* writes: a locus whose state can be replaced from
  outside lets a caller choose the key.
- **Take the NAME of a source, not the bytes.** `env_var:` /
  `key_file:`, loaded in `birth`. Param initialization is
  deliberately unrestricted — a parent writing `Signer { key: … }`
  already holds what it passes — so material arriving as a
  constructor argument means some line of the application held it.
  Make `birth` the only writer, including the no-source case;
  otherwise a caller can seed it regardless.
- **Refuse when unavailable.** Every privileged method must consult
  `ready()`. A signer that computes under an empty key returns a MAC
  anyone can forge.
- **Classify the one privileged method** with `secret_use`, so the
  law can find it.

When a revealed credential has to be reshaped on its way out — a client
secret percent-encoded into a form body — reshape it in the statement
that sends it, with what only computes a value (`std::url::encode`, a
conversion, `+`) or a fn of your own built from those: the result stays
the secret, held to the same one statement. A helper that also logs,
stores or calls a method is refused, and the message names that call.
**[error]**

A sealed locus may not also carry `contract { expose … }`: sealing
denies every external read, so the expose grants nothing, and the pair
is rejected rather than left to read as permission. Callers use the
locus's methods.

Then the application states its own rule with claim forms that
already exist:

```hale,fragment
claims {
    no_plugin_secrets: forbid reaches(plugins, effects(secret_use));
    one_op_per_request: bound secret_use <= 1 on paths from handlers;
    vault_confined: require sealed(all vaults);
}
```

Prefer `std::secret::Signer` / `Credential` to writing your own.
Write your own when the operation is neither HMAC nor credential
comparison, and keep it small enough to read: **the sealed locus's
own body is trusted**, which is the assumption the shape rests on.

This is confinement, not information flow. A signature *derived* from
the key is not tracked, and a constant-time compare still lets the
verdict be published.

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

    fn handle_io(e: DbError) -> Row fallible(ClosureViolation) {
        self.last_error = e.detail;
        if e.kind == "send_failed" { violate fatal_io; }
        return Row { data: "" };
    }

    fn on_query(q: Query) {
        let r = send_query(self.conn_fd, q)
            or (self.handle_io(err) or { return; });
        QueryResult <- r;
    }
}
```

- The error-check fn is the **one named site** owning both the
  audit-state update and the escalation choice.
- `violate NAME;` diverges: the runtime synthesizes a
  `ClosureViolation`, sets `draining`, routes to the parent's
  `on_failure`.
- A method that returns a value and may violate is `fallible(ClosureViolation)`
  (the checker refuses it otherwise), so the violation is the call's
  failure and the caller's `or` says what happens next: here, nothing
  is published.
- The two-channel rule: substrate-facing surfaces (lifecycle,
  modes, closure assertions, bus handlers) cannot declare
  `fallible(E)`; user-declared `fn` members and free fns can.
  See `spec/decisions.md` F.27.
- Loci whose methods can't return through the value channel use
  the **`last_error` scratch convention**: `last_error` /
  `last_kind` / `last_errno` fields the caller reads after a
  sentinel return (`pond` logfmt's file sink is the reference
  shape).

### 2.8 Quantity — a counted unit, not a bare Int

Declare a quantity when a number has a unit a reader could get
wrong: money, time, bytes, ticks. The value is still the `Int` it
counts, but the compiler now refuses the mix-ups (`5ms + 4KiB`, a
bare count where a duration goes) and makes every exact conversion
for you.

```hale
unit cent;
unit USD = 100 cent;
unit B;
unit KiB = 1024 B;
unit mK;

type Money = quantity Int in cent;
type ByteCount = quantity Int in B;
type TempDelta = quantity Int in mK;
type Celsius = point TempDelta { origin: 273_150 mK; }

type Line { qty: Int; price: Money; }

fn total(l: Line) -> Money { return l.qty * l.price; }

fn main() {
    let l = Line { qty: 3, price: 12USD };
    let cap: ByteCount = 4KiB;
    let room = Celsius(21_000mK);
    let started = std::time::monotonic();
    if std::time::monotonic() - started > 50ms { println("slow"); }
    println(total(l), " ", cap, " ", room);
}
```

- **Name the catalogue once.** A unit is one node of the whole
  program, so it is declared once, in the seed that owns its domain;
  an importer writes `12USD` with no prefix, and a second `unit
  cent;` anywhere in the program is an error. Time is declared
  already: `ns` to `day`, `Duration` and `Time` are the stdlib's.
- **A point for a reading whose zero is a convention** — a
  temperature, a price, an instant (`Time` is one). Two points
  differ by a quantity and never add; `origin:` says where the
  point's zero sits.
- **Hold clock readings as `Time` and `Duration`**, not as `Int`
  nanoseconds: every sum, difference, scaling and comparison of them
  is typed, and the representation is the same `i64`.
- **A bare `Int` is right for a count of things** — an index, a
  length, a number of items (`qty` above). `qty * price` is `Money`;
  the count has no unit to get wrong.

Narrowing is C8. The contract is `spec/units.md`, the walk-through
the book's "Units, end to end".

### 2.9 Message union — a set type

When a message is one of several kinds and each kind is a record of its
own, the union is a **set** of those types. The member's type is the
tag, so nothing is named twice:

```hale
type NewOrder { id: Int; qty: Int; }
type Cancel { id: Int; }
type Inbound = NewOrder | Cancel;

fn parse(line: String) -> Inbound {
    if line == "cancel" { return Cancel { id: 1 }; }
    return NewOrder { id: 2, qty: 5 };
}

fn describe(m: Inbound) -> String {
    return match m {
        NewOrder(n) -> "new " + to_string(n.qty),
        Cancel(c) -> "cancel " + to_string(c.id),
    };
}

fn main() { println(describe(parse("cancel"))); }
```

- **A set or an enum.** A set when the kinds are records with a life
  of their own: a handler method takes each one, a surface row names
  it, another set reuses it. An enum when the kinds are tags, or small
  payloads that mean nothing outside the union (`type Status = enum {
  Up, Degraded(String), Down };`).
- **Match without `_`.** `match` names each member by its type and is
  exhaustive. Leave the `_` arm out, so a member added to the set fails
  the build at every match that does not handle it. **[error for a
  non-exhaustive match]**
- **The place gives the set its type.** A member becomes the set where
  a set is expected: a return, an argument, a typed `let`, a field. An
  `if` or a `match` whose arms are different members is not joined into
  a set ("if-expression arms have mismatched types"). Give the branch a
  fn that returns the set, as `parse` does. **[error]**
- **Members are structs or enums.** To put an `Int` in a set, give it a
  type of its own (`type Count { n: Int; }`). A set is not a member of
  another set: list its members instead. **[error]**
- Underneath, a set is the enum whose variants are its members, at an
  enum's cost. On an API surface it crosses as that enum, with the
  member's type name as the tag (`{"Cancel": {"id": 1}}`).

When another locus consumes the union, a seam (2.10) writes the dispatch
for you. Don't write it by hand.

### 2.10 Seam — decoder, handler, encoder in one tower

A protocol edge is three pieces that should not know each other: a
decoder that knows the bytes, a handler that knows its domain, and an
encoder that writes the answer. Write each as a locus and let a `seam`
stitch them. The compiler writes the dispatch and checks it.

```hale
type NewOrder { id: Int; qty: Int; }
type Cancel { id: Int; }
type Ack { id: Int; }
type Reject { id: Int; why: String; }
type WireError { at: Int; }
type Inbound = NewOrder | Cancel;
type Outbound = Ack | Reject | WireError;

locus Wire {
    fn decode(b: String) -> Inbound fallible(WireError) {
        if b == "cancel" { return Cancel { id: 1 }; }
        if std::str::starts_with(b, "new ") {
            let qty = std::str::parse_int(b[4..len(b)]) or { fail WireError { at: 4 }; };
            return NewOrder { id: qty * 10, qty: qty };
        }
        fail WireError { at: 0 };
    }
    fn encode(o: Outbound) -> String {
        return match o {
            Ack(a) -> "ack " + to_string(a.id),
            Reject(r) -> "reject " + r.why,
            WireError(e) -> "bad frame at " + to_string(e.at),
        };
    }
}

locus Desk {
    params { open: Int = 0; }
    fn new_order(n: NewOrder) -> Outbound {
        if n.qty > 10 { return Reject { id: n.id, why: "too big" }; }
        self.open = self.open + 1;
        return Ack { id: n.id };
    }
    fn cancel(c: Cancel) -> Outbound {
        self.open = self.open - 1;
        return Ack { id: c.id };
    }
}

locus Gateway {
    params { wire: Wire = Wire { }; desk: Desk = Desk { }; }
    seam answer(frame: String) -> String {
        frame -> self.wire.decode -> self.desk -> self.wire.encode;
    }
}

fn main() {
    let g = Gateway { };
    println(g.answer("new 3"), "; ", g.answer("cancel"), "; ", g.answer("??"));
}
```

- **A locus hop is the dispatch.** `self.desk` sends each member of
  `Inbound` to the `Desk` method whose parameter takes it. Coverage is
  checked at every boundary, in both directions. A member that no method
  takes, or that two methods take, is refused by name: "`self.desk` has
  no method taking `Cancel`, which reaches it from `self.wire.decode`:
  every member has one eater". So is a member that a method hop's
  parameter does not hold. This replaces a hand-written `match` that
  forwards each member to a method, whose `_` arm is where a new member
  goes missing at run time. The seam is also the cheaper of the two: it
  hands a split set back in its own frame, with no tag read and no
  switch (S14). **[error]**
- **A protocol failure is an answer.** Make the decoder's error a member
  of the output set, as `WireError` is of `Outbound`. It enters the
  chain at the first later hop that takes it, and the encoder writes the
  reject. A failure that no later hop takes needs the seam's `or end;`
  (the flow ends; a callable seam fails with it) or `or violate
  <closure>;`. Without one it is refused, naming the hop and the type.
  **[error]**
- **Closed or callable.** With a source and a sink, `seam {
  self.feed.next() -> … -> self.log.write; }` is the locus's `run`: it
  pulls from the source until the source fails, and the locus
  declares no `run` of its own. Without them, `seam name(x: T) -> R { x
  -> …; }` is a method. One frame of a closed seam is a turn, a method
  of its own that `run` calls, so what a frame's hops make is freed when
  the frame ends and a gateway that runs for hours holds flat. **[error
  for a closed seam beside a `run`]**
- **One tower.** Hops are the locus's own methods and children, called
  directly and inlined. Nothing is queued, and no union is built that
  the program did not declare. A hop on an interface is refused, because
  the method it calls is not known statically. So is a hop on a child
  that `placement` puts on another pool: across pools, use the bus.
  **[error]**
- **An API receiver is this shape, written for you.** Each row of a
  served surface is a seam on its receiver. One decoder turns the body
  into the row's request type, the dispatch hands it to your method,
  and one encoder writes the answer, your error, or the server error. A
  handler is an ordinary method over its own types. It never sees
  bytes, and there is nothing to hand-write.

### 2.11 Wire record — what a surface carries

The records that a surface's rows name are its contract. The codec, the
description, the generated clients and the digest all read them, so
choose each field's shape for what the caller must be able to say:

```hale
role operator;

type Status = enum { Up, Degraded(String), Down };
type Check { name: String; status: Status; }

type HealthQuery {
    verbose: Bool = false;
    since: Time = `1970-01-01T00:00:00Z`;
    has_since: Bool = false;
}

type Health {
    checks: bounded[Check; 16] indexed_by name;
    note: String = "";
    has_note: Bool = false;
}

type Problem = enum { @not_found NotFound(String), @unavailable Busy };
type Restart { name: String; }

api Ops requires: [operator] {
    rpc Probe::health requires: [];
    rpc Probe::restart;
}

locus Probe {
    fn health(q: HealthQuery) -> Health {
        let mut h = Health { };
        push(h.checks, Check { name: "db", status: Status::Up }) or discard;
        push(h.checks, Check { name: "cache", status: Status::Degraded("cold") }) or discard;
        if q.has_since {
            h.note = "since " + std::time::iso8601(q.since);
            h.has_note = true;
        }
        return h;
    }
    fn restart(r: Restart) -> Health fallible(Problem) {
        if r.name != "db" { fail Problem::NotFound(r.name); }
        return Health { };
    }
}

fn main() {
    let p = Probe { };
    let q = HealthQuery::api_decode("{\"since\":\"2026-10-18T00:00:00Z\"}") or HealthQuery { };
    println(Health::api_encode(p.health(q)));
}
```

```text
{"checks":{"db":{"status":"Up"},"cache":{"status":{"Degraded":"cold"}}},"note":"since 2026-10-18T00:00:00Z"}
```

- **A default, when "not sent" means a value.** A caller that leaves
  out `verbose: Bool = false` has said `false`. Use a defaulted field
  wherever "not sent" and one value mean the same thing, of any type: an
  enum's (`scope: Scope = Scope::key`), a record's or a computed one is
  read as a literal's is, and the description lists none of them as
  required. It costs neither side anything. Write it of literals,
  constants, module fns and records of those: a default never reads
  the scope of the literal that leaves it (C11), so the decoder builds
  it on its own.
- **A presence pair, when "not sent" means not sent.** With `since`
  and `has_since: Bool`, the caller may leave the key out or send
  `null`, and the handler reads the flag. The encoder writes `note`
  only when `has_note` is set, and it never writes a flag. Declare
  both fields with defaults, so that a literal with nothing to say
  names neither. Never encode absence as a value the field's type can
  hold (`""`, `-1`, the epoch): the description cannot say so, and a
  client cannot tell it from a real value.
- **A presence triple, when "not sent", "clear" and a value all differ.**
  A PATCH keeps a field it is not sent, clears one sent as `null`, and
  sets one sent a value. Add `clear_note: Bool = false` beside the pair,
  and the handler reads `clear_note` before `has_note`. Use it only where
  a caller must be able to clear: on a pair, `null` and "not sent" are
  one answer, which is the right one for every other input.
- **A `std::json::Value`, when the program carries JSON it never
  reads.** A seat's configuration, a change's before and after: declare
  them `std::json::Value` and they cross byte for byte, described as any
  JSON. Never a `String` holding JSON, which crosses double-encoded
  (`"config": "{\"a\":1}"`) and is described as text. Reach for a record
  as soon as the program reads a field of it.
- **A map, when callers look entries up by name.** `bounded[Check; 16]
  indexed_by name` crosses as `{"db": {…}, "cache": {…}}`, and in the
  program it stays the bounded list it was, in order. A repeated key is
  refused `duplicate_key`. When each entry is one value (a status, a
  count), say so: `indexed_by name -> status` on a record of just the
  two crosses as `{"db": "ok"}`. Without the arrow it stays a map of
  records. Keep a plain `bounded` list when the order is
  the meaning, when an entry has no `String` name, or when two entries
  may share one. Either way the bound is part of the contract: size it
  for the largest legitimate answer, and page past it (`type Page {
  items: bounded[Item; 50]; more: Bool; }`).
- **An enum, for a closed set of states.** `Status` crosses as `"Up"` or
  `{"Degraded": "cold"}`, and the description lists the variants. A
  `String` status crosses as any string, and every client re-invents
  the list. An absent enum field is its default when it declares one,
  and `missing_field` otherwise, unless the field is a presence pair.
- **An error type that says which thing went wrong.** With
  `fallible(Problem)`, where `Problem` is an enum or a set of error
  records, a caller branches on the variant. A record of `code: String`
  makes it parse prose.
- **A class on every error variant that has one.** `@not_found`,
  `@conflict`, `@invalid`, `@forbidden` and `@unavailable` say what kind
  of failure a variant is, and each transport answers with its own
  status for it: HTTP 404, 409, 422, 403 and 503 (with a
  `Hale-Error-Class` header, since a refusal can share the status), gRPC
  `NOT_FOUND`, `ABORTED`, `INVALID_ARGUMENT`, `PERMISSION_DENIED` and
  `UNAVAILABLE`, a Unix reply its `"class"`. MCP's error object has no
  place for one, so there the description says it. A variant with no
  class is the plain handler error (422, `FAILED_PRECONDITION`), so
  leave a variant unclassed only when none of the five fits. A class is
  contract: declaring, changing or removing one moves the digest.
  **[error for a word that is not a class]**
- **Roles once, on the surface.** `api Ops requires: [operator]` is the
  default of every row, and a row that differs says so (`requires: []`
  for anyone). It is the same table, with the same digest, as writing
  the roles on each row.
- **Evolve by adding members.** Each member has a digest of its own,
  which moves only when that member's request, response, error or roles
  change. A client built against a member keeps working while the
  surface gains others. Changing the shape of a member refuses every
  client that names its old digest (`digest_mismatch`). So a new
  capability is a new row, and an incompatible change is a new member
  (`place_v2`) or a new surface, served beside the old one.
- **Speak the codec; don't hand-roll JSON.** Where a program writes the
  wire itself (a streamed event, a hand-written router, a test),
  `T::api_encode(v)` and `T::api_decode(text)` are the surface's own
  codec, as strict as a served row. JSON built with `+` is a second
  encoder, and it drifts.

### Naming conventions

| Construct | Convention | Example |
|---|---|---|
| Locus / type | PascalCase | `Listener`, `Request` |
| Method / field | snake_case | `name_to_motion`, `listen_fd` |
| Lifecycle decl | drop the `fn` keyword | `run() { ... }` |
| Free fn | bare snake_case | `drive`, `handle_one_connection` |
| Bus subject | dot-separated, lowercase | `log.app.db` |
| Constants | UPPER_SNAKE_CASE | `STDLIB_AP_SOURCE` |
| Unit | its symbol: short, in the case its domain writes it | `ms`, `bp`, `cent`, `KiB`, `USD` |

A unit name is written against a number with no space (`500ms`,
`3bp`), so it reads like the stdlib's time units: short and mostly
lower-case, with an established symbol keeping its case (`KiB`,
`mK`). Two names are refused outright: one a
literal already owns as a suffix (`d` is the Decimal literal's, `e5`
a Float exponent — "unit `d`: a unit named `d` collides with the
Decimal literal's suffix: `3d` is the Decimal `3`; give it another
name"), and a builtin type's (`Bytes`, `String`), which a type
position reads before any declaration. Hence `ByteCount`, not
`Bytes`, for a count of `B`.

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
  are pinned as tests, and so are the refusals and measurements the
  shapes and rules elsewhere rest on (2.9, 2.10, 2.11, C9, S15,
  S16). Absence is the thing nobody re-tests, so when a gap closes
  the prose keeps saying it is open. If one of these changes, the
  test fails and names the entry to update.
  (same file, plus `hale-cli/tests/styleguide_claims.rs` for the
  claims that are codegen limits rather than typecheck limits — a
  split worth knowing, because those are invisible to `hale check` —
  and for the retirement gaps, which only a running program's
  resident set shows.)

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
- TLS on an async_io pool parks like plain TCP: the handshake
  (`connect`, `connect_within`, `upgrade`), `send_bytes` and every
  read (`recv_bytes`, `recv_into`, `recv_stamped_into`) swap the
  coroutine out while the socket is not ready, so the `std::http`
  https client belongs there as much as the plain-http one. What
  still runs on the worker is the name lookup of a non-numeric
  host (`getaddrinfo`): a client of a host whose resolver is slow
  dials an address, or lives on its own pool. **[convention]**

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
- **A subscriber per connection is fine; key it by an `Int`.** A
  connection's peer that subscribes to its own replies, built when the
  connection opens and dissolved when it closes, costs the registry
  nothing that outlives it: a dissolved subscriber's entry goes to the
  next registration, so the registry holds what is live
  (`bus_scope_local_subscriber.rs` churns 10,000 and counts the
  table). Key it by the connection's `Int` id rather than a `String`:
  a String key's copy is kept until the process exits, one per
  subscriber.

```hale
type Reply { conn: Int; body: String; }
topic Replies { payload: Reply; subject: "app.replies"; keyed_by conn; }
locus Conn {
    params { id: Int = 0; }
    bus { subscribe Replies as on_reply where key == self.id; }
    fn on_reply(r: Reply) { println(r.body); }
}
```

  **[convention]**
- **An adapter binding's literal is written whole.** The adapter in
  `bindings { Topic: Relay { ... }; }` is built by the program's prelude,
  before the `main` locus that declares it, so it is checked as an
  expression in a free fn is: its fields cannot read that locus's params
  (`Relay { url: self.url }` is refused at the read), and a statement that
  acts on an enclosing locus (`terminate`, `violate`, a publish) is
  refused there too.
  Write the value in the literal, or give the adapter's own `params` the
  default, and read configuration the adapter needs at run time (an
  environment variable, a file) in its `birth()`. **[error]**
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

- Cell types: data, never a locus. A struct from another seed or
  the stdlib qualifies; spell the stdlib's time types bare, `Time`
  and `Duration`
  (§7). **[error]**
- Hashmap iteration is bucket-order → `seq` field for order;
  no delete → tombstone. **[convention]**

### C8. Narrow with a policy, never by hand

A conversion that divides can lose a remainder: `/` by a literal, a
quantity into a coarser denomination (`.in(USD)`, a binding or a
field of a coarser type). Each says what becomes of the remainder —
`or floor`, `or ceil`, `or trunc`, `or half_up`, `or half_even`,
`or <value>`, `or raise`, `or fail E { … }` — or the target type
says it once with `{ round: …; }`:

```hale
unit cent;
unit USD = 100 cent;
type Money = quantity Int in cent;

fn main() {
    let fee: Money = 250cent;
    let share = fee / 3 or half_even;
    let dollars = fee.in(USD) or floor;
    let count: Int = (fee.in(USD) or half_even) / 1USD;
    let wait = 1500ms.in(s) or ceil;
    println(share, " ", dollars, " ", count, " ", wait);
}
```

Two reasons. The compiler refuses the bare form, at the site:

```text
type error: `Money` divided by 2 leaves a remainder: say what happens to the remainder: `or floor`, `or <value>`, `or raise`, or give `Money` a `round:` policy
```

And the policy is the reader's record of the rounding: a ledger
that rounds half-even says so where it does, instead of in an
`Int` division three calls away. Don't route around it by dividing
`Int`s you pulled out of a quantity — that is the hand-rolled
rounding the rule exists to name.

The one division that takes no policy is a quantity's count in a
unit, `fee / 1USD`: an `Int` quotient that truncates toward zero,
as `Int`'s `/` does, and a runtime divisor (`d / n`) is the same
integer division. When the count must round another way, narrow
first, as `count` above does. **[error for an unnamed narrowing]**

### C9. A `bounded` lives in its holder: count it, copy it whole, return its record

A `bounded[T; N]` is laid out inline in the value that holds it. Its
operations are the intrinsics, and its lifetime is its holder's. Four
edges follow from that, and the check refuses the wrong side of each:

```hale
type Row { names: bounded[String; 4]; }
type Names { items: bounded[String; 4]; }

fn fill(r: Row) {
    push(r.names, "a") or discard;
    push(r.names, "b") or discard;
}

fn first_two() -> Names {
    let mut n = Names { };
    push(n.items, "x") or discard;
    push(n.items, "y") or discard;
    return n;
}

locus Roster {
    params { row: Row = Row { }; spare: Names = Names { }; }
    fn refill() {
        let mut mine = Row { };
        fill(mine);
        self.row = mine;
        self.spare.items = first_two().items;
        println(count(self.row.names), " ", count(self.spare.items));
    }
}

fn main() { let r = Roster { }; r.refill(); }
```

- **`count(x)`, not `len(x)`.** `len` of a `bounded` is refused, and the
  diagnostic points at `count`. **[error]**
- **Return the record, not the list.** A fn returning `bounded[T; N]` is
  refused. Return a record that holds it, as `first_two` does; its
  elements come back whole, and the caller can keep pushing.
  **[error]**
- **Fill a local, then store it whole.** A fn that pushes a `String`,
  `Bytes` or record into a `bounded` under its parameter allocates the
  element in the frame that runs the call. Handed a place under `self`
  (`fill(self.row)`), it would leave the locus holding elements that
  frame frees, so that argument is refused. Fill a local the frame owns
  and store it whole (`self.row = mine`), which anchors the elements in
  the locus's arena. Or write the elements through `self`, in a method
  of the locus that holds the place. **[error]**
- **Whole assignment is a copy.** Assigning one `bounded` place to
  another of the same element type and capacity (`self.spare.items =
  first_two().items`) copies the elements and the count. A later `push`
  on either side does not reach the other. Anything else on the right,
  such as an array literal or another capacity, is refused: build the
  list with `push`. **[error for a mismatched type]**

### C10. A served HTTP edge: cap it in your own shape, and stream from a pool that parks

```hale,fragment
std::http::Server {
    port: 8080,
    handler: Feed { },
    max_request_bytes: 65536,
    too_large: std::http::Response { status: 413, content_type: "application/json", body: "{\"error\":\"too large\"}" },
};

fn handle(req: std::http::Request) -> std::http::Response {
    let w = std::http::SseWriter { conn_fd: req.conn_fd };
    for t in self.ticks {
        w.event(std::http::SseEvent { event: "tick", data: Tick::api_encode(t) }) or { break; };
    }
    return w.end();
}
```

- **Set the request cap to what the API takes.** `std::http::Server`'s
  `max_request_bytes` (1 MiB by default, at most 1 GiB) counts the whole
  request, its head included. A declared body over the cap is answered
  with `too_large` before any of it is read. Size the cap for the
  largest legitimate request, and give `too_large` the API's own error
  shape, so that a caller over the cap gets an answer it can parse.
  **[convention]**
- **Stream from an `async_io` pool.** A `BodyWriter` or `SseWriter`
  holds its connection for as long as it streams. On a `where async_io`
  pool each connection is a coroutine of its own, and a stream parks
  only itself. Anywhere else the Server serves one connection at a
  time, so a ten-second stream holds every other request for ten
  seconds. A failed `chunk` or `event` means the client has gone: stop,
  and return `w.end()`. Encode each event's data with `T::api_encode`
  (2.11). **[convention]**

---

### C11. A default reads its own fields and constants, nothing of the caller

A default is evaluated where the literal or call that leaves it is
written, and the check refuses one that reads that scope. Build a field's
or a parameter's default of literals, constants, free fns, types and
variants. A locus param's default may also read earlier params through
`self.<param>`. **[enforced]**

```hale
const LIMIT: Int = 5;
type Page { size: Int = LIMIT; from: Int = 0; }
fn fetch(p: Page = Page { }) -> Int { return p.size + p.from; }
fn main() { println(fetch()); }
```

```hale,counter
type Page { size: Int = limit; }   // refused: `limit` is the caller's
fn main() { let limit = 9; let p = Page { }; }
```

**Why:** a default that reads a caller's local means something different
at every literal. It binds the declaration to names its callers happen to
use, and no codec can build it, because the decoder has no caller. Pass
what varies as an argument or a field instead. A caller binding with the
name of a constant a default reads (`let LIMIT = 9` before `Page { }`) is
refused at that literal for the same reason: rename it, or write the
field.

---

### C12. Log records through `std::log`, never free text

A running part's log is read by the platform as well as by people, so
every line it writes is a record: a time, a level, the logger's path,
a message and named fields. Log through a `std::log::Logger` and let a
sink write them (logfmt by default, `HALE_LOG_FORMAT=json` for one JSON
object a line), with the details in fields, not spliced into the
message:

```hale
fn main() {
    std::log::StdoutSink { };
    let log = std::log::Logger { name: "orders" };
    log.warn_kv("retrying", std::log::kv("order", "o-17") + " " + std::log::kv("attempt", "2"));
}
```

A bare `println` in a served or attached program is free text the
collector cannot parse; it is the standard-practices pack's first
clause (`standard/logs-are-structured`, GH #1591), which the Board
holds in review until the compiler can check it. **[convention]**

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

Two rules are the exception, and they apply to **every** path: S15 and
S16. Their cost is memory that grows with the square of a text's size,
so a cold path that handles one large body is enough to exhaust a host.

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
is *correct* — the struct bytes memcpy in place and the replaced
value's String clones, Bytes and nested blocks retire — but each
store still pays an anchor-clone + retire per String field and per
nested block, and a walk of the old and new values when the struct
nests one. On a certified-hot path
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
checksums) hit the memcpy path forever. Any other length clones
and retires the abandoned buffer, String or Bytes, for the next
store to reuse (a String field's blocks come in size classes, so
its memory stays bounded by its largest value) — but each such
store still pays the clone, so genuinely variable-length hot
fields belong in a reused `BytesBuilder` + view instead.
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
scalar payloads for high-rate topics. A `where zero_copy` binding
holds its payload to that shape at check: scalar fields, nested flat
structs, and fixed arrays of `Int` / `Float` / `Bool` / `Decimal` /
`Duration`, which are laid out inline; an array of structs or of
`Time`, a slice, a `String` or `Bytes` is refused ("payload type …
is not flat-shapeable"). For high-rate consumers,
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
- Writing into a value read from a map and setting it back (`let
  mut e = m.get(k) or E { key: k }; e.inner.n = n; m.set(e);`) —
  the write stays in the cell's block, uncopied, when the set
  follows on every path with no read of the map between and `e` is
  done with at the set. Break any of those and the written part is
  copied at its first write (a value you read is yours: a write into
  it never changes the cell); `hale check` names each such copy and
  why (spec/forms.md § Reading a cell).
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
let page = self.users.skip(20).take(10).count();
self.users.filter(it.active).sort_into(self.ranked, older_first);
self.orders.group_count_into(self.by_desk, it.desk);

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

A chain runs over a `@form(vec)` source directly (`self.users`), and
over a `@form(hashmap)` source anchored on `.entries`
(`self.book.entries.filter(...).count()`) — the same pseudo-field a
`for e in m.entries` loop iterates, walking the hashmap's occupied
slots. `.items` is the explicit vec anchor. The element `it` is the
stored value in both.

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

Whole-set operations (`sort_into`, `reverse_into`,
`group_count_into`) cannot fuse — they need every element before
producing any — so they materialize into caller storage: the chain
fills (or bumps) a collection you declared, then reorders it in
place. That cost is visible in the budget, which is the point.
Positional selection (`take(n)` / `skip(n)`) and the element's
ordinal (`enumerate()`, binding `idx`) DO fuse — they are counters
on the one loop, not materializations.

### S14. Let a seam split a producer's set

The shape is 2.10's; this is what it costs on a hot path.

```hale
type A { n: Int; }
type B { s: String; }
type In = A | B;
locus Parse { fn parse(x: Int) -> In { if x > 0 { return A { n: x }; } return B { s: "neg" }; } }
locus Handle {
    fn a(v: A) -> Int { return v.n * 2; }
    fn b(v: B) -> Int { return len(v.s); }
}
locus Front {
    params { p: Parse = Parse { }; h: Handle = Handle { }; }
    // idiomatic: the seam writes the dispatch
    seam go(x: Int) -> Int { x -> self.p.parse -> self.h; }
    // not this: the same chain written by hand
    fn go_by_hand(x: Int) -> Int {
        return match self.p.parse(x) { A(v) -> self.h.a(v), B(v) -> self.h.b(v), };
    }
}
```

The two compute the same thing, and only the seam is free of the set. A
method's set result is built in its scratch and copied whole into its
caller's arena, and the caller's `match` reads the tag back from that
copy. A seam that splits what a hop returns calls a clone of the hop
that hands the set back in the seam's own frame, the member copied out
as itself beside its tag; inlined into the seam, each of the producer's
`return`s goes straight to its arm, with no tag read and no switch
(`seam_fusion.rs` pins the optimized IR). A hop that hands the whole set
on to a method taking the set (an encoder of the outbound set, say) is
not split and is built as any set is.

### S15. Build a long text in one buffer, never with `s = s + chunk` in a loop

This rule applies on every path, hot or not. `s + chunk` makes a new
String as long as both and copies `s` into it. Inside a loop the old `s`
is not freed: it lives in the method's scratch until the method returns,
so every prefix the loop made is still held at the end. Building `n`
bytes in `k` appends holds about `n·k/2` bytes. Measured on v0.25.0,
with 500-byte rows, as the growth of the process's peak resident set:

| text | appends | `out = out + row` | `BytesBuilder` |
|---|---|---|---|
| 500 KB | 1 000 | +250 MB | +2 MB |
| 1 MB | 2 000 | +990 MB | +4 MB |
| 2 MB | 4 000 | +3.9 GB | +8 MB |

Append into one `std::bytes::BytesBuilder` instead. It grows by
doubling, so it holds the text once, plus slack. A list body is the
common case:

```hale
locus JsonList {
    params {
        buf: std::bytes::BytesBuilder = std::bytes::BytesBuilder { initial_cap: 4096 };
        n: Int = 0;
    }
    fn add(entry: String) {
        if self.n > 0 { self.buf.append_str(", "); }
        self.buf.append_str(entry);
        self.n = self.n + 1;
    }
    fn text() -> String {
        let b = self.buf.snapshot() or b"";
        return std::str::from_bytes(b);
    }
}

fn page(n: Int) -> String {
    let rows = JsonList { };
    let mut i = 0;
    while i < n {
        rows.add("{\"id\": " + to_string(i) + "}");
        i = i + 1;
    }
    return "[" + rows.text() + "]";
}

fn main() { println(page(3)); }
```

A `+` that joins a fixed handful of pieces, such as one record rendered
as one line, is fine. The rule is about a loop whose trip count grows
with the input. `std::str::join` joins a `@form(vec)` of Strings in one
allocation. `std::http::Server` assembled requests with `+` until
2026-10-18: a 12 MiB body held about 9.5 GB before any handler ran.
`hale check` does not flag the loop. **[convention]**

### S16. Read a long text by position, never by slicing off the rest

This rule also applies on every path. `rest = rest[(nl + 1)..len(rest)]`
copies the remainder for each line, and each copy stays in scratch until
the method returns: the same square as S15. Walk the text by position
instead. `std::str::bytes_view` measures it once, `byte_at` reads a byte
against that length, and `std::str::slice(v, start, end)` copies only
the line:

```hale
fn line_end(v: std::str::ByteView, start: Int) -> Int {
    let mut i = start;
    while i < v.n {
        if std::str::byte_at(v, i) == 10 { return i; }
        i = i + 1;
    }
    return v.n;
}

fn count_rows(all: String) -> Int {
    let v = std::str::bytes_view(all);
    let mut rows = 0;
    let mut start = 0;
    while start < v.n {
        let nl = line_end(v, start);
        let line = std::str::slice(v, start, nl);
        start = nl + 1;
        if len(line) > 0 { rows = rows + 1; }
    }
    return rows;
}

fn main() { println(count_rows("a\nb\n\nc")); }
```

Measured over 1 MB in 2 000 lines: slicing off the rest grew the peak
by 984 MB, and reading by position by under 1 MB, in under a
millisecond. To find the next line, or any separator, search the view:
`std::str::find(v, "\n", start)` answers the index into the text, at the
cost of the distance to the hit. `std::str::index_of_from(s, "\n",
start)` measures the whole String on every call, so a line-at-a-time
scan with it is quadratic in time: 16 MB in 32 000 lines took 4.7 s,
against under a millisecond with `find`. **[convention]**

Walk code points by position too: `std::str::view_cp_at(v, i)` and
`view_cp_size(v, i)` read a character against the view's `n`, where the
String forms `cp_at` / `cp_size` look for the String's end before every
read, so a walk with them is quadratic (1 MB of text: 2.4 s, against
0.5 ms over the view). **[convention]**

---

## 5. The enforcement ladder

The compiler backs this guide in tiers. Everything default is
advisory or genuinely-broken-only; strictness is opt-in where you
certify a path — plus one tier you get for free because your
*placement* already declared the intent.

| Tier | Check | Status |
|---|---|---|
| **error** (default) | blocking `run()` on a cooperative *subscriber* (dead receiver); unowned subscriber in a handler; non-exhaustive / type-mismatched match, a set's included (2.9); a seam's coverage, failure placement and one-tower rules (2.10); `@form` cell constraints; the unit laws, and a narrowing with no policy (C8); a `bounded`'s `len`, its return by value and an element stored through a parameter's place (C9); a default that reads the caller's scope (C11) | build fails |
| **warn** (default) | unbounded-alloc survey (self-escaping allocs in unbounded contexts; retirement-aware since v0.11.3); subscription nothing publishes to; locus/builder in a loop **or bus handler**; allocating recv in a loop; blocking call on a cooperative pool (interprocedural); accept-without-release on a daemon | advisory |
| **@hot** (opt-in) | all of the above as errors within the fn; `snapshot()`/`finish()` in a loop; whole-struct self-field replace | errors in certified fns |
| **@budget** (opt-in) | `alloc_per_call = N` counted transitively; `N=0` zero-alloc certificate | build fails on violation |
| **@effects** (opt-in) | `@effects(none: {syscall, block, time, entropy, env, ffi, publish, spawn, recursion, stdio})` / `@effects(publish: {T})`, and the `@no_*` / `@deterministic` sugar — checked transitively; diagnostics carry the call chain to the offending leaf | build fails on violation |
| **@sealed** (opt-in) | a sealed locus's `params` are reachable only from inside its own methods — reads and writes both. The tier that makes state confinement structural; loci are otherwise not field-encapsulated. `hale check --sealable` reports what taking it would cost | build fails on violation |
| **placement-implied** (automatic) | a handler on a `cooperative(pool = X) where async_io` locus that reaches a blocking call — the placement *is* the assertion, so no annotation is needed; writing `@no_block` upgrades it to an enforced error | advisory |
| **fmt** (CI gate) | `hale fmt --check` — canonical mechanical form (§2, "Canonical form"); exit 1 lists offenders | gate in CI; `hale fmt` fixes |
| escape hatches | `@unbounded` (fn or lifecycle hook) acknowledges intentional accumulation; `--allow-unowned-subscriber`; `--no-warn-unbounded-alloc` | |
| advisory tools | `hale check --sealable` (which loci could seal today); `hale check --flows` (which `release` clause makes each type a flow); `hale check --strict-secret` (the fail-closed `@secret` walk — loud by design, because one body's reasoning is not a containment proof); `hale check --units` (every narrowing by site, with its factor and the policy that discharged it and where that policy came from, and each quantity's headroom — record it and review its diff) | opt-in reports |

### Where law lives

The tiers above certify a *function*. Claims certify a *system*, and
the only real style question is which closed world owns each
sentence.

Two claim verbs quantify over the whole closed world rather than over
a path, which makes them the right shape for a baseline a team adopts
once: `require sealed(all G)` (every locus in the group keeps its
state to itself) and `require attributed(all C)` (every fn that
directly performs built-in class `C` names a user-declared purpose).
Both cover code nobody has written yet, where a group-scoped claim
does not. Prefer them for policy; keep `forbid reaches` for the
specific edges a design forbids.

The built-in group `program` (every locus and free fn of the
entrypoint's own seed) gives `forbid reaches` the same reach: a law
over `program` covers code nobody has written yet, and a policy seed
can state it without asking an adopter for vocabulary. The one every
served or attached program should adopt says it writes to the terminal
only through its logger:

```hale,fragment
constitution Logs {
    logs_structured: forbid reaches(program, effects(stdio)) avoiding std::log::*;
}
```

So log through `std::log`, never `println`, outside a CLI's own
output: `stdio` is the class of a write to standard output or standard
error, and `std::log` is the gate, so a bare print anywhere in the
program is the witness.

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
- **Floating state** — every named piece of state has one locus
  owner. State that "lives between loci" is a modeling error.
- **Growing a String with `+` in a loop, or slicing off the rest of
  one** — memory quadratic in the text, on every path (S15, S16).
- **A hand-written `match` forwarding a set's members to another
  locus's methods** — a seam checks the coverage both ways (2.10).
- **A sentinel for "absent" on the wire** (`""`, `-1`) — declare a
  presence pair (2.11).
- **JSON built by hand for a type the codec carries** — call
  `T::api_encode` (2.11).

---

## 7. Boundaries — deliberate absences, open gaps, sharp edges

Three different kinds of "the language doesn't do X" live here,
and they call for three different behaviors from you: absences
you should stop wanting (the design says no, and names the shape
to use instead), gaps you can expect to close (write the
workaround knowing it's temporary), and sharp edges you step
around (current limitations, no promise either way). Current as
of v0.25.0 (2026-10-18); shipped-and-gone entries are removed on
shipping — keyed String routing, match-as-expression, whole-struct
replace reclamation and its nested-compound fields, `err` in an `or
fail` payload, `Duration`
arithmetic, `return` in a lifecycle body, inline fixed-array
payload fields, qualified `@form` cell types, an `Int` returned
from a `-> Float` fn and an `Int` into a `Float` let or field all
lived here once.

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

  At an API boundary, absence is a **presence pair**: a field `x`
  beside a `has_x: Bool`, which the codec reads as an optional key
  (2.11).
- **Fn-pointer callbacks don't capture state.** Loci are the
  language's closures — state lives in a locus with its own
  `self`, or routes through bus subjects. A capturing lambda
  would be an anonymous locus without lifecycle or contracts;
  name it instead.
- **A locus is never a `@form` cell.** Cells are data a form
  copies, recycles and frees; a locus has a lifecycle that would be
  orphaned by it. The compiler says so: "cell type cannot be a
  locus. Cells are data; loci are managed entities … Route locus
  membership through `accept(c: Worker)` instead". Accept the
  children, and keep a `@form(hashmap)` of their keys beside them if
  you need lookup by name.

### Open gaps — expect these to close

Write the workaround knowing it's a placeholder.

- **Dynamic per-instance bus subjects** (the conversation-per-
  topic shape). Keyed routing covers the *bounded/known* key-set
  case — including String keys — but an unbounded,
  runtime-created subject set still has no shape.
- **A dial resolves its host name on the thread.** On a `where
  async_io` pool a connect (tcp or tls) and every TLS wait park
  the coroutine, but a non-numeric host is resolved with
  `getaddrinfo(3)` first, on the worker, and a connect deadline
  does not cover it. A slow resolver stalls the pool for as long
  as it takes: dial an address, or resolve once and keep it (C3).

### Sharp edges — current limitations, step around them

Implementation constraints, not positions; no promise attached.
Two of them pass `hale check` and fail only at `hale build`: know
those cold.

- **No char-level `s[i]`** — and the checker doesn't say so:
  `s[1]` on a `String` typechecks, then the build fails with
  "indexing a non-array value (type String)". Use `s[i..i + 1]`
  slices, `std::str::index_of`, or the UTF-8 accessors
  `std::str::cp_at` / `cp_size` / `cp_count` when you need code
  points rather than bytes. There is still no `Char` type;
  `cp_at` yields the code point as an `Int`.
- **Write the stdlib's time types bare: `Time` and `Duration`, not
  `std::time::Time` or `std::time::Duration`.** The qualified
  spelling typechecks as a field, a `@form` cell type or a fn
  parameter and fails at build ("qualified type
  `std::time::Duration` not in stdlib path-renames table"). A
  qualified `let` annotation builds, but the binding is not the
  quantity (`d / 1ms` is refused as "`Duration` is not an `Int`"), so
  write it bare there too.

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
