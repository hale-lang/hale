# Deterministic Record & Replay

Hale's flight recorder (`LOTUS_OBS_RECORD`) captures a program's message delivery schedule, wire ingress payloads, and non-deterministic standard library reads (`std::time`, `std::rand`, `std::env`). The `hale replay` command re-executes the program hermetically against that recording to reproduce bugs deterministically, verify behavior with `--diff`, or backtest modified code with `--feed`.

```sh
# 1. Record an execution to disk
LOTUS_OBS_RECORD=run.halerec hale run app.hl

# 2. Re-execute and verify that all schedules, events, and journals match
hale replay run.halerec app.hl --diff
```

## Recording a Run (`LOTUS_OBS_RECORD`)

Set `LOTUS_OBS_RECORD=<path>` to enable lossless flight recording (see `spec/runtime.md` § *Lossless recording mode*):

| Environment Variable | Default | Behavior |
|---|---|---|
| `LOTUS_OBS_RECORD=<path>` | Unset | Writes a lossless `.halerec` recording to `<path>` (implies `LOTUS_OBS=1`; requires no attached observer). |
| `LOTUS_OBS_RECORD_DURABLE=1` | Unset | Syncs each drain sweep and finalize trailer to disk (`fdatasync` on Linux, `F_FULLFSYNC` on macOS) and syncs the parent directory on file creation. |
| `LOTUS_OBS_RECORD_ENV=full` | Withheld | Captures full `std::env` variable values in the journal. By default, only variable names, existence, and byte lengths are recorded to protect secrets. |

Unlike live observation (`LOTUS_OBS=1`), which drops telemetry records rather than slowing down the application, `LOTUS_OBS_RECORD` prioritizes completeness:
- **Lossless backpressure**: If a thread's observation ring fills, the producer blocks until the in-process drain flushes the ring to disk.
- **Fail-loud integrity**: Any ring allocation, capture, write, or finalization error aborts the run immediately rather than writing a silently incomplete file.
- **Crash-prefix durability**: Frames stream to disk sequentially after an eagerly stamped header. If the process crashes before writing the finalize trailer, pass `--allow-truncated` to `hale replay` to replay the intact prefix up to the torn tail frame.

> [!WARNING]
> Because lossless recording blocks producers when observation rings fill, it alters application timing by design. Do not leave `LOTUS_OBS_RECORD` enabled in latency-critical production steady state.

### What a Recording Captures

Every `.halerec` file records four streams:
1. **Per-consumer delivery schedule**: Every queued handler invocation records `(target locus, message id)` on the consuming thread, capturing the exact interleaving across concurrent publishers.
2. **Public bus events**: All publish and delivery events—including synchronous devirtualized direct-dispatch calls—with file-side identity maps for cross-run alignment.
3. **Payloads and wire ingress tape**: Wire-encoded payloads (including all frames accepted by `unix://` and `udp://` listen bindings) are captured verbatim as an injectable ingress tape. In-process flat struct publishes are recorded as metadata only (declared byte size and publish identity).
4. **Non-deterministic input journal**: Every call to `std::time::now`, `std::time::monotonic`, `std::time::monotonic_ns`, `std::rand::next_int`, `std::os::getrandom`, and `std::env` is journaled per consumer with its encoded arguments and return value (unless `std::env` values are withheld by default).

## Replaying a Run (`hale replay`)

`hale replay` recompiles the source program using the same pipeline as `hale run` and executes it against the recording:

| Command / Flag | Description |
|---|---|
| `hale replay run.halerec app.hl` | Re-executes `app.hl` (or a directory seed `app/`) using the recorded schedule, ingress tape, and input journal. |
| `--diff` | Compares consume schedules, public bus events, payloads, and journals bidirectionally; exits non-zero on any divergence. |
| `--diff --json` | Emits the `--diff` verdict and per-category coverage counts as a single JSON object. |
| `--at <N>` / `--at <consumer>:<N>` | Sends `SIGSTOP` to the process at global consume `#N` or at `<consumer>`'s `N`th consume so you can attach `gdb` or `lldb` at the exact step. |
| `--allow-truncated` | Admits a crash-truncated recording (missing its finalize trailer) and replays the valid prefix. Under `--diff`, post-crash execution counts as surplus rather than divergence. |
| `--feed` | Backtesting mode: injects the recorded wire ingress tape into modified code while leaving clocks, RNG, `std::env`, and `argv` live. |
| `--allow-unmatched-feed` | Under `--feed`, permits the run to succeed even if some recorded ingress frames were not consumed. |
| `--allow-live-effects` | Permits replaying a program whose call graph reaches unjournaled `syscall`, `ffi`, or `unclassified` effects. |
| `--allow-unverified-model` | Bypasses the executable identity hash check to replay against modified source code or unstamped recordings. |
| `--dev` / `--env <name>` | Passes the build flags used when the recording was created (required to match the recorded execution identity). |

### Three-Stage Replay Admission

Before executing a single instruction of the target program, `hale replay` enforces three admission checks:

1. **Live-effect safety gate**: Re-executing a program would normally repeat real external side effects. If the program's effect frontier reaches `syscall`, `ffi`, or `unclassified`, `hale replay` refuses to run and names the offending call chain unless you pass `--allow-live-effects`. (`bindings` blocks do not trigger this refusal because bound network transports are suppressed hermetically during replay.)
2. **Executable identity verification**: The recording header stores a framed SHA-256 hash covering the compiler version, runtime/stdlib sources, build flags (`--dev`, `--env`), and the relative paths, lengths, and contents of all source files in the seed. `hale build` and `hale run` produce identical hashes (DWARF debug info is excluded), so a recording captured from a compiled production binary (`./myapp`) replays cleanly against its source directory (`hale replay run.halerec myapp/`). Any edit to a function body or mismatch in build flags is rejected unless `--allow-unverified-model` or `--feed` is passed.
3. **Artifact integrity**: The runtime re-reads the inherited file descriptor into private memory and validates every frame and entry count up to the finalize trailer.

During execution, the runtime serves journaled inputs per consumer and releases queued deliveries in the exact recorded order (holding early arrivals in a per-consumer reorder buffer). If execution runs past the recorded history, reads and deliveries fall back to live execution and increment divergence counters printed at exit.

## Verifying Execution & Coverage (`--diff`)

Passing `--diff` fails the replay if any divergence occurs and prints a per-category verification summary:

```text
replay matches the recording (26 ring records). Compared, by category:
  public bus events:     20 across 1 consumer (publish + deliver, subject-aligned)
  payloads:              10 (sizes and identities; contents not canonicalised)
  queued consumes:       not exercised: no queued consumer deliveries were recorded; dispatch was direct (valid — a synchronous delivery has no queue order to enforce), so no queued delivery order was verified
  async schedule steps:  not exercised: no async pool scheduling steps were recorded
  journal reads:         not exercised: no time, randomness or env reads were journaled
A category reported as not exercised was NOT verified by this match.
```

### Understanding Category Coverage

- **`public bus events` vs. `queued consumes`**: Intra-thread bus deliveries compile to synchronous direct calls and are verified under `public bus events`. `queued consumes` verifies cross-thread/mailbox ordering (such as two `pinned` publishers racing into a shared subscriber).
- **`payloads` (`contents_canonicalised` vs. `metadata_only`)**: Wire frames are compared byte-for-byte (`canonical bytes identical`). In-process struct payloads are compared by size and publish identity (`sizes and identities; contents not canonicalised`) because raw struct memory contains arena pointers and padding.
- **`async schedule steps`**: For `where async_io` pools, records and replays the worker drain's exact coroutine scheduling steps (cell starts, readiness wakes, and timed-park expiries, fast-forwarding recorded sleeps).
- **`--diff --json`**: Emits machine-readable JSON containing `result` (`"match"` or `"diverged"`), `ring_records`, `recorded_prefix_only`, and per-category `compared`, `count`, `consumers`, `not_exercised_because`, `contents_canonicalised`, and `metadata_only` fields.

## Hermetic Transports & Ingress Injection

During `hale replay`, native `unix://` and `udp://` transport bindings run hermetically:
- No OS sockets are opened, bound, or connected, and outbound publishes to bound topics are suppressed on the wire (while still recorded for `--diff`).
- One injector worker per recorded ingress source feeds the verbatim wire bytes captured during recording, paced against recorded execution progress so bounded queues experience the exact same load.

> [!IMPORTANT]
> Transport hermeticity is enforced per backend capability. Bindings that do not yet implement a hermetic replay adapter—such as `shm_ring`—are rejected at both the CLI and runtime seam so a replayed process never attaches to live shared memory.

## Backtesting with Feed Mode (`--feed`)

When you modify handler logic or strategy code and want to test your changes against a captured production traffic tape, use `--feed`:

```sh
hale replay prod-traffic.halerec app.hl --feed
```

`--feed` keeps the wire hermetic and injects the recorded `bindings` ingress tape, but drops replay constraints that only apply to identical code:
- **Bypasses model identity, schedule locking, and input journals**: Code changes are accepted, and `std::time`, `std::rand`, `std::env`, and `argv` execute live.
- **Preserves the live-effect safety gate**: Programs reaching raw `syscall`, `ffi`, or `unclassified` effects still require `--allow-live-effects`.
- **Audits every tape frame**: At exit, `--feed` classifies every recorded ingress frame (`injected`, `rejected`, `unmatched`, `incompatible shape`, `unprocessed`). If any frames remain unfed, the run exits non-zero unless `--allow-unmatched-feed` is specified.

## Single-Pool Determinism Without Recording

Any Hale program whose loci run on the default single-threaded `main` cooperative pool is deterministic by construction: given the same inputs, publishes and deliveries always execute in the exact same order (`spec/testing.md` § *Determinism*). For single-pool unit tests, combine [`@deterministic`](../effects.md) with standard `hale test` assertions; reserve `LOTUS_OBS_RECORD` and `hale replay` for multi-pool concurrency, `async_io` schedules, and captured network ingress.
