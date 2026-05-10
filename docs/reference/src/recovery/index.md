# Recovery operations

## Synopsis

When a child locus's closure fails, the runtime emits a
`ClosureViolation` and dispatches it to the parent's
`on_failure` body. From inside `on_failure`, the parent picks
exactly one of four recovery primitives — or returns without
calling any, which counts as *absorption*. Recovery is per
**F.9**.

## `on_failure` declaration

```text
on-failure-decl ::= "on_failure" "(" Ident ":" type-expr ","
                                       Ident ":" "ClosureViolation" ")"
                       block
```

```aperio
locus Coordinator {
    on_failure(c: Worker, err: ClosureViolation) {
        // recovery body
    }
}
```

The first parameter is a typed binding to the failing child;
the second is the structured violation.

A locus may declare multiple `on_failure` bodies — one per
child type — and the runtime dispatches based on the failing
child's static type.

## The four primitives

### `restart(child)`

Re-run the child's `birth` body on the same memory. State
mutated by the previous attempt is preserved.

```aperio
on_failure(c: Worker, err: ClosureViolation) {
    restart(c);
}
```

Cap: 2 attempts per locus lifetime. After the cap, the runtime
skips the re-run; the violation falls through to the parent's
collapse path.

Use when `birth`'s job is to *advance* state through transient
setup phases — the retry should pick up from where the
previous attempt left off.

### `restart_in_place(child)`

Same as `restart`, but adds a pre-pass that zeroes user fields
back to declared defaults *before* re-running `birth`.

```aperio
on_failure(c: Worker, err: ClosureViolation) {
    restart_in_place(c);
}
```

Shares the cap-2 budget with `restart` — at most 2 attempts
total, in any combination.

Use when each attempt must be independent — the previous
attempt left bookkeeping in a partial state that would skew a
re-run.

### `quarantine(child)`

Sticky stop. The child's `run` is skipped; `drain` and
`dissolve` still fire as cleanup. Bus subscribers that are
quarantined do not receive further messages on their
subscriptions (per m41b).

```aperio
on_failure(c: Worker, err: ClosureViolation) {
    quarantine(c);
}
```

Use when the child's state has demonstrated an unrepairable
inconsistency. `restart` would loop indefinitely against a
violation rooted in the locus's configuration; `quarantine`
stops the loop.

### `bubble(err)`

Propagate the violation to the grandparent. Equivalent to *not
declaring* `on_failure` at all (the runtime bubbles
implicitly). The explicit form is useful when the parent
inspects the violation, decides this case is out-of-scope, and
wants to make the bubble visible at the source level.

```aperio
on_failure(c: Worker, err: ClosureViolation) {
    if err.diff > 100 {
        bubble(err);
    } else {
        // small violations absorbed silently
    }
}
```

If a `ClosureViolation` bubbles past `main`, the process exits
non-zero with the violation report on stderr.

## Absorption (no primitive call)

If `on_failure` runs and returns without calling any primitive,
the violation is absorbed: the parent has acknowledged the
failure but chosen no specific recovery; the child's
dissolution proceeds normally from the parent's perspective.

```aperio
on_failure(c: Worker, err: ClosureViolation) {
    println("noted: ", err.closure, " on ", err.locus);
    // return without calling restart / quarantine / bubble
}
```

Use when the parent's job is to *observe* failures — logging,
metrics, alerting — but not to intervene in the child's
lifecycle.

## Explicit closure firing

For `epoch explicit;` closures, the locus body fires audits at
chosen checkpoints with `check_closures();`:

```aperio
fn on_post(t: Tx) {
    self.debits = self.debits + t.debit;
    self.credits = self.credits + t.credit;
    check_closures();
}
```

`check_closures();` fires every explicit-epoch closure on the
calling locus. A future `evaluate(closure_name)` form for
firing one closure by name is on the roadmap.

## `self.children` for population audits

A parent locus can iterate its currently-attached children via
`self.children`:

```aperio
for child in self.children {
    println(child.exposed_field);
}

closure within_capacity {
    self.children.length ~~ 0 within self.k_max;
    epoch tick;
}
```

The contract surface visible on each child is exactly its
declared `expose` set; reading any other field is a
compile-time error per **F.17**.

## See Also

- [Closure assertions](../closures/index.md)
- [ClosureViolation propagation](../closures/violation.md)
- [Locus declarations](../loci/index.md)
