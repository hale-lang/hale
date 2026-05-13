# Recovery and supervision

[Chapter 7](./07-closures.md) introduced
[`ClosureViolation`](../reference/glossary.md#closureviolation)
and named the three outcomes of a failed closure: **collapse**,
**absorb**, **bubble**. This chapter goes deeper. It covers the
parent's role as the supervision authority for its descendants,
the four recovery primitives a parent's `on_failure` body can
invoke, the `evaluate(closure_name)` explicit trigger, and how
`self.children` exposes the running child population.

The substrate's commitment is that **failure has a finite,
visible upward path** (per **F.8** vertical-only-flow). This
chapter is about the parent end of that path.

## `on_failure(child, err)`

A parent locus declares an `on_failure` method to handle child
violations:

```aperio
locus Coordinator {
    on_failure(w: Worker, err: ClosureViolation) {
        println("Coordinator: ", err.locus, " failed ",
                err.closure, " — quarantining");
        quarantine(w);
    }

    run() {
        Worker { valid: 1 };
        Worker { valid: 0 };  // closure fails
    }
}
```

The signature is:

```aperio
on_failure(child_binding: ChildLocusType, err: ClosureViolation) {
    // recovery primitive call(s)
}
```

- **`child_binding`** — a typed reference to the failing child.
  The body uses it as the argument to recovery primitives:
  `restart(child_binding)`, `quarantine(child_binding)`, etc.
- **`err`** — the structured violation, with the fields
  introduced in chapter 7 (`locus`, `closure`, `left`, `right`,
  `tolerance`, `diff`).

A parent without an `on_failure` body sees its children's
violations bubble — they propagate to the grandparent silently.
A parent with `on_failure` chooses one of four primitives, or
returns without calling any (which counts as **absorb**).

## The four recovery primitives

### `restart(child)`

*"Give birth another shot."*

Re-runs the child's `birth()` body on the same memory. State
mutated by the previous attempt is preserved — a `birth` whose
job is to *advance* state through transient setup phases is
the right candidate.

```aperio
locus Worker {
    params {
        attempts: Int = 0;
        target: Int = 2;
    }

    closure attempt_reached_target {
        self.attempts ~~ self.target within 0;
        epoch birth;
    }

    birth() {
        self.attempts = self.attempts + 1;
        println("Worker.birth: attempts=", self.attempts);
    }
}

locus Coordinator {
    on_failure(c: Worker, err: ClosureViolation) {
        restart(c);
    }

    run() {
        // First birth: attempts=1, fails (1 != 2).
        // restart fires; second birth: attempts=2, passes.
        Worker { target: 2 };
    }
}
```

A v0 cap of **2 attempts per locus lifetime** applies. After
the cap, calling `restart` still bumps the counter (so the
parent can observe the count) but the runtime skips the re-run;
the violation falls through to the parent's collapse path.

### `restart_in_place(child)`

*"Factory-reset, then give birth another shot."*

Same as `restart`, but adds a pre-pass that zeroes user fields
back to their declared defaults *before* re-running `birth`.
Use this when each attempt must be independent — for example,
when the previous attempt left bookkeeping in a partial state
that would skew a re-run.

The cap-2 budget is shared between `restart` and
`restart_in_place`: a locus can use at most two attempts
total, in any combination.

When to choose which:

| Pattern | Primitive |
|---|---|
| Build state forward; retry should pick up from where the previous attempt left off. | `restart` |
| State machine entered an inconsistent intermediate; retry must start clean. | `restart_in_place` |

The `38-restart-in-place` example walks the contrast
explicitly.

### `quarantine(child)`

*"This locus's invariants don't hold; stop trying."*

Sticky: once quarantined, the child's `run()` is skipped;
`drain` and `dissolve` still fire as cleanup. Bus subscribers
that are quarantined do not receive any further messages on
their subscriptions (per m41b).

```aperio
locus Coordinator {
    on_failure(w: Worker, err: ClosureViolation) {
        println("Coordinator: ", err.locus, " failed ",
                err.closure, " — quarantining");
        quarantine(w);
    }

    run() {
        Worker { valid: 1 };  // closure passes; run() fires
        Worker { valid: 0 };  // closure fails; quarantined;
                              // run() skipped
    }
}
```

`quarantine` is the right primitive when the child's state has
demonstrated an unrepairable inconsistency. `restart` would
loop indefinitely against a violation rooted in the locus's
configuration, not its transient state; `quarantine` stops the
loop and lets the parent decide what to do (continue without
this child, escalate further, etc.).

### `bubble(err)`

*"This is not mine to handle; it belongs to my parent."*

Propagates the violation to the grandparent. Calling
`bubble(err)` from `on_failure` is *equivalent* to not
declaring `on_failure` at all — the violation rises one frame
in either case. The explicit form is useful when the parent
inspects the violation, decides this particular case is
out-of-scope, and wants to make the bubble visible at the
source level rather than implicit through omission.

```aperio
on_failure(c: Worker, err: ClosureViolation) {
    if err.diff > 100 {
        // Big violations go up.
        bubble(err);
    } else {
        // Small ones we absorb here.
        println("absorbed minor violation");
    }
}
```

If a `ClosureViolation` bubbles past `main`, the process
exits non-zero with the violation report on stderr.

### Returning without calling a primitive: absorb

If `on_failure` runs and returns without calling any
primitive, the violation is **absorbed**: the parent has
acknowledged the failure and chosen no specific recovery; the
child's dissolution proceeds normally from the parent's
perspective.

```aperio
on_failure(c: Worker, err: ClosureViolation) {
    println("noted: ", err.closure, " on ", err.locus);
    // return without calling restart / quarantine / bubble.
    // This is absorption.
}
```

Absorption is the right choice when the parent's job is to
*observe* failures (logging, metrics, alerting) but not to
intervene in the child's lifecycle.

## `evaluate(closure_name)`: explicit-epoch firing

Closures with `epoch explicit;` fire only when the locus's body
calls a built-in trigger. The trigger is `check_closures();` —
a call with no arguments that fires every explicit-epoch
closure declared on the calling locus.

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
        // After both legs are applied, the double-entry
        // invariant should hold.
        check_closures();
    }
}
```

The discipline this enforces: the locus *chooses* when its
invariants are auditable. A `tick`-epoch closure would fire
after every `on_post` invocation — including in mid-batch
states where one side has been updated and the other has not.
An explicit-epoch closure fires only when the locus declares
"this is a meaningful checkpoint."

> **Aside on naming.** The trigger is called `check_closures()`
> in v0 — a single call that fires *all* explicit-epoch
> closures on the locus. A future variant
> `evaluate(closure_name)` for firing one specific closure by
> name is on the roadmap; v0 ships with the bulk form.

## `self.children`: the running child population

A parent locus reads its currently-attached children via
`self.children` — an iterable over every child the parent has
accepted that has not yet dissolved.

```aperio
locus RichCoord : projection rich {
    accept(c: Leaf) { }
    run() {
        let _l1 = Leaf { value: 1 };
        let _l2 = Leaf { value: 2 };
        let _l3 = Leaf { value: 3 };

        let mut total: Int = 0;
        for child in self.children {
            total = total + child.value;
        }
        println("rich: total=", total);
    }
}
```

`self.children` is iterable in `for x in self.children { ... }`
and exposes a length: `self.children.length` returns an `Int`.
The contract surface a parent can read on each child is exactly
the child's `expose` set — `child.value` here is permitted
because `Leaf` declares `expose value: Int`.

The canonical use of `self.children.length` is in a closure
that audits the population against `k_max`:

```aperio
closure within_capacity {
    self.children.length ~~ 0 within self.k_max;
    epoch tick;
}
```

— "the number of children should be within `k_max` of zero,"
i.e. not exceed `k_max`. A coordinator that accepts more
children than its capacity parameters permit fails this
closure at the next tick; the parent's `on_failure` decides
whether to `quarantine` the over-attached child, `restart` it,
or escalate.

## The supervision graph

Putting it all together, an Aperio program's supervision graph
is a tree:

- **Every locus has at most one parent.** (The root of the
  tree is `main`'s implicit locus.)
- **Every failure travels upward only.** Per F.8, failures do
  not flow laterally; a sibling cannot intercept another
  sibling's `ClosureViolation`.
- **Every parent decides for itself how to respond.** A
  particular violation reaches the first ancestor that has
  `on_failure` declared *and* does not bubble it; everyone
  before that point is implicitly bubbling.
- **The recovery decision is a function of the parent and
  the violation, not the child.** The same child might be
  restarted under one parent and quarantined under another;
  the supervision policy lives where it is decided.

This is Aperio's expression of **supervision trees** — the
classical pattern from Erlang, with the supervision policy
built into the language rather than a library. Each `locus`'s
`on_failure` body is the equivalent of an Erlang supervisor's
restart strategy; the difference is that here the
supervision-tree shape is the same as the structural-locus
tree, no separate supervisor declarations needed.

## What this chapter does not cover

- **Cross-process recovery** — the multi-binary version of
  the supervision pattern. When two cooperating apps are
  separate processes, recovery cannot reach across the bus
  the way it does within one binary; instead each process
  has its own supervision tree, and the bus carries
  *successful* values between them. The capstone in
  [chapter 13](./13-chat-fanout.md) walks the full pattern.
- **`KMaxExceeded`** and other typed runtime failures — the
  closure system handles structural violations; runtime
  failures from the substrate (e.g., over-attachment past
  k_max with dynamic params) raise their own typed errors,
  which `on_failure` can dispatch on. Full coverage is in
  the reference's recovery section.

The next chapter, **[Building
chat-fanout](./13-chat-fanout.md)**, is the capstone. It takes
everything introduced over chapters 1–11 — locus declarations,
lifecycles, contracts, bus pub-sub, closures, generics,
perspectives, recovery — and assembles a production-shaped
Aperio program: a multi-binary server/client chat app with a
shared schema, perspectives shipping authenticated session
records, and per-binary supervision.
