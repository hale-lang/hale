# `std::time`

Monotonic clock + cooperative sleep. m79 surfaces these under
the canonical `std::time::*` namespace; the legacy bare paths
(`time::sleep`, `time::monotonic`) still work for backward
compatibility but new code should use `std::time::*`.

## Functions

### `std::time::sleep`

#### Synopsis

```aperio
fn sleep(d: Duration)
```

Blocks the calling thread for at least `d`. Implemented via
`clock_nanosleep(CLOCK_MONOTONIC, 0, &req, &rem)` with EINTR
retry — signal interruptions resume from the remaining time
rather than shortening the total sleep.

#### Semantics

- **Monotonic**: NTP / wall-clock adjustments cannot warp the
  sleep duration. Always at-least-as-long-as-requested.
- **EINTR retry**: signals during the sleep don't shorten it;
  the loop reads the remaining time from the kernel's `rem`
  output and continues.
- Statement-position only — no return value.

#### Examples

```aperio
fn main() {
    println("waiting...");
    std::time::sleep(500ms);
    println("done");
}
```

### `std::time::monotonic`

#### Synopsis

```aperio
fn monotonic() -> Duration
```

Returns the current monotonic clock reading as a Duration
(i64 nanoseconds since an unspecified reference). Useful for
elapsed-time measurement; not for wall-clock timestamps.

#### Semantics

- Lowers to `clock_gettime(CLOCK_MONOTONIC, &ts)` followed
  by `ts.tv_sec * 1_000_000_000 + ts.tv_nsec`.
- The reference epoch is unspecified — the value alone is
  meaningless; only differences are. `t1 - t0` produces a
  Duration suitable for comparisons and arithmetic.

#### Examples

```aperio
fn main() {
    let t0 = std::time::monotonic();
    std::time::sleep(20ms);
    let t1 = std::time::monotonic();
    let elapsed = t1 - t0;
    if elapsed > 20ms {
        println("at least 20ms elapsed");
    }
}
```

## Limitations

- **No wall-clock surface yet.** `time::now()` (CLOCK_REALTIME)
  was reserved but isn't shipped in m79; lands when an
  application forces it (logging timestamps, clock-skew
  diagnostics).
- **No `tick(d)` ticker.** The recurring-interval primitive
  the original spec called out waits on a richer scheduling
  arc (probably folded into a Phase 2 test-fakes story or a
  Phase 3 server runtime).
- **No formatting / parsing.** ISO-8601 conversion needs a
  text-processing arc.
- **Legacy `time::sleep` / `time::monotonic` still work.**
  These remain registered as bare-path calls in codegen;
  m79 adds `std::time::*` aliases without removing the
  legacy entries. A future cleanup may deprecate the bare
  forms.

## See Also

- [Roadmap](./roadmap.md) — Phase 1+ stdlib build-out plan.
- [`std::process`](./process/index.md) — the canonical
  pairing for `monotonic` (uptime), `pid`, and `exit`.
