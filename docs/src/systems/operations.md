# Operations & Production Debugging

Hale binaries include built-in, zero-overhead hooks for diagnosing message drops, tracking arena memory residency, supervising OS subprocesses, and inspecting live processes with standard DWARF debuggers (`gdb`, `lldb`, `perf`). Every diagnostic environment variable and compiler report is observe-only and leaves program semantics unchanged.

```hale
fn on_tick() {
    std::process::dump_arena_residency(); // Emits live arena sizes when LOTUS_ARENA_RESIDENCY=1
    println("peak rss = ", std::process::rss_bytes() / 1048576, " MB");
}
```

## Runtime Diagnostic Environment Variables

Set these environment variables when launching a compiled Hale binary to inspect bus delivery, backpressure, arena memory, and live observation rings (see `spec/runtime.md` for the normative specification):

| Environment Variable | Default | Purpose |
|---|---|---|
| `LOTUS_BUS_LOG_DROP=1` | Unset | Logs one stderr line for every dropped bus message (no matching subscriber, serialize/deserialize `<= 0`, or null post target). Implies `LOTUS_BUS_LOG_UNMATCHED=1` and `LOTUS_BUS_LOG_DESERIALIZE_DROP=1`. |
| `LOTUS_BUS_LOG_UNMATCHED=1` | Unset | Logs keyed publishes (`where key == ...`) that matched zero subscribers, printing the subject, key, and per-topic subscriber counts. |
| `LOTUS_BUS_LOG_DESERIALIZE_DROP=1` | Unset | Logs frames dropped by the `udp://` reader thread due to missing deserializers or size mismatches. |
| `LOTUS_BUS_COUNTERS_DUMP=1` | Unset | Prints a summary line per remote binding at process exit: messages/bytes sent and delivered, send failures, link-down drops, `or wait` parks, listener re-arms, and reconnects. |
| `LOTUS_BUS_QUEUE_CAP=<N>` | `8192` | Bounds the main dispatch queue, each pinned-locus mailbox, and each cooperative pool queue to `N` cells (~4.3 MB per queue at default `8192`). |
| `LOTUS_ARENA_RESIDENCY=1` | Unset | Registers every top-level arena (locus regions, global arena, bus payload arena) with a creation backtrace for `std::process::dump_arena_residency()`. |
| `LOTUS_NO_CHUNK_POOL=1` | Unset (`1` under ASAN) | Returns freed 64 KiB arena chunks directly to `libc` instead of caching them in the thread-local pool, allowing AddressSanitizer to trap `heap-use-after-free` immediately. |
| `LOTUS_NO_DEBUGINFO=1` | Unset | Strips DWARF debug tables during `hale build`. |
| `LOTUS_OBS=1` | Unset | Publishes the `/hale-obs-<pid>` shared-memory observation segment under `$XDG_RUNTIME_DIR/hale/` (or `/tmp/hale-obs/`) for [Iris](./iris.md). |
| `LOTUS_OBS_RINGS=<N>` | `8` (`64` when recording) | Number of SPSC observation rings allocated per process. |
| `LOTUS_OBS_SLOTS=<N>` | `4096` | Number of record slots per observation ring. |
| `LOTUS_OBS_WIRE=1` | Unset | Embeds `(origin, seq)` headers in `udp://` and `unix://` (with `LOTUS_UNIX_STREAM=1`) frames to pair cross-process edges. Requires all communicating binaries to support the header. |
| `LOTUS_OBS_RECORD=<path>` | Unset | Enables lossless flight recording to `<path>` for `hale replay` (implies `LOTUS_OBS=1`; stalls producers on full rings rather than dropping records). |

## Diagnosing Bus Delivery & Backpressure

### Tracing Undelivered Publishes

For `on_unmatched: swallow` topics, publishing to a subject with no active subscriber silently drops the message by design. Run the binary with `LOTUS_BUS_LOG_DROP=1` to log every drop site to `stderr`:

```sh
LOTUS_BUS_LOG_DROP=1 ./myapp
```

If `LOTUS_BUS_LOG_DROP=1` prints nothing but a subscriber handler still never fires, the message was enqueued and the subscriber's worker pool is not draining:
1. Call `std::process::dump_pool_residency()` to inspect pending and in-flight task counts per pool.
2. Check whether a locus on the same cooperative pool is executing a blocking syscall or long `std::time::sleep` inside `run()`, starving sibling handlers (`hale check` warns on blocking calls in cooperative `run()` bodies).
3. Verify that the subscriber locus was instantiated as a persistent field or top-level binding before the publisher fired, rather than as an unowned statement literal that dissolved immediately.

### Bounding Queue Memory (`LOTUS_BUS_QUEUE_CAP`)

When a fast publisher outpaces a consumer, queue depth is capped at `LOTUS_BUS_QUEUE_CAP` cells (default `8192`):
- **Single-threaded cooperative producer**: Inline-drains the oldest queued handlers to free a slot before enqueuing.
- **Cross-thread producer to a pinned mailbox**: Blocks on a condition variable until the pinned consumer drains a slot.

No messages are dropped when the queue cap is reached. Lower `LOTUS_BUS_QUEUE_CAP` (`LOTUS_BUS_QUEUE_CAP=1024 ./myapp`) to reduce peak buffered memory, or raise it to absorb larger bursts without inline draining.

## Diagnosing Memory Growth & Arena Residency

### Runtime Arena Profiling

To inspect live memory consumption in a long-running service, enable `LOTUS_ARENA_RESIDENCY=1` and call `std::process::dump_arena_residency()` from a periodic timer or heartbeat handler (calling it only at exit misses locus arenas that have already dissolved):

```hale
fn on_heartbeat() {
    std::process::dump_arena_residency();
    println("rss_mb=", std::process::rss_bytes() / 1048576);
}
```

Each line printed by `dump_arena_residency()` reports:
- `bytes=`: Total chunk capacity attached to the arena (grows in 64 KiB increments).
- `used=`: Exact byte count currently occupied inside those chunks (compare `used=` across two heartbeats to measure exact per-request retention).
- `chunks=`, `parent=`, `label=`, and the construction backtrace identifying the locus and birth site.

> [!NOTE]
> `std::process::rss_bytes()` returns peak RSS (`getrusage.ru_maxrss`), which a child process inherits from its parent across `fork` + `exec`. Track the delta over time rather than the initial value, or read field 2 of `/proc/self/statm` on Linux for current instantaneous RSS.

### Compile-Time Allocation & Resource Flags

Before running a binary, use `hale check` flags to audit memory bounds and static system resources:

| CLI Flag | Output / Behavior |
|---|---|
| *(default on `hale check` / `build`)* | Warns on allocations escaping to `self` in unbounded loops/handlers and growing `@form(vec)`/`@form(hashmap)` inserts (exempts run-to-exit scripts automatically). |
| `--no-warn-unbounded-alloc` | Suppresses default unbounded-allocation warnings for the invocation. |
| `--report-unbounded-use` | Lists every `@unbounded` and `@bounded_by(<name>)` annotation in user code and reached stdlib files (`crates/hale-stdlib/hl/<file>:<line>`), showing how many warnings each silences. |
| `--dump-alloc-summary` | Prints every allocation site with its escape tag (`local`, `returned`, `stored-to-self`, `sent`), target `self.<field>` / slot, locus capacity shape, and boundedness verdict. |
| `--dump-resource-budget` | Prints static counts of spawned threads (pinned replicas and adapters), worker pools, bus subjects, and file-descriptor acquisition sites. |
| `--check-resource-budget <file>` | Verifies the program's static resource counts against ceiling limits declared in `<file>`. |
| `--locality-report` | Compares each locus's working-set size against CPU cache-tier budgets. |

To acknowledge intentional in-memory growth on a function or lifecycle hook, annotate it with `@bounded_by(<input_name>)` when bounded by a domain quantity, or `@unbounded` when operator-sized:

```hale
type Snapshot { seq: Int; total: Int; }

locus Aggregator {
    @unbounded fn on_snapshot(s: Snapshot) {
        // Intentional operator-sized cache accumulation
    }
}
```

## Subprocess Execution & Supervision (`std::process`)

### Synchronous Execution (`run` and `run_with`)

`std::process::run(argv)` forks, execs, and waits for a child process without invoking a shell. Arguments are passed as a **newline-separated** `String` (each line becomes one `argv` entry, capped at 64 KiB / 65,536 bytes total; exceeding 64 KiB raises `IoError { kind: "invalid", errno: 7 }` `E2BIG` before forking):

```hale,fragment
let out = std::process::run("git\nstatus\n--short") or raise;
println("exit ", to_string(out.code));
println(out.stdout);
if len(out.stderr) > 0 {
    println("stderr: ", out.stderr);
}
```

`ProcessOutput` contains:
- `code: Int`: Exit status (`-1` if terminated by a signal or timeout).
- `signal: Int`: Terminating POSIX signal number (`0` on normal exit).
- `stdout: String` and `stderr: String`: Captured output streams.
- `timed_out: Bool`: `true` if stopped by a `SpawnOptions` timeout.

Use `std::process::run_with(argv, opts)` (or `spawn_with(argv, opts)`) with `std::process::SpawnOptions` to configure working directory, environment variables, timeouts, and POSIX `rlimit` ceilings in the child before `exec`:

```hale,fragment
let opts = std::process::SpawnOptions {
    cwd: "/srv/build",                 // "" inherits parent cwd
    env: "CI=1\nLANG=C",               // newline-separated NAME=value overrides
    unset_env: "HOME",                 // newline-separated variable names to remove
    clear_env: false,                  // true starts from an empty environment
    timeout: 30s,                      // 0ns disables timeout
    cpu_seconds: 60,                   // RLIMIT_CPU
    address_space: 512 * 1024 * 1024,  // RLIMIT_AS (bytes)
    open_files: 64,                    // RLIMIT_NOFILE
    file_size: 16 * 1024 * 1024,       // RLIMIT_FSIZE (bytes)
};
let out = std::process::run_with("make\ntest", opts) or raise;
if out.timed_out {
    println("make test exceeded 30s timeout");
}
```

When `timeout` expires, the runtime sends `SIGTERM` to the child's process group, waits 100 ms, sends `SIGKILL`, reaps the child, and returns a normal `ProcessOutput` with `timed_out: true` and `code: -1`.

### Incremental I/O and Single-Owner Supervision (`spawn` and `adopt`)

For interactive or long-running child processes, use the `std::process::Child` handle operations:

| Function | Behavior |
|---|---|
| `spawn(argv)` / `spawn_with(argv, opts)` | Forks and execs the child with piped `stdin`, `stdout`, and `stderr`. |
| `write_stdin(c, data)` / `close_stdin(c)` | Writes to or closes the child's `stdin` pipe (`close_stdin` sends `EOF` to filters like `sort` or `cat`). |
| `read_stdout(c)` / `read_stderr(c)` | Reads available output from the child's pipes. |
| `wait(c)` | Blocks until the child exits (or hits its `SpawnOptions` deadline) and returns its exit code. |
| `try_wait(c)` | Non-blocking poll: returns `-2` if still running, `-1` if killed by signal/timeout, or the exit code (`0..255`). |
| `kill(c)` | Escalates `SIGTERM` -> grace period -> `SIGKILL` and reaps the child. |
| `signal(c, sig)` | Sends a specific POSIX signal (e.g., `15` for `SIGTERM`, `1` for `SIGHUP`) without automatic `SIGKILL` escalation. |
| `adopt(dest, spawned)` | Transfers ownership of `spawned`'s PID and pipe descriptors into `dest` (`self.child`). |

```hale,fragment
let c = std::process::spawn("/usr/bin/sort") or raise;
std::process::write_stdin(c, "b\na\n") or raise;
std::process::close_stdin(c);
let code = std::process::wait(c) or raise;
let sorted = std::process::read_stdout(c) or "";
```

Because `Child` is a locus, assigning `self.child = std::process::spawn(argv)` directly is rejected to prevent dual ownership. Instead, initialize `child: std::process::Child = std::process::Child { }` in `params` and transfer the spawned process into the field with `std::process::adopt`:

```hale
locus Supervisor {
    params { child: std::process::Child = std::process::Child { }; }

    fn start(argv: String) {
        let spawned = std::process::spawn(argv) or std::process::Child { };
        std::process::adopt(self.child, spawned);
    }

    fn tick() {
        let code = std::process::try_wait(self.child) or -2;
        if code != -2 {
            self.start("./worker");
        }
    }
}
```

`std::process::adopt(self.child, spawned)` guarantees safe lifecycle management:
- **Automatic stop on replace**: If `self.child` already holds a live process, `adopt` terminates (`SIGTERM` -> grace -> `SIGKILL`) and reaps it before installing the new descriptors.
- **No descriptor-reuse races**: Old pipe descriptors remain open until the new child is spawned and adopted, preventing the kernel from reassigning those fd numbers to the new child.
- **Clean failure handling**: Adopting an empty `Child { }` (from a failed `spawn`) clears `self.child`, and when `Supervisor` dissolves, `self.child` automatically closes its pipes and reaps its subprocess.

### Process Self-Introspection

- `std::process::pid() -> Int`: Returns the current OS process ID.
- `std::process::uid() -> Int`: Returns the real user ID (matches the `uid:<n>` peer identity reported by `std::io::unix::Rpc`).
- `std::process::exit(code: Int)`: Terminates the process immediately with `code`.

## Live Observation, Recording & Native Debugging

### Live Observation & Flight Recording

Setting `LOTUS_OBS=1` exposes a lock-free SPSC observation ring per thread at `/hale-obs-<pid>`, allowing [Iris](./iris.md) (`hale run --observe app.hl`) to visualize the live locus hierarchy, bus traffic, and supervision events. Setting `LOTUS_OBS_RECORD=run.halerec` captures a deterministic recording that can be re-executed and diffed with `hale replay` (see [Record & Replay](./replay.md)):

```sh
hale replay run.halerec app.hl --diff
```

### Debugging with `gdb`, `lldb`, and `perf`

Hale binaries emit full DWARF debug information (source line tables, parameter types, local variables, `char*` strings, and struct member layouts) by default at zero runtime cost:

```sh
hale build myservice
gdb ./myservice
(gdb) break myservice.hl:42
(gdb) run
(gdb) backtrace
(gdb) info args
(gdb) info locals
(gdb) print msg
```

- **Preventing `<optimized out>` locals**: Build with `hale build --dev` when stepping through code interactively so `-O3` register allocation does not elide dead variables.
- **Resolving crash addresses**: Run `addr2line -e ./myservice 0x4a2f10` to map instruction pointers directly to `.hl` file and line numbers.
- **Detecting arena use-after-free**: Pass `LOTUS_NO_CHUNK_POOL=1` when running outside AddressSanitizer so destroyed 64 KiB arena chunks return immediately to `libc` rather than recycling in the thread-local pool.
- **CPU profiling**: Use `perf record --call-graph dwarf ./myservice` (Hale omits forced frame pointers by default because they add ~22% overhead on runtime fast paths).
