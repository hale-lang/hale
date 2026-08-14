# Record & replay

"Reproduce the incident exactly" is worth more to someone running a
system than any counter you can print. Hale's substrate is unusually
close to being able to promise it: a cooperative pool is one
consumer thread by construction, messages are copies, and every
delivery already passes an observation probe. Record & replay is
those ingredients combined — a flight recorder over the observation
plane, and a `hale replay` command that re-executes what it
captured.

The claim, stated exactly: **re-run a recorded execution and get
the same schedule and the same journaled inputs, with an explicit,
checked coverage boundary.** What sits outside that boundary —
live sockets, files, subprocesses — is refused by default rather
than silently re-executed. This chapter walks the whole surface;
the normative contract is `spec/runtime.md` § *Lossless recording
mode* and § *Replay*.

## Recording a run

```sh
LOTUS_OBS_RECORD=run.halerec hale run app.hl
```

Recording turns the observation plane's sampler disposition into a
flight recorder's. A live observer prefers losing records to
stalling the observed program; a recorder makes the opposite trade,
because a dropped record is a replay that diverges silently:

- a producer whose ring is full **blocks** until the in-process
  drain catches up, instead of overwriting the oldest record;
- a thread that cannot get a ring, a capture allocation failure, a
  write failure, or a finalize failure **fails the run** — never a
  silent gap;
- the file ends with a clean-finalize trailer, and a finalized
  recording is admitted for replay only when the *entire* artifact
  validates — exact parse to the trailer, matching entry count.

The recording is also **crash-durable** (a durable flight
recorder — not a write-ahead log: the application never waits on
the recording, so a crash can lose the last instants before it):
frames stream to disk in order and the header identity is stamped
eagerly, so a run that dies without teardown — the run whose
recording you want most — leaves an artifact that is exact up to
one torn frame at the tail. Replaying it is an explicit opt-in
(`--allow-truncated`): the loader stops at the torn frame, says
how much it kept, and replays that prefix; under `--diff` the
post-crash suffix is surplus, not divergence, while anything
inside the prefix still must match. `LOTUS_OBS_RECORD_DURABLE=1`
is the power-loss grade: `fdatasync` per drain sweep AND on the
finalize trailer, plus a parent-directory sync at creation, with
the grade recorded in the artifact header.

The recording needs no observer attached and works from the first
probe. It captures four things:

1. **The schedule.** Every queued delivery gets a consume record on
   the consuming thread at handler invoke, carrying the delivery's
   identity — (target locus, message id), where the message id is
   deterministic per publisher thread. Per consumer, that stream
   *is* the order the handlers ran in.
2. **The public bus stream.** Publishes and deliveries as the iris
   protocol already emits them, with file-side identity maps so the
   comparator can align them across runs. Synchronous
   direct-dispatch traffic — the devirtualized flavor most
   intra-app traffic compiles to — is visible here.
3. **Payloads.** Wire-encoded payloads verbatim — including every
   message a listen binding receives, captured in its wire form as
   an **injectable ingress tape** — and raw in-process structs as
   *metadata only* (an ABI snapshot would carry heap pointers and
   padding while being useless for comparison).
4. **The input journal.** Every user-facing nondeterministic read —
   `std::time::now`, `std::time::monotonic[_ns]`,
   `std::rand::next_int`, `std::os::getrandom`, the `std::env`
   surface — with its exact encoded arguments, per consumer.

**Secrets:** env *values* are withheld from recordings by default —
names, existence, and lengths are captured; the value itself
requires `LOTUS_OBS_RECORD_ENV=full`, and the artifact header
records which policy applied. A withheld read replays as a named
divergence, never as a substituted value.

Recording changes your program's timing by design; don't leave it
on in production.

## Replaying one

```sh
hale replay run.halerec app.hl            # re-execute it
hale replay run.halerec app.hl --diff     # + compare, fail on any divergence
hale replay run.halerec app.hl --at 4120  # SIGSTOP at consume #4120
hale replay run.halerec app.hl --at 65:12 # ...at consumer 65's 12th consume
```

`hale replay` recompiles the program through the same pipeline as
`hale run`, then admits the recording — strongest check first:

- **Safety, before anything else.** Re-execution repeats real side
  effects, so a program whose effect frontier reaches `syscall`,
  `ffi`, or `unclassified` is refused with the residue named,
  unless you pass `--allow-live-effects`. This is deliberately
  coarse (a lone `sleep` trips it); over-refusing is the safe
  direction. `bindings` blocks are *not* on the list: under replay
  the wire is hermetic — bound transports never open, and the
  recorded ingress is injected instead (below).
- **Executable identity.** The recording carries a framed SHA-256
  over the full compiler/runtime/stdlib source tree, the compiler
  version, build options, and every application source's path,
  length, and contents. A structurally compatible model with a
  changed function body is *not* the same executable — it is
  rejected, with `--allow-unverified-model` as the explicit
  override for unstamped or divergent-build recordings.
- **Artifact integrity.** The child re-reads the same validated
  file object (an inherited descriptor, no path re-resolution
  window), snapshots the bytes into private memory, and
  independently re-validates the whole structure.

During re-execution the runtime serves journaled reads back per
consumer — refusing to serve an entry whose kind or arguments
differ from the caller's — and re-consumes each consumer's queued
deliveries in the recorded order, holding early arrivals in a
per-consumer buffer. **Replay degrades, never refuses**: a read or
delivery past the recorded history falls back live and is counted,
and the divergence summary (journal misses, order holds, unconsumed
or unexpected deliveries) prints at exit. Under `--diff`, *any* of
those fails the comparison, alongside a bidirectional comparison of
consume streams, public bus streams, payloads, and journals.

Two pinned publishers racing into one sink replay in exactly the
interleaving that was recorded — that is the per-consumer order
enforcement working, and it is pinned by tests that run the race
repeatedly.

## The wire is hermetic (for backends that support it)

A replayed server must not talk to the real world — in either
direction. Under replay, the native `unix://` and `udp://`
binding transports are suppressed at realization: no socket is
created, listeners don't bind, connectors don't connect, and
publishes to bound topics send nothing (they are still recorded
and compared under `--diff`). In the listeners' stead, an
injector — started at an explicit boot phase, one worker per
recorded ingress source — re-dispatches the recording's ingress
tape: the verbatim wire bytes each listener received, in recorded
order, paced against recorded progress (so bounded queues shed
nothing they didn't shed originally), each delivery carrying its
*recorded* identity and matched by its full subject and payload
shape. Replay a server binary with no peers, no free ports, and
no network, and its subscribers see exactly the traffic the
original accepted — rejected frames never re-fed, and every
non-injected tape entry classified and counted.

Hermeticity is a per-backend capability, not a blanket claim: a
binding backend with no replay class — `shm_ring` today — is
refused outright, by the CLI and again at the runtime seam, so a
replayed process cannot touch live shared memory.

## Feed mode: same recorded ingress, changed code

```sh
hale replay run.halerec app.hl --feed
```

Backtesting is a different contract from replay: not "the same run
again" but "the same **recorded ingress** again," against code you
have changed — time, randomness, env, and argv stay live, which
is why the name is precise rather than "same inputs." `--feed`
consumes the recording as an input tape only — recorded ingress
injected, wire hermetic — and deliberately drops the rest of
replay: no journal serving, no order enforcement, no model
admission (a hash mismatch prints as information; feeding a tape
to changed code is the point). What it does **not** drop is the
effects gate: `--feed` bypasses identity, never safety — a
program that reaches `syscall`/`ffi`/`unclassified` still needs
`--allow-live-effects` beside it. Record a day of production
traffic once; run every strategy revision against it. The exit
report classifies every tape entry (injected, rejected,
unmatched, incompatible shape, unprocessed), and an unfed
remainder fails the run unless you accept it explicitly with
`--allow-unmatched-feed` — a dropped tape is a fact, never a
silence.

## The coverage boundary, honestly

- **Per consumer, not global.** Each consumer reproduces its own
  recorded order; cross-consumer wall-clock alignment is not a
  replay property (and is exactly what the recording never
  promised).
- **The injected tape covers `bindings` ingress.** Raw user-level
  socket reads are syscall-class live effects (the gate names
  them), and adapter loci re-execute their own protocol logic.
  Fleet-scale replay — multiple binaries against one composed
  tape — is the next milestone.
- **`where async_io` pools refuse replay** loudly; their coroutine
  interleaving is a later milestone.
- **The artifact format is pre-stable** while the remaining phases
  (fleet replay, replay-under-a-different-plan, durability grades)
  land.

## Determinism without the recorder

A program whose loci all run on the main scheduler is deterministic
by construction — same publishes, same deliveries, same order,
given the same inputs. That is a stated guarantee
(`spec/testing.md` § Determinism), which means a flaky single-pool
test is never the scheduler's fault: look for a real input feeding
the assertion, or mark the code under test
[`@deterministic`](../effects.md) and let the compiler prove there
isn't one. The recorder exists for everything that guarantee
doesn't cover.
