# Verification & Model-Checking Reference

Hale uses three complementary forms of verification:

1. **Compile-time program checks:** `hale check` checks topology, placement, allocation behavior, effect contracts and architectural `claims` over the compiler's represented program facts.
2. **Independent graph certificates:** Selected reachability claims can export finite-graph certificates and Lean statements for kernel checking.
3. **Runtime protocol models:** GenMC checks C11 concurrency models of `lotus` primitives; a finite TLA+ pilot checks the mailbox/classic-pool wake handshake under explicit bounds and fairness assumptions.

These checks have different scopes. A graph theorem does not prove compiler
extraction or native execution correct, and a protocol model does not cover
every runtime execution. The boundaries are described below.

For the exhaustive specification of every static check, see [`spec/verification.md`](https://github.com/hale-lang/hale/blob/main/spec/verification.md).

---

## 1. Runtime Substrate Model-Checking (GenMC)

Hale's C runtime (`lotus`) uses pthreads and C11 atomics. Its core cross-thread protocols are transcribed into standalone C11 models and checked exhaustively across all legal memory-model interleavings by GenMC in CI. Every model is paired with a negative control (removing a barrier or atomic ordering) to verify that GenMC catches the resulting race or use-after-free.

| Runtime Primitive | Verified Properties |
|---|---|
| **Lock-free hashmap** | Concurrent `enter` / `drain` / `grow` protocol safety |
| **Hashmap iteration** | Table growth occurring between iteration steps (verified under sequential consistency; see limitations below) |
| **Pinned mailbox** | Lock-free MPSC ring buffer and wake-when-parked handshake |
| **Cooperative pool queue** | MPSC ring buffer and worker wakeup protocol |
| **Bus queue** | Cooperative-pool conditional lock and concurrent grow-during-drain |
| **Arena subregion lock** | Parent locus child-slot freelist concurrency |
| **Failure cascade** | Deferred failure delivery during parent birth, child/violation retention until `on_failure` returns, queued run cancellation, and memory retention while an off-pool `run()` is active |

### Scope and Limitations of the GenMC Models

See [`verification/README.md`](https://github.com/hale-lang/hale/blob/main/verification/README.md) for the live audit inventory:

- **Hand-transcribed models:** Models are hand-maintained transcriptions of the protocol atomics in `lotus`, not mechanically extracted from the production C files.
- **Hashmap iterator memory ordering:** The hashmap iterator model passes under sequential consistency (`-sc`) but reports a use-after-free under GenMC's release-acquire model; closing that gap is tracked in the audit log.
- **Safety vs. liveness:** GenMC verifies memory safety and assertions, not liveness (condition variables are stubbed). Liveness, absence of deadlock during teardown, and handler-boundary release queues are covered by the runtime deadline, lifecycle-trace, and ASan test suites.

---

## 2. Default Compile-Time Checks (`hale check`)

These checks run automatically on every `hale check` and `hale build`:

### Data-Race Freedom by Construction

- **Copy-on-publish bus semantics:** Publishing to a `topic` copies the payload into the subscriber's arena (or transfers a frame via a lock-free `shm_ring`). Loci never share mutable pointers across threads.
- **Single-threaded locus methods:** Direct method calls across different thread pools are rejected at compile time.
- **Vertical-only ownership:** Sibling loci cannot hold references to one another; failures propagate strictly upward to the parent's `on_failure` block.

### Bus Topology & Design Rules

| Check | Severity | Condition Flagged |
|---|---|---|
| **Subject type mismatch** | Error | Two declarations or bindings disagree on a topic's payload type |
| **Intra-locus re-entrant publish** | Error | Direct synchronous self-publish cycle with no queued hop |
| **No locus-return** | Error | A locus method returns a managed locus value (violating single-tree ownership / CQRS) |
| **Codec purity** | Error | A bus codec's `encode`/`decode` reaches state mutation, parent admission/tracking, structural failure, scheduler/recovery actions, `syscall`, `block`, or `publish`, or lacks a complete execution/confinement account |
| **`ring_layout` conformance** | Error | A foreign shared-memory ring layout has overlapping fields or invalid offsets |
| **Orphan topic** | Warning | A topic has publishers but no subscribers (or vice versa) in a closed binary |
| **Bus queue cycle** | Warning | A publish-subscribe cycle across loci that can spin indefinitely |
| **Unthrottled publish loop** | Warning | Publishing inside an unbounded loop without a sleep/yield/blocking gate |

Codec purity follows implicit work as well as explicit calls. Constructing a
child can evaluate omitted defaults, enter lifecycle bodies, or invoke its
parent's `accept`/`release` callbacks. Even an empty `accept` can update the
parent's child tracking. Keep that work outside codecs, whose methods may run
concurrently on several threads. Local data transformations and classified
allocation remain allowed; an unresolved callback or receiver-confinement
question cannot establish that a codec is safe.

Local record fields use their declared value semantics even when the record
comes from the stdlib and its executable bodies are outside the analysis
horizon. This does not establish confinement for a locus held inside a record.
An unresolved type parameter also cannot inherit a record's purity merely
by sharing its name. Unknown callbacks and default execution still need their
own coverage.

Method calls on an array element follow the element's known type, including
uniform array literals. The index expression is checked too: indexing with an
OS-only call still requires that capability on the selected target. An unknown
element type cannot supply a method or effect guarantee.

### Concurrency & Thread Placement Checks

| Check | Severity | Condition Flagged |
|---|---|---|
| **Dead bus receiver** | Error | A non-`main` cooperative locus subscribes to a topic *and* makes a blocking call in `run()`, starving its own bus handlers |
| **Nested long-running child** | Error | A non-`main` locus with its own work holds a `params` child whose `run()` is long-running (delaying parent startup). Fix: hoist the child to `main` with its own placement |
| **Unowned subscriber locus** | Error | A bus-subscribing locus is instantiated inside a handler without being `accept`ed by an ancestor on all paths, causing it to dissolve when the handler returns |
| **Blocking on cooperative pool** | Warning | Calling blocking I/O (`recv`, `accept`, `process::run`) on a cooperative pool without `where async_io` |
| **Cooperative pool starvation** | Warning | Two or more loci on the same cooperative pool have non-terminating `run()` loops (`while true`, `while !self.draining`), preventing later loci in birth order from starting `run()` |
| **Dropped scope-local delivery** | Warning | A bus subscriber is constructed and dissolved within a single function call on a non-`async_io` cooperative pool, so its handlers never get the thread to run |

---

## 3. Arena Memory-Bound Proofs & Budgets

### Default Escape & Loop Survey

On any program with a long-running `run()` loop or bus handler, `hale check` analyzes every loop and handler across the call graph (including imported seeds and reachable `std::*` methods) to detect allocations that escape into a long-lived arena and accumulate across iterations:

- **Automatically proven safe (no warning):**
  - Bounded loops (`while i < N` where the counter is ranked).
  - Allocations inside a locus method's **per-call scratch region** that do not escape to `self` or the return value.
  - Non-recursive **scratch-local free functions** (functions taking/returning only primitives/`String` that construct no structs and call no locus methods), whose temporaries are freed on return.
  - String/accumulator loops (`out = out + piece(i)`) inside a method or scratch-local helper where intermediate values are reclaimed on return.
  - **Whole-field replacement** (`self.last = "seen " + id`), where the runtime retires the previous field value and reuses its arena block (whereas `self.log = self.log + line` appends and is flagged).
- **Acknowledging intentional growth:**
  - `@bounded_by(record)` (preferred): Documents the external domain bound that caps growth and silences the warning.
  - `@unbounded`: Acknowledges unbounded growth that is not yet bounded by a known input.
- **Inspecting memory analysis:**
  - `--dump-alloc-summary`: Prints per-function allocation behavior, scratch-local status, and `@bounded_by` tags.
  - `--report-unbounded-use`: Lists every `@bounded_by` and `@unbounded` annotation and which warnings it silences.
  - `--no-warn-unbounded-alloc`: Disables the advisory for a run.

### Opt-In Hard Ceilings (`@budget` & `--check-resource-budget`)

- **`@budget(alloc_per_call = N)`:** Hard compile error if a function or its transitive callees perform more than `N` arena allocations per invocation (`N = 0` certifies a zero-allocation hot path).
- **`--check-resource-budget budget.toml`:** Static ceiling gate on OS threads, cooperative pools, file descriptors, and bus subjects spawned by `main`.

### Declared Queue Reservations

The claim `bound queue_slots <= N on instances G` sums the subscription
capacities of arranged instances selected by `G`. Repeated instances count
separately; unknown populations prevent certification. This budgets declared
queue slots, not payload bytes, current occupancy, work in flight or total
retained memory. See [Queue Reservation Budgets](./claims.md#10-queue-reservation-budgets).

---

## 4. Transitive Effect Contracts & Architectural Claims

### Function & Phase Effect Contracts

Functions can forbid or restrict side effects with `@effects(...)` or shorthands (`@no_syscall`, `@no_block`, `@deterministic`, `@no_ffi`, `@no_publish`, `@no_spawn`, `@no_recursion`), and loci can restrict lifecycle phases with `@phase_effects(birth: {alloc}, run: {})`.

- Checked transitively across direct calls, method calls, imported libraries, and all concrete loci satisfying an `interface` slot.
- **Fails closed:** Any unclassified or unknown external call violates the contract rather than being assumed pure.

See **[Effect Contracts & Resource Budgets](./effects.md)** for full details.

### Whole-Program `claims`

A `main locus` or `constitution` can assert compile-time laws over `group` declarations:

```hale,fragment
claims {
    isolated:   forbid reaches(delta_wing, gamma_wing);
    gated_io:   forbid reaches(app, effects(syscall)) avoiding safe_io;
    attributed: require attributed(all syscall);
    one_llm:    bound llm <= 1 on paths from planners;
    wired:      require subscribes(some delta_wing, topic t::Tasks);
    single_pub: count publishers(topic t::Tasks) == 1;
}
```

When a claim fails, `hale check` prints the exact **witness path** in your source identifiers. Topology models can be exported and diffed in CI using `hale check --dump-topology` and `--check-topology <baseline>`. See **[Architectural Claims & Topology Proofs](./claims.md)**.

The policy `require flow(all G)` checks that every selected concrete locus
type has the shared flow reclamation classification. It does not establish
that a run returns or that the number of live children is bounded. See
[Flow Reclamation Policy](./claims.md#require-flow-reclamation-for-selected-child-types).

---

## 5. Secret Confinement (`@sealed` & `std::secret`)

Hale enforces secret handling through **state confinement** (`@sealed`), **effect classification** (`secret_use`), and **single-statement reveal rules**, rather than heuristic taint analysis.

### 1. Confining State with `@sealed`

Annotating a locus with `@sealed` makes its `params` fields private to its own methods (including across generic specializations and parameter defaults). Combine `@sealed` with `require sealed(all vaults);` in `claims` to guarantee every secret-holding locus is sealed.

Run `hale check --sealable` on any existing project to see which loci can be marked `@sealed` immediately without breaking callers.

### 2. Built-In Secret Containers (`std::secret::Signer` & `Credential`)

Instead of passing raw key bytes through `main`, pass the *source name* to `std::secret::Signer` or `std::secret::Credential` so the secret material enters the program already inside a `@sealed` locus:

```hale,fragment
locus ApiClient {
    params {
        // Loaded at birth from $SIGNING_KEY (optionally decoded with `decode: "base64"`):
        signer: std::secret::Signer = std::secret::Signer { env_var: "SIGNING_KEY" };
        // Resolved fresh on each privileged call from Vault or local vault dir:
        token:  std::secret::Credential = std::secret::Credential { vault: "api-token" };
    }

    fn call(url: std::http::Url, body: Bytes) -> std::http::ClientResponse fallible(std::http::HttpError) {
        return std::http::request(std::http::ClientRequest {
            method: "POST",
            url: url,
            headers: "Authorization: Bearer " + self.token.reveal_text(),
            body: body
        }) or raise;
    }
}
```

| Type / Method | Behavior |
|---|---|
| `std::secret::Signer { env_var: "...", decode: "base64" }` | Reads key at `birth`. Exposes `sign(bytes)` (`@effects(is: {secret_use})`). Raw key never leaves the locus. |
| `std::secret::Credential { env_var: "..." \| file: "..." \| vault: "..." }` | Holds a token or password. `vault: "<name>"` resolves on each call from `HALE_VAULT_ADDR` (`GET <addr>/v1/secret/<name>` with `HALE_VAULT_TOKEN`) or local directory `HALE_VAULT_DIR` / `~/.cache/hale/vault`. |
| `cred.fingerprint()` / `cred.sign_ed25519(m)` / `cred.ed25519_public()` | Safe operations that never expose the underlying secret bytes. |
| `cred. matches(candidate)` | Constant-time full-value comparison (`==` and slice comparisons are rejected). |
| `cred.write_private(path)` | Writes the secret to a `0600` file in a private directory without revealing it to caller code. |
| `cred.reveal()` / `cred.reveal_text()` | **Single-statement wire rule:** Must be called inside a locus method and consumed in the *same statement* by an outbound wire payload (HTTP request `body` or `headers`, never `url`), `matches`, or a `@secret` parameter. May pass only through pure transforms (`+`, `std::url::encode`, `std::json::escape_string`, hashes). Cannot be bound to `let`, matched, stored, logged, or returned. |

---

## 6. Runtime State Invariants (`closure`)

Alongside compile-time checks, any locus can declare runtime `closure` assertions checked automatically across lifecycle transitions or triggered inline via `violate`:

```hale,fragment
closure balanced {
    self.debits ~~ self.credits within 0.01d;
}
```

When a closure invariant fails, execution immediately transitions the locus into `drain` and routes the violation upward to the parent locus's `on_failure` policy (`restart`, `restart_in_place`, or `bubble`). See **[Supervision & Structural Failure](./services/failure.md)**.

---

## 7. Soundness and Analysis Boundaries

1. **Sound over-approximation (fails closed):** Whenever the static analyzer encounters an unresolvable call or unclassified symbol, it refuses certification (`uncertified` or build error) rather than assuming safety.
2. **Closed-world horizon:** Static guarantees hold up to explicit external boundaries (`@ffi("c")`, network/IPC sockets, OS filesystems).
3. **Source-level cost models:** `@budget(alloc_per_call = N)` and resource budgets count source-level arena operations and placement declarations prior to LLVM inlining.

Runtime counter measurements complement these static checks on particular
executions. `std::diag::heap_alloc_count` and `syscall_count` require the
Linux native counter backend without sanitizers. A reading of `-1` means
the counter is unavailable, including on macOS; subtracting two unavailable
readings cannot establish that an operation had zero effects.

## 8. Independent Proof Checks

### Reachability Certificates and Lean

For selected `forbid reaches` laws, export the canonical report and generated
Lean document, then replay both against the current source:

```sh
hale check app --dump-reachability-proofs=Claims.json --dump-reachability-lean=Claims.lean
hale check app --check-reachability-proofs=Claims.json --check-reachability-lean=Claims.lean
```

Export and replay do not run Lean. To check the actual generated proposition
with the pinned toolchain, run from a Hale compiler checkout:

```sh
cd verification/lean
lake build
lake env lean /absolute/path/to/Claims.lean
```

A path certificate proves a forbidden path in the supplied graph; it is not
a passing claim. Invalid or uncertified laws have no theorem, and other claim
families are excluded. Source extraction, selection, completeness and
correspondence with native behavior remain trusted. See
[Reachability Certificates and Lean](./claims.md#reachability-certificates-and-lean)
and the [compiler-to-kernel guide](https://github.com/hale-lang/hale/blob/main/verification/lean/README.md)
for replay rules, negative controls and the independent checker.

### TLA+ Wake-Handshake Pilot

The [TLA+ guide](https://github.com/hale-lang/hale/blob/main/verification/tla/README.md)
gives the pinned TLC command, runtime mapping and required negative controls.
The finite model has two producers and checks eventual delivery and shutdown
after ingress quiescence, with weak fairness and explicit visibility
assumptions. It excludes full-ring backpressure, posts racing shutdown,
arbitrary handler behavior and several other runtime paths. Runtime source
fingerprints detect drift; they do not prove refinement. This pilot adds no
general termination, latency, lifecycle-safety or durability claim to Hale.

### Lifecycle source effects

The compiler includes a constructed locus's nonempty `birth`, `run`, `drain`
and `dissolve` bodies in its source call and effect account. If every body and
entry is known, pure lifecycle work can satisfy an effect prohibition. A
forbidden effect inside a hook still fails it; an unresolved callback still
prevents certification.

This source account does not establish when a hook runs or how long its
storage stays live. Resource budgets retain the missing runtime context.
Automatic assertions, failure dispatch and parent protocol work also retain
their separate coverage requirements.
