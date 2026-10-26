# Wake-handshake temporal pilot

`WakeHandshake.tla` checks the finite condition-variable protocol used by the
pinned mailbox and the classic cooperative pool. It complements the C11
store-buffer litmus and queue-safety checks in `mailbox_model.c` and
`coop_pool_model.c` with explicit waiting, signalling, spurious wakeups and
fair execution. The checker is the official TLC v1.7.4 jar, pinned by SHA-256
in `check.py`; Python only runs the checker and checks its results.

```sh
curl --fail --location https://github.com/tlaplus/tlaplus/releases/download/v1.7.4/tla2tools.jar -o /tmp/tla2tools.jar
python3 verification/tla/check.py --jar /tmp/tla2tools.jar
```

Java 11 or later is required. `--java /path/to/java` selects an installed
runtime. Logs, including the required negative counterexample traces, go
under `target/verification/tla/`; temporary TLC state and generated modules
are removed. Each model has a 60-second execution limit. Timeout, parse
failure, checker failure and an unexpectedly successful negative control
all fail the runner. No depth limit is used. TLC checks all reachable states
and the temporal properties over that finite state graph.

## Checked properties and controls

- `TypeOK`: the finite domains and queue-capacity bound.
- `Accounted`: each published cell is queued or consumed exactly once.
- `ExitDrained`: the consumer exits only after shutdown and an empty queue.
- `AcceptedEventuallyConsumed`: every published cell eventually drains.
- `ShutdownEventuallyExits`: after ingress quiescence, shutdown eventually
  ends the consumer, including when it was asleep.

The positive configurations check ordinary delivery and quiescent shutdown.
Three negative configurations must produce a **temporal-property violation**:
remove the consumer recheck, suppress the producer signal, or signal without
the mutex. Merely receiving any nonzero process status does not count as a
successful negative control. Deadlock checking is disabled because an idle
consumer asleep with no input is a permitted terminal situation; this pilot
does not certify general deadlock freedom.

## Runtime correspondence and assumptions

The runtime source is `crates/hale-codegen/runtime/lotus_arena.c`:

| Model action | Runtime operation |
|---|---|
| Producer `post` | Successful release-publication in `lotus_mailbox_post`; completed queue publication precedes the classic `lotus_coop_pool_wake_consumer` call |
| Producer `observe` | `seq_cst` fence and read of `parked` |
| Producer `lock`, `signal`, `unlock` | The mutex-protected `pthread_cond_signal` |
| Consumer `try` | The outer dequeue in `lotus_mailbox_drain_one` / `lotus_coop_pool_drain_one` |
| Consumer `announce`, `recheck` | Under the mutex: store `parked`, fence, dequeue again, test shutdown |
| Consumer `wait` | `pthread_cond_wait` atomically registers a waiter and releases the mutex |
| Consumer `reacquire`, `clear`, `unlock` | Wait return with the mutex, clear `parked`, unlock, retry |
| `Stop` | Quiesce producers, set shutdown under the mutex and broadcast |

Weak fairness is assumed for each producer, the consumer and the shutdown
actor: an action that remains enabled eventually runs. Mutex operations and
an admitted handler finish; spurious wakes are permitted but are not assumed
to rescue an otherwise missed wake. The two producers publish one cell each;
the abstract ring has room for both. Successful publication is atomic and
visible to the ordered dequeue recheck. The C11 justification for that
visibility/order remains the GenMC litmus, not this TLA+ model.

Shutdown is tested after the producers finish, reflecting the caller's
ingress-quiescence obligation. This pilot does not prove that lifecycle
lowering discharges that obligation, nor cover a post racing shutdown after
its admission check. It excludes ring-internal reservation stalls, full-ring
backpressure, spills, consumer-local overflow, replay-held cells, failure
delivery, coroutine/eventfd wakeups, arbitrary producer counts and handler
behavior. Their existing runtime and GenMC contracts remain necessary.

`check.py --check-source` fingerprints the reviewed mailbox post/wait/shutdown
regions and classic-pool wake/wait regions. A change to those regions fails
before TLC runs and requires reviewing this mapping, the abstraction and
its exclusions. Updating a fingerprint alone is not correspondence evidence.
The pinned-handler retention change adds carry release before the mailbox's
replay-idle wait and before the empty-queue park/shutdown path. These calls
precede the queue mutex and parked announcement; publication, parked stores,
fences, rechecks, signals and shutdown ordering are unchanged. Handler tickets
and nested mailbox execution remain outside this wake abstraction. The updated
mailbox-wait fingerprint records that review, while native regression tests
check retention and the carry's release boundaries.

The guards prevent silent source drift; the model is a reviewed transcription,
and these finite-model results are not a proof of every runtime execution.
