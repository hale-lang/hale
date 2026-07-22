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

There's no `panic`, no `assert`, no exceptions. Every legitimate
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
`dissolve`, `duration(1m)`, or `inline` (only when fired by
hand). If the assertion holds, nothing happens; closures are
silent on success. If it breaks, the runtime constructs a typed
`ClosureViolation` and routes it to the parent's `on_failure`.

## Handling failure: `on_failure`

The parent is the supervisor. It decides policy per child type:

```hale
locus Bank {
    accept(a: Account) { }

    on_failure(a: Account, err: Error) {
        match err {
            Error::ClosureViolation(v) -> { quarantine(a) for 60s; },
            _                          -> { bubble(err); },
        }
    }
}
```

The recovery primitives:

- **absorb** — just return; the failure is noted and contained.
- **`restart(child)`** — dissolve and re-create it fresh.
- **`restart_in_place(child)`** — reset it, keeping its region.
- **`quarantine(child) for d`** — pause it, preserving state for
  inspection, optionally auto-restarting after `d`.
- **`bubble(err)`** — pass it up to *this* locus's parent.
- **`dissolve(child)`** — force it down.

If a failure bubbles past the root with no one absorbing it, the
process exits non-zero with a structured report. That's the only
way a Hale program "crashes" — and it's a deliberate, typed
event, not a surprise. This is Erlang's let-it-crash, but the
recovery policy is *typed* and written next to the locus it
governs.

## Crossing from value to structural

Sometimes a method catches a value-level error and decides it's
fatal — the right move is to stop this locus and let the
supervisor take over. You bridge with an *inline* closure and the
`violate` statement:

```hale
locus DbConnection {
    params { last_error: String = ""; }

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
  `captures:` clause snapshots locus state into the violation
  payload.
- `violate fatal_io;` fires it. It's divergent (the `Never` type,
  like `fail` and `bubble`), so the branches that violate need no
  `return`. The locus enters drain at the next yield; the parent's
  `on_failure` gets the typed violation with the captured state.
- `self.draining` is a Bool every locus can read — true once it's
  decided to wind down. Use it to stop publishing after the
  decision.

That's the canonical "catch an error and shut this locus down"
shape: one closure, one error-check method, one `violate`. You
don't reach for a hand-rolled `should_exit` flag and a polling
loop — these primitives are the supported form.

Next: splitting a program across processes — [Across
binaries](./multi-binary.md).
