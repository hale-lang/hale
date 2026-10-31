# Locus Lifecycle

A long-running locus executes through a deterministic sequence of runtime-managed phases: `birth → run → drain → dissolve`. Instead of threading cancellation contexts or manual `defer` callbacks through functions, you implement lifecycle hooks on the locus and the runtime sequences initialization, steady-state execution, depth-first shutdown cascading, and arena reclamation automatically.

```hale
locus Ticker {
    params { count: Int = 0; limit: Int = 3; }

    birth() {
        println("starting ticker");
    }

    run() {
        while !self.draining && self.count < self.limit {
            println("tick ", self.count);
            std::time::sleep(500ms);
            self.count = self.count + 1;
        }
    }

    drain() {
        println("stopping intake");
    }

    dissolve() {
        println("cleaning up resources");
    }
}

fn main() {
    Ticker { limit: 3 };
}
```

## Lifecycle Phases

Every lifecycle hook is optional; the compiler supplies a no-op default for any phase you omit.

| Phase / Hook | Target | Execution Timing & Purpose |
|---|---|---|
| `accept(c: Child)` | Parent | Runs after `c`'s `params` are initialized and before `c.birth()`. Admits the child into `self.children`; cannot reject it. See [Parents & Children](./parents-children.md). |
| `birth()` | Locus | Runs once at construction after `params` are initialized and bus subscriptions are registered. Acquire sockets, file descriptors, and buffers here. |
| `run()` | Locus | Steady-state execution loop. Runs until it returns naturally or shutdown begins. Omitting `run()` is identical to writing `run() { }`. |
| `drain()` | Locus | Runs when shutdown begins (`self.draining` becomes `true`). Cascades depth-first through children before running on the parent. Stop accepting new work and finish in-flight operations here. |
| `release(c: Child)` | Parent | Runs when an accepted flow child's `run()` completes, after `c.drain()` and before `c.dissolve()`. Marks `Child` as a per-request flow rather than a long-lived resident. |
| `dissolve()` | Locus | Runs last during teardown. Release external OS handles acquired in `birth()`. The runtime frees the locus's memory arena wholesale immediately after `dissolve()` completes. |
| `on_failure(c: Child, err: ClosureViolation)` | Parent | Supervisor hook invoked immediately when a child's `closure`, `birth_check`, or `violate` statement triggers. See [Failure & Supervision](./failure.md). |

```hale
locus Server {
    params { listen_fd: Int = -1; }

    birth()    { /* acquire sockets, files, buffers */ }
    run()      { /* steady-state main loop */ }
    drain()    { /* stop taking new work; finish in-flight work */ }
    dissolve() { /* close descriptors acquired in birth() */ }
}
```

> [!IMPORTANT]
> Lifecycle bodies (`birth`, `run`, `drain`, `dissolve`) are invoked by the runtime and return no value. A bare `return;` exits the phase early, whereas `return <value>;` and `fallible(E)` annotations on lifecycle hooks are rejected at compile time.

## Instantiation & Teardown Scope

Hale determines when a locus drains and dissolves from the syntactic position where it is constructed:

| Instantiation Form | Example | Teardown Timing |
|---|---|---|
| **Statement position** | `Ticker { };` or `make_ticker();` | Runs `birth → run → drain → dissolve` immediately at the end of the statement (fire-and-forget). |
| **`let`-bound local** | `let t = Ticker { };` | Born and runs immediately; `drain` and `dissolve` are deferred to the exit of the enclosing function scope. Multiple `let` bindings dissolve in reverse creation order (LIFO). |
| **Consumed inline expression** | `Ticker { }.tick()`, `serve(Ticker { })`, `combine(a, make_ticker())` | Treated identically to a `let` binding in the enclosing function: born before the expression evaluates and dissolved when the enclosing function exits. |
| **Owned `params` child** | `params { s: Server = Server { }; }` | Owned by the parent locus; drains and dissolves as part of the parent's teardown cascade. |
| **Returned to caller** | `return Ticker { };` or `return pass(t);` | Ownership transfers to the caller's receiving binding or expression. |
| **Long-lived subscriber** | Any locus with bus subscriptions or an active `run()` | Remains alive until its owning scope exits so bus handlers can continue firing. |

### Factory Calls, Conditionals, and Collections

When a function returns a locus handle, the consumer of that call owns the instance:

- **Fallible factories**: Reaching a factory through `or` preserves ownership rules. `let c = std::process::spawn(argv) or raise;` closes the child's pipes and reaps the process when the enclosing function exits. When a fallback builds an alternative locus (`Mid { leaf: make_leaf_f() or backup_leaf() }`), the parent field owns whichever instance was constructed.
- **Pass-through and builder functions**: Returning a locus parameter (`fn pass(t: Ticker) -> Ticker { return t; }`) or wrapping it in a returned parent (`fn routes(w: W) -> Routes { return Routes { w: w }; }`) transfers ownership to the caller or returned parent locus.
- **Branching expressions (`if`, `match`, blocks)**: Ownership is resolved per execution path on the tail expression of each arm. Any locus constructed inside a condition, match scrutinee, or non-final statement of a block is owned by the enclosing function and dissolved at function exit.
- **Arrays and tuples**: In `let ts: [Ticker; 2] = [make_ticker(), make_ticker()];`, the enclosing function owns and releases each element at scope exit. In a locus parameter (`cs: [C; 2] = [C { }, C { }]` or `p: (H, W) = both(W { })`), the parent locus owns every element: elements drain in index order before the parent and dissolve after the parent's `dissolve()`.
- **Non-locus parameter initializers**: If a temporary locus is evaluated purely to compute a scalar parameter (`Lonely { n: Queries { }.total() }`), `Queries { }` is not stored in `Lonely`; it belongs to the enclosing function scope.

### Loop Instantiation

While a `let` binding's storage slot is allocated for the enclosing function (and its variable name is scoped to the enclosing block), a locus constructed inside a `while` or `for` loop is reclaimed on each iteration when execution reaches that line again:

```hale,fragment
while i < steps {
    let m = zeros(rows, cols);   // fresh arena allocated each iteration
    i = i + 1;
}                                // previous `m` drains and dissolves at the top of the next iteration;
                                 // the final instance dissolves when the function returns
```

- The loop holds only one live instance of `m` at a time rather than accumulating `steps` arenas.
- This per-iteration boundary applies equally to `let` bindings, factory calls, and consumed inline literals (`Matrix { }.trace()`, `sum(Matrix { })`). A bare statement literal (`Matrix { };`) still dissolves immediately at the semicolon.
- If a single loop iteration binds two loci in sequence, each line reclaims its own previous instance as it is reached—meaning they reclaim in declaration order across iterations (whereas function exit reclaims in reverse order).
- `hale check` emits a performance warning when a locus is instantiated inside a loop because allocating and tearing down an arena every iteration adds overhead. For hot loops, hoist a single instance outside the loop and reset its state in place.

### Early `return` Teardown

Exiting a function via an early `return` (including inside `fn main()`) drains and dissolves all loci constructed up to that point in reverse creation order:

```hale,fragment
let store = Store { path: dir };
if std::env::args_count() < 2 {
    println("usage: report <name>");
    return 2;                     // drains and dissolves `store`, then exits with code 2
}
let report = Report { store: store };
                                  // normal exit dissolves `report`, then `store`
```

A locus constructed only inside a conditional branch is dissolved only when that branch executes.

## Child Initialization Order & Blocking `run()`

A parent locus initializes its `params` fields strictly in declaration order on the constructing thread:

1. **Parameter defaults**: A parameter's default expression may read `self.<field>` for any parameter declared *before* it. `hale check` rejects reading a parameter declared after it (or reading the parameter itself).
2. **Inline child execution**: When a child locus shares the parent's thread (the default `cooperative(pool = main)`), the child's `birth()` and `run()` execute immediately when its parameter slot is initialized—before the next sibling parameter is constructed.

If an inline child's `run()` contains an infinite loop, subsequent sibling parameters are never born:

```hale
locus Server {
    run() { while true { std::time::sleep(1s); } }   // blocks construction thread
}
locus Metrics {
    birth() { println("metrics up"); }               // never reached
}

main locus App {
    params {
        server:  Server  = Server { };
        metrics: Metrics = Metrics { };              // never born!
    }
}

fn main() { App { }; }
```

Because `Metrics` is never born, its bus subscriptions and sockets are never initialized. `hale check` warns whenever it can statically prove that an inline child on `main` runs an unbounded `while` loop before a later sibling parameter.

To prevent a blocking child from starving later siblings:

1. **Declare the blocking child last** in `params`:
   ```hale,fragment
   params {
       metrics: Metrics = Metrics { };
       server:  Server  = Server { };
   }
   ```
2. **Place the blocking child on its own thread or pool** so `main` can continue constructing subsequent parameters:
   ```hale,fragment
   params {
       server:  Server  = Server { };
       metrics: Metrics = Metrics { };
   }
   placement { server: pinned; }   // or cooperative(pool = io)
   ```
3. **Move coordinator loops into `main`'s `run()`**, which executes only after all `params` children have been born.

## Ownership Tree, Borrowing, and Field Replacement

### No Recursive Self-Containment

A locus cannot contain an instance of itself by value with a default initializer, either directly or through a cycle of locus types (`Alpha → Beta → Alpha`):

```hale,refused
locus Node {
    params { n: Int = 0; next: Node = Node { n: 1 }; }   // error: recursive default construction
}
```

Because every default `Node` would construct another `Node`, initialization could never terminate. To model collections or linked structures, omit the default (`next: Node;`), store elements in a [`@form` collection](../systems/forms.md) or `capacity` slot, or use a plain data `type`.

### Owned Children vs. Borrowed Handles

How a child is supplied to a `params` field determines whether the parent owns it or borrows it:

- **Owned child**: Constructed directly in the `params` initializer (via a literal `Leaf { }`, a factory `make_leaf()`, an `or` fallback `make_leaf_f() or backup_leaf()`, an `interface` slot `j: Counter = Churner { }`, or a `perspective` slot `router: perspective(Router) = RouterV1 { }`). The child's lifecycle is tied to the parent's tree.
- **Borrowed handle**: Passed in by referencing an existing variable or field (`Mid { leaf: shared }`, or assigning `self.journal = shared` on an interface/perspective slot). The parent's teardown cascade skips borrowed handles; the scope that originally created `shared` remains responsible for dissolving it.

```hale,fragment
locus Queries {
    params { j: Counter = Churner { }; }                  // owned interface child
}
locus Gateway {
    params { router: perspective(Router) = RouterV1 { }; } // owned perspective child
}
```

> [!WARNING]
> **Borrow Lifetime and Thread Constraints**
> `hale check` enforces two invariants on every borrowed locus handle:
> 1. **Must outlive holder**: You cannot store a function's local `let` binding or a bus handler's payload into a persistent child of `self`, return a literal that borrows `self.<field>` or a local `let`, or store an inner-block `let` into an outer binding. The sole exception is a borrowed handle read exclusively inside the holder's `birth()` hook.
> 2. **Must stay on the same thread/pool**: Passing a handle across pools (e.g., borrowing a `main`-pool locus inside an `io`-pool locus) is rejected at compile time unless the target is a synchronized `@form` (`sync = serialized`). Bus messages copy values (`String`, structs) into the receiver's storage; to pass a collection from a handler to a child, copy the elements into the child's own `@form(vec)` inside `birth()`.

### Replacing a Locus Field (Break-Before-Make)

Assigning a new locus literal to an existing locus-typed parameter field (or array element `self.cs[0] = C { }`) triggers a **break-before-make** lifecycle replacement:

```hale,fragment
self.conn = Connection { url: next };   // drains and dissolves old conn, then births new conn
self.conn.url = next;                   // mutates existing instance in place; no teardown
```

1. The runtime drains and dissolves the previous `self.conn` instance (releasing its descriptors and children).
2. The new `Connection` literal is constructed in the parent's arena and assigned to `self.conn`.
3. The right-hand side of a field replacement **must be a locus literal**, not a factory call:
   ```hale,fragment
   self.conn = Connection { url: next };   // valid: constructed directly in parent arena
   self.conn = make_connection(next);      // compile error: ambiguous ownership between factory frame and field
   ```

## Graceful Shutdown, `self.draining`, and `terminate;`

### Signal Handling and `self.draining`

Every locus has a synthetic boolean field `self.draining` (initialized to `false`). When the process receives `SIGINT` (Ctrl-C) or `SIGTERM`, or when `main`'s `run()` returns:

1. `self.draining` becomes `true` across all loci in the program.
2. Any in-progress `std::time::sleep` inside `run()` wakes immediately so loops conditioned on `while !self.draining` exit without waiting out their timer. (Sleeps executed inside `drain()` or `dissolve()` run for their full duration so final network flushes can pace themselves.)
3. `drain()` cascades depth-first (deepest children drain first, then their parents).
4. `dissolve()` executes and arenas are reclaimed leaves-first, after which the process exits with code `0`.

```hale
locus Poller {
    run() {
        while !self.draining {
            std::time::sleep(100ms);
        }
    }
    dissolve() { /* close socket */ }
}
```

```hale,fragment
main locus App {
    params { api: Gateway = Gateway { }; }
    placement { api: pinned; }
    run() { while !self.draining { std::time::sleep(100ms); } }
}
```

- **Grace period (`LOTUS_DRAIN_GRACE_MS`)**: The runtime allows 5000 ms (configurable via `LOTUS_DRAIN_GRACE_MS`) for loops reading `self.draining` to finish. If the deadline expires, the runtime logs the stuck pool/worker and loci to stderr and terminates the process. Sending a second `SIGINT` terminates immediately.
- **Immediate exit when idle**: If no live locus checks `self.draining` (or all such `run()` hooks have already returned and hold no active bus subscriptions), `SIGINT`/`SIGTERM` terminates the process immediately without waiting for the grace period.
- **Default `main` wait behavior**: A `main locus` with no `run()` exits as soon as its children complete, except when `main` (or a child server locus beneath it) holds an active `api::Exposure` socket, in which case `main` implicitly waits until the exposure stops or the process drains.

### Self-Termination (`terminate;`)

A locus can end its own lifecycle from inside any of its methods by executing `terminate;`:

```hale,fragment
fn on_close() {
    terminate;   // exits method immediately and triggers drain -> dissolve
}
```

`terminate;` returns from the current method immediately and schedules the locus for `drain()` and `dissolve()`.

## Runtime Ordering Guarantees

The compiler and runtime enforce the following execution invariants across the ownership tree:

1. **`accept` precedes `birth`**: A parent's `accept(c)` runs after `c`'s `params` are initialized and before `c.birth()`.
2. **Subscriptions register before `birth`; delivery starts after `birth`**: Within a single locus literal's static tree (including nested params, `pinned` fields, pool-placed fields, and `replicas = K` shards), all bus subscriptions are reserved before any child's `birth()` runs. Messages published during `birth()` (including self-publishes and messages from earlier sibling params) are queued and delivered in order once the subscriber's `birth()` returns. Elsewhere, dynamically constructing a subscriber *after* publishing misses the earlier message (`hale check` warns when both occur in the same body).
3. **Pinned `birth_check` thread affinity**: On a `pinned` locus, `birth_check` runs on the locus's dedicated OS thread between `birth()` and `run()`.
4. **Pool inheritance**: A locus stored as a parameter field of a pool-placed locus inherits that cooperative pool for its own `run()` and handlers.
5. **Immediate failure routing**: A failing closure, `birth_check`, or `violate` statement delivers a `ClosureViolation` to the parent's `on_failure` immediately. If the parent is still initializing its `params` block, the violation is held until all parent `params` are initialized and delivered just before the parent's `birth()`.
6. **Field teardown order**: A locus's owned `params` fields drain in declaration order (depth-first, each on its own thread/pool) *before* the owner drains, and each field's arena is dissolved *after* the owner's `dissolve()` body completes. Pinned fields of a `main` locus returned from a helper function remain live until the caller finishes with that `main` locus.
7. **Dissolve-epoch closures**: Closures with `epoch dissolve` run immediately before `dissolve()`, routing any violation to the parent before cleanup code executes.
8. **Safe field replacement**: Replacing an owned field (`self.child = Child { }`) runs the old child's `dissolve()` before the new child's `birth()`, while keeping the old child's memory arena alive until any in-flight `run()` or pool handler on the old child returns.
9. **Aborted waits on shutdown**: When `main` begins teardown, any publisher on a pool or pinned thread blocked in `or wait` (or waiting on a full `shm_ring` with `on_overflow: block`) is woken before thread join and fails with `BusWaitAborted`.
10. **No restarts during drain**: Calling `restart(child)` inside `on_failure` while the process is draining (or after `main`'s `run()` has returned) is a no-op; the child terminates cleanly.

> [!NOTE]
> **Current Runtime Edge Cases (`spec/runtime.md`)**
> - A dissolve-epoch closure violation on a flow child or a field torn down directly from `fn main` exits the process rather than invoking `on_failure`.
> - A subscriber inside a `pinned` or pool-placed field that is itself nested inside another placed or replicated field may miss messages published by earlier tree branches before its construction completes.
> - Subscribing loci constructed inside stdlib or imported modules may receive messages during `birth()` rather than queuing them until `birth()` returns.
> - Unbounded self-publishing inside `birth()` buffers messages in memory without a static cap.

Next: [The Bus](./bus.md).
