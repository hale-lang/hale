# When things fail

> **Coming from Go?** This is the part that's more Erlang than
> Go. Alongside the value-level [`fallible`](../basics/fallible.md)
> channel you already know, a long-running locus has a *structural*
> failure channel: when an invariant it promised to keep breaks,
> the failure flows **up** to its parent, which decides recovery —
> restart, quarantine, or escalate. Supervisors, let-it-crash, and
> typed recovery policy, built into the language.

## Two channels, on purpose

Hale keeps two failure mechanisms strictly separate:

- **The value channel** — `fallible(E)` + `or`, from the basics.
  "This call didn't produce a value; the caller decides what to
  do." Routes up the *call stack*, addressed inline.
- **The structural channel** — a locus's declared invariant
  breaks, the runtime builds a typed event and routes it up the
  *locus tower* to the parent's `on_failure`. "A promised
  property no longer holds; the supervisor decides."

There's no `panic` statement, no exceptions, and `assert` lives only
in the test library. Every legitimate
failure is one of these two, and they only meet at the program's
root.

## Declaring an invariant: `closure`

A `closure` is a property a locus promises to keep, checked by
the runtime at a declared moment:

```hale
locus Account {
    params { debits: Decimal = 0.00d; credits: Decimal = 0.00d; }

    closure balanced {
        self.debits ~~ self.credits within 0.01d;
        epoch tick;
    }
}
```

`~~` is "approximately equal, within tolerance." The `epoch`
says when to check — `tick` (each event-loop iteration), `birth`,
`dissolve`, `duration(1min)`, or `inline` (only when fired by
hand). If the assertion holds, nothing happens; closures are
silent on success. If it breaks, the runtime constructs a typed
`ClosureViolation` and routes it to the parent's `on_failure`.

## Handling failure: `on_failure`

The parent is the supervisor. It decides policy per child type:

```hale
locus Bank {
    accept(a: Account) { }

    on_failure(a: Account, err: ClosureViolation) {
        quarantine(a);
    }
}
```

A parent with children of several types writes one handler per
type. The failing child's type picks the handler, so each child's
failure reaches its own:

```hale
locus Cache { params { hits: Int = 0; } }

locus App {
    params { cache: Cache = Cache { }; server: std::http::Server; }
    on_failure(s: std::http::Server, err: ClosureViolation) { eprintln("server down"); }
    on_failure(c: Cache, err: ClosureViolation) { restart(c); }
}
```

One handler per type, and only one: a second `on_failure` for a
type that already has one could never run — the first is the one a
failing child reaches — so `hale check` refuses it and points at
the first. Everything a failure of that type needs goes in the one
handler.

The parent is the locus that holds the child in a field. It
does not matter whether the child's literal is the field's
default or is written in the parent's literal where the parent
is built — the usual way to configure a child from `main()`:

```hale
locus Conn { params { url: String = ""; } }

main locus App {
    params { conn: Conn; }
    on_failure(c: Conn, err: ClosureViolation) { restart(c); }
}

fn main() {
    App { conn: Conn { url: std::env::var("URL") } };
}
```

`Conn` fails to `App`'s `on_failure` either way. What does not
carry supervision is a child built somewhere else and handed
over by name — `let c = Conn { … }; App { conn: c };` — since it
was built before `App` existed; it keeps the route of the place
that built it. Write the literal in the parent's.

A handler never runs while its locus is still setting params.
A child declared early in `params` can fail before the later
params are set: a child on the main thread runs its whole `run()`
during that setup, and a pinned child's thread starts during it.
The runtime holds that failure and delivers it once every param
has its value, just before the parent's `birth()`. So the handler
can read any param, and what it writes is not overwritten by a
later default. The failing child stays alive until then, even if
its `run()` has already ended, and a restart the handler asks for
happens as soon as the handler returns.

A handler runs where its locus runs, never on the failing child's
thread. When the child is pinned to its own thread or placed on
another pool, its failure is handed to the parent's thread, and the
child waits there for the handler's answer before it restarts,
carries on or ends. The parent hears it at its next yield (a
`sleep`, a wait, its queue's next message), or while it joins the
child at shutdown, so the handler is never running beside the
parent's own code on a second thread, and what it writes to `self`
needs no lock. The parent does not tear the child down under a
handler that is still hearing it. A handler can even replace a
*different* child whose failure is still waiting
(`self.b = Kid { … }` while handling `a`): the field takes the new
child at once, and the replaced one is kept until its own failure
has been heard, after the current handler returns (handlers never
run inside each other), then torn down.

Once a child has been replaced, its teardown wins over any recovery.
A `restart(c)` or `restart_in_place(c)` about a child the handler has
already swapped out of its field, or one whose own teardown its parent
had already begun, is not carried out: the old
child is torn down once, after its handler, and never born or run
again, and the new child in the field is left alone.

The recovery primitives:

- **absorb** — just return; the failure is noted and contained.
  The child stops, but a child the parent holds in a field (or a
  binding holds) stays readable until the parent tears it down.
- **`restart(child)`** — run it again: `birth()`, then `run()`,
  on the same instance. A child whose `run()` failed restarts once
  that `run()` has returned — a pinned child on its own thread.
- **`restart_in_place(child)`** — the same, after putting every
  param back to the value this instance was built with (what its
  literal said, or its default as it evaluated then). A child held in
  a param stays the same child.
- **`quarantine(child)`** — pause it, preserving state for
  inspection.
- **`bubble(err)`** — pass it up to *this* locus's parent.
- **`dissolve(child)`** — force it down.

### Saying how many times: `restart(child) for N`

Restarting forever is rarely what you want. A child that fails
for a structural reason will keep failing, and a supervisor that
keeps retrying just turns a broken child into a busy loop.

```hale,fragment
on_failure(c: Book, err: ClosureViolation) {
    restart(c) for 3;
}
```

That gives this child three restarts. The fourth failure
**quarantines** it instead: you said when to stop trying, so
stopping means the child no longer runs. The count is per child
and cumulative over its life, so a child that recovers keeps
whatever budget it had left. `for 0` is a legitimate policy — do
not restart this one at all.

Write the bound here rather than counting failures by hand in the
handler. Both do the same thing at runtime, but the declared form
is the one the topology artifact records (`retry_bound`), so the
policy you wrote and the restarts actually observed can be
compared. A counter in a param is invisible to everything outside
the handler.

Without the modifier, `restart(child)` stops re-running after a
default of two attempts and leaves the child live.

If a failure bubbles past the root with no one absorbing it, the
process exits non-zero with a structured report. That's the only
way a Hale program "crashes" — and it's a deliberate, typed
event, not a surprise. This is Erlang's let-it-crash, but the
recovery policy is *typed* and written next to the locus it
governs.

### What a recovery does to a closure's running totals

A closure that accumulates (`sum(...)`, `count(...)`, `mean(...)`)
keeps its totals across epochs. A recovery zeroes them, so a
restarted child starts its audit fresh. A closure that should
keep its totals through a recovery says which recoveries:

```hale,fragment
closure within_band {
    sum(self.delta) ~~ 0 within 100;
    epoch tick;
    persists_through(quarantine);
}
```

The recoveries a clause can name are the three a parent applies:
`restart`, `restart_in_place` and `quarantine` (a spent
`restart(c) for N` quarantines, so it counts as `quarantine`).
Any other name is refused at the name, and a misspelling one
letter away from a recovery is pointed at it. `dissolve` is
refused in `persists_through(...)`: a closure's totals end with
its locus, so there is nothing to keep.

`resets_on(...)` says the default out loud: `resets_on(restart)`
documents that the totals start over on a restart, which they do
anyway. It is checked like `persists_through`, and naming the same
recovery in both is refused, since the two cannot both hold.

`hale check` warns when a clause cannot take effect: when no
`on_failure` (and no recovery statement elsewhere) in the program
applies that recovery to the locus, it lists the handlers that do
handle it and what they apply; and when a closure with no `sum`,
`count` or `mean` says `persists_through`, since it has no totals to
keep.

## Crossing from value to structural

Sometimes a method catches a value-level error and decides it's
fatal — the right move is to stop this locus and let the
supervisor take over. You bridge with an *inline* closure and the
`violate` statement:

```hale
type Query { sql: String; }
type Row   { data: String; }
topic QueryRequest { payload: Query; }
topic QueryResult  { payload: Row; }

fn send_query(fd: Int, q: Query) -> Row fallible(IoError) {
    if fd < 0 { fail IoError { kind: "broken_pipe", errno: 32, path: "" }; }
    return Row { data: q.sql };
}

locus DbConnection {
    params { conn_fd: Int = -1; last_error: String = ""; }
    bus {
        subscribe QueryRequest as on_query;
        publish   QueryResult;
    }

    closure fatal_io { captures: last_error; epoch inline; }

    // an error-check fn: takes the error, returns the success type,
    // and either substitutes a value or escalates.
    fn handle_io(e: IoError) -> Row {
        self.last_error = e.kind;
        if e.kind == "broken_pipe" {
            violate fatal_io;        // diverges — escalate structurally
        }
        return Row { data: "" };     // transient — substitute and continue
    }

    fn on_query(q: Query) {
        let r = send_query(self.conn_fd, q) or self.handle_io(err);
        if !self.draining { QueryResult <- r; }
    }
}
```

- `closure fatal_io { ... epoch inline; }` is a *named structural
  failure* with no assertion — it only fires when you say so. The
  `captures:` clause names the state the failure is about. The
  `ClosureViolation` itself has a fixed shape (`err.locus` and
  `err.closure`, which locus and which closure, and `err.diff`), so
  the supervisor reads that state through the child handle it is
  given: `c.last_error` in `on_failure(c, err)`. `err.last_error` is
  a type error, since the violation has no such field.
- `violate fatal_io;` fires it. It's divergent (the `Never` type,
  like `fail` and `bubble`), so the branches that violate need no
  `return`. It fires on the spot: `self.draining` turns true, the
  parent's `on_failure` runs with the typed violation, and the
  method exits as a `return` would.
- `self.draining` is a Bool every locus can read — true once it's
  decided to wind down. Use it to stop publishing after the
  decision.

That's the canonical "catch an error and shut this locus down"
shape: one closure, one error-check method, one `violate`. You
don't reach for a hand-rolled `should_exit` flag and a polling
loop — these primitives are the supported form.

Next: splitting a program across processes — [Across
binaries](./multi-binary.md).
