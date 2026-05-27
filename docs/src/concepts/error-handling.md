# Error handling

> Why does Hale have two separate failure
> mechanisms, and how do you choose between them?

Error-handling is where most languages quietly accumulate the
largest amount of accidental complexity. Exceptions vs.
sentinels vs. error returns vs. `Result<T, E>` vs. panics —
many languages have several of these layered, with different
disciplines for when to use which, often in the same codebase.

Hale carves the space cleanly into two **orthogonal**
channels, with strict rules about which is allowed where:

- **The structural channel** (`↑`): a locus's declared
  invariant breaks. The runtime constructs a typed event and
  routes it upward to the parent's `on_failure` handler.
  Recovery primitives (`restart`, `quarantine`, `bubble`,
  `dissolve`) decide what to do.
- **The value channel** (`fallible(E)`): an individual call
  can fail with a payload. The caller MUST address the error
  inline via an `or` clause before consuming the value.

There is **no** `panic`, no `assert`, no `try`/`catch`, no
implicitly-propagating exception system. The two channels
above cover every legitimate failure case; anything else
indicates a category error in the modeling.

## The structural channel

A locus has *commitments it must hold across its lifetime*.
Those commitments are declared in `closure` blocks:

```hale
locus PnLAttribution {
    params { intent_pnl: Decimal = 0.00d; book_pnl: Decimal = 0.00d; }

    closure books_balance {
        self.intent_pnl ~~ self.book_pnl within 0.05d;
        epoch tick;
    }
}
```

The `~~` operator is *approximate equality within tolerance*.
The closure says: at each tick, my intent PnL and book PnL
must agree within five cents. The runtime evaluates the
expression at each declared epoch; if it holds, nothing
happens (closures are silent on success). If it doesn't, the
runtime constructs a typed `ClosureViolation` event and routes
it to the parent's `on_failure`:

```hale
locus TradingDesk {
    accept(p: PnLAttribution) { /* ... */ }

    on_failure(p: PnLAttribution, err: Error) {
        match err {
            Error::ClosureViolation(v) -> {
                // err.closure is "books_balance"
                // err.left, err.right are the two values
                // err.tolerance is 0.05d
                // err.diff is left - right
                quarantine(p) for 60s;
            }
            _ -> bubble(err);
        }
    }
}
```

The parent's recovery options:

- **Absorb** — return from `on_failure` without calling any
  recovery primitive. The child's failure is treated as
  "noted, not propagating."
- **`restart(child)`** — dissolve the child and instantiate a
  fresh one with the same declared params.
- **`restart_in_place(child)`** — reset the child to
  post-birth state while preserving its arena.
- **`quarantine(child) for d`** — pause the child but
  preserve its state, optionally auto-restart after `d`.
- **`bubble(err)`** — pass the failure up to *this* locus's
  parent. Recursive propagation.
- **`dissolve(child)`** — force-dissolve the child.

If a failure bubbles all the way past the runtime root with
no handler absorbing, the process exits non-zero with a
structured violation report on stderr. That's the only way
the program "crashes" — and it's a deliberate, structured
event, not an unexpected exception.

This is Erlang's let-it-crash philosophy with one important
addition: the parent's policy is *typed* and *declared*. You
write the recovery rule next to the locus it applies to, and
it can be different for different child types. The runtime
enforces the state machine — a child can't be running and
quarantined at the same time, can't accept while draining,
etc.

## The value channel

Sometimes a function can fail in a way that's not a structural
event — just "this call didn't produce a value, here's why":

```hale
fn parse_player_id(s: String) -> PlayerId fallible(ParseError) {
    if !std::str::can_parse_int(s) {
        fail ParseError { kind: "not_int", input: s };
    }
    return PlayerId { value: std::str::parse_int(s) };
}
```

A function declared `fallible(E)` returns *either* a value of
the success type or a `FallibleErr(E)` payload. The caller
**must address** the error — the typechecker rejects a bare
call result:

```hale
let id = parse_player_id(input);     // ERROR: "error not addressed"
```

You address it with an **`or` clause**, in one of five motions.
At a glance:

```hale
let id = parse_player_id(input) or raise;          // propagate up
let id = parse_player_id(input) or default_id();   // substitute
let id = parse_player_id(input) or handle(err);    // hand off
let id = parse_player_id(input)
    or fail GameErr { reason: "bad-id" };          // translate
mkdir(path) or discard;                            // swallow (Unit only)
```

The error's payload type is fully typed — you don't downcast or
pattern-match a generic Error. The `fallible(E)` declaration
says exactly what shape the payload has, and the implicit `err`
binding inside a fallback / handler has type `E`.

### `or raise` — propagate

The enclosing function must itself be `fallible(E)` (with the
same payload type or a compatible one), so the error has
somewhere to go.

```hale
fn load_config() -> String fallible(IoError) {
    return std::io::fs::read_file("config.toml") or raise;
}
```

This is the value channel's version of "let it propagate."

### `or <expression>` — substitute

Provide a fallback value of the success type. `err` is in scope
inside the fallback expression.

```hale
let body = std::io::fs::read_file("welcome.txt") or "(no welcome message)";
let size = std::io::fs::file_size(path)         or 0;
```

The fallback's type must match the success type. Substituting
`""` for `read_file` works because `read_file` returns `String`;
substituting `0` for `mkdir` is a type error (`mkdir` returns
`()`) — use `or discard` instead.

### `or self.handler(err)` — hand off

Call a member function on the current locus that takes the
error and returns the success type. Useful when several call
sites share a recovery policy.

```hale
locus Importer {
    params { failed: Int = 0; }

    fn handle_io(e: IoError) -> String {
        self.failed = self.failed + 1;
        eprintln("skipped ", e.path, ": ", e.kind);
        return "";
    }

    fn process(p: String) {
        let body = std::io::fs::read_file(p) or self.handle_io(err);
        if len(body) > 0 { /* ... */ }
    }
}
```

The member fn IS a real function — pick a descriptive name
(`handle_io`, `recover_index`), not a placeholder. See
["Bridging the channels"](#bridging-the-channels-structural-failure-from-value-error-context)
below for the pattern that lets the handler escalate via
`violate NAME` instead of substituting.

### `or fail <payload>` — translate to your error type

Symmetric to `or raise`, but you supply a fresh payload of the
enclosing fallible fn's declared error type instead of
forwarding the inner call's payload verbatim. Use when your
library has its own error vocabulary and you don't want to leak
a stdlib type (`IoError`, `ParseError`, etc.) through it.

```hale
type ConfigErr { reason: String; path: String; }

fn load_config(p: String) -> Config fallible(ConfigErr) {
    let body = std::io::fs::read_file(p)
        or fail ConfigErr { reason: "read failed", path: p };
    return parse(body)
        or fail ConfigErr { reason: "parse", path: p };
}
```

The enclosing fn must itself be `fallible(T)`; outside one, the
typechecker rejects with a hint to use `or raise` or
`or <fallback>`. Diverges like `or raise` — the chain value
collapses to the inner call's success type.

### `or discard` — swallow (Unit-only)

For calls whose success type is `()`, when you genuinely don't
care:

```hale
std::io::fs::mkdir("/tmp/cache") or discard;     // ok if it already exists
```

`or discard` is rejected on value-bearing calls — the
typechecker tells you "this returns `String`, can't discard"
and suggests `or ""` or `or raise`.

### Chains

Chains work right-associatively:

```hale
let id = parse_player_id(input) or lookup_default() or raise;
```

Reads as: try parse; on failure, try `lookup_default()`; on
*that* failure, propagate up. Each `or` disposes one fallible
in turn, reducing the chain toward a non-fallible value.

The value channel is value-level. It propagates through the
*static call stack*, not the locus tower. Two functions that
both `fallible(ParseError)` and call each other share the same
payload type and pass it up the stack until something addresses
it.

## Where each channel lives

This is the rule that often surprises people coming from
other languages:

> **`fallible(E)` may be declared on free functions and on
> stdlib-synthesized `@form(...)` methods. It may NOT be
> declared on user-declared locus methods.**

Why the restriction? Because locus methods are
*substrate-facing*. They participate in the locus's lifecycle
— bus subscription handlers, mode projections, contract reads.
Failures at this layer are *structural events*, not
value-level errors. They belong on the closure-violation
channel, where the parent's `on_failure` is the policy
handler.

If a locus method needs to expose application-layer failure
semantics, it wraps a fallible free function:

```hale
fn parse_message(b: Bytes) -> Message fallible(ParseError) { ... }

locus Reader {
    bus { subscribe Input as on_input; }
    fn on_input(b: Bytes) {
        let m = parse_message(b) or default_message();
        // ... handle m
    }
}
```

The typechecker enforces this. Trying to declare `fn ... ->
T fallible(E)` on a user locus method produces a focused
diagnostic naming the rule.

The reverse direction has a complementary rule: only stdlib-
synthesized form methods (`@form(vec).get`, `@form(vec).pop`,
`@form(hashmap).get`, `@form(hashmap).remove`,
`@form(ring_buffer).pop`) declare `fallible(E)`. These are
application-layer storage substrate, not lifecycle-bearing
loci, so the value channel fits.

## Bridging the channels: structural failure from value-error context

The two-channel rule (narrowed 2026-05-25) keeps **substrate-facing
surfaces** — lifecycle / mode / closure-assertion / bus-handler
bodies — off the value channel. User-declared `fn` member fns
on a locus and free fns DO carry `fallible(E)` and live on the
value channel like normal. Real systems regularly need to *cross
from value to structural* — a method catches a value error in an
`or` clause, decides the error is unrecoverable, and wants to
immediately escalate into the structural channel so the parent's
`on_failure` policy takes over.

Hale's primitive for this is **inline closure violation**: a
locus declares a *named structural-failure type* as an
assertion-less closure with `epoch inline`, then any member
function can fire it with the `violate` statement.

```hale
type Query    { sql: String; }
type Row      { data: String; }
type DbError  { kind: String; detail: String; }
topic ExecuteQuery { payload: Query; }
topic QueryResult  { payload: Row; }

fn send_query(fd: Int, q: Query) -> Row fallible(DbError) {
    let sent = std::io::tcp::send_bytes(fd, std::bytes::from_string(q.sql));
    if sent < 0 { fail DbError { kind: "send_failed", detail: "connection lost" }; }
    let resp = std::io::tcp::recv_bytes(fd, 4096);
    if len(resp) == 0 { fail DbError { kind: "recv_empty", detail: "peer closed" }; }
    return Row { data: std::str::from_bytes(resp) };
}

locus DbConnection {
    params {
        host:       String = "127.0.0.1";
        port:       Int    = 5432;
        conn_fd:    Int    = -1;
        last_error: String = "";
    }

    bus { subscribe ExecuteQuery as on_query; publish QueryResult; }

    // Named structural-failure type. No assertion body; the fire
    // IS the violation. The captures clause snapshots state into
    // the ClosureViolation payload at the violate site.
    closure fatal_io {
        captures: last_error;
        epoch inline;
    }

    birth()    { self.conn_fd = std::io::tcp::connect(self.host, self.port); }
    dissolve() { if self.conn_fd >= 0 { std::io::tcp::close_fd(self.conn_fd); } }

    // The "error-check function": takes the error type, returns
    // the success type expected at the call site, and chooses
    // recovery (return a value) or escalation (violate).
    fn handle_io(e: DbError) -> Row {
        self.last_error = e.detail;
        if e.kind == "send_failed" || e.kind == "recv_empty" {
            violate fatal_io;        // diverges — no return needed
        }
        return Row { data: "" };     // transient; substitute
    }

    fn on_query(q: Query) {
        let r = send_query(self.conn_fd, q) or self.handle_io(err);
        if !self.draining { QueryResult <- r; }
    }
}
```

Three primitives are doing the work:

- **`closure fatal_io { ... epoch inline; }`** — the *vocabulary*.
  A named structural-failure type local to this locus. The
  `captures:` clause names locus state to snapshot when fired.
- **`fn handle_io(e: DbError) -> Row`** — the *policy*. A member
  fn shaped exactly for the `or` clause: takes the error type,
  returns the success type. Inside, the body decides between
  recovery (return a value) and escalation (`violate`). One
  function can be reused across every fallible call site on
  this locus that produces `Row` from `DbError`.
- **`violate fatal_io`** — the *trigger*. Statement-level,
  divergent (typechecker treats as `Never`, same as `fail` in
  fallible fns and `bubble` in `on_failure`). At the next
  cooperative yield, the runtime transitions this locus to
  drain. At dissolve, the parent receives the typed
  `ClosureViolation` with the captured `last_error`.

The flow when a value error propagates up:

1. `send_query(self.conn_fd, q)` fails — returns
   `FallibleErr(DbError {...})`.
2. The `or self.handle_io(err)` clause fires — `err` binds to
   the `DbError`; `handle_io` runs.
3. `handle_io` writes `e.detail` to `self.last_error`, sees the
   fatal kind, and executes `violate fatal_io`.
4. The runtime constructs `ClosureViolation { locus: "DbConnection",
   closure: "fatal_io", captures: { last_error: "connection lost" } }`
   and sets the locus's internal `__drain_requested` flag.
   Control diverges — `handle_io` never returns to its caller.
5. At the next cooperative yield, the runtime begins drain.
   `dissolve()` runs, closing the fd.
6. The parent's `on_failure(c, ClosureViolation { ... })` fires
   with the snapshot, decides policy (`restart` / `quarantine` /
   `bubble` / absorb).

## Why this composes well

Three roles, three slots, no double duty:

| Slot | Role | Reusable across |
|---|---|---|
| Closure declaration | Vocabulary — named failure type with optional payload schema | The locus type |
| Member fn (error-check) | Policy — decide recovery vs escalation per error kind | Every call site on the locus with same `(ErrType, SuccessType)` |
| `or self.handler(err)` at call site | Binding — typechecker-enforced disposition | Every fallible call returning the matching success type |

Compare to the older workaround pattern (a `should_exit: Bool`
flag, a `fatal_error: Bool` flag, a `while !should_exit { yield; }`
loop in `run()`, a separate diagnostic field, plus a closure to
audit at dissolve): five pieces of state doing what one
`closure` + one `violate` + one member fn now do.

## A note on Never

`violate NAME;` is *divergent*. The typechecker treats it
as the `Never` type: code after a `violate` is unreachable
within the current function. This is the same shape `fail E;`
takes inside a fallible function and `bubble(err);` takes
inside an `on_failure` handler — three statement forms whose
"return type" is "control doesn't return through here."

That's what makes the error-check function work cleanly:

```hale
fn handle_io(e: DbError) -> Row {
    if e.kind == "fatal" {
        violate fatal_io;          // Never; no return required
    }
    return Row { data: "" };       // Row; required on the other branch
}
```

The branches that violate don't need a `return`; the branches
that return must provide a value of the declared type. The
typechecker enforces total coverage exactly as it would for a
function that mixes `fail` and `return`.

## Why two channels and not one?

Languages that have only structural failure (Erlang) make
value-level errors awkward — you end up modeling "couldn't
parse this int" as a process crash, which is too heavy.
Languages that have only value failure (Rust, Go) make
*structural* errors awkward — invariant violations end up
sprinkled across every call site as `Result<T, Error>`
returns, which is too granular and loses the parent-policy-
oriented recovery model.

Hale splits the concern: structural failure routes up the
locus tower with typed policy, and value failure routes up
the static call stack with required inline disposition. The
two never mix at intermediate frames; the only place they
meet is the implicit root boundary (where any unhandled
error of either kind ends the process).

In practice the rule of thumb is:

| Failure shape | Channel |
|---|---|
| "This invariant I declared broke" | structural (closure → on_failure) |
| "This individual call can fail and the caller should choose" | value (fallible(E)) |
| "Couldn't parse" / "key not found" / "out of bounds" | value |
| "Books don't balance" / "k_max exceeded" / "child wedged" | structural |

## No panic / assert

Hale has no `panic(msg)`, no `assert(cond)`, no `throw`.
"Impossible state" becomes "a closure asserting the state is
possible" — and when it isn't, the runtime constructs the
typed violation and routes it up. "Bail from this function"
becomes either `or raise` (value channel) or "make this a
closure on the locus" (structural channel).

This isn't asceticism. It's that every legitimate use of
`panic` falls cleanly into one of the two channels above,
with better typing and better recovery shape than `panic`
itself provides.

## Next

The next chapter, [Lifecycle & time](./lifecycle-time.md),
covers how loci come into being, run, and dissolve — the
state machine the failure channels operate over.
