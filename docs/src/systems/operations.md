# Operations & debugging

Most of the time an Hale program either works or fails loudly. The
two exceptions — the ones that send you here — are **a message that
quietly doesn't arrive** and **resident memory that quietly grows**.
Both are silent by design (the steady-state behavior is correct), so
the runtime ships opt-in diagnostics you switch on with an
environment variable or a build flag. This chapter is the operator's
map: what each knob shows, and two worked triage walkthroughs.

Nothing here changes behavior — every switch is observe-only. The
canonical reference for each variable is `spec/runtime.md`; this is
the pedagogical version.

## Bus: "my publish isn't arriving"

A `publish` that compiles is not a `publish` that's delivered — the
subject might match no subscriber, the payload might fail to
deserialize, or the subscriber might be on a pool that never runs.
The bus drops these silently because for an `on_unmatched: swallow`
topic in steady state that *is* the right behavior. To see the
drops, set one variable:

```sh
LOTUS_BUS_LOG_DROP=1 ./myapp
```

`LOTUS_BUS_LOG_DROP` is the broad net — reach for it first. It
prints one stderr line at every silent-drop site, naming the call
site, subject, and size/index info: no-matching-subscriber,
serialize-returned-≤0, deserialize-returned-≤0, and the
"matched-but-no-post-target" case (mailbox / pool / queue all null).
It *implies* the two narrower variables, which you can use on their
own once you know which class you're chasing:

| Variable | Surfaces |
|---|---|
| `LOTUS_BUS_LOG_DROP=1` | everything below, plus serialize-fail and no-post-target |
| `LOTUS_BUS_LOG_UNMATCHED=1` | a keyed publish (`where key == …`) that matched no subscriber — prints subject, key, and the per-topic subscriber counts |
| `LOTUS_BUS_LOG_DESERIALIZE_DROP=1` | the `udp://` reader thread dropping a frame (no deserializer registered, or a size-mismatched read) |

**The shape that produces no line at all.** If `LOTUS_BUS_LOG_DROP`
is silent but the handler still never fires, the message *was*
delivered to the queue and the problem is downstream: the
subscriber's pool isn't draining. The classic cause is a `run()` on
a cooperative pool that blocks (a long `time::sleep`, a blocking
syscall) and starves the handler — `hale check` warns on blocking
syscalls in a cooperative `run()`, and `std::process::dump_pool_residency()`
shows pending counts per pool so you can see work piling up unserved.

## Memory: "my RSS is growing"

Hale frees a locus's whole region on dissolve, so a *leak* is
usually one of two things: an allocation that escapes to a
long-lived arena (it never dissolves), or a queue/buffer whose
high-water mark keeps climbing. Two layers of instrumentation pin it
down — one at runtime, one at compile time.

**Runtime residency.** Set `LOTUS_ARENA_RESIDENCY=1` to register
every top-level arena (each locus's region, the global, the bus
payload arena) with a construction backtrace. Then call
`std::process::dump_arena_residency()` to emit one line per live
arena — bytes, chunks, parent, label — sorted by bytes descending,
each with the backtrace of where it was created:

```hale
// In a long-running daemon, sample from a heartbeat tick so locus
// arenas are caught *while alive* — the atexit dump fires only
// after every locus has torn down.
fn on_tick() {
    std::process::dump_arena_residency();   // → stderr, needs LOTUS_ARENA_RESIDENCY=1
    println("rss=", std::process::rss_bytes() / 1048576, " MB");
}
```

`std::process::rss_bytes()` is the cheap top-line number — poll it
to confirm growth before you go digging. `dump_pool_residency()` is
the per-pool view (pending/in-flight work), useful when the growth
is a queue rather than an arena.

**Compile-time proofs.** Before the program even runs, three build
flags report on allocation shape:

| Flag | Reports |
|---|---|
| *(default on every check/build)* | flag an allocation that escapes into an unbounded context and accumulates until its locus dissolves (advisory warnings; `--no-warn-unbounded-alloc` opts out) |
| `--dump-alloc-summary` | every allocation site, escape-tagged (local / returned / stored-to-self / sent), with the bounded-vs-unbounded verdict; plus each locus's storage shape (capacity slots, `@form`, projection cap) and the `self.<field>` / `self.<slot>` an allocation targets |
| `--dump-resource-budget` | per-locus resource counts (allocations, held fds) against declared ceilings |
| `--locality-report` | per-locus working-set size against cache-tier budgets |

The memory-bound warnings run **by default** on every `hale check`
and `hale build`. Run-to-exit programs are exempt
automatically: a binary whose `main` starts no `run` loop and
subscribes no handler owes no memory-bound proof, so scripts and
one-shot tools stay silent.

For a long-lived service, the surface is:

- **`@unbounded fn`** — the greppable in-source carve-out for an
  acknowledged accumulation (an operator-sized cache, an
  idempotency log). Silences that body's sites. Also valid on a
  lifecycle hook (`@unbounded run { … }`).

  ```hale
  locus Aggregator {
      // ... handlers checked for unbounded accumulation ...

      @unbounded fn on_snapshot(s: Snapshot) {
          // acknowledged: this cache is operator-sized on purpose.
      }
  }
  ```

- **`--no-warn-unbounded-alloc`** — opts a whole run out.
- `@bounded locus L { … }` is now redundant with the default and
  still accepted.

The warnings are advisory — they print but don't fail the build. A
warning here is the compile-time complement to the residency dump:
it tells you *which site* can grow before you've watched it grow.

## Bus backpressure: bounding a flood

A producer that outruns its consumer used to grow the dispatch queue
without limit. It no longer does — the queue and each pinned-locus
mailbox are capped at `LOTUS_BUS_QUEUE_CAP` cells (default 8192 ≈
4.5 MB):

```sh
LOTUS_BUS_QUEUE_CAP=1024 ./myapp   # tighter bound, more frequent drains
```

Past the cap the producer **back-pressures** rather than buffering:
a single-threaded cooperative producer inline-drains the queue
(runs the oldest handlers) to make space; a cross-thread producer to
a pinned mailbox blocks on a condvar until the consumer drains a
slot. Every message is still delivered — only the timing and memory
profile change. Lower the cap to tighten the memory bound; raise it
to reduce drain bursts.

## Shelling out to other programs

Ops glue often means running another tool. `std::process::run`
does a synchronous fork + exec + wait and captures the result. The
argument vector is **newline-separated** (no shell, no word
splitting — each line is one `argv` entry):

```hale
let out = std::process::run("git\nstatus\n--short") or raise;
println("exit ", to_string(out.code));
println(out.stdout);
if len(out.stderr) > 0 { println("stderr: ", out.stderr); }
```

The returned `ProcessOutput` carries `code: Int` (the exit code,
or `-1` if killed by a signal), `signal: Int` (the killing signal,
`0` if it exited normally), and `stdout` / `stderr` as captured
`String`s. `run` is `fallible(IoError)` — a missing binary or a
fork failure raises rather than returning a bogus output.

For a long-running child you drive incrementally, the lower-level
`spawn` / `wait` / `kill` / `write_stdin` / `read_stdout` /
`read_stderr` surface over a `Child` handle is in
[`spec/stdlib.md`](https://github.com/hale-lang/hale/blob/main/spec/stdlib.md).
A supervising daemon reaps without blocking via
`std::process::try_wait(c)` — `-2` means still running (poll again
on your next tick), any other value is the exit code (`-1` =
killed by a signal), and the child is reaped:

```hale
fn tick() {
    let code = std::process::try_wait(self.child) or -2;
    if code != -2 { self.on_child_exit(code); }
}
```

`std::process::signal(c, sig)` sends an arbitrary POSIX signal
(15 = TERM, 1 = HUP for a config reload, …) when the fixed
TERM→KILL escalation of `kill` is more than you want.

Other process self-introspection: `std::process::pid()`,
`std::process::exit(code)`, and `std::process::rss_bytes()` (peak
RSS — see [Memory](#memory-my-rss-is-growing) above).

## Worked triage

**"My subscriber's handler never runs."**

1. `LOTUS_BUS_LOG_DROP=1 ./app`. A line at the publish? → the subject
   or key doesn't match, or the payload won't deserialize. Fix the
   subject/key or the payload type.
2. No line, but still no delivery? → the message reached the queue;
   the consumer isn't draining. Check the subscriber's pool: a
   cooperative `run()` that blocks starves handlers. `hale check`
   flags blocking syscalls; `dump_pool_residency()` shows the
   pending pileup.
3. Subscriber is an inline child or on `where async_io`? → confirm
   it's instantiated as an owned param or top-level, not unowned in
   a method body (which dissolves at scope exit before it can fire —
   `hale check` errors on this).

**"My RSS climbs over hours."**

1. `rss_bytes()` from a heartbeat — confirm it's monotonic, not
   sawtooth (sawtooth is healthy churn).
2. `LOTUS_ARENA_RESIDENCY=1` + `dump_arena_residency()` from the same
   heartbeat — find the arena whose `bytes` grows. The `label` and
   backtrace name the locus and birth site.
3. A `root`-kind arena growing is the leak; a `sub` arena recycles.
   If it's the bus payload arena, the high-water is queue depth —
   lower `LOTUS_BUS_QUEUE_CAP`. If it's a locus arena, you're
   accumulating into a field: prefer in-place mutation
   (`self.f.x = v`) over whole-value replace (`self.f = T{…}`), which
   bump-allocates fresh each time. `--dump-alloc-summary` names the
   site at compile time.

## Debugging with the native toolchain

Hale binaries carry full DWARF by default (zero runtime cost):
line tables *and* variable info. That means real debugging — stop
AND inspect:

```sh
hale build myservice
gdb ./myservice
(gdb) break myservice.hl:42
(gdb) run
(gdb) backtrace          # real .hl file:line frames, inline stacks
(gdb) info args          # typed parameters: n = 21
(gdb) info locals        # typed lets: doubled = 42, frac = 0.5
(gdb) print msg          # Strings print their text: "hello!"
```

Hale scalars map to proper DWARF base types (`Int`, `Float`,
`Bool`, `Decimal`, `Time`, `Duration`), `String` is a `char*` so
debuggers print the contents, and struct-typed values carry full
member info — `p *r` prints `{key = "alpha!", n = 41, f = 2.5}`
with nested structs as typed pointers. A variable can read `<optimized out>` after its
last use — that's the optimizer, not missing debug info; `hale
build --dev` keeps more of the frame live.

`addr2line -e ./myservice 0x4a2f10` resolves crash-dump addresses
to source lines, and ASAN reports carry file:line through both the
Hale code and the runtime. Profile with
`perf record --call-graph dwarf` (frame pointers are deliberately
not forced — they cost ~22% on runtime fast paths). Opt out of
debug info with `LOTUS_NO_DEBUGINFO=1`.
