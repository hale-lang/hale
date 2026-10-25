# Failure & Supervision

Hale separates recoverable call errors from supervisor-level component failures into two distinct channels: the **value channel** (`fallible(E)` and `or`) and the **structural channel** (`closure`, `violate`, and `on_failure`). When a locus's invariant breaks, the runtime constructs a typed `ClosureViolation` and routes it up the ownership tower to the parent locus's `on_failure` hook, which decides whether to absorb, restart, quarantine, dissolve, or escalate.

```hale
locus Account {
    params { debits: Decimal = 0.00d; credits: Decimal = 0.00d; }

    closure balanced {
        self.debits ~~ self.credits within 0.01d;
        epoch tick;
    }
}

locus Bank {
    accept(a: Account) { }

    on_failure(a: Account, err: ClosureViolation) {
        quarantine(a);
    }
}
```

## Two Failure Channels

Hale provides no `panic` statement or uncaught exceptions (`assert` exists only in the test library). Every error flows through one of two typed channels:

| Channel | Mechanism | Direction | Allowed Surfaces & Purpose |
|---|---|---|---|
| **Value channel** | [`fallible(E)`](../basics/fallible.md) + `or` | Up the **call stack** | Free functions and user-declared locus `fn` methods. Rejected on runtime-driven hooks (`birth`, `run`, `dissolve`, mode bodies, closure assertions, bus handlers) because they have no caller frame. |
| **Structural channel (`↑`)** | `closure` / `violate` → `on_failure` | Up the **locus ownership tower** | Every locus. Triggered by broken invariant closures or explicit `violate` statements; handled by the parent supervisor's `on_failure` hook. |

If a structural failure bubbles past the root `main locus` without being absorbed by an `on_failure` handler, the process exits non-zero and prints a structured violation report.

## Declaring Invariants (`closure`)

A `closure` block inside a locus declares an invariant condition and the `epoch` at which the runtime evaluates it:

| Epoch | Syntax | Evaluation Timing |
|---|---|---|
| **Tick** | `epoch tick;` | Checked after each event-loop iteration or handler turn. |
| **Birth** | `epoch birth;` | Checked immediately after `birth()` completes (including after every `restart` re-runs `birth()`). |
| **Dissolve** | `epoch dissolve;` *(default)* | Checked immediately before `dissolve()` runs. |
| **Periodic** | `epoch duration(1min);` | Checked on the specified time interval. |
| **Inline** | `epoch inline;` | Assertion-less closure (`captures: field1, field2;`) triggered explicitly via `violate NAME;`. |

```hale
locus Account {
    params { debits: Decimal = 0.00d; credits: Decimal = 0.00d; }

    closure balanced {
        self.debits ~~ self.credits within 0.01d;
        epoch tick;
    }
}
```

- The `~~ ... within <tolerance>` operator tests approximate equality within a numeric bound.
- Closures are silent when their assertion holds. When an assertion evaluates to `false`, the runtime builds a `ClosureViolation` (`err.locus`, `err.closure`, `err.diff`) and routes it to the parent's `on_failure` handler.

### Accumulator Closures (`persists_through` & `resets_on`)

Closures that aggregate values across epochs using `sum(...)`, `count(...)`, or `mean(...)` reset their running totals to zero whenever the locus is recovered. To preserve running totals across specific recovery actions, add `persists_through(...)`:

```hale,fragment
closure within_band {
    sum(self.delta) ~~ 0 within 100;
    epoch tick;
    persists_through(quarantine);
}
```

- Valid recovery names in `persists_through(...)` and `resets_on(...)` are `restart`, `restart_in_place`, and `quarantine` (an exhausted `restart(c) for N` budget transitions to `quarantine` and matches `quarantine`). Passing `dissolve` is a compile error because totals never outlive the locus.
- `resets_on(restart)` explicitly documents the default reset behavior. Naming the same recovery in both `persists_through` and `resets_on` is a compile error.
- `hale check` warns if a closure names a recovery that no `on_failure` handler in the program ever applies to that locus, or if `persists_through` is declared on a closure that contains no `sum`, `count`, or `mean` accumulator.

## Supervising Children (`on_failure`)

A parent locus supervises its children—both dynamic children admitted via `accept` and static children held in `params`—by declaring one `on_failure` handler per child type:

```hale
locus Cache { params { hits: Int = 0; } }

locus App {
    params { cache: Cache = Cache { }; server: std::http::Server; }
    on_failure(s: std::http::Server, err: ClosureViolation) { eprintln("server down"); }
    on_failure(c: Cache, err: ClosureViolation) { restart(c); }
}
```

- **One handler per child type**: A parent can declare at most one `on_failure` per child type; `hale check` rejects duplicate handlers for the same type.
- **Supervision of `params` children**: A `params` child routes failures to its parent whether constructed via the parameter's default initializer or passed as an inline literal at parent construction (`App { conn: Conn { url: std::env::var("URL") } }`). However, constructing a child in an outer `let` variable and passing it by name (`let c = Conn { }; App { conn: c };`) borrows `c` and preserves the supervision route of the scope that created `c`.
- **Mandatory subtree supervision (`@supervised`)**: Annotating a locus with `@supervised` requires every locus in its ownership subtree to be covered by an `on_failure` policy in scope.

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

### Recovery Actions

Inside an `on_failure(child: T, err: ClosureViolation)` body, the supervisor selects a recovery policy:

| Recovery Action | Syntax | Runtime Behavior |
|---|---|---|
| **Absorb** | *(return from handler or `return;`)* | Contains the failure. The child stops running, and its fields remain readable on the parent until parent teardown. |
| **Restart** | `restart(child);` | Re-runs `birth()` (and any `birth_check` closures) and `run()` on the same instance on its assigned thread/pool, and re-arms its bus subscriptions. Defaults to a limit of **2** restart attempts. |
| **Bounded restart** | `restart(child) for N;` | Allows at most `N` cumulative restarts over the child's lifetime (recorded as `retry_bound` in the topology artifact; `for 0` never restarts). On failure `N + 1`, the child is **quarantined**. |
| **Restart in place** | `restart_in_place(child);` | Resets all `params` fields of `child` back to the initial values it was constructed with, then re-runs `birth()` and `run()`. |
| **Quarantine** | `quarantine(child);` | Pauses the child while preserving its memory and fields for inspection. (`quarantine(child) for d` and `until` modifiers are not implemented and are rejected at compile time.) |
| **Escalate** | `bubble(err);` | Forwards the `ClosureViolation` up to this supervisor's own parent locus. |
| **Force teardown** | `dissolve(child);` | Immediately tears the child down (`dissolve()`). |

```hale,fragment
on_failure(c: Book, err: ClosureViolation) {
    restart(c) for 3;
}
```

> [!TIP]
> Always use `restart(c) for N;` rather than maintaining a manual retry counter in a parameter field. Both enforce the limit at runtime, but `for N` exports the bound as `retry_bound` in the topology artifact so deployment verification and runtime telemetry can audit retry budgets.

Recovery operations apply to their argument, which may be another child
held by the supervisor. `hale check` uses each argument's resolved type,
including in generic handlers, when checking whether a closure's
`persists_through` or `resets_on` event can occur. The handled child's
type alone does not establish that an operation applies to it. This is
a type-level check; it does not prove which instance an alias names or
whether a conditional recovery executes. The compiler also uses typed
recovery receivers to retain original parameters for `restart_in_place`,
including when it is called through a helper. An unresolved receiver
conservatively retains those snapshots.

A dissolve-epoch closure uses the supervisor route stored when that child
was constructed, including during flow reclaim and recursive field teardown.
Two instances of the same supervisor type keep distinct routes. The handler
completes while the child is still readable, before its arena is reclaimed;
without a handler, the violation remains a structural failure.

### Inspecting recovery targets

Inspect the represented handler and helper paths with:

```sh
hale model dump app/
```

The experimental dump's `first conditional recovery targets` section keeps
same-type siblings distinct: a helper receiving `App.parent.first` does not
implicitly target `App.parent.second`. It shows the first represented target
and its source call path, or reports incomplete inspection. Each search is
limited to 512 states; reaching that limit is reported explicitly.

These observations are conditional on the represented construction entering
its handler. They do not prove that a failure occurs, a handler is admitted,
the target is live, or recovery remains inside a runtime boundary. An empty
result supplies no containment guarantee. Fields, missing argument bindings,
implicit lifecycle work and incarnation identity still need separate coverage.
See [Recovery observations](../the-model.md#recovery-observations) for the
model's scope. This inspection adds no recovery-containment claim or proof.

### Handler Threading & Initialization Timing

- **Deferred delivery during parent `params` initialization**: If an earlier sibling parameter fails while the parent is still initializing later parameters in its `params` block, the runtime holds the `ClosureViolation` until all parent `params` are initialized and delivers it immediately before the parent's `birth()`. This guarantees that `on_failure` never reads uninitialized sibling parameters or has its writes clobbered by later defaults.
- **Executes on the parent's thread**: Even if the failing child runs on a `pinned` thread or a separate cooperative pool, `on_failure` runs on the parent's own thread (at the parent's next sleep, wait, yield, bus message, or shutdown join) while the child thread pauses for the verdict.
- **Field replacement precedence**: If an `on_failure` handler replaces a child field (`self.b = Kid { }`) while `b` has a pending failure or is being handled, the field immediately points to the new instance; the old instance is torn down once after its handler finishes, and any `restart(b)` or `restart_in_place(b)` on a replaced or tearing-down child is ignored.

## Bridging Value Errors to Structural Failure (`violate`)

When a locus method encounters a fatal `fallible(E)` value error (such as a broken socket) and needs to shut the locus down and trigger parent supervision, use the four-part inline violation pattern:

1. **`closure NAME { captures: f1, f2; epoch inline; }`**: Declares a named structural failure that fires only when explicitly invoked.
2. **`violate NAME;` (or `violate NAME with expr;`)**: Divergent statement (`Never` type) that sets `self.draining = true`, delivers a `ClosureViolation` to the parent's `on_failure`, and exits the method.
3. **Error-check bridge method**: A locus method `fn handle_err(e: E) -> T fallible(ClosureViolation)` called via `or` at fallible call sites that either returns a fallback `T` or executes `violate NAME;`.
4. **`self.draining`**: Synthetic boolean field set to `true` as soon as `violate` (or shutdown) triggers, used to guard subsequent publishes.

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

    fn handle_io(e: IoError) -> Row fallible(ClosureViolation) {
        self.last_error = e.kind;
        if e.kind == "broken_pipe" {
            violate fatal_io;        // diverges: triggers parent on_failure
        }
        return Row { data: "" };     // transient error: substitute fallback Row
    }

    fn on_query(q: Query) {
        let r = send_query(self.conn_fd, q)
            or (self.handle_io(err) or { return; });
        QueryResult <- r;
    }
}
```

### Rules for `violate` and `ClosureViolation`

- **Reading captured state**: `ClosureViolation` exposes fixed metadata fields (`err.locus`, `err.closure`, and `err.diff`). The supervisor reads the fields listed in `captures:` directly off the child parameter passed to `on_failure(c, err)` (e.g., `c.last_error`, not `err.last_error`).
- **Required `fallible(ClosureViolation)` on value-returning methods**: Because a value-returning method that executes `violate` produces no return value after `on_failure` finishes, it must be declared `fallible(ClosureViolation)` so the caller's outer `or` (`or { return; }`, `or fallback`, or `or handler(err)`) handles the aborted call.
- **Void and perspective methods**: A method with no return type that executes `violate` simply returns after `on_failure` (emitting a warning that `fallible(ClosureViolation)` will be required in a future release). A method serving a [`perspective`](./perspectives.md) cannot be marked `fallible`, so the compiler emits a warning and callers must not read its return value after a violation.

Next: [Multi-Binary Deployment & Fleets](./multi-binary.md).
