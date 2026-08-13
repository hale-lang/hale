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
- the file ends with a clean-finalize trailer, and a recording is
  admitted for replay only when the *entire* artifact validates —
  exact parse to the trailer, matching entry count.

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
3. **Payloads.** Wire-encoded payloads verbatim; raw in-process
   structs as *metadata only* (an ABI snapshot would carry heap
   pointers and padding while being useless for comparison).
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
  `ffi`, `unclassified`, or any transport-bound `bindings` block is
  refused with the residue named, unless you pass
  `--allow-live-effects`. This is deliberately coarse (a lone
  `sleep` trips it); over-refusing is the safe direction.
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

## The coverage boundary, honestly

- **Per consumer, not global.** Each consumer reproduces its own
  recorded order; cross-consumer wall-clock alignment is not a
  replay property (and is exactly what the recording never
  promised).
- **External ingress re-executes live.** Socket and adapter input
  is captured in the artifact (flagged as ingress) but not yet
  injected — a replayed server talks to the real world, which is
  half of why the safety gate exists. Ingress injection is the
  fleet-replay milestone.
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
