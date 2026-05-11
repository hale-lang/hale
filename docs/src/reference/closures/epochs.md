# Epoch semantics

## Synopsis

Every closure has an *epoch*: the moment in the locus's
lifecycle when the runtime evaluates the assertion. v0 ships
five epochs.

## The five epochs

| Epoch | Fires |
|---|---|
| `birth` | Once, after the locus's `birth()` body returns |
| `dissolve` | Once, as the locus dissolves (default) |
| `tick` | After every substrate cell — each bus handler invocation, and after `run()` returns |
| `duration` | At fixed time intervals (e.g., `every 5s`) |
| `explicit` | Only when user code calls `check_closures();` |

## Default

If a closure declaration omits `epoch`, the default is
`dissolve`. This is the most common case — *did the locus end
in a valid state?*.

```aperio
closure xy_match {
    self.x ~~ self.y within 0;
    // epoch dissolve;   -- implicit
}
```

## `epoch birth`

Fires once, after `birth()` returns. Audit invariants that
should hold after initial setup:

```aperio
closure cap_initialized {
    self.cap ~~ 0 within 0;
    epoch birth;
}
```

A failing birth-epoch closure puts the locus into an
explosion path immediately after birth — it is the right hook
for "this locus failed to initialize correctly" patterns. The
parent's `on_failure` body decides whether to `restart`,
`restart_in_place`, `quarantine`, or `bubble`.

## `epoch dissolve`

Fires once, as the locus dissolves. Audit invariants that
should hold at end-of-life:

```aperio
closure all_intents_filled {
    self.intents_emitted ~~ self.fills_received within 5;
    epoch dissolve;
}
```

The canonical use is the at-shutdown audit — "did this locus
balance its emissions against its receipts?"

## `epoch tick`

Fires after every substrate cell — every bus handler
invocation, every lifecycle transition. The closure sees `self`
in its post-cell state.

```aperio
closure under_cap {
    self.n ~~ 0 within 5;
    epoch tick;
}
```

Useful for monitoring drift mid-life. Where birth audits the
initial state and dissolve audits the final state, tick audits
the *steady-state pulse*: does the invariant still hold at each
step?

## `epoch duration`

Fires at fixed time intervals, independent of cell boundaries.

```aperio
closure heartbeat {
    self.last_event_at ~~ time::monotonic() within 10s;
    epoch duration every 1s;
}
```

(Surface form `every <duration>` is part of the duration-epoch
syntax.) Useful for timeout-shaped invariants where wall-clock
elapsed time matters more than work performed.

## `epoch explicit`

Fires only when user code calls the built-in
`check_closures();`:

```aperio
locus Ledger {
    params {
        debits: Int = 0;
        credits: Int = 0;
    }

    closure balanced {
        self.debits ~~ self.credits within 0;
        epoch explicit;
    }

    bus {
        subscribe "post" as on_post of type Tx;
    }

    fn on_post(t: Tx) {
        self.debits = self.debits + t.debit;
        self.credits = self.credits + t.credit;
        check_closures();   // fires every explicit-epoch closure
    }
}
```

The discipline this enforces: the locus chooses when its
invariants are auditable. A `tick`-epoch closure would fire
after every `on_post` — including in mid-batch states where
one side is updated and the other is not. An explicit-epoch
closure fires only when the locus declares "this is a
meaningful checkpoint."

`check_closures();` is a built-in trigger that fires *all*
explicit-epoch closures on the calling locus. A future
`evaluate(closure_name)` form for firing one specific closure
by name is on the roadmap.

## Accumulator state across epochs

A closure with accumulators (`sum`, `count`, `mean`) maintains
per-instance accumulator state across fires of the same epoch.
At each fire:

1. Evaluate the accumulator's argument expression.
2. Update the per-closure accumulator state.
3. Evaluate the assertion against the post-update value.

Accumulator state lives in the locus's arena and is freed at
dissolve.

## See Also

- [Closure assertions](./index.md)
- [Closures (locus members)](../loci/closures.md)
- [ClosureViolation propagation](./violation.md)
