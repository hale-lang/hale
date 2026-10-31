# Effects & Contracts

In Hale, functions and loci can declare **effect contracts** specifying which side effects, resource allocations, or bus publications they are permitted to perform. The compiler verifies these contracts transitively across the entire closed call and bus graph—through helper functions, locus handle calls, imported library seeds, and the standard library—failing the build with a full call-chain witness if any path violates the contract.

```hale,fragment
@no_syscall @deterministic
fn price(book: OrderBook, qty: Int) -> Decimal { ... }
```

Effect contracts are opt-in and have zero runtime cost. The only implicit check applies to loci placed on an `async_io` coroutine pool, where the compiler warns if a handler reaches a thread-blocking operation.

---

## Reference Tables

### Built-in Effect Classes

Every built-in effect class name is reserved by the compiler (redeclaring `effect syscall;` or `effect secret_use;` is a compile error). Run `hale doc --stdlib` to inspect the exact effect classes of every standard library function.

| Effect Class | What It Covers | Representative Operations |
|---|---|---|
| `syscall` | Kernel boundary crossings: filesystem, sockets, child processes, terminal, and stdio (including `println`). | `std::io::fs::read_file`, `println` |
| `stdio` | Writing to standard output or standard error. `stdio` is a sub-class of `syscall` (every `stdio` operation also carries `syscall`). | `println`, `std::io::stdout::write_bytes` |
| `block` | Operations that wait or block execution (see [`async_io` pool parking rules](#implicit-async_io-placement-checks) for coroutine exceptions). | `std::http::get`, `std::time::sleep` |
| `time` | Reading a wall or monotonic clock. Pure transformations on passed-in `Time` values do not carry `time`. | `std::time::monotonic_ns`, `std::time::current` |
| `entropy` | Reading random numbers or entropy sources. | `std::rand::next_int` |
| `env` | Reading environment variables or command-line arguments (`argv`). | `std::env::var` |
| `ffi` | Calling an external C function declared with `@ffi("c")`. | Any `@ffi("c")` function |
| `publish` | Publishing a message to a bus topic or subject. | `Orders <- o` |
| `spawn` | Instantiating a locus. | `Worker { }` |
| `recursion` | A direct or mutual cycle in the call graph. | Any recursive function chain |
| `alloc` | Dynamic allocation in the locus arena (strings, non-elided records, collection growth). | `"a" + "b"`, `Buf { n: 1 }` |
| `secret_use` | Compiler-owned class on privileged operations over confined secret material ([Verification](./verification.md#5-secret-confinement-sealed--stdsecret)). | `std::secret::Signer.sign` |
| `client(S)` | Calling an external API surface `S` bound via `api::client` (`client(*)` matches all consumed surfaces). | `self.shelf.get_item(req)` |

### Effect & Contract Decorators

| Decorator | Target | Meaning |
|---|---|---|
| `@effects(none: {c1, c2})` | `fn` | Forbids reaching any listed effect class directly or transitively. |
| `@effects(only: {c1, c2})` | `fn` | Restricts transitive effects to a subset of the listed classes; forbids all other built-in and user-declared classes. |
| `@effects(is: {u1, u2})` | `fn` | Classifies the function as a direct source of user-declared effect classes `u1, u2`. |
| `@effects(publish: {T1, T2})` | `fn` | Restricts bus publications reachable from the function to the exact listed topics. |
| `@effects(causes: {c1, c2})` | `fn` | Declares the complete set of effects transitively triggered *across the bus* by messages published from this function. |
| `@effects(depends: {T1, T2})` | `locus` | Declares the complete set of bus topics or wire subjects that can transitively reach any handler on the locus. |
| `@no_syscall` | `fn` | Shorthand for `@effects(none: {syscall})`. |
| `@no_block` | `fn` | Shorthand for `@effects(none: {block})`. |
| `@no_ffi` | `fn` | Shorthand for `@effects(none: {ffi})`. |
| `@no_publish` | `fn` | Shorthand for `@effects(none: {publish})`. |
| `@no_spawn` | `fn` | Shorthand for `@effects(none: {spawn})`. |
| `@no_recursion` | `fn` | Shorthand for `@effects(none: {recursion})`. |
| `@deterministic` | `fn` | Shorthand for `@effects(none: {time, entropy, env})`. |
| `@no_panic` | `fn` | Forbids unhandled failures (`violate`, `or raise`, or trapping array indices). |
| `@budget(dim = N, ...)` | `fn` | Bounds per-call resource dimensions (`alloc_per_call`, `stack_bytes`, `block_points`, `publish`, `fanout`, or `<user_class>`). |
| `@phase_effects(phase: {...})` | `locus` | Restricts each named lifecycle hook (`birth`, `run`, etc.) or method to the exact listed effect classes. |
| `@hot` | `fn` | Enforces zero arena allocations and hot-path speed rules on the function. |
| `@unbounded` / `@bounded_by(x)` | `fn`, hook, `mode` | Acknowledges intentional arena accumulation and silences the unbounded-allocation advisory. |

---

## Function Effect Contracts (`@effects`)

### Forbidding Effects (`none:` and Shorthands)

Use `@effects(none: {...})` or its shorthand decorators to forbid specific effect classes across everything reachable from a function:

```hale,fragment
@effects(none: {syscall, block}) fn decode(b: Bytes) -> Msg { ... }
@effects(none: {time})           fn backoff(n: Int) -> Int { ... }
@effects(publish: {OrderFill})   fn route(o: Order) { ... }
```

Decorators can be stacked in any order to form a composite hot-path contract:

```hale,fragment
@no_block @no_syscall @deterministic @no_recursion @hot
@budget(alloc_per_call = 0)
fn on_tick(a: Int, b: Int) -> Int { ... }
```

When an assertion fails, `hale check` prints the complete call chain from the annotated function to the offending leaf operation, along with the source location in your code or the standard library:

```text
type error: effect assertion violated: `quiet` must not reach `syscall`,
but reaches quiet -> Reader::slurp [std::io::fs::file_size — a
syscall-class operation (filesystem, socket, process, terminal, stdio)].
Move the effect behind a locus this fn doesn't reach (the
reader/writer-locus shape), or pass the value in as a parameter.
    note: the `alloc` effect happens here (in the standard library, io_tcp.hl:98:18)
```

Key verification rules:

- **`println` is a `syscall` (and `stdio`)**: Writing to stdout/stderr invokes `write(2)` and can block. Debug `println` calls inside a `@no_syscall` function are rejected as contract violations. To forbid terminal output while still permitting file or socket I/O, use `@effects(none: {stdio})`.
- **Handle calls are followed**: Calling `reader.slurp()` resolves `reader`'s locus type and walks into `Reader::slurp`. Moving an effect into a child locus that the annotated function still calls directly does not hide the effect; instead, decouple via bus publication or pass precomputed values as arguments.
- **Pure operations vs. effectful sources**: `@deterministic` (`none: {time, entropy, env}`) forbids *reading* nondeterministic sources, not computing over passed-in values:

| Pure (Allowed in `@deterministic`) | Effectful (Rejected in `@deterministic`) |
|---|---|
| `std::time::time_from_unix(n)` | `std::time::monotonic_ns()` (`time`) |
| `std::time::iso8601(t)`, `parse_time(s)` | `std::time::current()` (`time`) |
| `std::str::parse_int(s)` | `std::env::var(k)` (`env`) |
| `std::http::parse_request(b)` | `std::http::get(u)` (`syscall`, `block`) |

- **Fail-closed on unknowns**: Any unclassified or unresolved call is treated as potentially performing all effects and fails the assertion.

### Closed-World Allowlists (`only:`)

While `none:` forbids specific classes and permits everything else, `@effects(only: {...})` requires the function's transitive effects to be a **subset** of the listed classes:

```hale
@effects(only: { alloc })
fn label(n: Int) -> String {
    return "a" + "b";
}
```

The compiler computes the forbidden complement against the entire closed universe of built-in and user-declared atomic effect classes at check time. If a new user effect class is added to the program later, existing `only:` contracts automatically forbid it without requiring annotation updates:

```hale,refused
effect money;

@effects(is: { money })
fn charge(cents: Int) -> Int { return cents; }

@effects(only: { alloc })
fn quote(n: Int) -> Int { return charge(n); }   // error: reaches `money`

fn main() { println(quote(1)); }
```

### Decorator Stacking and Contradiction Rules

Function decorators are order-independent, including memory advisories like `@unbounded` and `@bounded_by(<name>)`:

```hale,fragment
@unbounded @no_syscall fn absorb(log: Lines, line: String) { ... }
@no_syscall @unbounded fn absorb(log: Lines, line: String) { ... }
```

The compiler rejects contradictory or duplicate decorator combinations at the declaration site:

- **Duplicate decorators**: Writing the same decorator twice on one declaration is an error.
- **`@unbounded` vs. `@bounded_by(<name>)`**: Mutually exclusive; use `@bounded_by(<param>)` when growth is bounded by a named input, or `@unbounded` when growth has no static bound.
- **Zero-allocation contradictions**: Neither `@unbounded` nor `@bounded_by` can be paired with `@hot`, `@effects(none: {alloc})`, or `@budget(alloc_per_call = 0)`. However, pairing `@unbounded` with a non-zero per-call ceiling (such as `@budget(alloc_per_call = 2)`) is valid.
- **Lifecycle hooks and modes**: Lifecycle hooks (`birth`, `run`, `dissolve`, etc.) and `mode` blocks (`@unbounded mode bulk() { ... }`) execute once per phase rather than per call, so they accept only `@unbounded` or `@bounded_by(<name>)`. Place per-call `@effects` and `@budget` contracts on helper `fn` members invoked by the hook.

### Forbidding Unhandled Failures (`@no_panic`)

`@no_panic` verifies that no code path in the function can trap or propagate an unhandled failure:

```hale,fragment
@no_panic fn parse_frame(b: Bytes) -> Frame {
    return decode(b) or Frame { };   // handled fallback — valid
}
```

- **Rejected inside `@no_panic`**: Explicit `violate` statements, `or raise` error propagation, and unguarded trapping array/slice indexing.
- **Allowed inside `@no_panic`**: Fallible calls handled via `or discard`, `or <fallback_value>`, or `or handler(err)`.

---

## Quantitative Budgets (`@budget`)

`@budget` enforces compile-time upper bounds on per-call resource consumption across a function's transitive call tree:

```hale,fragment
@budget(alloc_per_call = 0, stack_bytes = 4096, block_points = 0)
fn on_frame(x: Int) -> Int { ... }

@budget(publish = 1) fn reply(o: Order) { ... }   // at most 1 bus publish per call
@budget(fanout = 8)  fn notify(e: Ev) { ... }     // at most 8 transitive bus deliveries
```

| Budget Dimension | What It Counts | Unbounded Conditions (Always Violates) |
|---|---|---|
| `alloc_per_call = N` | Arena allocations along the heaviest execution path per call. | Allocation inside a loop or recursion. |
| `stack_bytes = N` | Deepest call-chain stack frame total (sibling calls take `max`, not sum). | Any recursion in the reachable call graph (pair with `@no_recursion`). |
| `block_points = N` | Blocking operations along the call tree. | Blocking call inside a loop or recursion. |
| `publish = N` | Direct and transitive bus publish statements per call. | Publish inside a loop or recursion. |
| `fanout = N` | Transitive runtime bus subscriber *deliveries* triggered by a call. | Computed publish subject, subscriber born dynamically outside `main`, or topic bound to an external transport. |
| `<user_class> = N` | Calls to functions carrying `@effects(is: {<user_class>})`. | Carrier inside a loop, recursion, or unresolved indirect call. |

### How `fanout` Is Calculated

Unlike `publish` (which counts `Topic <- msg` statements), `fanout` measures downstream delivery amplification:

1. **Runtime registrations, not source lines**: A single `subscribe` declaration on a locus placed with `replicas = 3` counts as 3 deliveries.
2. **Transitive across republishers**: If a subscriber handler republishes to another topic with 3 subscribers, a publish to the first topic has a `fanout` of `1 + 3 = 4`.
3. **Key-aware routing**: A publish to a `keyed_by` topic only counts subscriptions whose `where key == ...` filter matches the message's key. For example, publishing one keyed message to `where key == replica` across `replicas = 4` costs `1` delivery, not `4`.

---

## Lifecycle Phase Contracts (`@phase_effects`)

Attach `@phase_effects` to a **locus** to restrict which effect classes each lifecycle hook (`birth`, `accept`, `release`, `run`, `drain`, `dissolve`) or named handler/method may perform:

```hale,fragment
@phase_effects(birth: {alloc}, run: {})
locus Engine { ... }

@phase_effects(on_order: {publish, alloc})
locus Router { ... }
```

| Phase Specification | Meaning |
|---|---|
| `run: {}` | `run` may perform **no effects at all** (neither built-in nor user-declared). |
| `run: {alloc}` | `run` may only perform `alloc`; all other built-in and user-declared atomic effects are forbidden. |
| *(phase omitted)* | The omitted phase is **unconstrained**. |

> [!IMPORTANT]
> - **Omission vs. Empty Braces**: `@phase_effects(birth: {alloc})` constrains *only* `birth` and leaves `run` unconstrained. To enforce "no dynamic allocation or I/O after startup," explicitly write `@phase_effects(birth: {alloc}, run: {})`.
> - **Publishing Requires `alloc`**: Constructing a bus message payload allocates in the arena, so a handler that constructs and publishes a message requires `{publish, alloc}`, not `{publish}` alone.
> - **Typo Checking**: The six lifecycle hook names are always valid on any locus; any other phase name must match a declared method or handler on that locus, or compilation fails.

---

## Implicit `async_io` Placement Checks

Loci placed on a shared `cooperative(pool = <name>) where async_io` pool share a single OS worker thread across coroutines. Without requiring any decorator, the compiler warns if a handler on an `async_io` pool reaches a call that blocks the underlying OS thread:

```text
`Worker::on_e` is placed on the async_io pool `web`, whose single worker
it shares — but it reaches Worker::on_e -> slurp [std::io::stdin::read_line].
A blocking call here stalls every other locus on `web` until it returns.
```

Adding `@no_block` to the handler upgrades this check from a warning to a build error. The placement check inspects the pools spawned by the active `main locus` (placement blocks in imported libraries or unspawned modules are inert).

### Coroutine-Parking Calls vs. Thread-Blocking Calls

On an `async_io` pool, operations that suspend the current coroutine and yield the worker thread to other coroutines do **not** stall the pool and do **not** trigger the placement warning:

| Coroutine-Parking (No `async_io` Warning) | Thread-Blocking (Triggers `async_io` Warning) |
|---|---|
| `std::time::sleep` | `std::io::tcp::connect`, `std::io::tls::{connect, upgrade}` |
| `std::io::tcp::{accept_one, recv_into, recv_stamped_into}`, `__recv`, `__recv_bytes` | `std::http::*` client calls |
| `std::io::udp::{recv, recv_with_source, recv_into}` | `std::io::fs::*` file I/O, `std::io::stdin::*` |
| `std::io::tls::{recv_into, recv_stamped_into}` | `std::process::{run, wait, read_stdout, read_stderr}` |

Note that `std::time::sleep` still carries the `block` effect class in the call graph because it blocks an OS thread when called on a classic thread pool. Consequently, an explicit `@no_block` annotation on a function will still reject `std::time::sleep`.

---

## Bus-Graph Effect Contracts (`causes:` and `depends:`)

Because Hale's bus topology is closed and statically declared, effect contracts can traverse message publications and subscriptions across the bus.

### Downstream Causal Contracts (`@effects(causes: {...})`)

While `none:` and `only:` inspect the direct call graph of a function, `@effects(causes: {...})` declares the **complete set** of effects triggered transitively by any bus messages published from the function:

```hale,fragment
@effects(causes: {publish, syscall})
fn handle(o: Order) { Orders <- o; }
```

If a subscriber to `Orders` (or a further downstream republisher) performs an effect not listed in `causes:`, compilation fails with the bus path:

```text
type error: declared causal set violated: `Api::handle` can transitively
cause syscall through the bus, which its `@effects(causes: …)` does not
declare. Path: `Api::handle` -> subject `Orders` -> `Audit::on_order`.
Add the class to the declaration, or route the publish to a subject
whose subscribers don't perform it.
```

### Upstream Dependency Contracts (`@effects(depends: {...})`)

Placed on a **locus**, `@effects(depends: {...})` declares the complete set of bus topics (or wire subject strings) that can transitively reach any of the locus's subscription handlers:

```hale,fragment
@effects(depends: {SumLookup, Recalled})
locus StatedCarry {
    bus { subscribe SumLookup as on_sum; }
}
```

If an upstream locus subscribes to `Recalled` and republishes onto `SumLookup`, `StatedCarry` transitively depends on `Recalled`. Omitting `Recalled` from `depends:` fails the build with the upstream laundering path:

```text
type error: declared dependency set violated: `StatedCarry` can
transitively depend on `Recalled` through the bus, which its
`@effects(depends: …)` does not declare. Path: subject `Recalled` ->
`Launderer` -> subject `SumLookup` -> `StatedCarry`.
```

You can list either topic identifiers (`Recalled`) or wire subject literals (`"recalled"`); the compiler resolves both to the same underlying bus identity.

---

## User-Declared, Composed, and Domain-Indexed Effects

### Declaring Custom Effect Classes (`effect` and `is:`)

Declare custom effect classes with `effect <name>;` and mark source functions with `@effects(is: {<name>})`:

```hale
effect money;

@effects(is: {money})
fn charge(cents: Int) -> Bool { return cents > 0; }

@effects(none: {money})
fn quote(cents: Int) -> Int { return cents * 2; }

fn main() { println(quote(100)); }
```

Custom effect classes propagate through the call graph, across seed imports, and over the bus (`causes:`), and work inside `@effects(none:/only:)`, `@phase_effects`, `@budget(<class> = N)`, and architectural `claims` (`forbid reaches(G, effects(money))`, `bound money <= N`). Up to 54 user-defined atomic effect classes can exist in a compiled program (exceeding the 64-bit effect mask capacity is rejected at the `effect` declaration).

### Composed Effect Classes (`effect name = {...}`)

Define an effect class as the union of existing built-in or user-declared classes:

```hale,fragment
effect io = { syscall, block };
effect wallclock = { time };
```

A composed class owns no separate bit in the effect mask; its mask is the bitwise union of its members (`syscall | block`). This has two consequences:

1. Forbidding `io` forbids both `syscall` and `block`, and any function performing `syscall` or `block` carries `io`.
2. Defining `effect wallclock = { time };` includes the built-in `time` bit in `wallclock`, meaning `@deterministic` (`none: {time, entropy, env}`) automatically rejects functions tagged `@effects(is: {wallclock})`.

Cyclic definitions (`effect a = { b }; effect b = { a };`) are rejected at compile time.

### Domain-Indexed Effect Families (`domain` and `effect name(domain)`)

To track effects parameterized over a finite set of partitions or tenants, declare a `domain` followed by an indexed `effect` family in the same file:

```hale,fragment
domain wing = { delta, gamma };

effect knowledge(wing);

@effects(is: {knowledge(delta)})
fn read_delta(key: String) -> Idea { ... }
```

This expands eagerly into atomic classes `knowledge(delta)` and `knowledge(gamma)`, plus the wildcard union class `knowledge(*)`. Misspelled indices (`knowledge(delt)`) are rejected with a did-you-mean diagnostic. See [Claims & the Law](./claims.md#domain-and-effect-families--the-data-plane-vocabulary) for using domain-indexed effects in isolation claims.

---

## Tracking Effect Drift in CI (`.hale.effects`)

Even on unannotated code, the compiler infers the transitive effect set (`does={...}`) of every free function, locus method, and lifecycle hook across the program and its imported seeds.

Export the inferred effects manifest and commit it to version control:

```sh
hale check app.hl --dump-effects-manifest > .hale.effects
```

```text
# .hale.effects v1 — declared effect contracts
App::run  does={syscall,block,time}
Pusher::run  does={syscall,block,publish,time,alloc}
```

In CI, gate against unintended effect drift with `--check-effects-manifest`:

```sh
hale check app.hl --check-effects-manifest .hale.effects
```

```text
effect manifest changed — .hale.effects no longer matches the program's effects.
  - Api::emit  none={block}  does={publish,alloc}
  + Api::emit  none={block}  does={syscall,publish,alloc}
```

If a helper function several calls deep introduces a `syscall`, `--check-effects-manifest` flags the exact callers whose transitive `does={...}` set widened, even if those callers had no `@effects` annotation of their own.
