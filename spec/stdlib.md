# Standard library

Bundled with the toolchain, no separate install required. This
document describes the **current** stdlib surface. Milestone /
phase history lives in [`../CHANGELOG.md`](../CHANGELOG.md).

## Path resolution

`.hl` source references stdlib symbols by fully-qualified path:

```hale
let p = std::process::pid();
let contents = std::io::fs::read_file("config.toml");
std::io::tcp::Listener { host: "127.0.0.1", port: 8080 };
```

The parser tokenizes `::` as a path separator. The type checker
resolves `std::*` paths that name Hale-source stdlib declarations
to their real nominal types (GH #470 — literals, fields, methods,
and interface coercions check like user code, and a `std::`
literal that matches nothing is an error); remaining namespaced
paths (Rust-implemented builtins) type permissively, with their
path-call names validated against the stdlib surface registry.
A path-call carrying a row in that registry's signature table
also checks its arity, its argument types and its RETURN type,
and every function the compiler lowers natively or through a named
Hale body has one: the signature its lowering enforced (F.40 phase 4,
S6; before, 118 had none, so any arguments passed `hale check`
and a wrong count or type failed at build, without a location). The
seven `std::io::mirror` cursor primitives are signed with the
arguments their lowering reads, so a call with more is refused by
`hale check`, where lowering ignored the rest. A function the rename
table maps onto a Hale body is checked against that body's own
signature. The return may itself be a stdlib struct (GH #771): a
`std::json::string_field(...)` is a `std::json::JsonString`, so a
misspelled field on the result is a located error naming the
public path, and passing the result where a `String` is expected
is refused. A row also says whether the function can fail, and every
function lowering lowers only under an `or` has a row that does (F.40
phase 4, S5): the `std::io::tcp` and `std::io::tls` timeout and
option setters (`set_recv_timeout`, `set_send_timeout`, and tls's
`set_nodelay` and `set_rx_timestamps`) and the `__` primitives the
stdlib's own `File`, udp and `Child` wrappers call
(`std::io::file::__open` / `__seek` / `__write_bytes`,
`std::io::udp::__bind` / `__send` / `__recv`, the seven
`std::process::__*` primitives) all fail with `IoError`, so a bare
call of one is the located bare-fallible error from `hale check`
(spec/semantics.md § "A bare fallible call is an error"), where
lowering used to refuse it without a span or answer "not
implemented". Lowering keeps no list of its own: whether it refuses a
stdlib call is that same row. The check refuses a bare call of a
function whose row can fail, and an `or` over one whose row cannot;
the one such call that still reaches the build is an `or` over one of
those unsigned functions, or over one whose success the table cannot
state (`std::bytes::builder::__view` / `__text_view`, a view; the check
types both permissively), which the build refuses with "is not a
fallible call — remove the `or` clause" (some used to read "`or` over
unknown path call"). A stdlib function written in Hale (a body the
stdlib's rename table names, like `std::process::adopt` or the
`std::io::file` wrappers) is called like any fn: a statement drops what
it returns, and one that returns nothing can be called as a statement
and only there (F.40 phase 4, S5: before, only `adopt` could, through
a branch of lowering's own). An ARGUMENT position names a stdlib type the same way
(GH #829): `recv_into`'s `buf` is a `std::bytes::BytesBuilder`,
the one type its lowering accepts, so
`std::io::tcp::recv_into(fd, 0, 64)` is a located error at the
argument instead of a spanless codegen refusal. Only a path-call
with no row at all — or a row whose parameter is deliberately
polymorphic, like the user `@form(vec)` destination of
`std::str::split_into` — types permissively, as does an argument
whose own type the checker cannot see; incompleteness therefore
degrades to the historical tolerance, never to a false error.
Lowering resolves a `std::*` path through the function's row (the
`stdlib_surface` family): the row names an intrinsic id, a Hale
body, a rename, or nothing, and one exhaustive `match` on the id
lowers the call at each position.

Some operations are bare **builtins** rather than stdlib
functions (`len`, `to_string`, `abs`, `min`, `max`, the printers —
grammar intrinsics, see [`types.md`](./types.md)). A path that is
a conventional spelling of one of those — `std::str::len`,
`std::string::length`, `std::math::abs`, `std::cmp::min`,
`std::io::println` and their siblings — is answered with that
builtin's call shape (``the length of a String is the builtin
`len(s)` ``) rather than an edit-distance guess, as is the member
spelling `s.len()` / `s.length` / `s.size` on a `String` or
`Bytes` (GH #722). Every other unknown name keeps the plain
unknown-function or unknown-namespace diagnostic: an operation
with no builtin equivalent is never pointed anywhere.

There is **no general module system** at v1 — no `use`
statements, no user-defined modules, no multi-file `.hl`
packages via the std-style mechanism. `std::*` is the only
recognized prefix. Adding a new stdlib function means: one row
in `SURFACES` (`crates/hale-types/src/stdlib_surface.rs`: its
signature, effect classes, fallibility and how it lowers), its
`IntrinsicId` variant, an arm in `lower_std_intrinsic` and one in
`lower_std_intrinsic_fallible` (the compiler refuses a missing
arm), its libc backer in `declare_builtins` and the C function,
a program that calls it (the coverage gate fails without one),
the table-answers golden, this document's row and the book's.

Cross-binary user code uses the F.25 cross-seed-imports mechanism
(`import "path/to/lib" as alias;`) — distinct from the `std::*`
magic path; see [`decisions.md` § F.25](./decisions.md).

## Design principles

- **Batteries included.** Go's approach: if a typical Hale
  program needs it, it ships. A new Hale user shouldn't need
  third-party packages for table-stakes coordinator-system work.
- **One canonical implementation.** Per Go's "one obvious way":
  one `std::collections::Map`, not seven. Multiple options live
  in third-party packages.
- **Framework-aware.** Stdlib types use the language's projection
  classes, modes, and closure tests where appropriate. The
  stdlib is itself disciplined.
- **Replaceable.** Anything in stdlib can be replaced by a
  third-party module; nothing in stdlib is tied into the
  compiler.

`std::*` is the curated path for ships-with-the-compiler bindings
(libc + OpenSSL only at the link floor). User-extensible C-ABI
bindings live outside this surface — see [`spec/ffi.md`](./ffi.md)
for `@ffi("c")` declarations, the mechanism by which library
authors land bindings to third-party C libraries (raylib, sqlite,
curl, ...) in community repos like pond. The stdlib's narrow link
surface is preserved exactly because user code can extend the FFI
surface without touching the compiler.

## Module surface

| Namespace | Surface | Source |
|---|---|---|
| `std::process` | `pid() -> Int`, `uid() -> Int` (the process's real uid), `exit(code: Int)`, `rss_bytes() -> Int`, `dump_arena_residency() -> Int` (no-op unless `LOTUS_ARENA_RESIDENCY=1`; writes per-arena residency snapshot to stderr), `dump_pool_residency() -> Int` (F.35, 2026-05-28; writes one stderr line per cooperative pool — name, async_io vs blocking mode, parked-coro count, pending cell-queue depth — for ops embedding in heartbeat ticks), `run(argv) -> ProcessOutput fallible(IoError)`, `spawn` / `wait` / `kill` / `write_stdin` / `read_stdout` / `read_stderr` over `Child`. **`run_with(argv, opts: SpawnOptions) -> ProcessOutput fallible(IoError)` and `spawn_with(argv, opts) -> Child fallible(IoError)`** (2026-10-12, downstream handoff) are `run` / `spawn` under a `std::process::SpawnOptions` record whose every field defaults to "unchanged": `cwd: String` (`""` inherits), `env: String` (newline-separated `NAME=value` lines set in the child, replacing an inherited value; the last line wins when a name repeats; a line with no `=` or an empty name is `kind="invalid"`), `unset_env: String` (newline-separated names removed from the inherited environment; `env` applies after, so it wins), `clear_env: Bool` (start from an empty environment, then `env`; with no `PATH`, `argv[0]` resolves as `execvp` does without one), `timeout: Duration` (`0ns` = none), and the rlimits `cpu_seconds` (RLIMIT_CPU), `address_space` (RLIMIT_AS, bytes; enforced on Linux, accepted and not enforced on macOS), `open_files` (RLIMIT_NOFILE) and `file_size` (RLIMIT_FSIZE, bytes), each `0` = unchanged and set as both the soft and the hard limit. Everything is applied in the forked child before its exec (the environment built in the parent, the child only installing it, so nothing past the fork allocates), and the parent's own cwd, environment and limits never change. A setup step that fails — a `cwd` that does not exist (`not_found`), a limit above the hard one (`permission_denied` / `invalid`) — fails the CALL: the child reports its errno down a close-on-exec pipe and exits, and the call reaps it and returns the IoError; a missing program is still exit code 127 from a later wait (GH #794). The timeout is a result, not an error: `run_with` stops the child's process group at the deadline (TERM, 100 ms grace, KILL), reaps it, and returns what it wrote so far with `ProcessOutput.timed_out = true` (`false` from `run`, and the field's default) and `code = -1`; a `spawn_with` handle carries the deadline (`Child.deadline_ns`, CLOCK_MONOTONIC nanoseconds, `0` = none), and `wait` returns at it at the latest and `try_wait` past it, each stopping and reaping a child still running, returning `-1` and setting `Child.timed_out`. `close_stdin`, the reads, `signal` and `adopt` (which moves the deadline with the handle) work on a `spawn_with` handle as on `spawn`'s. **`close_stdin(c) -> ()`** (2026-10-12, downstream handoff) closes the parent's end of the child's stdin, so a child that reads its input to the end (`cat`, `sort`) sees EOF; without it `wait` on such a child never returned (writing `""` writes nothing and closes nothing). Idempotent and infallible, like `adopt`: the handle's `stdin_fd` goes to the `-1` sentinel as it closes, so a second call, the handle's dissolve and an `adopt` release close nothing twice, and a later `write_stdin` fails with the closed fd's `IoError`. argv is **newline-separated**, not shell-split, and capped at **64 KiB**: the blob as passed (newlines included) may be at most 65536 bytes (`LOTUS_PROCESS_ARGV_MAX`), and a longer one fails `run`/`spawn` before any fork with `IoError { kind: "invalid", errno: 7 }` (E2BIG) — the cap sits well under the kernel's `ARG_MAX` so a runaway concatenation surfaces at the call, not as an exec failure; pass bulk data on stdin (`spawn` + `write_stdin` + `close_stdin`) or in a file instead; when run/spawn fails ENOENT and argv[0] contains a space, the IoError's `path` label carries a hint naming that mistake (2026-07-27, hale-bun handoff item 4) instead of reading as "binary not on PATH". **`try_wait(c) -> Int fallible(IoError)`** (2026-07-17) — non-blocking reap via `waitpid(WNOHANG)`: `-2` = still running (the retryable sentinel shape `recv_into` uses; poll again), `0..255` = exited, `-1` = killed by a signal; ECHILD (already reaped) surfaces `kind="not_found"`. The supervisor idiom: a periodic tick polls `try_wait` per child without ever parking its pool — closes the styleguide §7 "daemons can't non-blocking-reap" gap. **`signal(c, sig) -> () fallible(IoError)`** (promoted from pond/subprocess) — arbitrary POSIX signal to the child's pid (15=TERM, 9=KILL, 1=HUP, …); the TERM→KILL escalation remains `kill`'s job; ESRCH surfaces `kind="not_found"` (usually benign post-exit — `or discard`). Both honor the manual-`Child` convention: `pid <= 0` answers exited-with-0 / no-ops. **`adopt(dest, src) -> ()`** (GH #716, 2026-09-19) — move a spawned handle into a `Child` the caller already owns, so a service can retain its subprocess past the method that started it. `self.child = std::process::spawn(…)` is refused (a locus-typed field takes only a locus LITERAL — see spec/semantics.md § "Reassigning a locus-typed field"), and `adopt` is not a locus store: it transfers the handle's STATE into the instance the field already owns, so there is never a second owner. Three steps, in this order: `dest`'s outgoing handle is released first (fds closed, process TERM→KILL→reaped) while its descriptors are still open and therefore still uniquely its own — the kernel cannot have reissued those numbers to `src`, so a re-spawn into the same field never closes the new child's pipes; then `src`'s pid + fds move in; then `src` is **disarmed** to the not-spawned sentinels, so its own teardown closes nothing. Infallible (the release is best-effort teardown, like dissolve), so it needs no disposition. `adopt` **terminates** the handle `dest` held — `wait`/`try_wait` first if the outgoing child's exit code matters. Adopting an empty `Child { }` — what the `or` disposition on a failed `spawn` hands back — is the documented way to leave the field empty, so a spawn failure never leaves a half-built handle. Adopting the handle already in place (`adopt(x, x)`) is a no-op. **Ownership:** exactly one `Child` owns a given pid + fd triple, and it is the one whose dissolve closes them; a handle whose fields were copied out by hand is a second owner and double-closes. The binding that names a spawned handle owns it: a `let`-bound `spawn` dissolves at the end of its scope — a fn body, a `birth`, a param default's fn — closing the three pipes and reaping the child (TERM → 100 ms → KILL → `waitpid`). (Until GH #793 a factory-returned locus was never reclaimed, so its dissolve never ran and a `let`-bound spawn left its child running with PPID 1 after the program exited; until 2026-10-12, #1537, that dissolve TERM-ing a child that had not exec'd yet could end a program reading `draining` with exit 128 — see spec/runtime.md § "Process control".) To keep the child past that scope, `adopt` it into a field, which gives the handle an owner that outlives the method that spawned it. | path-call dispatch + C primitives |
| `std::env` | `args_count()`, `arg(i)`, `arg_or(i, default)`, `var(name)`, `var_exists(name)` | path-call dispatch + main-prelude argv stash |
| `std::cli` | `Resolver` locus — layered config resolution with precedence **CLI argv > env var > fallback**. Params: `env_prefix: String = "HALE_"` (each `get(key, …)` looks up `<prefix><UPPER(key)>` in the process env — `prefix="HALE_"`, `key="dir"` → `HALE_DIR`) and `argv_keys: String = ""` (newline-separated positional keys — first line maps to `argv[1]`, second to `argv[2]`, …; a blank line doesn't shift positions; a key absent from `argv_keys` skips the CLI layer). Methods: `get(key, fallback) -> String` (the highest *populated* layer wins; empty/unset at a layer falls through) and `get_int(key, fallback) -> Int` (same precedence; a non-parseable value falls through to `fallback` rather than crash). No birth/run/dissolve lifecycle — the params *are* the configuration, so re-prefixing the Resolver retargets it without touching the body. | `runtime/stdlib/cli.hl` |
| `std::time` | `monotonic() -> Duration`, `monotonic_ns() -> Int`, `sleep(d: Duration)`, `now() -> Int`, `time_from_unix(n: Int) -> Time`, **`current() -> Time`**, **`iso8601(t: Time) -> String`**, **`parse_time(s) -> Time fallible(ParseError)`**, **`unix(t: Time) -> Int`**, **`nanos(t: Time) -> Int`**, **`from_nanos(n: Int) -> Time`** | `clock_gettime` + EINTR-retrying `clock_nanosleep`; **on a `where async_io` pool `sleep` PARKS the coroutine on a deadline** (timer-only park, no fd — Crumb batch-5, 2026-07-28), yielding the shared worker so N sleeping coros overlap instead of serializing and a sleeping handler never holds the pool against unrelated work; off async pools `sleep` slices the request into ≤100ms intervals and folds in a cooperative bus drain after each slice, so a long keep-alive sleep doesn't starve main-pool handlers (see `spec/runtime.md` § "`time::sleep` drain semantics"); `now()` is `CLOCK_REALTIME` seconds and stays; **`current()` is the same clock as a `Time` (nanoseconds; journaled and replayed like `now()`)**. `Time` is i64 nanoseconds since the epoch (GH #607): `time_from_unix(n)` is `n × 10⁹`, `iso8601` renders it (the fraction only when not zero; `to_string` and `println` do the same), `parse_time` is `parse_iso8601` yielding the instant, and both refuse a day past its month's length, leap years counted (`2099-02-30`, `2100-02-29`, `2026-04-31`), as the time literal does at check time (2026-10-17: such a day used to roll over, `2099-02-30` reading as `2099-03-02`), `unix` floors to seconds, `nanos` / `from_nanos` are the integer view. **A `Time` past i64 is refused where it would be made, never wrapped** (2026-10-12, downstream handoff: `parse_time("2999-01-01T00:00:00Z")` used to read as an instant in 1829): a `Time` holds `1677-09-21T00:12:43.145224192Z` through `2262-04-11T23:47:16.854775807Z`; `parse_time` of a well-formed date outside that is its `ParseError` (kind `"parse_time"`, like a malformed one); a time literal outside it is a check error at the literal ("time literal \`2999-01-01T00:00:00Z\` is outside \`Time\`'s range"); `time_from_unix(n)` and `Time ± Duration` (and `Duration + Time`), which are total in the type and have no caller frame for an error, end the program with a `violation:` naming the operation and the range (exit 1), the stale-view panic's shape. `parse_iso8601` is seconds, so it still reads any four-digit year; `from_nanos` is total. `Time − Time` is a `Duration` and follows `Int`'s arithmetic like every `Duration` operator. **`Duration` and `Time` are this module's declarations** (GH #1076, U4): its seed (`hale-stdlib/hl/time.hl`) declares the time catalogue (`unit ns; unit us = 1_000 ns; unit ms = 1_000 us; unit s = 1_000 ms; unit min = 60 s; unit h = 60 min; unit day = 24 h;`), `type Duration = quantity Int in ns;` and `type Time = point Duration;`: a time literal's unit is one of these, and the unit dialect's algebra types the two (spec/units.md § The stdlib's time catalogue); every function above takes and returns them as the nanoseconds they always were. **`parse_iso8601(s) -> Int fallible(ParseError)`** with the probe sibling **`can_parse_iso8601(s) -> Bool`** (2026-08-03) — the inverse of `time_from_unix`, returning unix seconds. Formatting was never missing (`time_from_unix` already yields ISO-8601 text, which is why `println` on a `Time` renders a date); parsing had no counterpart, so a timestamp could be emitted and never read back. UTC only: a timezone database is megabytes against the wasm target, and local time additionally reads `TZ`, making it an `env` effect rather than a pure computation — it can arrive later as a distinct, effectful call. A trailing offset such as `+01:00` is REJECTED rather than ignored, because an hour-wrong timestamp that never announces itself is worse than a failure |
| `std::decimal` | **`from_int(n: Int) -> Decimal`** and **`from_float(x: Float) -> Decimal fallible(ParseError)`** — plain `Decimal`, that is `Decimal<9>`: unlike `parse_decimal`, the result does not take the scale of the place it flows into, so a `Decimal<S>` of another scale is `.rescale(S)` of it (a place of another scale refuses it, naming both) (2026-10-12, downstream handoff: both went through `parse_decimal(to_string(x))`). `from_int` is exact — every `Int` fits (`|n × 10⁹| < 2⁹⁴`). `from_float` is the Decimal nearest `x`'s **exact binary value**, rounded **half away from zero** at the ninth fractional digit (`from_float(0.1)` is `0.1d`, since `0.1` is `0.1000000000000000055…`; `from_float(1.5e-9)` is `0.000000001d`, since `1.5e-9` is `1.49999999999999999…e-9`; `1/1024 = 0.0009765625` rounds to `0.000976563d`), computed exactly — no Float multiplication loses a digit (`from_float(1e29)` is `99999999999999991433150857216d`, the Float's every digit); NaN, an infinity, or a magnitude whose mantissa no i128 holds (past about `1.7e29`) is `ParseError { kind: "from_float", input: to_string(x) }`; an `Int` argument widens to `Float` first. `to_float(d: Decimal<S>) -> Float` `format(d: Decimal<S>, places: Int) -> String` (GH #230, 2026-07-22; every scale since `Decimal<S>`, 2026-10-12): render with exactly `places` fraction digits (0..=S clamped: no digit past the scale exists), round half-up away from zero — the fixed-places money-display surface; default printing still trims trailing zeros (`12.50d` and `12.5d` are one value). | Direct i128 → f64 conversion at the value's scale (`mantissa × 10^-S`) — skips an ASCII round-trip |
| `std::str` | `parse_int(s) -> Int fallible(ParseError)`, `parse_float(s) -> Float fallible(ParseError)`, `parse_decimal(s) -> Decimal<S> fallible(ParseError)` (read at the scale of the place the result flows into — `let p: Decimal<18> = std::str::parse_decimal(s) or raise;` — and at 9, plain `Decimal`, where nothing gives one; a nonzero digit past that scale's last place is a parse failure, like `range_parse_decimal`'s, where the digits used to be truncated in silence — see spec/types.md § Decimal<S>; zeros past it are accepted); predicate siblings `can_parse_int` / `can_parse_float` (`can_parse_decimal` is NOT dispatched — listed here historically; implement or drop, tracked in notes/typecheck-m3.md); range-bounded variants `range_eq(json, start, end_exclusive, expected) -> Bool` / `range_parse_int(json, start, end_exclusive) -> Int fallible(ParseError)` / `range_parse_decimal(json, start, end_exclusive) -> Decimal<S> fallible(ParseError)` (the scale as `parse_decimal`'s) (2026-05-26 — operate on byte ranges within an existing `String` without materializing a substring, paired with `std::json::iter_find_*_range` for allocation-free JSON walks); `byte_at_unchecked(s, i) -> Int` (2026-05-26 — direct byte access at offset i with NO bounds check; caller must guarantee 0 ≤ i < len(s); used by stdlib scan helpers (JSON walkers) where the bound is externally known and a per-access strlen / bytes_from_string would tank perf; misuse → UB); **the `ByteView` byte-scanning surface (GH #720, 2026-09-19): `bytes_view(s) -> ByteView` (one strlen, kept in the view's `n` field), `byte_at(v, i) -> Int` (the byte 0..255, or -1 outside `[0, n)`), `slice(v, lo, hi) -> String` (clamped into `[0, n]`, no strlen), `find(v, sub, start) -> Int` (`index_of_from` over the view: the first `sub` at or after `start`, the index into the text, or -1, at the cost of the distance to the hit), and the length-taking primitives `range_copy(s, n, lo, hi) -> String` and `range_index_of(s, n, sub, start) -> Int` they are built on** — the SAFE linear-time way to walk a large String a byte at a time; see [§ Linear byte scanning](#linear-byte-scanning--the-string-cost-model) for the cost model and why the obvious loop is quadratic; `index_of`, **`index_of_from(s, sub, start) -> Int`** (2026-10-12, downstream handoff) — `index_of` from byte `start` on, the index into `s` (not into the tail), so a scanner passes `hit + len(sub)` straight back as the next start instead of slicing the tail and adding the offset back; a negative `start` is 0, a `start` past the end finds nothing (`-1`), and an empty `sub` is found at `start` itself while `start <= len(s)`; it measures `s` on every call, so a scan over a long text uses the view's `find`, `lower` / `upper`, `trim`, `substring(s, lo, hi)`, `replace`, `repeat`, `pad_left` / `pad_right`, `from_bytes`, `clone(v) -> String` (deep-copy a `StringView` to an owned blob; identity on a `String` for generic callers); `builder_new() -> Bytes` / `builder_append(b: Bytes, s: String) -> Bytes` / `builder_len(b: Bytes) -> Int` / `builder_finish(b: Bytes) -> String` (String-builder primitives, the builder carried as a `Bytes` handle and `builder_append` handing back the same one — for binary-safe accumulator use `std::bytes::BytesBuilder`); **the everyday predicates `contains` / `starts_with` / `ends_with` -> Bool** (2026-08-03 — the first two existed in the runtime with `memory(read)` attributes long before they were reachable from Hale; `ends_with` is new, and its absence is why the trio was unusable as a set); **`split_into(s, sep, target: @form(vec))`** (2026-08-03 — writes into caller-supplied storage rather than returning a sequence, following `text::tokenize_words_into`, because Hale has no sequence VALUE to return: arrays are fixed-size and growable collections are locus-owned. That is also the allocation-visible shape — the caller owns the storage, so the cost lands in the caller's budget and a `@hot` handler can reuse one vec. Empty fields are preserved: `"a,,b,"` is four); **`join(v: @form(vec) of String, sep) -> String`** (2026-08-03 — note this RETURNS where split writes: a String is already a value, so joining never meets the sequence question. One arena allocation sized in a first pass, not repeated concatenation); **UTF-8 accessors `cp_count(s)` / `cp_at(s, byte_off)` / `cp_size(s, byte_off)`** (2026-08-03 — `String` stays byte-oriented; these let a caller walk code points deliberately rather than pretending bytes are characters. Invalid UTF-8 (a sequence the end of the string cuts short among it), a byte offset landing mid-sequence, and an offset at or past the end (or negative) all yield -1 rather than U+FFFD, so corruption cannot be mistaken for content; no byte past the terminator is read. A String carries no length, so `cp_at` / `cp_size` find that the offset is inside the string by looking for the terminator before it, which costs the offset (one `memchr`): a walk by code point over a long String is quadratic, and the view forms **`view_cp_at(v, i)` / `view_cp_size(v, i)`** answer the same in O(1), bounded by the view's `n`, so a walk over a view is linear. Normalization, case folding beyond ASCII, grapheme segmentation and locale collation are each a separate commitment with megabytes of tables against the wasm target, and are deliberately NOT provided) | `lotus_str_*` C runtime primitives |
| `std::bytes` | `at(b, i) -> Int fallible(IndexError)`, `slice(b, lo, hi) -> Bytes`, `from_string(s) -> Bytes`, `from_int(v) -> Bytes`, `concat(a, b) -> Bytes`, `clone(v) -> Bytes` (deep-copy a view to an owned blob). **Word-scan + masked-XOR** (2026-06-13, fast-protocol-I/O #4): `find_byte(b, off, needle) -> Int` returns the first index `>= off` whose byte equals `needle` (low 8 bits) or `-1` (non-fallible; `memchr` word-at-a-time scan — the length/delimiter-framing primitive for HTTP CRLF etc.). `read_*` / `at` / `find_byte` also accept a **`BytesMut`** raw `{ptr,len}` window (a `MirrorRing.readable()` window or a `Topic.write` slot) — read directly via their `_raw` siblings (length is the window length, no `[i64 len]` prefix), so a mirror-ring parse is zero-copy. `BytesBuilder.xor_mask(src: Bytes, key: Int)` (and its primitive `std::bytes::builder::__xor_mask_into(handle: Int, src: Bytes, key: Int) -> Int`) appends `src` XOR'd with a repeating 4-byte key (`masked[i] = src[i] ^ key[i % 4]`, key bytes packed little-endian) in one reserve + word-at-a-time pass — the WebSocket masking primitive, replacing a per-byte `from_int` + `append` loop. **Binary-pack readers** (2026-06-06, shm-ring-interop Proposal A): `read_u8` / `read_u16_{le,be}` / `read_u32_{le,be}` / `read_u64_{le,be}` and the signed `read_i8` / `read_i16_{le,be}` / `read_i32_{le,be}` / `read_i64_{le,be}` (sign-extended), each `(b, off) -> Int fallible(IndexError)`; plus `read_f32_le` / `read_f64_{le,be}` `-> Float fallible(IndexError)`. Fixed-width scalar reads at a byte offset, bounds-checked (`[off, off+width)` past the buffer raises `IndexError { kind: "out_of_bounds", index: off, len }`, same error as `at`). Endianness is explicit (`_le` is the x86-native common case); a `u64` with the top bit set wraps to a negative `Int` (i64). **Binary-pack writers** (2026-06-08, shm-ring-interop A1): the mirror `write_u8` / `write_u16_{le,be}` / `write_u32_{le,be}` / `write_u64_{le,be}`, signed `write_i8` / `write_i16_{le,be}` / `write_i32_{le,be}` / `write_i64_{le,be}`, and `write_f32_le` / `write_f64_{le,be}`, each `(buf, off, val) -> Int fallible(IndexError)` — a fixed-width scalar write at a byte offset into a **`BytesMut`** raw window (a `Topic.write` slot or a `MirrorRing.writable()` window), bounds-checked identically to the readers (`[off, off+width)` past the window raises `IndexError`), returning the offset past the write. These back the `Topic.write(max) { … }` zero-copy ring producer and the `repr:`-tagged `Type::set_field`. **The `BytesBuilder` primitives** (`std::bytes::builder::__*`, called by `bytes_builder.hl`; the handle `h` is an `Int`): `__new(cap: Int) -> Int`, `__append(h, chunk: Bytes) -> Int`, `__append_str(h, s: String) -> Int`, `__append_slice(h, src: Bytes, lo: Int, hi: Int) -> Int`, `__append_scalar(h, v: Int, width: Int, big_endian: Int) -> Int`, `__append_f32` / `__append_f64(h, v: Float, big_endian: Int) -> Int`, `__append_pad(h, to_align: Int) -> Int`, `__len(h) -> Int`, `__clear(h)` / `__free(h)` / `__shift_front(h, n: Int)` (each `-> Int`, the handle back), `__finish(h)` / `__snapshot(h) -> Bytes`, `__view(h) -> BytesView` and `__text_view(h) -> StringView`; `std::bytes::__is_alloc_fail(b: Bytes) -> Int` answers `1` for the allocation-failure sentinel. Growing-buffer accumulator surface lives on the `BytesBuilder` locus — see [§ Builders vs Bytes](#builders-vs-bytes--the-recv-loop-pattern) | `lotus_bytes_*` C runtime primitives |
| `std::regex` | `matches(pattern, text) -> Bool` (full match), `find(pattern, text) -> Int` (leftmost byte offset, `-1` when absent), `valid(pattern) -> Bool`. Syntax: literals, `.`, `*`, `+`, `?`, `|`, grouping, character classes with ranges and negation, `\` escapes. **No backreferences, no lookaround** — the engine class is forced, not chosen: backtracking's exponential worst case cannot be bounded, so it is incompatible with `@budget` and `@hot`, and a linear-time Thompson NFA is the only option. Classified PURE; the match path allocates nothing beyond fixed state lists sized from the pattern, so a match is countable against a budget. `valid` exists so a malformed pattern is reported rather than silently matching nothing | `lotus_regex_*` C runtime (Thompson NFA over a state set) |
| `std::hex` | Base16 text of bytes (2026-10-12, downstream handoff). `encode(b: Bytes) -> String` — two lowercase digits per byte, nothing between them (`b"\x01\xab"` is `"01ab"`; empty is `""`); `decode(s: String) -> Bytes fallible(ParseError)` — two digits a byte, either case; an odd length or a byte that is not a hex digit is `ParseError { kind: "hex", input: s }` (no whitespace, no `0x` prefix); `digit(c: Int) -> Int` — the value of one digit byte (`'0'`–`'9'`, `'a'`–`'f'`, `'A'`–`'F'`), `-1` for any other byte. The stdlib's own readers (`std::json`'s `\uHHHH`, the api payload checker, `std::secret`'s `fingerprint`) use these; there is no private copy of the table. | `runtime/stdlib/hex.hl` |
| `std::url` | Percent-encoding, RFC 3986 § 2.1 (2026-10-12, downstream handoff). `decode(s: String) -> String` — each `%XX` (two hex digits, either case) becomes the byte it names; **total**, the WHATWG URL standard's percent-decode: a `%` not followed by two hex digits stays as it is, never an error; `%00` stays as it is too (a `String` ends at its first NUL, so decoding it would cut the value short in silence); `+` stays a `+`. `decode_query(s: String) -> String` — `decode`, and `+` is a space (the `application/x-www-form-urlencoded` rule, which holds in a query string and nowhere else). `encode(s: String) -> String` — every byte but the unreserved ones (`A`–`Z` `a`–`z` `0`–`9` `-` `.` `_` `~`) written `%XX`, uppercase, so the result is safe as a path segment, a query key or a query value (a space is `%20`). `std::http::path_param` / `query_param` decode through these. | `runtime/stdlib/url.hl` |
| `std::text` | `md_to_html(md: String) -> String`, `base64::encode` / `base64::decode` / `base64::url_encode` (RFC 4648 §5 URL-safe, unpadded — for JWT/JWS, OAuth, webhooks), `Sink` interface + `StdoutSink` / `StringSink` / `FileSink` loci, byte-class predicates (`is_alpha` / `is_digit` / `is_alnum` / `is_whitespace` / `is_word_char`), `tokenize_words_into(s, target_vec)` | `runtime/stdlib/text.hl` + C runtime |
| `std::io::fs` | `read_file(path) -> String`, `write_file(path, s) -> ()`, `write_file_append(path, s) -> ()` (GH #535 corrected the earlier "returns bytes appended" and the `Int` success type the checker advertised for the `or` form; the bare call that returned an Int status, 0 ok / -1 error, is refused like every bare fallible call since v0.22.0, GH #738), `read_bytes -> Bytes`, `write_bytes(path, b: Bytes) -> ()` (binary-safe write — the companion `std::compress`/`std::tar` output needs; `write_file` ships String content via strlen and truncates at the first NUL), `file_size -> Int`, `mkdir`, `rename`, `unlink`, `mktemp(prefix, suffix) -> String` (creates the file `prefix` + six random characters + `suffix` with mode 0600 through `mkstemps(3)`, closes it and returns its path, which the caller removes; a directory goes in the prefix, `mktemp("/tmp/run-", ".log")`, and a failure's `IoError.path` is the template), `list_dir_count -> Int`, `list_dir_at(path, i) -> String` — all `fallible(IoError)`. (`list_dir` is listed in older notes but not dispatched — use list_dir_count + list_dir_at.) `file_exists(path) -> Bool` is a predicate (non-fallible). One-shot path-call surface: each call opens, does the op, closes. For held-open shapes use `std::io::file::File`. | `lotus_fs_*` C runtime primitives |
| `std::compress` | GH #254 (2026-07-27). One-shot compression over `Bytes`, all `fallible(IoError)`: `gzip(b) -> Bytes` (gzip container, zlib level 6), `gunzip(b) -> Bytes` (accepts gzip OR bare zlib streams — windowBits 15+32 auto-detect), `zstd(b) -> Bytes` (zstd frame, level 3), `unzstd(b) -> Bytes`. Error kinds: `invalid` (corrupt / truncated / unsupported input, incl. a zstd frame with no declared content size — streaming frames are the v2 path), `not_found` (**zstd only**: libzstd not installed — see below), plus EFBIG-backed kinds when decompressed output would exceed the **1 GiB one-shot guard** (zip-bomb / corrupt-length protection; streaming is the path for bigger). Linkage: gzip rides zlib (`-lz`, universally present — distro zlib1g / the macOS SDK); zstd is **dlopen'd at first use** (`libzstd.so.1` / Homebrew dylib paths on macOS), so neither `hale build` nor emitted binaries carry a link-time libzstd dependency — a program that never calls the zstd pair runs anywhere, and one that does gets a clean fallible `not_found` on machines without the library. Compression levels are fixed at v1 (no level parameter yet). | `lotus_compress_*` in `runtime/lotus_compress.c` (own TU, same rationale as lotus_tls.c) |
| `std::tar` | GH #254 (2026-07-27). One-shot ustar archives over `Bytes`, all `fallible(IoError)` (kind `invalid` = malformed archive / index out of range). Read side is indexed (the list-then-extract shape; each call re-walks headers — O(n), fine for tool-sized archives): `entries(b) -> Int`, `entry_name(b, i) -> String` (ustar prefix field honored: `prefix/name`), `entry_size(b, i) -> Int`, `entry_type(b, i) -> String` (`file` / `dir` / `link` / `symlink` / `other`), `entry_data(b, i) -> Bytes`. Write side is append-style: start from empty Bytes, `pack(archive, name, data) -> Bytes` appends a regular file (mode 644), `pack_dir(archive, name) -> Bytes` a directory (mode 755), `finish(archive) -> Bytes` appends the two terminating zero blocks. Names over 100 bytes are split into the ustar prefix field (≤ 155 + `/` + ≤ 100; ENAMETOOLONG otherwise); mtime is written as 0 for reproducible archives; octal sizes only (no GNU base-256). Compose with `std::compress::gzip` for `.tar.gz` and `std::io::fs::write_bytes` to ship to disk — the output is accepted by system `tar` (pinned by `compress_tar.rs`). | tar section of `runtime/lotus_compress.c` |
| `std::io::file` | `File` locus (held-open fd with auto-dissolve close). `open(path, mode) -> File fallible(IoError)`; `read_line(f) -> String` (returns "" at EOF or error — pair with `at_eof`); `at_eof(f) -> Bool`; `write_bytes(f, b)`, `write_line(f, s)`, `seek(f, offset)` all `fallible(IoError)`. Mode strings `"r"` / `"w"` / `"a"` / `"r+"` / `"w+"`. Returned Strings live in the bus payload arena. The locus's primitives take the fd as an `Int`: `__open(path: String, mode: String) -> Int`, `__seek(fd, offset: Int) -> ()` and `__write_bytes(fd, b: Bytes) -> ()`, each `fallible(IoError)`; `__read_line(fd) -> String`, `__at_eof(fd) -> Bool` and `__close(fd) -> Int` (`0` or `-1`). | `lotus_file_*` C primitives + `runtime/stdlib/file.hl` |
| `std::io::stdin` | `read_line() -> String`, `read_line_status() -> Int` (status `-1` = EOF/IO error; `0` = OK including empty-line). `read_byte(timeout_ms: Int) -> Int` — `poll` up to `timeout_ms` then a 1-byte `read`; returns `0..255` = the byte, `-1` = timeout, `-2` = EOF/error (sentinel; a timeout is a control outcome, not an error). `timeout_ms <= 0` is a pure poll. For interactive raw-mode input paired with `std::term::RawMode`. | POSIX `getline` / `poll`+`read` + payload-arena copy |
| `std::io::stdout` | `write_bytes(s: String) -> Int` — `fflush(stdout)` then a raw `write(1, ...)` that bypasses the prelude's `_IOLBF` line-buffering, so a multi-line frame isn't flushed per newline. Returns bytes written, `-1` on error (sentinel). The `fflush` keeps output ordered consistently with buffered `println`. | `lotus_term_write_stdout` C primitive |
| `std::term` | `is_tty(fd: Int) -> Bool` (POSIX `isatty` — probe whether an fd is a terminal, e.g. so a logger can decide on color without vendoring an FFI shim). `RawMode` — an RAII guard locus: `let r = std::term::RawMode { };` puts stdin in raw mode (no echo/canon/ISIG — unbuffered byte input, Ctrl-C as byte `0x03`) at birth and restores the saved termios at dissolve (scope exit). Soft-fails (runs unstyled) when stdin isn't a tty. `RawMode` also registers a runtime `atexit` termios restore, so a panic / unhandled error (which `exit()` through `atexit`) restores the terminal too — no FFI glue needed for terminal hygiene. `size() -> TermSize` (record `{ cols: Int; rows: Int }` via `ioctl(TIOCGWINSZ)`; `{ 0, 0 }` when stdout isn't a tty — poll per frame, no SIGWINCH handling). POSIX-only; non-tty / non-POSIX degrades soft. The primitives `term.hl` calls: `__raw_enable() -> Int` / `__raw_disable() -> Int` (`1` ok / `0` failed) and `__size_packed() -> Int` (`(cols << 16) | rows`). | `lotus_term_*` C primitives + `runtime/stdlib/term.hl` |
| `std::io::tcp` | `Listener` locus (a listener of the TCP connection substrate calling an `on_connection` callback per connection: § The TCP connection substrate), `Stream` locus with `send` / `send_bytes` (Unit success) / `recv` / `recv_bytes` — all `fallible(IoError)` since #209 (EOF and recv-timeout return empty, only genuine errors fail; see § Stream methods below), `recv_into(fd, buf: std::bytes::BytesBuilder, max_bytes) -> Int` (caller-provided builder destination). Path-calls `listen_socket`, `connect`, `accept_one` are `fallible(IoError)`. **IPv4 and IPv6** (2026-10-12, downstream handoff; it was IPv4 only): a host is an IPv4 literal, an IPv6 literal written bare (`::1`) or bracketed (`[::1]`, as a URL or a listener name writes it; an IPv6 literal may carry a `%zone`), or, for a dial, a name: `connect`, `connect_wait` and `connect_within` resolve a name with `getaddrinfo(AF_UNSPEC)` and try the addresses it answers in the resolver's order, the next after one that refuses or cannot be reached, every attempt within the dial's one deadline (a deadline that passes ends the dial, kind `timeout`), so a `localhost` answering `::1` first still reaches a server on `127.0.0.1`; it is refused only when every address refused. `listen_socket` / `__listen_socket` take a literal of either family (`::` is the IPv6 wildcard, dual-stack where the host's default is) and `__local_port` reads the port of either. A substrate listener on an IPv6 host goes by `[<address>]:<bound port>` (`[::1]:8080`), bracketed however its host was written, and `stop(name)` finds it by that name. The bus's own `tcp://` and `udp://` transports remain IPv4. **A refused connect is an answer (GH #1030):** `connect` makes one attempt, and a connect nothing listens for fails at once with kind `connection_refused` (it used to retry for ~1 s, which made a client of a store that was down pay a second per dial). A caller racing its peer's `listen()` asks for the wait: `connect_wait(host, port, wait: Duration) -> Int fallible(IoError)` retries a refused connect every 5 ms until `wait` has passed, or the process drain begins, then fails with `connection_refused`; any other error ends it at once. `std::io::tls::connect` dials through `connect`, so it fails fast too. **A connect deadline (2026-10-12, downstream handoff):** a host that never answers — a blackholed address, a SYN nobody acknowledges — is neither a refusal nor an error, and `connect` waits on it until the kernel gives up (about two minutes of SYN retransmits). `connect_within(host, port, timeout: Duration) -> Int fallible(IoError)` makes one attempt bounded by `timeout`: a refusal still answers at once (`connection_refused`), and a connect neither accepted nor refused when `timeout` passes fails with kind `timeout` (errno `ETIMEDOUT`, the IoError's `path` the host) and no stderr line. `connect_wait`'s `wait` bounds the whole dial the same way: a pending connect when it passes fails `timeout`, where it used to hold the caller past its wait. A `timeout` (or `wait`) of `0` is no deadline: `connect_within(h, p, 0)` is `connect`. The deadline covers the connect, not name resolution: a non-numeric host is resolved with `getaddrinfo(3)` on the calling thread first. On a `where async_io` pool every dial — `connect`, `connect_wait`, `connect_within` and `__connect` — is a non-blocking `connect(2)` whose wait parks the coroutine until the socket is writable or the deadline passes, and `connect_wait`'s 5 ms backoff is a timer park, so a pending dial holds no other cell of the pool; elsewhere the deadline is a `poll(2)` on the thread, and with no deadline the classic blocking `connect(2)`. The connected fd is handed back in the blocking mode it was created in. The bus transports' own CONNECT role (`unix://`, `LOTUS_BUS_CONFIG`) keeps its ~1 s retry, for a peer process starting at the same moment. **Exclusive bind (2026-07-15, downstream handoff item 4):** `listen_socket` sets `SO_REUSEADDR` (so a restart can rebind a port still lingering in `TIME_WAIT`) but deliberately **not** `SO_REUSEPORT` — so a second live process binding the same host:port fails with `EADDRINUSE` instead of silently sharing the port with the kernel round-robining connections between two divergent-state servers (a split-brain foot-gun). Intentional multi-process load balancing across a shared port is not the default; it would need an explicit opt-in if a workload ever wants it. Send/recv timeouts: `set_recv_timeout(fd, d: Duration) -> () fallible(IoError)` wraps `SO_RCVTIMEO`; `set_send_timeout(fd, d: Duration)` wraps `SO_SNDTIMEO`. `d = 0` disables (blocking default). The send timeout holds on a non-blocking socket too (2026-10-12, PR #1538 review): the kernel applies `SO_SNDTIMEO` only to a send that blocks, so a send that waits on a full buffer — a park on a `where async_io` pool, a `poll(2)` in a classic context writing a non-blocking fd — reads it back and fails once that long passes with no byte written (EAGAIN, kind `would_block`, as the blocking send's expiry fails), where it used to wait forever on a peer that stopped reading. After `set_recv_timeout(fd, 100ms)` a `recv_bytes` on a quiet socket returns ~100ms instead of blocking forever — unblocks recv loops that need periodic silence-detection / heartbeat / watchdog work. **Sub-millisecond timeouts aren't real:** `SO_RCVTIMEO` is rounded to the kernel's scheduling tick, so a `set_recv_timeout(fd, 50us)` parses but an idle recv still returns after ~1–1.5ms, not 50µs. Poll loops that need finer cadence must amortize their probes rather than lean on a sub-ms deadline (the async_io timed park inherits the same floor — its deadline comes from the same `SO_RCVTIMEO` value). Shares the `sock_set_timeout_ns` helper with the udp siblings (P4). **async_io parking (2026-07-14, downstream handoff):** on a `where async_io` pool, `recv_into` / `recv_stamped_into` / `recv_bytes` / `Stream.recv` park the coroutine on EPOLLIN until the fd is readable or the fd's `set_recv_timeout` deadline expires — `-2` (or `recv_bytes`'s empty return) means the *deadline expired*, never an instant would-block; with no timeout set they park indefinitely. The park deadline mirrors the socket's `SO_RCVTIMEO` (read back via `getsockopt` at call entry), so timeout semantics are identical on every pool type and liveness machinery built on the `-2` sentinel (pond/websocket ping-pong) composes unchanged. **`Stream.release_fd() -> Int`** (2026-07-19): flips `owns_fd` off (dissolve becomes a no-op) and returns the fd — the ownership hand-off primitive behind `std::http` takeover and any live-connection transfer to a longer-lived owner. **`send_fd(fd, b: Bytes) -> () fallible(IoError)`** (2026-07-27, hale-bun handoff item 4b): public raw-fd send — the write-side takeover companion to `close_fd` / `recv_into` / `set_recv_timeout`, for a handler that keeps a released fd without adopting `Stream`'s close-at-scope-exit lifecycle. Same contract as `Stream.send_bytes` (Unit success; genuine I/O errors fail); previously the only persistent-write path was the internal `__send_bytes`. Nagle control: `set_nodelay(fd, on: Bool) -> () fallible(IoError)` sets `TCP_NODELAY` on a connected fd — `on = true` disables Nagle so small writes hit the wire immediately instead of waiting up to ~40ms to coalesce, the first thing a latency-sensitive request/response or market-data socket reaches for. Previously impossible from Hale (`std::io::tcp` had no setsockopt surface); the generic udp `set_option_*` / `get_option_int` work on any fd if a less-common TCP option is needed. Kernel RX timestamps: `set_rx_timestamps(fd, on: Bool) -> () fallible(IoError)` enables `SO_TIMESTAMPNS` once at setup; `recv_stamped_into(fd, buf: std::bytes::BytesBuilder, max_bytes) -> Int` is the timestamped sibling of `recv_into` (identical `>0` / `0` EOF / `-1` fatal / `-2` retryable contract) that issues one `recvmsg(2)` capturing the kernel's wire-arrival timestamp alongside the bytes — no extra syscall on the hot path. Read the stamps with `last_recv_kernel_ns() -> Int` / `last_recv_user_ns() -> Int` immediately after the call (errno-style thread-local, same idiom as `udp::last_source_*`). `last_recv_user_ns` is a `CLOCK_REALTIME` stamp taken at `recvmsg` return; `last_recv_kernel_ns` is the kernel's `SCM_TIMESTAMPNS` value, or **`0` when the platform/path delivered none** — notably loopback TCP and any NIC without RX software/hardware timestamping (it never returns garbage, so `>= 0` is the contract). The cmsg is parsed defensively (first control message only, length-validated, no `CMSG_NXTHDR` walk — some libcs loop forever on a zero-length cmsg). The `Listener` / `Stream` loci's primitives take fds as `Int`s and answer status the sentinel way (`-1` on failure, the cause in `__last_io_status() -> Int` / `__io_error_kind(errno: Int) -> String`, and its message in `__io_error_detail(errno: Int) -> String`, the `IoError.detail` the `Stream` bodies fill): `__listen_socket(host: String, port: Int) -> Int`, `__accept_one(listen_fd) -> Int` (the connection's fd; `-1` when the wait ended on purpose — the socket shut by `stop` / `shutdown()`, including a Unix one on an `async_io` pool, which answers a non-blocking accept `EAGAIN` once shut — or the socket is no listener; `-2` when the failure ended one connection and not the listener: out of descriptors or buffers, or a network error the kernel passed up from a connection that failed before it was taken), `__connect(host: String, port: Int) -> Int`, `__recv(fd, max_bytes: Int) -> String`, `__recv_bytes(fd, max_bytes: Int) -> Bytes`, `__send(fd, msg: String) -> Int`, `__send_bytes(fd, b: Bytes) -> Int`, `__send_bytes_until(fd, b: Bytes, until_ns: Int) -> Int` (`__send_bytes` that fails, EAGAIN, once the absolute monotonic instant `until_ns` passes however the peer drains it; `0` is none; the HTTP client's call deadline), `__set_recv_timeout_ns(fd, ns: Int) -> Int`, `__close_fd(fd) -> Int` and `__shutdown_listen_socket(fd) -> Int`; `__parks() -> Int` is `1` where a blocking socket call parks the calling coroutine (an `async_io` pool's worker) and `0` where it holds the thread, and `__on_pool() -> Int` is `1` on any cooperative pool's worker (where a `yield` in a loop that never returns runs none of the pool's queued cells), which is how the TCP connection substrate picks its connection shape; `__name_listener(name, fd) -> Int` and `__stop_listener(name) -> Int` keep the process's listeners by name, behind `stop(name: String) -> Bool` (§ Stopping a server). The substrate's loci and handle — `Acceptor`, `Connections`, `Accepted`, `ConnHandler`, and the `Listener` built on them — are in § The TCP connection substrate. `__listen_socket` / `listen_socket` listen with the kernel's largest backlog (`SOMAXCONN`, clamped to `net.core.somaxconn`; it was 16, and a burst of clients larger than that lost SYNs and paid TCP's 1 s retransmit). | `lotus_tcp_*` C primitives |
| `std::io::h2` | GH #1417 (R7). An HTTP/2 server over a parked socket, built on nghttp2 (MIT, vendored under `crates/hale-codegen/runtime/third_party/nghttp2`) compiled into the runtime by its cc step and linked statically into a program that reaches it. The library is sans-I/O and owns no thread, socket or blocking call: `runtime/lotus_h2.c` is bytes in (`__feed`), bytes out (`__drain`), a queue of events (`__poll`, `__ev_stream`, `__ev_code`, `__ev_bytes`: 1 a request opened with its headers, 2 data, 3 end, 4 reset, 5 GOAWAY, 6 stream closed) and `__respond` (headers, body, trailers), `__reset`, `__goaway`, `__alive`, `__open`, `__close`, all on an `Int` handle, all computation on the calling thread. `Listener` and `Conn` (`io_h2.hl`, appended to a program that serves over `grpc::Rpc` or names one of them) accept on the `async_io` pool and own a socket and a session each — the `Listener` is a listener of the TCP connection substrate (§ The TCP connection substrate), named `host:<bound port>`, whose consumer starts a `Conn` per accepted connection as a flow child on its pool, so a connection nothing takes is closed and `std::io::tcp::stop(name)` closes it while its connections are served on: a connection feeds each read to its session, writes what it drains and raises the session's stream events (`Event`, subject `__api.h2.event`) for a transport, which asks it to answer or reset a stream, or say GOAWAY (`Cmd`, `__api.h2.cmd`). SETTINGS, PING and the flow-control windows are the library's; a connection ends alone on an EOF, a failed write, a protocol error or the peer's GOAWAY once its streams are done. The compiler's own expansion places the listener by its stdlib name `__StdIoH2Listener`; a param spelled `std::io::h2::Listener` and placed on a pool does not have its `run()` posted to that pool (observed in R7, not changed there). Refused on wasm32. |
| `std::io::unix` | GH #1106. AF_UNIX stream sockets with the fd-level shape `std::io::tcp` has: `listen_socket(path) -> Int fallible(IoError)` unlinks a stale socket file, binds and listens (with the kernel's largest backlog, `SOMAXCONN`, as a TCP listener; it was 16, and a burst of clients larger than that waited in `connect(2)`); `std::io::tcp::Acceptor { path }` is the substrate's listener on one (§ The TCP connection substrate); `__release_path(path, fd) -> Int` removes the socket file the listener on `fd` bound at `path` only while the path still names that file (its device and inode recorded at bind, and any record of the path retired by a later bind of it in the process; 1 when removed), called before `fd` is closed — a shut listener's path may meanwhile have been taken as stale by a replacement (a shut Unix socket refuses connects, and the stale check steals it rather than refuse a path whose listener has stopped), whose file the old listener's cleanup must not delete; `connect(path) -> Int fallible(IoError)` is one attempt (a missing or refusing socket fails at once); `connect_wait(path, wait: Duration) -> Int fallible(IoError)` retries a missing or refusing socket until `wait` has passed, for a caller racing its peer's listen. The fd is address-family agnostic: wrap it in `std::io::tcp::Stream { conn_fd }`, and accept, recv, send, shutdown and close are the tcp primitives. The api binding's socket side is built on these (`spec/semantics.md` § "The api binding"). `peer_uid(fd)`, `peer_gid(fd)`, `peer_pid(fd) -> Int` (GH #1108) answer the peer's credentials on a connected socket, -1 when the kernel will not say; the api binding's caller is built from them. `user_id(name) -> Int`, `group_id(name) -> Int` (GH #1109) resolve a name per the host's account database (`getpwnam` / `getgrnam`), -1 when it has none — the static role table's `user:` / `group:` spellings stand on these, once at birth; `peer_groups_count(fd) -> Int`, `peer_group_at(fd, i) -> Int` are the supplementary groups the kernel holds for a connected peer (`SO_PEERGROUPS` on Linux, the first 256; 0 and -1 elsewhere), read once per connection so no account lookup runs per request. |
| `std::api` | GH #1108. `Principal { mode: String = "local"; name: String = "local"; uid: Int = -1; gid: Int = -1; pid: Int = -1; groups: String = ""; via: String = ""; }` (`groups`, GH #1109: the supplementary groups the kernel holds for the connection, comma-separated; not on the receipt) is who is calling as the exposure established it (`local` for a call that never crossed a transport, `unix` for a peer on a `unix::Rpc` socket named `uid:<n>`, `bearer` for a token on an HTTP, MCP or hub transport, named as the program's `BearerSource` answers) — the shape DNA carried by hand as `principal_mode` / `principal_name`. `Context { caller: Principal; role: String = ""; request_id: Int = 0; via: String = "local"; }` is what a served handler that declares a second parameter receives (`spec/api.md` § Serving); `local_context() -> Context` is the one a call that never crossed a transport carries. Both are ordinary structs: constructible in a test, forwardable in a payload. GH #1109: `interface RoleSource { fn holds(p: Principal, r: String) -> Bool; }` is the membership source an exposure consults at its `requires` — the direct question only; the exposure walks `includes` itself — handed to the transport instance as `roles: self.<param>`; `StaticRoles { table: String; known: String }` is the stdlib's implementation, its table `role=member,member;role=` passed as the `table` param and overridden by `LOTUS_API_ROLES`, `known` the roles the program declares; at birth a table naming another role, or a member outside `uid:<n>`, `gid:<n>`, `user:<name>` / `group:<name>` / `bearer:<name>` (a name: letters, digits, `.`, `_`, `-`, `@`) and `*`, refuses to start, saying which entry. `*` matches any authenticated caller: a socket peer (uid ≥ 0) or a bearer principal its source named (`mode: "bearer"`, a non-empty `name`). `bearer:<name>` matches a bearer principal whose `name` is exactly `<name>` (never resolved: the name is the bearer source's) and never a socket peer; `uid:` / `gid:` / `user:` / `group:` match socket peers and never a bearer principal. A `gid:` member matches the peer's primary group or one of `Principal.groups`; `user:` / `group:` resolve once at birth per the host's account database (an unknown name is reported to stderr and holds nobody); an unauthenticated principal (a socket peer with uid -1, a bearer principal with an empty `name`) holds nothing. GH #1417 (`spec/api.md`, the contract; R2a ships the runtime, R2b to R6 the transports): `api::serve(Surface, transport, as: "name", receivers: { … }, bound: N, on_full: refuse)` puts a surface on a transport instance; it is the literal of an exposure param (`web: api::Exposure = api::serve(…);`, GH #1565), the handle the locus that holds it waits on and stops. R2a: `Rpc` is the transport interface (`attach`, `frame`, `reply`, `close_connection`, `stop_listening`; the stdlib's are `std::api::test::Rpc`, the in-process fixture, and (R2b) `std::api::unix::Rpc`, a Unix stream socket carrying one JSON object per line whose principal is the peer's kernel credentials, served from any locus the main locus holds (its listener, `std::api::unix::Listener`, is a param the serve expansion adds to the main locus, placed by the exposure's row or its server's on an `async_io` pool, else on the `async_io` pool `__api_unix`); `std::api::ws::Hub` and `std::api::udp::Hub` (R5, `hale_stdlib::API_HUB_SOURCE`, appended after the runtime to a program that binds a topic to a hub: the hub is a main-locus param whose address is bound at birth, its listener a param the compiler adds, placed on the `async_io` pool `__api_ws`; a topic bound to it is a stream, an adapter binding with the topic's JSON codec; it admits a subscription against the row's `requires`, delivers from a queue of `bound` frames shedding under `on_full`, ends a subscription at the credential's expiry and at a role-source revision, sends `closed` at `stop()`, answers `GET /.description`, and may be the transport a surface is served over; its methods are `events()`, `subscribers()`, `connections()`, `announce(revision)` and `stop()`); `http::Rpc` (R3) and `mcp::Rpc` (R6: JSON-RPC over `POST /mcp`, built on the HTTP listener) are shipped too, and `grpc::Rpc` (R7, `hale_stdlib::API_GRPC_SOURCE`: unary gRPC calls over the HTTP/2 server of `std::io::h2`, `hale_stdlib::API_H2_SOURCE`, whose listener is a main-locus param on the `async_io` pool `__api_grpc`; both are appended only to a program that names `grpc::Rpc`)); `Request { kind; member; bytes; digest; correlation; credential; peer; reason }` is what `frame` returns, `Outcome` (`Result(String)`, `HandlerError(String)`, `Refusal(kind, reason, extra)`, `ServerError`) what `reply` is handed; `Exposure` is the locus the runtime builds per serve site (admission, dispatch, the pending table that completes each request once, `stop()`), `Handle { fn stop(); }` what a serve returns; `Surface` is the rows the exposure reads (generated per surface); `Grants { roles; revision }`, `RevisedRoleSource { fn grants(p) -> Grants; }`, `RevisedStaticRoles` (the static table's pair) and the wire subject `__api.roles.revision` (`Revision { source; revision }`) are a role source's grants-and-revision pair, `ExpiringBearerSource { fn expiry(token) -> Int; }` a bearer credential's expiry (Unix nanoseconds, 0 is none), `ServedContext` is `Context` with the `exposure` and `generation` of a served call (`Context` is unchanged); the runtime is `hale_stdlib::API_RUNTIME_SOURCE`, appended to a program that serves or names it, not part of the bundled stdlib; `test::Rpc` is the fixture transport (`call`, `call_as_peer`, `call_digest`, `describe`, `lose`, `outcome`, `line`, `await_outcome`); `Context`, `Principal`, `RoleSource` and `BearerSource` stay, the two sources named on the transport instance (`principals:`, `roles:`). |
| `std::io::udp` | `bind(host, port) -> Int fallible(IoError)` (`host=""` → INADDR_ANY; an IPv6 literal, bare or bracketed, binds an IPv6 socket — 2026-10-12); `send(fd: Int, host: String, port: Int, msg: String) -> () fallible(IoError)` (the message is text: a `Bytes` is refused; the peer is a literal of the socket's family, an IPv4 peer of an IPv6 socket sent to as v4-mapped, an IPv6 peer of an IPv4 socket `invalid`; `last_source_host()` reads an IPv6 source unbracketed, `::1`, with its zone when it has one — `fe80::1%eth0`, or the interface's index, `fe80::1%7`, when it has no name — so `send` back to it reaches a link-local sender), `recv(fd, max_bytes)`, `recv_into(fd, buf: std::bytes::BytesBuilder, max_bytes)` (`>0` bytes / `0` empty datagram / `-1` fatal / `-2` retryable — a `set_recv_timeout` expiry or async_io park deadline; **2026-07-14 behavior change**: EAGAIN previously fell into `-1` fatal, and on `where async_io` pools `recv_into` now parks until readable or deadline like the tcp sibling), `close(fd) -> Int` (and its primitive `__close(fd: Int) -> Int`). Multicast (2026-05-26 P1): `join_group(fd, group, iface) -> () fallible(IoError)` (iface=`""` → INADDR_ANY), `leave_group(fd, group, iface)`, `set_multicast_ttl(fd, ttl)` (0..255), `set_multicast_loop(fd, enabled: Bool)` (whether the sender receives its own packets), `set_multicast_iface(fd, addr)`. Transparent setsockopt pass-through (P2): `set_option_int(fd, level, name, value)`, `set_option_bool(fd, level, name, enabled)`, `get_option_int(fd, level, name) -> Int` — paired with `std::io::sockopt::<NAME>()` named constants below for the `level` / `name` args. Source-bearing recv + timeouts (2026-05-26 P4): `recv_with_source(fd, max_bytes) -> Bytes fallible(IoError)` populates a thread-local source cache; `last_source_host() -> String` / `last_source_port() -> Int` read the cache from the most-recent recv_with_source on the current thread (errno-style; read immediately after recv). `set_recv_timeout(fd, d: Duration) -> () fallible(IoError)` / `set_send_timeout(fd, d: Duration)` wrap `SO_RCVTIMEO` / `SO_SNDTIMEO` (they take a struct timeval so can't ride `set_option_int`); `d = 0` disables (blocking default). Datagram boundaries preserved. **async_io parking (2026-07-15, downstream handoff item 3):** the Bytes-returning `recv` / `recv_with_source` (like `recv_into` before them) park the coroutine on EPOLLIN on a `where async_io` pool — bounded by the socket's `set_recv_timeout` deadline, or indefinitely when no timeout is set — instead of blocking the single pool worker inside `recvfrom`. This is what lets N reader loci, each parked on its own socket, share one async pool concurrently: previously the first blocking `recv` pinned the worker and every reader queued behind it on the same pool never started (with no timeout, never at all). Off async pools it stays a plain blocking `recvfrom`, and a park-deadline expiry surfaces the same "no datagram" result (NULL → the fallible error path) a blocking `SO_RCVTIMEO` timeout already produced. **`std::io::udp` is the raw-socket primitive, not a bus transport** — its `recv` / `send` calls don't carry the bus's typed-payload-dispatch contract. For UDP-as-bus see the `std::bus` row's `udp://host:port` substrate transport (shipped 2026-05-26): single URL scheme covers IPv4 unicast and multicast, dispatch goes through the same `LOTUS_BUS_CONFIG` route as `unix://`, lossy delivery (publisher-side "sendto returned" durability; subscribers best-effort). **`Reader` handle (2026-07-16) — the ergonomic event-driven ingest default.** `std::io::udp::Reader { addr, port, cap }` bundles a bound socket + a single reused `BytesBuilder` and exposes `next() -> BytesView fallible(IoError)`: it binds lazily on the first call (so a bind failure propagates through `next()`'s fallible channel), clears + refills the buffer in place per call (no per-datagram allocation), and returns a **zero-copy `BytesView`** aliasing the buffer — valid until the next `next()`. On a `where async_io` pool `next()` parks on EPOLLIN (kernel-woken, no busy-poll, no `SO_RCVTIMEO` quantum); on a classic pool it blocks the worker in `recvfrom`. It parks until a datagram arrives, so its only failures are a bind failure (first call) or a fatal recv error — both genuinely exceptional, so `or raise` is the disposition (`or discard` doesn't apply — `next()` yields a value). This is the hand-rolled "bind + BytesBuilder + `recv_into` + `.view()`" fast path baked into one handle so it's the path of least resistance; unlike the allocating `recv` it copies no per-datagram payload (the view aliases the buffer). `dissolve()` closes the socket; the nested `BytesBuilder` frees with it. | `lotus_udp_*` C primitives |
| `std::io::sockopt` | Named-constant getters returning the platform's numeric value for each setsockopt level / name. Use as the `level` / `name` args to `std::io::udp::set_option_*` / `get_option_int`. Each is a zero-arg fn `() -> Int` (`std::io::sockopt::SO_RCVBUF()` etc.) so the value tracks the kernel headers; cross-platform without hardcoding. Shipped: `SOL_SOCKET`, `IPPROTO_IP`, `IPPROTO_IPV6`, `IPPROTO_TCP`, `IPPROTO_UDP`, `SO_REUSEADDR`, `SO_REUSEPORT`, `SO_RCVBUF`, `SO_SNDBUF`, `SO_RCVTIMEO`, `SO_SNDTIMEO`, `SO_BROADCAST`, `SO_KEEPALIVE`, `SO_LINGER`, `SO_PRIORITY`, `SO_BINDTODEVICE`, `IP_TTL`, `IP_TOS`, `IP_MULTICAST_TTL`, `IP_MULTICAST_LOOP`, `IP_MULTICAST_IF`, `IP_ADD_MEMBERSHIP`, `IP_DROP_MEMBERSHIP`, `IP_PKTINFO`. PMTU surface added 2026-05-27: `IP_MTU_DISCOVER` + `IP_PMTUDISC_DONT` / `IP_PMTUDISC_WANT` / `IP_PMTUDISC_DO` / `IP_PMTUDISC_PROBE` (Linux-only; returns -1 on platforms missing the constant — caller can detect and skip). TCP option added 2026-06-13: `TCP_NODELAY` (use with `IPPROTO_TCP` as `level`; or reach for the `std::io::tcp::set_nodelay` convenience). | `lotus_sockopt_<NAME>` C getters |
| `std::io::mirror` | The primitives behind the `std::io::MirrorRing` locus, called by `mirror_ring.hl` (not a user surface): `__new(cap: Int) -> Int` maps the double-mapped ring and returns its handle; `__recv_into(handle: Int, fd: Int, max: Int) -> Int` reads into the free window (`> 0` bytes / `0` / `-1` fatal / `-2` retryable). The cursor primitives `__free`, `__commit`, `__consume`, `__readable`, `__writable`, `__len` and `__capacity` take the handle (and a byte count for `__commit` / `__consume`) and have no signature: the compiler does not count their arguments. | `lotus_mirror_ring_*` C primitives |
| `std::io::tls` | Client-side TLS via system OpenSSL. `connect(host, port) -> Int fallible(IoError)` does the TCP connection + TLS 1.2+ handshake with SNI + system-trust-store cert verification. **Socket upgrade** (2026-07-14): `upgrade(fd: Int, host: String, verify: Bool) -> Int fallible(IoError)` wraps an *already-connected* TCP fd (from `std::io::tcp::connect`) in a client TLS session — the STARTTLS-style path a protocol takes after speaking a plaintext prologue on the socket (e.g. pond/pq sends the pgwire `SSLRequest`, reads the `'S'` byte, then upgrades the same fd). The returned handle is fully interchangeable with `connect`'s across `send_bytes` / `recv_bytes` / `recv_into` / `close` / `set_nodelay` / etc. SNI (`SSL_set_tlsext_host_name`) is **always** sent. `verify = true` authenticates the peer: hostname-checked against the cert via `SSL_set1_host` against the system trust store (identical to `connect`). `verify = false` gives **sslmode=require semantics** — encrypt *without* authenticating the peer (`SSL_VERIFY_NONE` per-connection override, no hostname check), for endpoints whose CA is not in the system trust store (e.g. AWS RDS); SNI is still sent. **FD-ownership asymmetry (important):** `upgrade` does **not** close `fd` on handshake failure — the caller already owned the fd (it came from `tcp::connect`, not from this call), so teardown stays with the caller (`std::io::tcp::close_fd(fd)` in the `or` handler). This is the *opposite* of `connect`, which dials its own fd and therefore closes it on any failure. On success the returned handle owns the fd (`close` closes it), same as `connect`. **A connect deadline (2026-10-12):** `connect_within(host, port, timeout: Duration) -> Int fallible(IoError)` is `connect` under one deadline over the TCP dial (`std::io::tcp::connect_within`) and the handshake: past it the call fails with kind `timeout`, closes the socket it dialed, and writes no stderr line; `timeout = 0` is `connect`. The handshake under a deadline runs on the socket made non-blocking for it (restored after), each `WANT_READ` / `WANT_WRITE` waiting for readiness until the deadline — a park on a `where async_io` pool, a `poll(2)` elsewhere. **Caveats:** pass a *hostname*, not an IP literal, for `host` when `verify = true` — `SSL_set1_host` treats the value as a DNS name for certificate hostname matching, and using an IP address as SNI is discouraged (some servers reject or mismatch it). Do **not** `upgrade` the same fd twice — each returned handle assumes sole ownership of the fd at `close` time, so a double-upgrade produces two handles that both believe they own the fd, leading to a double-close. **STARTTLS over-read:** before calling `upgrade`, read *exactly* the plaintext prologue and no further (pond/pq reads the single `'S'` byte and stops). Any bytes consumed past the prologue are gone from the kernel receive buffer, so the peer's TLS `ServerHello` — which follows immediately on the wire — would be handed to the caller as plaintext and the handshake would then read a truncated, corrupt stream (the classic STARTTLS plaintext-injection footgun). The current design is safe because `upgrade`'s recvmsg BIO and the `tcp::recv_*` primitives share **no** read buffer — every read pulls straight from the socket, so there is nowhere to accidentally buffer post-prologue bytes — but any future read-ahead/buffered `tcp` reader would break this property, so a consumer must keep its prologue reads unbuffered and byte-exact right up to the `upgrade` call. **`Int` is overloaded — a raw fd and a TLS handle are not interchangeable even though both are typed `Int`.** TLS handles are small table indices (0, 1, 2, …) that numerically alias real fd numbers, so the type system cannot catch a mix-up: passing a TLS *handle* to `upgrade` (which expects a raw fd) runs `SSL_connect` on whatever kernel fd happens to share that number (e.g. fd 0/1/2 = stdio), and passing a raw *fd* to `tls::send_bytes` / `recv_bytes` indexes the handle table with a value that isn't a handle — **both are silent misuse** (no error raised, just corrupt/garbage I/O), not a type error. Only ever pass `upgrade` an fd from `tcp::connect`, and only ever pass the `tls::*` send/recv/close primitives a handle returned by `tls::connect` / `tls::upgrade`. `connect` is now internally `dial + upgrade(fd, host, verify=1)`, so the `connect` behavior is unchanged and its network tests double as regression coverage for the shared upgrade path. `send_bytes` / `recv_bytes` / `recv_into` / `close` over the handshaked connection. **`send_bytes(handle, b: Bytes) -> Int` answers a status, not an error:** `0` once the whole buffer is written, `-1` on a failure (errno set; an `SSL_write` failure is also printed to stderr) — never a byte count, and not `fallible`, so an `or` after it is refused ("\`std::io::tls::send_bytes\` is not fallible (it returns \`Int\`); drop the \`or\` clause"). That is the asymmetry with `std::io::tcp`, whose `Stream.send_bytes` and `send_fd` are `-> () fallible(IoError)`: a TLS writer tests the `Int`. `recv_bytes(handle, max) -> Bytes` answers empty on a failure as on EOF. `__send_bytes_until(handle, b, until_ns) -> Int` and `__recv_bytes_until(handle, max, until_ns) -> Bytes` are those two ended by the absolute monotonic instant `until_ns` (`0` is none): the socket is made non-blocking for the call, so every wait, on any thread, is a poll or a park bounded by it, where a blocking `SSL_write` or `SSL_read` restarted its socket timeout on every partial write or read of a record (the HTTP client's call deadline; past it the send answers `-1`, EAGAIN, and the read empty). Send/recv timeouts: `set_recv_timeout(handle, d: Duration) -> () fallible(IoError)` / `set_send_timeout(handle, d: Duration)` wrap `SO_RCVTIMEO` / `SO_SNDTIMEO` on the connection's underlying socket fd (the handle-aware siblings of the `std::io::tcp` ones, which take a raw fd). With a recv timeout set, `recv_into` returns the `-2` "timed out, retryable" sentinel (not `-1`/fatal) when `SSL_read` yields `SSL_ERROR_WANT_READ` — so a long-lived client can bound a blocking read and run connection-liveness work (ping/pong) instead of hanging forever on a half-open connection. On a `where async_io` pool every TLS wait parks the coroutine on the raw fd (2026-10-12, downstream handoff; until then only `recv_into` / `recv_stamped_into` did, and the handshake, `recv_bytes` and `send_bytes` held the worker): the handshake of `connect` / `connect_within` / `upgrade` runs `SSL_connect` on the socket made non-blocking for it (its mode restored after), `send_bytes` runs `SSL_write` on the non-blocking socket, and every read — `recv_bytes`, `recv_into`, `recv_stamped_into` — runs `SSL_read` on it; each `SSL_ERROR_WANT_READ` / `WANT_WRITE` parks on EPOLLIN / EPOLLOUT until the socket is ready, and a read's park ends at the recv-timeout deadline — same 2026-07-14 parking semantics as `std::io::tcp`; `-2` (or `recv_bytes`'s empty return) means the deadline expired, never an instant would-block. A retried `SSL_write` passes the same buffer and length, as OpenSSL requires. The socket's own timeouts bound those waits as the kernel bounds a blocking call (PR #1538 review): `send_bytes` fails (`-1`, with a `send timed out` line on stderr) once `set_send_timeout`'s window passes with no record written — partial writes are on for every connection, so each record written restarts it — and a handshake wait for the peer is bounded by the fd's `SO_RCVTIMEO` (a write wait by its `SO_SNDTIMEO`), so `upgrade` of a fd with a recv timeout armed ends on a peer that never answers. A handshake that ends on a timeout — the call's deadline or the fd's own, on any pool — fails with kind `timeout` and no stderr line (on a blocking socket the kernel's expiry used to surface as a failed handshake, kind `connection_refused`). Off an async_io pool the socket stays blocking and each call is the single blocking `SSL_*` call it was. **Fast-path siblings**: `set_nodelay(handle, on: Bool) -> () fallible(IoError)` (TCP_NODELAY on the underlying fd — reuses the tcp primitive) and `recv_stamped_into(handle, buf: std::bytes::BytesBuilder, max) -> Int` with `last_recv_kernel_ns() -> Int` / `last_recv_user_ns() -> Int`, the TLS siblings of the `std::io::tcp` versions. `set_rx_timestamps(handle, on: Bool) -> () fallible(IoError)` enables `SO_TIMESTAMPNS`. The kernel timestamp rides the *socket* recvmsg but `SSL_read` sits in front of the socket, so the TLS connection uses a custom BIO whose read does `recvmsg` + the defensive `SCM_TIMESTAMPNS` cmsg walk, capturing the stamp on the socket fill that feeds `SSL_read` — `last_recv_kernel_ns` is the last segment's kernel RX stamp (0 when the path delivered none, e.g. no NIC RX timestamping), `last_recv_user_ns` is a `CLOCK_REALTIME` stamp at `SSL_read` return. (The recvmsg cmsg is the path that carries `SO_TIMESTAMPNS`; the `SIOCGSTAMPNS` ioctl reads `sk_stamp`, which that option does not populate.) Process-global `SSL_CTX` runs with `SSL_MODE_RELEASE_BUFFERS` — OpenSSL releases its read/write buffers between records so long-running TLS clients don't accumulate ~32 KiB per idle connection. **Allocation (audit, fast-protocol-I/O #6):** `recv_into` is zero-alloc on the Hale side — `SSL_read` decrypts straight into the caller's reserved buffer, no per-record malloc in the binding (the `tcp`/`udp` `recv_into` siblings likewise; pinned by `crates/hale-codegen/tests/recv_zero_alloc.rs` via the `std::diag` counter). The only per-record TLS allocation is OpenSSL-internal: with `SSL_MODE_RELEASE_BUFFERS` set, a released read buffer is re-malloc'd on the next record — a deliberate memory-vs-malloc tradeoff. An always-busy latency-critical connection that prefers zero per-record malloc over frugal idle memory would clear that mode (retain the buffers); that knob isn't exposed yet (no consumer). The `lotus_tls.c` TU compiles separately so helper tests linking `lotus_arena.c` directly don't drag in libssl/libcrypto. | `lotus_tls_*` in `runtime/lotus_tls.c` |
| `std::shm` | In-band record-header field delivery for a foreign ring (2026-06-13, shm-ring-interop). When a `layout:`-bound subscriber's `ring_layout` declares `record_header_bytes` with in-band header scalars (a per-record fixed header before the payload — e.g. a sequence number and a producer wire-arrival timestamp), `last_record_seq() -> Int` / `last_record_kernel_ns() -> Int` / `last_record_user_ns() -> Int` read the header fields of the record currently being delivered, called from inside the handler (errno-style thread-local, the same read-immediately idiom as `tcp::recv_stamped` — the value is the most-recent record's, valid for the duration of the handler). Each returns `0` when the bound layout declares no corresponding header field. The names map to the layout's declared header scalars by role; the layout's `recheck post_copy` guard ensures a torn header isn't surfaced. | `lotus_shm_*` in `runtime/lotus_shm_ring.c` |
| `std::diag` | Test-time gate counters. `heap_alloc_count() -> Int` returns the cumulative number of heap allocations (malloc / realloc / calloc / mmap) the runtime has made; `syscall_count(name: String) -> Int` returns the cumulative count of a wrapped I/O syscall (`"recv"`, `"recvmsg"`, `"read"`, `"write"`, `"send"`, `"sendto"`). Read a counter before and after a steady-state region and assert the delta — the runtime/test-time complement to compile-time `--warn-unbounded-alloc` ("this loop did zero heap allocs" / "exactly one read per poll"). Both return `-1` when the counting shim is absent (sanitizer builds — TSan/ASan interceptors collide with the `-Wl,--wrap` shim), so a caller can distinguish "gate unavailable in this build" from a real `0`. Counters are process-wide and monotonic; `syscall_count` returns `-1` for an unrecognized name. The wrap shim is compiled into every default (`-O2`) build at the cost of one relaxed-atomic increment per allocation / wrapped syscall — only the runtime's own calls are routed (libc- and libssl-internal I/O is untouched). | `lotus_diag_*` + `__wrap_*` in `runtime/lotus_arena.c` |
| `std::http` | `Request` + `Response` types (`Response.headers: String` carries CRLF-joined user-supplied headers — no trailing CRLF — for Set-Cookie / CORS / custom headers); `parse_request`, `write_response(s: std::io::tcp::Stream, resp: Response)` (a statement; the status line's reason phrase — there and in every status line the stdlib writes: a takeover's, the api runtime's — comes from one table holding every status RFC 9110 § 15 defines, in its wording (`413 Content Too Large`, `422 Unprocessable Content`), and the others the IANA registry assigns (`429 Too Many Requests`, `451`, `103`, …); an unassigned status reads `Status`. It held seven statuses until 2026-10-12, downstream handoff: a `201` went out as `201 Status`); case-insensitive symmetric `header(receiver, name: String) -> String` lookup (the receiver is a `Request` or a `Response`, evaluated once, and its type picks the reader: the stdlib table's row names one Hale body per receiver type, F.40 phase 4, S5; it used to be evaluated twice); `Handler` interface (`fn handle(req: Request) -> Response`); `Server` locus with `shutdown()` and `std::io::tcp::stop(name)` (see [§ Stopping a server](#stopping-a-server--stdiotcpstopname-and-servershutdown)) and optional `ready_signal: String` for piped oracles. **Request reassembly** (2026-07-14, downstream handoff): the per-connection loop reads until the `\r\n\r\n` header terminator and then until `Content-Length` body bytes arrive, so clients that split headers and body across TCP segments (python urllib et al.) are served whole. The bytes are assembled in a `BytesBuilder`, so what a connection holds while it reads is linear in what it sent (2026-10-18, downstream handoff: each read was appended to a String, which kept every prefix alive until the request was served, memory quadratic in the body before any handler or authentication ran: a 12 MiB request held about 9.5 GB). Guards: a total-request cap, the Server's `max_request_bytes: Int = 1048576` (1 MiB; a larger value acts as 1 GiB, the connection loop's const ceiling), where a declared `Content-Length` over the cap is answered with the Server's `too_large: Response` (default `413` `text/plain` `payload too large`, today's bytes) before the body is read, and overflow before a complete header block closes without a response (2026-10-17, downstream handoff: both were constants of the connection loop, so an API could neither raise the cap nor answer it in its own error shape; a handler that answers the over-cap request itself, per request, is not offered) and a 5s recv timeout (bounds a stalled client wherever it reads: on an `async_io` pool the read parks only that connection's coroutine and the deadline still ends it). **Concurrency follows placement** (see [§ Server concurrency](#server-concurrency--a-coroutine-per-connection-on-async_io)): placed `cooperative(pool = X) where async_io`, the Server serves its connections concurrently, a coroutine each, at most `max_connections: Int = 256` at once; anywhere else (main, `pinned`, a classic pool) one at a time. The Server is a listener and a consumer of the TCP connection substrate (see [§ The TCP connection substrate](#the-tcp-connection-substrate)); `name: String` (left empty, `host:<bound port>`, unique even for `port: 0`; the name it goes by is its `listener` field; an explicit one another listener holds is the `name_taken` violation at birth) is its address for `std::io::tcp::stop(name)`; birth binds, so a restart binds afresh. Keep-alive remains unsupported (`Connection: close` hardcoded); a request's `Transfer-Encoding: chunked` is not parsed. **Streamed body** (2026-10-17, see [§ Streamed responses](#streamed-responses--chunked-bodies-and-server-sent-events)): a handler builds a `BodyWriter` (chunked transfer encoding) or an `SseWriter` (Server-Sent Events over it) on `req.conn_fd`, sends chunks or events, each one write, and returns the writer's `end()`, a `Response { streamed: true }` on which the Server writes the last chunk and closes; a handler that builds none writes the same bytes as before. **Per-connection region** (GH #578): each connection is served in a child of the Server that dissolves when its response is written (inline, or on `async_io` a flow child reclaimed as its `run()` returns), so nothing a request allocates — the reassembled bytes, the parsed `Request`, the handler's `Response` and its body — outlives its response; a Server's memory does not grow with the requests it answers (it used to keep every response, a 450 KB body growing the process 450 KB a request). The handler locus itself is the Server's param and keeps its own state across requests as before. **Connection takeover / Upgrade (2026-07-19):** `Request.conn_fd` carries the live connection fd into the handler (`-1` outside a Server), and `Response { takeover: true }` makes the Server write ONLY the status line + the response's `headers` + a blank line — no Content-Type/Content-Length/`Connection: close`, no body — and return WITHOUT closing the fd (`Stream.release_fd()` disarms the per-connection scope close). From that moment the handler owns the connection: stash `req.conn_fd` (typically publish it to a session locus on its own pool) and drive it through the raw-fd tcp surface (`send_fd` / `recv_into` / `close_fd`) or a borrowed `Stream { conn_fd: fd, owns_fd: false }`. Status-agnostic (101 for WebSocket-class upgrades — `__status_phrase` knows `101 Switching Protocols` — or a CONNECT tunnel's 200). Caveats: the conn loop's 5s recv timeout is still armed on the fd (clear via `set_recv_timeout(fd, 0)`), and a handler that sets `takeover` without stashing the fd leaks it — the accept/release daemon warn class. This is the surface WebSocket promotion was blocked on. **Raw takeover (Crumb batch-3, 2026-07-28):** `Response { takeover_raw: true }` transfers the fd with **nothing written** — no status line, no headers. For the deferred-response shape: the handler returns before the answer exists (a promise resolved later, a bus reply from another locus), so whoever ends up owning the fd writes the entire response — status line included — via the raw-fd surface (`send_fd`). Also the CONNECT-tunnel / server-initiated-protocol shape. `status`/`headers`/`body` are ignored; takes precedence over `takeover`; same timeout + fd-leak caveats. **Router** (promoted from pond/router, 2026-07-17): `Router` locus — `add(method, pattern, h)` registers `METHOD /path/:capture` patterns against `RouteHandler` loci (`fn handle(ctx: Context) -> Response`; first match wins, register specific-before-general; method matching is case-insensitive at register time), `add_fn(method, pattern, f)` registers a bare `fn(Context) -> Response` — no handler locus; the fn pointer is stored in the route entry itself, sharing one list and one precedence order with locus routes (2026-08-11, downstream request), `use(m)` registers `Middleware` (`before(ctx)` forward / `after(ctx, resp)` backward — onion order), `dispatch(req)` runs the chain, and `handle(req)` satisfies the `Handler` interface so `Server { handler: router }` plugs in directly. `add` and `use` **keep** their argument: the router holds the handler or middleware as a borrow and never reclaims it, so it must outlive the router — a field of the locus that owns the router, or a router built and used in one frame; `hale check` refuses a literal from a frame the router outlives (`spec/semantics.md` § "A handle a method keeps is the same borrow", GH #1048). **Direct dispatch** (2026-08-14): `build_context(req) -> Context` builds the per-request bundle the Router would (query string split into `params.qs`, captures empty) and `is_route(ctx, method, pattern) -> Bool` runs the Router's own matcher against one method + pattern — `:name` captures fill `ctx.params` on a hit; **any** miss (method or pattern) leaves the captures cleared, so an `if`-ladder of `is_route` checks in a locus's own `handle` is first-match-wins in written order with no stale-capture bleed. The captures are written into the context's own `params`: a `let p = ctx.params;` taken before the match is a copy without them, and `hale check` warns at a `path_param(p, …)` read of such a copy when an `is_route` on the same context stands between the copy and the read (2026-10-17, downstream handoff: a router that copied the params once before its ladder answered every route with an empty id). Method compare is case-insensitive on the argument side (like `add`); both fns are pure. This is the one-locus-many-endpoints shape: endpoint methods are direct `self` calls, so shared per-instance state (a db handle) needs no Router and no per-endpoint handler loci, and under `pinned(..., replicas = K)` placement that state is per-replica by construction. `Context` bundles the parsed `Request` (`ctx.req`, raw target incl. query string) with `RouteParams` (`ctx.params`); `path_param(params, name)` / `query_param(params, name)` return the capture / `k=v` value or `""` (sentinel shape), **percent-decoded** (2026-10-12, downstream handoff: they handed back `hello%20world`): `path_param` with `std::url::decode` (`%XX` only; a `+` in a path is a plus), `query_param` with `std::url::decode_query` (`%XX`, and `+` as a space), which it applies to the key as well before comparing, so `?first%20name=a+b` answers `query_param(p, "first name")` with `"a b"`; a pair with no `=` is its key with the empty value. Route matching still runs on the raw path. Patterns bound at 8 captures (`bounded[String; 8]` — exceeding it raises at register-authored routes); the 404 default is the overridable `not_found: RouteHandler` param. Trailing-slash tolerant on both sides; no implicit wildcard suffix. **Client** (promoted from pond/http/client, 2026-07-17): one-shot free fns `get(url)` / `post(url, body, content_type)` / `request(req)` — all `fallible(HttpError)`, `Connection: close`, read-to-close, the whole call bounded at **30 s** (the `Client`'s default `timeout_ms`, below: a dial it ends fails `connect_failed` with `detail` `<host>:<port> timeout: no connection within 30000 ms`, and an exchange it ends after the dial fails `recv_failed` with `<host>:<port> timeout: no complete response within 30000 ms`; 2026-10-12 — the bound was missing, so a host that never answered held the caller for the kernel's ~2 min of SYN retransmits; another deadline is a `Client { timeout_ms: …, max_retries: 0 }`, which without `keep_alive` is the same one-shot exchange) — plus the pooled `Client` locus (`user_agent` / `timeout_ms` / `max_retries` / `max_body` params; `timeout_ms` (default `30000`) bounds each **call** whole, on both paths: every attempt's dial — the TCP connect and, for https, the handshake, through `connect_within` with what is left — its writes and its reads of the headers and the body (each write, and each https read, takes the absolute deadline itself — `std::io::tcp::__send_bytes_until` / `std::io::tls::__send_bytes_until` / `__recv_bytes_until` — since a stall window like `SO_SNDTIMEO` restarts on every partial write, so a peer accepting a slice at a time held a large body past any window armed before it; a plain read is one `recv`, bounded by `SO_RCVTIMEO` armed with what is left, which an `async_io` pool's park mirrors), and the retries, which start, and back off, only while the deadline has not passed. A call it ends fails `retries_exhausted` with a `detail` that names it — `<host>:<port> timeout: no connection within <timeout_ms> ms` for a dial, `… timeout: no complete response within <timeout_ms> ms` after one — and a response the deadline cut short is no response, never handed back truncated; `0` leaves the call unbounded (2026-10-18, downstream handoff: it bounded only the dial, so a server that accepted and then held the response blocked the caller indefinitely, and the default three retries repeated the wait: a request against a server holding each call 30 s took 120 s); same fallible method surface; opt-in `keep_alive: true` switches to framed reads — Content-Length or `Transfer-Encoding: chunked` — over a 4-slot per-host:port connection pool with retry-and-backoff; the pool is hand-rolled rather than a `@form(lru_cache)`: it predates the form's `remove(k)` take-semantics and `on_evict` hook, which an fd-owning cache needs and which `spec/forms.md` now specifies). Client-side types are distinct from the server side on purpose: `ClientRequest` (`method` / `url: Url` / packed `headers` / `body: Bytes`) and `ClientResponse` (`status` / packed `headers` / `body: Bytes` — Bytes for binary safety, embedded NULs survive); `parse_url(s) -> Url fallible(HttpError)` decomposes scheme/host/port/path (query rides in `path`; no userinfo/fragments; no URL-decoding). **An IPv6 literal host is bracketed** (2026-10-12, downstream handoff: `http://[::1]:9107/x` was `bad_url`): `Url.host` is the address without its brackets (`::1`), only a `:port` may follow the `]`, brackets around anything but an IPv6 address, an unclosed `[`, an unbracketed IPv6 address and a port outside 1..65535 are `bad_url`; the client dials the address and writes `Host: [::1]:<port>`. `HttpError` (`kind` / `status` / `detail` / `headers` / `body: Bytes`) kinds: bad_url / unsupported_scheme / connect_failed / send_failed / recv_failed / bad_response / too_large / retries_exhausted. **A 5xx is retried by the `Client` (both paths) and, once the attempts are spent, fails `retries_exhausted` carrying the last 5xx response whole** (2026-10-12, downstream handoff): `status`, `headers` and `body` are that response's — the retry path used to drop the body and the keep-alive path the status as well (status 0) — and `0` / empty when no attempt received a response; `detail` names the last attempt's failure and the attempt count. The one-shot free fns do not retry: a 5xx is their response. https rides `std::io::tls`, and on a `where async_io` pool every wait of an https call — the dial, the handshake, the writes and the reads — parks the coroutine as a plain-http one's does (2026-10-12; it used to block the worker, and https callers had to stay off async_io pools); the name lookup of a non-numeric host still runs on the worker (§ `std::io::tcp`). Not implemented at v1: redirects (3xx returns as-is), proxies, compression. | `runtime/stdlib/http.hl` |
| `std::json` | `Builder` locus (streaming output assembly — see [§ json::Builder](#stdjsonbuilder--streaming-output-api)); `escape_string` / `unescape_string` (RFC 8259 — `unescape_string` decodes \uHHHH to its scalar, combines surrogate pairs, and answers U+FFFD for an unpaired surrogate or \u0000; both helpers and the Builder are linear in output bytes — see [§ String escaping](#string-escaping--escape_string--unescape_string)); `find_string_field` / `find_int_field` / `find_bool_field` (flat-object lookup — PERMISSIVE by contract: a non-String value comes back as its raw scalar spelling, so `"owner": null` reads as the String "null"); `string_field(json, name) -> JsonString` (the TYPED read, GH #719 — `{kind, text}` where `kind` is one of `"string"` / `"null"` / `"missing"` / `"number"` / `"bool"` / `"array"` / `"object"` / `"invalid"` and `text` carries the decoded string only for `"string"` — see [§ Typed field access](#typed-field-access)); `find_field_raw(json, name) -> String` (the raw value of the named TOP-LEVEL member — rebuilt on the object cursor 2026-07-28 (Crumb batch-4): matches key POSITIONS only, so key text recurring inside an earlier string VALUE no longer shadows the real key (on a real npm packument the old text-scan lost 12 of 35 version keys). Depth-aware and string-safe; nested walks re-feed the returned object/array substring — the documented chaining contract, now actually enforced); `ArrayIter` + `array_first` / `array_next`, and the span-bearing `ArrayIterSpan` cursor `array_first_span(json) -> ArrayIterSpan` / `array_next_span(it, json) -> ArrayIterSpan` (carry the element's byte range rather than an owned substring — the allocation-free array-walk sibling of the object cursor below). Range-bearing iter family: `iter_find_field_range(it, json, name) -> JsonFieldRange` and `iter_find_string_field_range(it, json, name) -> JsonFieldRange` return `{ok, start, end_pos}` instead of an owned-String substring (the owned siblings: `iter_find_string_field(it, json, name) -> String`, `iter_find_int_field(..) -> Int`, `iter_find_bool_field(..) -> Bool`, `iter_find_field_raw(..) -> String`, `iter_substring(it, json) -> String`, and the bounded `find_field_raw_in(json, name, start: Int, end_exclusive: Int) -> String`; the byte scanners under them are `next_non_ws` / `next_quote_or_bs` / `next_struct_or_quote(json, from: Int, len: Int) -> Int`); paired with the `std::str::range_*` family for fully allocation-free per-element walks on large arrays (the high-throughput workload class where per-field allocation dominates arena pressure). No nested-tree shape at v1 — re-feed substrings into the same surface for nested walks. Single-pass object member cursor: `object_first(json) -> ObjectIterSpan` / `object_next(it, json)` walk `{...}` members once, with `obj_key_eq(it, json, name) -> Bool` / `obj_key_len(it) -> Int` for key dispatch, `obj_key_string(it, json) -> String` for unknown-key iteration (the key-side sibling of `obj_value_string`, incl. escape decoding — hand-slicing `key_start..key_end` silently skips it) and `obj_value_int(it, json) -> Int` / `obj_value_bool(it, json) -> Bool` / `obj_value_float(it, json) -> Float` / `obj_value_string(it, json) -> String` / `obj_value_raw(it, json) -> String` reading the current value from its source range (no per-field rescan; nested objects/arrays on unmatched keys are skipped whole by the depth scan). This is the substrate a compiler-generated, schema-specialized parser drives — and the seam a future SIMD structural index slots under. Strict counterparts to all of the above: `valid(text) -> Bool` (one well-formed RFC 8259 value and nothing else) and `valid_object(text) -> Bool` (…and a top-level object with unique, unescaped keys) — see [§ Validation](#validation--valid--valid_object). **`Value`** (2026-10-19): free-form JSON an api field carries byte for byte (spec/api.md § Codecs), a record of its `text`, read with the cursors above; `Value::parse(text) -> Value fallible(ParseError)` admits one well-formed value at most 64 levels deep (`kind: "json"`) and 64 KiB long (`kind: "json_too_long"`), `Value::null()` is JSON `null`; a literal `Value { text: … }` and a store into one (`v.text = …`) are refused at the check, so every `Value` passed `parse` (a codec reads through it too). | `runtime/stdlib/json.hl` |
| `std::test` | `assert(cond: Bool, msg: String)`, `assert_eq_int(actual: Int, expected: Int, msg: String)`, `assert_eq_str(actual: String, expected: String, msg: String)`, `assert_near(actual: Float, expected: Float, tolerance: Float, msg: String)` (2026-10-12, downstream handoff: passes when the two are equal — two equal infinities included — or `|actual - expected| <= tolerance`; a NaN on either side never passes; the failure prints `expected`, `actual`, and `difference: <d> (tolerance <t>)`; an `Int` argument widens) — statements, returning nothing | `runtime/stdlib/test.hl` |
| `std::law` | The stdlib's standard constitutions, adopted by path with no import (`--adopt std::law::Standard`, an environment's `constitution = "std::law::Standard"`) and loaded only when adopted: `Standard` (`logs_structured`, the program writes to the terminal only through `std::log`). See verification.md § Adopting from outside the source. | `runtime/stdlib/law.hl` |
| `std::log` | `Logger`, `LogEvent`, `StdoutSink` (subscribes `log.**`). **Records (GH #1592, 2026-10-19):** `StdoutSink` and `FileSink` write one record per event, the event's time as RFC 3339 UTC, its level, the logger's path, the message and the fields: logfmt by default (`time=… level=warn logger=app.db msg=retrying attempt=2`), every level on stdout; `HALE_LOG_FORMAT=json` writes one JSON object per line (the fields as string members), and `HALE_LOG_FORMAT=text` the format before records (`[WARN app.db] retrying attempt=2`, WARN and ERROR on stderr). A sink's `format:` param (`"logfmt"`, `"json"`, `"text"`) pins it; empty asks the variable, and an unknown name is logfmt. A value is quoted (an empty one stays bare, `key=`) when it holds a space, a quote, an `=`, a backslash or a control character (U+0000 to U+001F, U+007F), and inside the quotes `\\`, `\"`, `\n`, `\r`, `\t` and `\u00XX` for the other control characters, so a record is one line every logfmt reader splits where the writer did; the JSON form escapes every control character as JSON does. These are the records the standard-practices pack's first clause (`standard/logs-are-structured`, GH #1591) requires. The stdlib's own terminal lines (a binding that cannot be realized before the process exits, a role naming nobody, a listener's `ready_signal`) are written by two file-private fns of `log.hl`, so a law gated `avoiding std::log::*` (`std::law::Standard`) reads them as the logger's, not as a program's bare write. **Sinks promoted from pond/logfmt (2026-07-18):** `FileSink` — appends every event to `path`, rotates by size (`max_size_bytes`, `keep_files`; chain shifts via atomic `rename(2)` overwrite, oldest evicted, active file recreated on next append), I/O failures captured in the `last_error_kind/errno/path` scratch triple (the styleguide-2.7 convention), also wears the `std::text::Sink` shape (write/line/newline with the same rotation). `ConsoleSink` — dim HH:MM:SS + colored width-5 level badge + dim path + message; WARN/ERROR on stderr (StdoutSink's lane split); color AUTO (tty probe on stderr; FORCE_COLOR/CLICOLOR_FORCE override; NO_COLOR always wins; `color: false` = never). pond's OtlpSink stays in pond (vendor-protocol integration is app-tier). **Structured logging (GH #469):** `LogEvent` carries `fields` (logfmt text) and `ts` (unix seconds, stamped at the PUBLISH site, not the render site — they differ under a queued or bridged sink and under `hale replay`). `kv(key, value)` renders one pair, quoting and escaping the value as a record's are; join pairs with a space. Each level has a `_kv` variant (`info_kv`, `warn_kv`, …). Fields are text rather than a map because a map is a locus and a locus cannot be a payload; the flat record crosses every transport the bus supports. **Level filter:** `HALE_LOG=error|warn|info|debug` sets a threshold; `min_severity` on a `Logger` or any sink pins it in code (0=trace 1=debug 2=info 3=warn 4=error — a severity ORDER distinct from the wire level constants, which are historical). Filtering is at the PUBLISHER, so a suppressed call emits no event at all; sinks filter too, for the case where two sinks want different levels. | `runtime/stdlib/log.hl` |
| `std::metrics` | Prometheus-shaped metrics (promoted from pond/metrics, 2026-07-18). `Registry` locus (`namespace` prefix; **owns its storage** — the `MetricMap` (`@form(hashmap, sync = serialized)`) and `HistogramList` (`@form(vec)`) are param-default children, so `Registry { namespace: "app" }` is the whole construction and a Registry returned from a builder fn keeps its series alive; explicit `store:`/`histograms:` overrides remain accepted but a let-bound override dissolves at the constructing fn's scope exit — prefer the owned default). Factory free fns `counter(reg, name, labels)` / `gauge(reg, name, labels)` / `histogram(reg, name, bounds, labels)` are **idempotent on (name, labels)** — a repeat call returns a handle to the same series without resetting it; handles reference the storage slots directly (resolve once at boot, cache the handle as a field, mutate from the hot path — styleguide S12). `Counter` (`inc` / `add`, monotonic by convention), `Gauge` (`set` / `add` / `sub` / `inc` / `dec`), `Histogram` (`observe(v)` — cumulative buckets + implicit `+Inf` + `sum` + `count`; bucket bounds passed as a space-separated ascending String `"0.005 0.01 0.05"`, parsed once at registration, max 32 buckets with over-cap clamping). Labels via `labels_empty()` / `labels_one(k, v)` / `labels_two(...)` / `labels_append(l, k, v)`. `render()` emits Prometheus text exposition (each family written whole under one `# TYPE` line — no `# HELP`, as a series carries no help text — through the renderer `std::observe::Scrape` shares, `__metrics_render_series`; it wrote a TYPE line per series, which the text format refuses for a family of several labelled series; `ns_name{k="v"} value` samples, histogram `_bucket{le=...}` / `_sum` / `_count`), every value to the digit: an integral one below 2^53 as its integer, any other in fixed point to nine decimals with its trailing zeros cut (one too small for that in exponent form, never `0`), and `NaN` / `+Inf` / `-Inf` as the exposition spells them (GH #988: it was `to_string`'s six significant digits, so epoch seconds and any counter past 999999 were rounded); `Endpoint { registry: reg }` satisfies `std::http::Handler` and answers any request with the rendering under `Content-Type: text/plain; version=0.0.4`, so `std::http::Server { handler: Endpoint { ... } }` is a complete /metrics scrape target. The MetricMap is `sync = serialized` because the canonical topology scrapes from one pool while handlers write from another. In a program whose main locus holds a `std::observe::Scrape` param (GH #1592), a Registry also registers at birth (and leaves at dissolve) in the process-wide series table `std::observe::Scrape` renders, and each handle writes its updates there too; in any other program those calls lower to nothing. | `runtime/stdlib/metrics.hl` |
| `std::observe` | GH #1592. The observe backends, each a param of the main locus recognized by its type (`spec/semantics.md` § "Observe backends"). `Scrape { host: String = "127.0.0.1", port: Int, path: String = "/metrics" }` is the metrics backend: a locus whose `run()` serves a `std::http::Server` on `host:port`, wherever `placement` puts it, answering a GET of `path` (a query aside; any other path is 404) with the Prometheus text exposition of every live `std::metrics::Registry`'s series and the runtime's own (`hale_*`, `spec/runtime.md` § "Observe backends"), each family's `# TYPE` line once, rendered by `std::metrics`'s renderers. A main locus holds one, and a construction cannot supply it. `port` is required, and the scrape is a listener of its own: `Probes` serves no `/metrics`. The `__`-prefixed primitives (`__registry`, `__drop_registry`, `__series`, `__add`, `__set`, `__observe`, `__snapshot`, `__snap_str`, `__snap_num`, `__snap_int`) are the series table's, lowered to constants in a program that holds no scrape. `Otlp { endpoint: String = "", service: String = "", interval: Duration = 1s, timeout_ms: Int = 2000, max_batch: Int = 512 }` is the events backend (S4), a param of the main locus in the same way: placed by `placement`, it sends the runtime's lifecycle events as OTLP/HTTP JSON log records to `endpoint` (empty: `OTEL_EXPORTER_OTLP_LOGS_ENDPOINT`, else `OTEL_EXPORTER_OTLP_ENDPOINT` + `/v1/logs`; none at all: every batch is dropped and counted), a batch every `interval`, each send bounded by `timeout_ms` in all through a `std::http::Client` with no retry; `service` empty is `OTEL_SERVICE_NAME`, else `hale`; `OTEL_RESOURCE_ATTRIBUTES` joins the resource; the first export opens with a `hale.start` record (`spec/runtime.md` § "Observe backends"). `__events_take`, `__event_str`, `__event_int` and `__events_dropped` are the event ring's, constants in a program that holds no `Otlp`. `Probes` (GH #1477, 2026-10-19): a locus of one param, `port: Int = 9090`. A main-locus param of the type serves the process's `/healthz`, `/readyz` and `/capacity` over HTTP from a runtime thread (spec/semantics.md § Probes); the check allows one, on the main locus, built by its default. | `runtime/stdlib/observe.hl`, `runtime/stdlib/probes.hl` |
| `std::secret` | GH #436 (2026-08-08). Key material an application never holds. `Signer` — HMAC over a key the caller never sees: `sign(msg) -> Bytes` (HMAC-SHA256, 32 bytes), `sign_512(msg) -> Bytes` (64), `verify(msg, mac) -> Bool`, `ready() -> Bool`. `Credential` — an opaque token/password: `matches(candidate) -> Bool`, `ready() -> Bool`, `fingerprint() -> String` (first 8 bytes of SHA-256, hex — a stable non-reversible handle that is safe to log and correlates across processes), `sign_ed25519(message) -> Bytes` and `ed25519_public() -> Bytes` (GH #1590 — an Ed25519 signature, 64 bytes, over `message` and its 32-byte public key, under the key whose seed is the SHA-256 of the credential: a record signs what it sends another record with a key the vault holds, and neither the credential nor the seed leaves the sealed side; `b""` for both when there is no credential; deterministic, as RFC 8032 makes it), `reveal() -> Bytes` / `reveal_text() -> String` (GH #989 — the resolved credential in the open, for the one statement that puts it on a wire or compares it: the checker refuses a reveal outside a locus method, and one whose value is not consumed in its statement by a wire write's payload, a comparison of the whole value (`matches` is the constant-time one) or a `@secret` parameter — bound, stored, returned, used to decide a branch, or handed to `std::process` — `spec/semantics.md` § "`@sealed` and a revealed secret"; a `Signer`'s key never leaves at all), and `write_private(path) -> Bool` (the credential written on the sealed side, nothing revealed, for a tool that reads a secret from a file: `path` is a NEW file created mode 0600 by `openat(2)` itself — `O_CREAT|O_EXCL|O_NOFOLLOW`, never chmod after — in a directory the process's effective user owns with no group or world access, that is not itself a symlink, and outside any git work tree (no `.git` in it or an ancestor); every check and the create go through one descriptor of the directory, so no path component can be swapped between them; `false` when it refuses, when there is no credential, or when the write fails, a partial file removed. The reveal rule follows values, not files: the file is the tool's to read, and removing it is the caller's). Both are **`@sealed`**, so their `params` are readable only from inside them: `self.signer.key` from a holder is a compile error naming the method to call instead. Both take the **name of a source**, never the bytes — `env_var:` (read via `std::env::var`) or, for `Signer`, `key_file:` (`std::io::fs::read_bytes`) — and load during `birth`, so the material exists only inside a sealed locus from the moment it enters the program and there is no construction site at which the caller held it. `Signer` also takes an optional `decode:` transform (`""` = the source's bytes verbatim, `"base64"` = decode first): a venue that issues an HMAC secret base64-encoded requires the key to be the DECODED bytes, and a signer pointed at such a credential without `decode:` keys under the text of the base64 — valid-looking MACs under the wrong key, silently rejected downstream. An unavailable source, undecodable input, or an unrecognized transform yields an empty key (fail closed), never a key that isn't the one the source names. Check `ready()` at startup: a missing source yields an empty key rather than a failure. `Credential` also takes `vault:` (GH #989), a name resolved against a vault instead of an env var or a file — and resolved DIFFERENTLY from the other two: `env_var`/`key_file` resolve once, at `birth`, into the sealed value; `vault:` resolves FRESH on every privileged call, never cached, because a vault's whole point is that a value can change without restarting every process that reads it. `env_var` wins if both are set, same precedence `Signer` already has between `env_var` and `key_file`. Local mode (`HALE_VAULT_ADDR` unset) reads a directory of files keyed by name — `HALE_VAULT_DIR`, or the per-user cache root the toolchain already uses (`std::secret::vault_local_dir()`: `$XDG_CACHE_HOME/hale/vault`, `~/.cache/hale/vault`) — filesystem permissions as the only access control, no server, no token, never a system path a process cannot write without root. `HALE_VAULT_ADDR` set makes it an HTTP `GET <addr>/v1/secret/<name>` instead, authenticated with the per-host token in `HALE_VAULT_TOKEN` as a bearer credential. Either way: fail closed exactly like the other two sources — a missing directory, a missing file, a missing token, a non-200, an unreachable address are all empty, never a partial or stale value. Rotation and provisioning a real vault are out of scope for this cut (a real vault is provisioned out of band; `vault:` only reads it). Every privileged operation carries the `secret_use` effect class, declared by the module and travelling with the import, so an application constrains it with claim forms that already exist (`forbid reaches(plugins, effects(secret_use))`, `bound secret_use <= 1 on paths from handlers`). **Scope:** confinement, not information flow — the MAC `sign` returns is derived from the key and is NOT tracked, a `verify` verdict is observable and may be published, the compare is not a constant-time primitive, and zeroization is out of scope (the bytes live in the locus arena for the process lifetime). See `spec/verification.md` § Secrets. | Hale source in `crates/hale-stdlib/hl/secret.hl` over `std::crypto` / `std::env` / `std::io::fs` / `std::http` |
| `std::math` | `sqrt`, `exp`, `log`, `floor`, `ceil`, `pow`, `tanh`, `nan`, `is_nan`, `inf`, `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`, `int_to_float`, `float_to_int`, `round`, `trunc` | path-call dispatch into libm (`nan`/`inf`/`is_nan` are IEEE 754 sentinels; trig added 2026-05-23 for spatial / animation code). `int_to_float`/`float_to_int` are named `sitofp`/`fptosi` conversions — round-toward-zero, callable in any expression position; see `spec/types.md` § Explicit numeric conversions. `round(Float) -> Int` / `trunc(Float) -> Int` are the Float→Int siblings that return `Int` directly: `round` is round-half-away-from-zero (`3.7 → 4`, `2.5 → 3`, `-2.5 → -3`), `trunc` is round-toward-zero (an alias of `float_to_int`). Both lower to pure LLVM (`fptosi`, plus a compare/select half-shift for `round`) — **no libm symbol**, so unlike `floor`/`ceil` (which stay libm and return `Float`) they need no host import on the `wasm32` target. |
| `std::crypto` | `sha1(b) -> Bytes` (20-byte), `sha256(b) -> Bytes` (32-byte), `hmac_sha256(key, msg) -> Bytes` (32-byte), `sha512(b) -> Bytes` (64-byte) / `hmac_sha512(key, msg) -> Bytes` (64-byte) (the 64-bit-word SHA-2 sibling — FIPS 180-4 SHA-512 + RFC 2104 HMAC over a 128-byte block; same non-fallible shape as `hmac_sha256`, hand-rolled, no libcrypto; added 2026-06-25 for venue order-entry auth, which sign with HMAC-SHA512), `crc32(b) -> Int` (4-byte IEEE 802.3 checksum returned as Int; reversed polynomial `0xEDB88320`, init `0xFFFFFFFF`, final XOR `0xFFFFFFFF` — the zlib / Python `binascii.crc32` variant; added 2026-05-27). `ecdsa_p256_sign(key, message) -> Bytes fallible(CryptoError)` / `ecdsa_p256_verify(pubkey: Bytes, message: Bytes, sig: Bytes) -> Bool` (ES256 — ECDSA over NIST P-256 + SHA-256; `key` is a PEM EC private key, SEC1 or PKCS#8; `pubkey` is PEM SPKI; signature is raw `r‖s`, 64 bytes, the JWS/COSE form JWT wants; added 2026-06-03 for venue/JWT auth). `ecdsa_p256_sign` has one mode, fallible (F.40 phase 4, S5: the bare call that returned an empty Bytes on failure is a compile error now, the bare-fallible law's; write `or b""` for that substitute), so `let sig = std::crypto::ecdsa_p256_sign(key, msg) or raise;` propagates a structured `CryptoError { kind: String, detail: String }` (`kind` = the op tag `"ecdsa_p256_sign"`; `detail` = the failure reason) — read it via `or handler(err)` / `or fail err` / `or <substitute>` exactly like `IoError` / `ParseError`. The hashes + crc32 are hand-rolled (no libcrypto); ECDSA is OpenSSL-backed (rides the libssl/libcrypto link TLS already pulls). **Ed25519** (RFC 8032; 2026-10-12, downstream handoff: NATS NKey / JWT auth): `ed25519_keypair(seed: Bytes) -> (Bytes, Bytes) fallible(CryptoError)` answers `(public, secret)` for a 32-byte seed — the 32-byte public key, and the 64-byte secret key that is the seed followed by the public key (the form NaCl, libsodium and Go call the private key); a seed of any other length is `CryptoError { kind: "ed25519_keypair" }`. `ed25519_sign(secret: Bytes, message: Bytes) -> Bytes fallible(CryptoError)` is the 64-byte signature, deterministic as RFC 8032 makes it (no entropy is read); `secret` is the 32-byte seed or the 64-byte seed + public key, and anything else — including a 64-byte key whose public half is not the seed's, which is refused rather than used, since signing with a mismatched public half leaks the private scalar — is `CryptoError { kind: "ed25519_sign" }`. `ed25519_verify(public: Bytes, message: Bytes, signature: Bytes) -> Bool` is RFC 8032 verification with `S < L` enforced; a public key that is not 32 bytes, a signature that is not 64, or a point that does not decode is `false`. All three are pure computation, sans-I/O (no thread, syscall, clock or entropy), backed by **Monocypher 4.0.3** (BSD-2-Clause or CC0-1.0, vendored unmodified under `crates/hale-codegen/runtime/third_party/monocypher` with its licence: the core and its optional SHA-512 Ed25519; 4.0.3 is the first release free of the compiler-dependent EdDSA timing leak upstream lists for 4.0.2 and earlier, monocypher.org/bugs) and `runtime/lotus_ed25519.c`, compiled by the runtime's cc step and linked only into a program that reaches them, as nghttp2 is — on wasm32 too (Monocypher calls no libc), where they run as natively (`portable_subset`). Secret intermediates are wiped before a call returns; the secret key a caller holds is the caller's to confine (`std::secret`, `@sealed`). | `lotus_crypto_*` (hashes in `runtime/lotus_arena.c`; ECDSA in `runtime/lotus_tls.c`); `lotus_ed25519_*` (`runtime/lotus_ed25519.c` over Monocypher); `ed25519_keypair` in `runtime/stdlib/crypto.hl` |
| `std::os` | `getrandom(n: Int) -> Bytes fallible(IoError)` (CSPRNG; `getrandom(2)` with `/dev/urandom` fallback) | `lotus_os_getrandom` C primitive |
| `std::rand` | `next_int(max: Int) -> Int` — a uniform-ish integer in `[0, max)` drawn from a shared xorshift64\* generator; `seed_from_time()` re-seeds that generator from the wall clock. **Not cryptographic** (deterministic PRNG, process-shared state) — for security-sensitive randomness use `std::os::getrandom`. **`Rng { seed: Int }`** (2026-10-12, downstream handoff: the shared generator cannot give the same sequence twice) — a generator of one's own, deterministic per seed on every run and target, sharing no state with any other: SplitMix64 (Steele, Lea & Flood 2014; the seeding generator of xoshiro256\*\*; period 2⁶⁴, passes BigCrush), `seed` copied into its `state` at birth (any `Int`, 0 included). Methods: `next() -> Int` (the next 64 bits, any `Int`; seed 0 begins `0xe220a8397b1dcdaf`, `0x6e789e6aa1b965f4`, `0x06c45d188009454f`), `next_int(max) -> Int` (uniform in `[0, max)`, rejection-sampled so there is no modulo bias; `max <= 0` answers 0 and draws nothing, as `next_int` does), `next_float() -> Float` (uniform in `[0, 1)`, the top 53 bits of a draw), `range(lo, hi) -> Int` (uniform in `[lo, hi)`; `hi <= lo`, or a span too wide for an `Int`, answers `lo`). Pure Hale arithmetic, so its calls carry no effect class (not `entropy`) and are not journaled: a replay re-derives them from the seed. **Not cryptographic either** — its output reveals its state. | `lotus_rand_*` C runtime; `runtime/stdlib/rand.hl` (`Rng`) |
| `std::ts` | Tree-sitter parse substrate (m96 — the `std::ts::*` routes back the higher-level `Lang` locus). `parse_go(src: String) -> Int` parses Go source and returns an opaque **tree handle** (`Int`); `root_node(tree) -> Int` returns the root **node handle**. Node navigation (all handles are `Int`): `node_child_count(node) -> Int` / `node_named_child_count(node) -> Int`, `node_child(parent, i: Int) -> Int` / `node_named_child(parent, i: Int) -> Int`, `node_is_named(node) -> Int` (`0`/`1`). Kind, text, spans: `node_kind(node) -> String`, `node_text(node) -> String`, `node_start_byte(node) -> Int` / `node_end_byte(node) -> Int`. Go is the only bundled grammar at v1; the tree-sitter shim staticlib is linked into the build (gating the link on actual `std::ts` use is future work). **The shim is a build requirement, and a missing one is a build error, not a link error** (GH #808): `std::ts` is the one stdlib namespace whose implementation ships as a separate artifact (`libhale_ts_shim.a`, from the `hale-ts-shim` crate), and a toolchain built without it refuses any program that reaches `std::ts::*` with a located diagnostic naming the artifact and `cargo build --release`. Programs that don't reach `std::ts` — including those that merely merge the stdlib loci that use it — build normally against a toolchain that lacks the shim. | `lotus_ts_*` + tree-sitter shim |
| `std::bus` | `__StdBusAdapter` interface (contract for user-supplied bus transports — a single `fn send(subject: String, bytes: Bytes)` method); `__local_dispatch(subject: String, bytes: Bytes) -> Int` primitive (the `Int` is always `0`) lets an adapter relay received wire-bytes into the local handler set. **Substrate transport URL schemes** (resolved at runtime by `lotus_bus_load_config` from `LOTUS_BUS_CONFIG=<file>`): `unix://<path>` (AF_UNIX SEQPACKET; m58); `udp://<host>:<port>` (2026-05-26 — IPv4 UDP, single scheme covers unicast and multicast: addresses in `224.0.0.0/4` trigger `IP_ADD_MEMBERSHIP` on the subscribe side, everything else takes the plain unicast bind/sendto path; lossy delivery — publishers get "sendto returned" durability, subscribers best-effort; gap recovery is a deployment concern via app-layer repeaters, MoldUDP-style). Each LOTUS_BUS_CONFIG line: `subject = <url>:<role>` where role is `listen` or `connect`. A well-formed route that cannot be *opened* fails the boot via `lotus_bus_binding_fail` (the publish contract, spec/semantics.md — no requested route may silently not exist); malformed lines warn-and-skip. Other protocol-layer transports (NATS, MQTT, raw-TCP-with-framing) come in via user adapters (the `__StdBusAdapter` route above). **Payload size**: the substrate handles bus payloads up to ~64 KB (`LOTUS_PAYLOAD_MAX`, sized to UDP datagram max). Mailbox cells inline payloads ≤ 512 B (`LOTUS_PAYLOAD_INLINE` — zero malloc on the hot path); larger payloads route through a per-cell `malloc` that the drain path frees after the handler returns. The UDP transport's wire buffer is sized at LOTUS_PAYLOAD_MAX; the kernel-side socket receive buffer takes its size from `SO_RCVBUF` (default ~208 KB on Linux, raise via `LOTUS_BUS_UDP_RCVBUF=<bytes>` env). UDP `sendto` failures log once per errno class to stderr (`EMSGSIZE` typically means path-MTU mismatch; see `std::io::sockopt::IP_MTU_DISCOVER` for the DF-bit knob). | `runtime/stdlib/bus.hl` + `lotus_bus_*` C runtime |

Hale doesn't use parametric stdlib collection types (`Map<K,
V>`, `Vec<T>`, `Set<T>`, etc.). Storage is locus-shaped via the
`@form(...)` annotation machinery — see
[`forms.md`](./forms.md). v1 ships `@form(vec)`
(contiguous-buffer), `@form(hashmap)` (intrusive open-addressing,
String / Int keys), and `@form(ring_buffer)` (fixed-capacity
FIFO).

## Builders vs Bytes — the recv-loop pattern

`Bytes` and `std::bytes::BytesBuilder` are deliberately distinct
types because their runtime ABIs are **incompatible**:

- **`Bytes` blob.** Single contiguous allocation: `[i64 len][u8 data[len]]`.
  The handle points at the length prefix. `lotus_bytes_len(b)`
  reads `*(int64_t*)b`. `lotus_bytes_at(b, i)` reads
  `((u8*)b)[8 + i]`. Stable across the value's lifetime.
- **`BytesBuilder` locus.** Owns a `lotus_bytes_builder_t`
  header `{cap, buf, mutation_epoch}` whose body lives in a
  separately malloc'd region pointed to by `buf` and can move
  on realloc (the header is stable; the body is not).

The two ABIs cannot be unified without giving up stable handles
(the body has to be relocatable; the Bytes blob layout doesn't
tolerate that). Lifting the builder into its own locus type
turns `std::bytes::at(builder, i)` into a typecheck error rather
than the silent footgun it was when the builder shared the
`Bytes` static type.

The discipline that follows:

1. **Pick one role per binding.** A binding is either a
   `BytesBuilder` (long-lived growable buffer with methods
   `append` (chunk: Bytes) / `append_str` (s: String — append the
   string's bytes verbatim in one strlen + memcpy; `String` only,
   a non-NUL-terminated `StringView` must be materialized first) /
   `len` / `shift_front` / `clear` / `snapshot` /
   `finish` / `view` / `text_view`, plus the binary-pack writers
   below) or a `Bytes` (immutable length-prefixed blob with
   functions `at` / `slice` / `len` / `concat`). The typechecker
   enforces this; no implicit coercion between them.

   **Binary-pack writers** (2026-06-06, shm-ring-interop Proposal A
   — the inverse of `std::bytes::read_*`): `b.append_u8(n)`,
   `b.append_u16_{le,be}(n)` / `u32` / `u64`, the signed
   `b.append_i8`/`i16_{le,be}`/`i32`/`i64` (identical byte pattern —
   width is what matters), `b.append_f32_le(x)` /
   `b.append_f64_{le,be}(x)` (x: Float), and `b.append_pad(to_align)`
   (zero-fill to the next `to_align` boundary). Each appends the low
   `width` bytes in the named endianness; a realloc failure routes
   through `violate alloc_failed` like `append`.
2. **Cross between roles via explicit calls.** `BytesBuilder →
   Bytes` is either `b.snapshot()` (copies into the bus
   payload arena — stable across mutations) or `b.view()` (no
   copy, aliases the builder's buffer — valid until the next
   mutation; the right choice for parser passes). `Bytes →
   BytesBuilder` is `let b = std::bytes::BytesBuilder { ... };
   b.append(bytes)` (copies). The Builder → Bytes direction has
   a zero-cost path via `view()`; the reverse does not.
   `snapshot()` and `finish()` are `-> Bytes
   fallible(ClosureViolation)` (F.42): a failed allocation violates
   `alloc_failed`, the owner's `on_failure` runs, and the call
   fails, so each call carries an `or` (`b.snapshot() or raise`
   where the activation should not go on). `view()` and
   `text_view()` allocate nothing and are not fallible.
3. **Long-lived accumulators live as locus state.** Either a
   method-body `let`-binding (dissolves at scope exit) or a
   param-typed field on the owning service locus (dissolves via
   the F.29 cascade at the parent's dissolve). Method-body
   `let` is simpler when the lifetime fits in one method call;
   field-typed is needed when the buffer must outlive a single
   call (e.g., state held across bus subscription callbacks).
4. **Read syscalls write directly into the builder.** The
   `recv_into` family takes a `BytesBuilder` as `buf` and
   writes into its tail. Combined with `b.shift_front` after
   each peeled frame (streaming) or `b.clear()` at message
   boundaries (per-message accumulator), the steady-state recv
   loop is zero-alloc against `g_bus_payload_arena`.

Canonical pattern, a held-open socket locus that accumulates
inbound frames:

```hale
locus FrameClient {
  params { sock: Int = -1; recv_chunk: Int = 4096; }
  run() {
    let rx_buf = std::bytes::BytesBuilder {
      initial_cap: 4096,
    };
    loop {
      let got = std::io::tcp::recv_into(
        self.sock, rx_buf, self.recv_chunk);
      if got <= 0 { break; }
      // peel complete frames off the front via rx_buf.len() /
      // rx_buf.shift_front(consumed); for a per-frame snapshot
      // use rx_buf.snapshot() only at the point of handoff to
      // logic that needs a Bytes view (parsers that read via
      // std::bytes::at / slice).
    }
    // rx_buf dissolves here → buffer freed, no explicit cleanup
  }
}
```

Try writing `std::bytes::at(rx_buf, 0)` inside that loop — it
fails at typecheck (`at` expects `Bytes`, got
`__StdBytesBytesBuilder`). The discipline is mechanical, not
documentary.

The same pattern with the builder held as locus state (per
F.29), for cases where the accumulator must survive across
multiple method calls (bus callbacks, message state held
between handler firings):

```hale
locus WsClient {
  params {
    sock: Int = -1;
    recv_chunk: Int = 4096;
    rx_buf:   std::bytes::BytesBuilder
            = std::bytes::BytesBuilder { initial_cap: 4096 };
    last_msg: std::bytes::BytesBuilder
            = std::bytes::BytesBuilder { initial_cap: 4096 };
  }
  fn read_one() {
    let got = std::io::tcp::recv_into(
      self.sock, self.rx_buf, self.recv_chunk);
    // ... peel frames, append payload bytes into self.last_msg
    // via self.last_msg.clear() + .append(...) between message
    // boundaries — no per-frame allocation.
  }
  // No dissolve() needed for rx_buf / last_msg — the cascade
  // fires their dissolve when WsClient itself dissolves.
}
```

Consumer reads `self.last_msg` via a contract that exposes
`b.view()` — zero-copy across the F.14 interface.

## Linear byte scanning — the `String` cost model

A `String` is a NUL-terminated blob with no stored length, and
the operations a hand-written parser reaches for first are the
O(n) ones. The costs below are the shipped ones, measured on a
1 MiB input (GH #720):

| operation | cost | notes |
| --- | --- | --- |
| `len(s)` | O(n) — `strlen` | The call is `memory(read)`, so LLVM hoists it out of a loop whose body writes no memory. Put an allocating call in that body — a slice, a concat, a builder append — and the hoist fails and every iteration re-walks the input. Measured: the same 1 MiB slice loop took 13.2 s with `len(s)` in the condition and 7.7 s with the length in a local. |
| `s[lo..hi]`, `std::str::substring(s, lo, hi)` | O(n) + O(hi-lo) | The clamp does its own `strlen`, so the cost does not depend on how few bytes were asked for. One-byte slices in a bounded loop are the quadratic shape: 1 Mi of them cost 7.7 s. |
| `std::str::byte_at_unchecked(s, i)` | O(1) — one load | No bounds check at all. Correct only while the caller's bound is; an out-of-range `i` is an out-of-bounds read. |
| `std::str::bytes_view(s)` | O(n), once | One `strlen` for `n`, plus one copy of the text into the caller's arena (a `String` field of a returned struct is deep-copied at the fn boundary). Hoist it out of the loop that uses it. |
| `std::str::byte_at(v, i)` | O(1) — compare + load | Checked against the view's `n`. |
| `std::str::slice(v, lo, hi)` | O(hi-lo) | Clamped into `[0, n]`; no `strlen`. |
| `std::str::range_copy(s, n, lo, hi)` | O(hi-lo) | What `slice` is built on, for a caller that already tracks its own length. |
| `std::str::index_of_from(s, sub, start)` | O(n) + the search | One `strlen` of `s` per call, to refuse a `start` past the end. A line-at-a-time scan with it is quadratic: 16 MB in 32,000 lines took 4.7 s. |
| `std::str::find(v, sub, start)` | the distance to the hit | `index_of_from` over the view, bounded by its `n`: the same 16 MB scan takes under a millisecond. |
| `std::str::range_index_of(s, n, sub, start)` | the distance to the hit | What `find` is built on; the caller owns the bound. |

So the natural bounded-parser loop is quadratic:

```hale
let mut i = 0;
while i < len(s) {            // hoisted, but see above
    let c = s[i..(i + 1)];    // O(n) EVERY iteration
    i = i + 1;
}
```

and the view is the linear form of the same loop, with the bound
kept in a register and every read checked against it:

```hale
let v = std::str::bytes_view(s);   // one strlen + one copy
let mut i = 0;
while i < v.n {
    let c = std::str::byte_at(v, i);   // 0..255, or -1 if out of range
    if c == 44 { … }                   // 44 = ','
    i = i + 1;
}
```

Measured on the same 1 MiB input: **13.2 s → 29 µs** for the scan
(the loop vectorizes, because the bound is loop-invariant), and
the whole program including construction runs in 2 ms. At 1 / 2 /
4 MiB the scan costs 29 / 57 / 114 µs — linear, pinned by
`byte_view_scan_scales_linearly` in
`crates/hale-codegen/tests/stdlib_str.rs`.

Three properties the surface commits to:

1. **Refusal, not UB.** `byte_at` answers -1 for any `i` outside
   `[0, n)` and `slice` clamps into `[0, n]`, so a truncated frame
   or an attacker-supplied length field yields short data rather
   than a read past the end. A length taken from untrusted input
   needs no validation before it reaches `slice`.
2. **Bytes, not characters.** `n` is a byte count, `byte_at`
   returns one byte, and `slice` can cut a multi-byte sequence in
   half. That is the right default for protocol and format work
   (every delimiter, digit and quote is one byte in UTF-8, and a
   continuation byte is never mistaken for one). When the unit of
   meaning is a character, walk with `std::str::cp_at` /
   `cp_size`, which refuse mid-sequence offsets with -1.
3. **The view's `n` is the bound.** It comes from `bytes_view`,
   which is why the checks can be one compare. A view fabricated
   by hand with an `n` larger than its text is the one way to get
   past them (the `byte_at` byte-is-NUL guard still stops a
   sequential scan at the real end).

`std::str::byte_at_unchecked` stays for the case where the bound
is already proven by the surrounding scan — the stdlib JSON
range-walkers use it — and keeps its "misuse is UB" contract.

## `~~std::panic~~` — not a thing

Hale doesn't have `panic(msg)`, `assert(cond)`, or any other
value-level "bail from this function" primitive. Failure is
structural, not parametric:

1. Declare a **closure** in the relevant locus asserting the
   invariant you want enforced.
2. When the assertion fails at the closure's epoch, the runtime
   constructs a `ClosureViolation` and routes it to the parent's
   `on_failure` handler per F.9.
3. The parent picks one of `restart` / `restart_in_place` /
   `quarantine` / `reorganize` / `bubble`, or absorbs the
   violation by returning without calling any of them.
4. A violation that bubbles past `main` exits the process
   non-zero with the violation report on stderr.

That covers every legitimate use of `panic`. "Impossible state"
becomes "a closure asserting state is possible." "Bail from this
function" is a category error in Hale — functions return
values; failure lives at the locus level.

Writing `panic("…")` anyway is an ordinary unbound callee: `hale
check <directory>` reports ``call to `panic`: no free fn, generic
fn or fn-pointer binding with that name is in scope`` at the call.
Until GH #800 the name sat in the checker's bare-builtin exemption
table, so `check` accepted it and only `hale build` objected, at
lowering, without a source location.

## Form-synthesized error types

Beyond the explicit `std::*` namespace, the resolver injects
form-specific error payload types into the top scope when any
locus in the bundle uses the corresponding form. These behave
like ordinary user types after injection — they can be the
target of `fallible(...)`, declared as fn parameters / fields,
or pattern-matched in `match`. They are NOT importable via
`std::*` (they're not in a namespace); their names live at the
top level.

| Form / source | Synthesized type | Fields |
|---|---|---|
| `@form(vec)` | `IndexError` | `kind: String`, `index: Int`, `len: Int` |
| `@form(hashmap)` | `KeyError` | `kind: String` |
| `@form(ring_buffer)` | `EmptyError` | `kind: String` |
| `std::io::fs` / `std::io::tcp` | `IoError` | `kind: String`, `errno: Int`, `path: String`, `detail: String` |
| `std::str::parse_int` / `parse_float` / `parse_decimal`, `std::hex::decode`, `std::decimal::from_float` | `ParseError` | `kind: String`, `input: String` |

Idempotency: if a user / library declares a type with the same
name, the user declaration wins. The injection only runs if the
target name isn't already in scope.

### `IoError`

`std::io::fs::*` (except `file_exists`) and the path-call surface
of `std::io::tcp::*` (`listen_socket`, `connect`, `accept_one`)
return `fallible(IoError)`. Agents address failures uniformly:

```hale
let s = std::io::fs::read_file(path) or raise;
let n = std::io::fs::file_size(path) or 0;
std::io::fs::mkdir(out_dir) or show(err);
```

The `kind` tag is errno-derived — `"not_found"`,
`"permission_denied"`, `"is_dir"`, `"already_exists"`,
`"would_block"`, `"connection_refused"`, `"timeout"`,
`"host_unreachable"`, `"broken_pipe"`, `"interrupted"`, etc.,
with `"io"` as the catch-all for unmapped codes. `errno` carries
the raw platform errno for callers that want it; `path` carries
the file path / connection target / empty string for socket-fd
ops without a useful path label; `detail` (2026-10-12, downstream
handoff: `HttpError` and `DbError` carried a human text and
`IoError` did not) carries the human text — the platform's message
for the errno (`strerror`: `"No such file or directory"`,
`"Connection refused"`), `"errno N"` where the platform has none, or
what failed where there is no errno (`std::io::udp::Reader.recv`'s
timeout and failure) — filled at every construction site, the
fallible path-calls' and the `Stream` / `send_fd` / `Reader`
bodies'. Its wording is the platform's: branch on `kind`, show
`detail`. It is declared `detail: String = ""`, so a program's own
`fail IoError { kind, errno, path }` still builds.

`Stream.send` / `Stream.send_bytes` / `Stream.recv` /
`Stream.recv_bytes` are *locus methods* on
`std::io::tcp::Stream`, all `fallible(IoError)` since
2026-07-15 (#209; they previously used a legacy -1/0-sentinel
shape predating open-question #24 lifting the "no fallible on
locus methods" restriction). Semantics:
`send(msg: String)` / `send_bytes(b: Bytes)` succeed with Unit
(the old Int return was only ever a 0/-1 status), so
fire-and-forget callers write `s.send(x) or discard;` and
strict ones `or raise`. `recv(max) -> String` /
`recv_bytes(max) -> Bytes` **fail only on a genuine I/O
error** (connection reset, broken pipe, bad fd, …): a clean
EOF and a `set_recv_timeout` expiry both return the empty
value as before — timeout is a liveness signal, not an error.
The error path is built from the errno the primitive records
in a thread-local (`__last_io_status` / `__io_error_kind`, the
same taxonomy the fallible path-calls use). `IoError` itself
is now declared in the stdlib seed (`io_tcp.hl`), so user code
can construct / `fail` it directly.
`std::io::stdin::read_line` keeps its sentinel shape (path-call
pairing with `read_line_status` for EOF-vs-error distinction).

**`Stream` fd ownership** (`owns_fd: Bool = true`).
By default a `Stream` *owns* its `conn_fd` and closes it on
dissolve — the contract the `Listener` / `http::Server`
accept-loop helpers rely on for per-connection cleanup
(`__handle_one_connection` wraps the accepted fd in a Stream
whose scope-exit dissolve closes it). Set `owns_fd: false` to
*borrow* an fd owned elsewhere: a transient
`Stream { conn_fd: self.conn_fd, owns_fd: false }` built only to
call `send`/`recv` against a long-lived connection. A borrowed
Stream's dissolve is a no-op (no `__close_fd`, no `close`
LogEvent), so building one per operation against a persistent
connection — e.g. a WebSocket conn locus that wraps its fd per
frame — no longer closes the shared fd out from under the next
operation. Owning a fd from two live Streams at once is still a
double-close bug; `owns_fd: false` is precisely the opt-out for
the borrow case.

### `ParseError`

`std::str::parse_int(s)` / `parse_float(s)` / `parse_decimal(s)`
return `fallible(ParseError)`. The non-fallible siblings exist
only as `can_parse_*` predicate spellings; every parsing call
site must address the failure with `or`. `ParseError` carries:

- `kind: String` — `"empty"` (s was `""`), `"trailing_chars"`
  (s parsed a prefix and had junk after), `"invalid"` (no
  leading numeric prefix), `"overflow"` (`parse_int` only —
  magnitude exceeds Int range).
  `parse_float` reads every double's text, a subnormal (`5e-324`)
  included; a magnitude past the largest double, or one that rounds
  to zero, is refused.
- `input: String` — the original `s` (truncated to a reasonable
  preview if very long), for diagnostic surfaces.

```hale
let n = std::str::parse_int(s) or 0;
let n = std::str::parse_int(s) or raise;
let n = std::str::parse_int(s) or self.report(err);
```

Reach for the predicate sibling `can_parse_int(s) -> Bool` only
when you genuinely want to branch *before* parsing. In most
cases `or` is shorter.

The qualified form `std::str::ParseError` resolves to the same
struct as bare `ParseError` — useful in projects that also
declare a local error type with the same name.

## `std::process::rss_bytes()` — observability

Returns the calling process's **peak** resident-set size in
bytes via `getrusage(RUSAGE_SELF)`. There's no syscall for
*current* RSS that doesn't go through `/proc/self/statm`; for
alarm thresholds peak is usually what matters anyway. Returns
0 if `getrusage` rejects (vanishingly rare). On Linux the
underlying value is reported in KiB and we multiply by 1024.

```hale
let bytes = std::process::rss_bytes();
println("rss=", to_string(bytes));
```

**A spawned process inherits its parent's peak.** `execve` does
not reset `ru_maxrss`: `fork` hands the child a copy-on-write
duplicate of the parent's address space, so the pre-exec child's
RSS high-water mark *is* the parent's RSS, and the exec folds that
mark into the new image's `ru_maxrss` for the life of the process.
A program started by a large parent therefore reports at least
that parent's RSS forever after — measured, one unmodified binary
reads 4.7 MB standalone and 411 MB when spawned from a parent
holding 400 MB. `rss_bytes()` is a peak for a process that owns
its own history (a service started by an init system, a CLI run
from a shell); it is not a way to measure a child you spawned, and
a supervised worker's reading carries its supervisor's footprint.

For the *current* RSS — and for any measurement that must be the
program's own — parse `/proc/self/statm`'s line one field two via
`read_file` (size-tolerant for synthesized files); that file is
read from the post-exec address space and carries none of the
inherited history. Both surfaces ship; pick by use case (peak for
alarms on a top-level process, current for heartbeat gauges and
for anything spawned).

## Stopping a server — `std::io::tcp::stop(name)` and `Server.shutdown()`

A listener — a `std::http::Server`, a `std::io::tcp::Acceptor` or a
`std::io::tcp::Listener` — is stopped by shutting its listening socket
down (`shutdown(SHUT_RDWR)`): its blocked or parked `accept` returns an
error, the accept loop ends, and `dissolve()` does the `close()`. The wake has to be the syscall: a
`pinned` listener blocked in `accept(2)` services no mailbox, so a stop
delivered over the bus would wait for the next connection.

`std::io::tcp::stop(name: String) -> Bool` stops the listener the
process knows by `name` (its `name` param; a listener enters its name
when it is born and leaves when its socket is closed, 64 listeners a
process). **A listener's name is unique.** A listener binds in its
`birth()` and then takes its name: the `name` it was given, or, left
empty, the host and the port the socket is actually bound to
(`127.0.0.1:<port>`, the kernel's pick for `port: 0`, so two `port: 0`
servers are two names and two routes). The name it goes by is its
`listener` field. An explicit `name` another live listener holds is
the listener's `name_taken` violation at birth, with its socket closed
before anything runs. A listener whose birth failed holds no
subscription that matches a live route: a Server builds its consumer
in `birth()`, on the route of its final name, only once the name is
its own, and its relay closes nothing until then — so a refused server
can never shadow another listener's catch-unmatched close. Because
birth binds, `restart(server)` binds afresh (a supervisor that
corrected `host`, a port that came free), closing the socket a
previous birth left. It is a free call, pool-neutral, so
**a checked program stops a listener on another pool with it** — where
a method call across pools is refused (F.31's single-threaded-method
invariant, `spec/types.md`). `true` when a listener had the name:

```hale
main locus App {
    params {
        web: std::http::Server = std::http::Server { port: 8080, name: "web", handler: Routes { } };
    }
    placement { web: cooperative(pool = web) where async_io; }
    run() {
        while !self.draining { std::time::sleep(20ms); }
        std::io::tcp::stop("web");
    }
}
```

`Server.shutdown()` does the same from the Server's own thread (a
locus on its pool, or a test harness that runs no checker).

The accept loop treats any negative `accept_one` return as a
clean shutdown signal, so even degenerate fd closes (external
fd-closes, etc.) terminate gracefully.

**Shutdown is a drain, never a cut.** Once the Server's accept loop
ends — `stop`, `shutdown()`, `max_accepts` reached, or the process's
drain on a SIGINT / SIGTERM, which stops every named listener
(`spec/semantics.md` § Drain cascade; `main`'s `run()` returning does
not) — its `run()`
returns only after every connection it accepted has been answered, so
`dissolve()` never tears down a connection mid-request. Inline (one at
a time) that is the connection being served; on an `async_io` pool it
is every connection in flight, each finishing its handler and its
write. A connection is bounded by its 5 s recv timeout while it reads
and by the handler's own time after that. The drain is the Server's: a
program that ends (its main locus's `run()` returns, the pool workers
are joined) while connections are still in flight ends them with the
process, unanswered — wait for them before returning when they must be
answered. (Under SIGTERM the process drain also ends the sleeps a
handler was in when it began, `spec/runtime.md`, so a sleeping handler
answers early.)

## Server concurrency — a coroutine per connection on `async_io`

`std::http::Server` is a listener and a consumer of the TCP connection
substrate (below), both in the Server's own placement. It serves
concurrently exactly where a blocking call parks rather than holds the
thread: on a cooperative pool declared `where async_io`.

```hale
main locus App {
    params {
        web: std::http::Server = std::http::Server { port: 8080, handler: Routes { } };
    }
    placement { web: cooperative(pool = web) where async_io; }
}
```

There the accept loop parks on the listening socket between
connections and relays each one to the Server's consumer, which serves
it in a child of its own (an accept'd flow child) whose `run()` reads
the request, calls the handler and writes the response. A park anywhere
in a connection — the request's recv, a `std::time::sleep`, a socket
read or an outbound plain-HTTP call in the handler, a send to a slow
client — yields the pool's worker to the other connections and to the
accept loop. Five concurrent requests to a handler that sleeps a second
take one second, not five; fifty take one second. At most
`max_connections` (default 256) are served at once; the next connection
waits for a slot (a parked coroutine holding its descriptor), as many
again may wait, and past that a connection is closed unserved — which
bounds the descriptors and the 64 KiB coroutine stacks a burst holds.
Each connection's region is reclaimed as its `run()` returns.

Anywhere else — `fn main()`'s thread, the main pool, a `pinned`
thread, a classic cooperative pool — a blocking call holds the thread,
so the Server serves one connection at a time, inline, as it always
has: a handler that takes a minute holds every other request for that
minute. (On main and a pinned thread the accept loop yields after each
relay, which runs the consumer's delivery on the same thread before the
next accept; on a classic pool, which runs its queued cells only between
cells, the Server hands the connection to its consumer directly.)
Concurrency is not the default because it cannot be: the main pool
cannot be `async_io`, placement is written only on the main locus, and
the Server's consumer is a param of the Server, so it inherits the
Server's thread like any nested locus (`spec/runtime.md` §
"Nested-instantiation inheritance") — making the consumer a separate
locus does not let the statement form `std::http::Server { … };` in
`fn main()` place it elsewhere, and moving the user's handler to a
thread its program did not name would change what that handler may
assume. macOS and musl targets have no `async_io` pools at all. The one
switch is the Server's placement entry; a program that wants the
listener and the serving apart places a `std::io::tcp::Acceptor` and
`std::io::tcp::Connections` itself (below).

**The handler's state.** Every connection calls the same handler locus.
On an `async_io` pool its `handle` runs inside the connection's
coroutine, on the pool's one worker thread (`spec/runtime.md` § "One
worker thread per named cooperative pool"), so two calls never run in
parallel: each runs uninterrupted until it parks, and while it is
parked another connection's `handle` may run and change the handler's
`params`. A value a handler read before a park may be stale after it;
state that must hold across a park is re-read, or kept in the request's
own locals. Off `async_io` every call runs to completion before the
next begins. Either way the handler's code runs on the consumer's
thread — the Server's; a handler locus that is a field of a locus on
another pool is entered from that thread, not its own.

## Streamed responses — chunked bodies and Server-Sent Events

A handler that answers before its body is complete — a token stream, a
progress feed, an event push — streams it in chunked transfer encoding
(RFC 9112 § 7.1) instead of returning it whole. It builds a writer on
the connection inside `handle` and returns the writer's `end()`
(2026-10-17):

```hale
locus Feed {
    fn handle(req: std::http::Request) -> std::http::Response {
        let w = std::http::SseWriter { conn_fd: req.conn_fd };
        w.event(std::http::SseEvent { event: "tick", id: "1", data: "one" }) or discard;
        w.event(std::http::SseEvent { event: "tick", id: "2", data: "two" }) or discard;
        return w.end();
    }
}
```

- **`std::http::BodyWriter { conn_fd, status = 200, content_type =
  "application/octet-stream", headers = "" }`** — a chunked body. Its
  birth sends the head at once: the status line, `Content-Type`,
  `Transfer-Encoding: chunked`, `Connection: close`, then `headers`
  (CRLF-joined, no trailing CRLF, as `Response.headers`) and the blank
  line. `chunk(data: String) -> () fallible(IoError)` sends `data` as
  one chunk (its size in hex, CRLF, the bytes, CRLF) in one `write(2)`:
  nothing is buffered in user space, so the chunk is on the wire when
  the call returns. An empty `data` sends nothing, since a zero-length
  chunk ends a body. A failure is a genuine I/O error (the client gone,
  `EPIPE`); the handler stops and returns `end()`. `end() -> Response`
  answers `Response { status, content_type, body: "", streamed: true }`.
  The writer borrows the connection (`Stream { owns_fd: false }`) and
  never closes it.
- **`std::http::SseWriter { conn_fd, status = 200, headers = "" }`** —
  Server-Sent Events (the WHATWG `text/event-stream` format) over a
  `BodyWriter` with `content_type: "text/event-stream"` and the header
  `Cache-Control: no-cache` before `headers`. `event(e: SseEvent) -> ()
  fallible(IoError)` sends one event as one chunk in one write: the
  fields `event: <event>` (when non-empty), `id: <id>` (when non-empty)
  and `retry: <ms>` (when `retry` is at least a millisecond, in whole
  milliseconds), one `data: <line>` per line of `data` (CRLF, CR and LF
  each end a line; an empty `data` is one empty `data:` line, so the
  event is still dispatched), and the blank line that dispatches it. A
  line break in `event` or `id` is dropped. `end()` is the body's.
- **`std::http::SseEvent { data: String, event = "", id = "", retry:
  Duration = 0ns }`** — one event.
- **`Response.streamed: Bool = false`** — set by `end()`, never by hand.
  The Server, seeing it, writes only the last chunk (`0\r\n\r\n`) and
  closes the connection as it does after every response; `status`,
  `content_type`, `headers` and `body` are not written again. The body
  ends when the handler returns, however it returns that `Response`; a
  handler that builds a writer and then returns any other `Response`
  writes a second response into the stream, which is the handler's bug.

A handler that builds no writer is unchanged: the Server tests one more
`Bool` per response, and the wire bytes of a `Content-Length` response
are the same to the byte. The writers are ordinary stdlib loci, so a
program that names none carries none of their code.

**Placement.** A streaming handler holds its connection for as long as
it streams, so it is served where the Server is placed, as every
handler is (§ Server concurrency). On a `where async_io` pool each
connection is its own coroutine: a `std::time::sleep` between events,
an outbound call that feeds the stream, or a send to a slow client parks
only that connection's coroutine, and the Server keeps accepting and
answering other connections; a stream counts against `max_connections`
while it is open. Anywhere else the Server serves one connection at a
time, so a stream holds every other request until it ends, as a handler
that sleeps does there. A client that stops reading without closing
holds a send until the socket's send timeout, which is unset by default:
a stream to untrusted clients sets one first
(`std::io::tcp::set_send_timeout(req.conn_fd, …)`, § `std::io::tcp`).

Not provided: SSE comment lines (`: keep-alive`), a binary-safe
`chunk_bytes`, trailers, and request bodies sent chunked (the Server
reads `Content-Length` bodies only).

## The TCP connection substrate

Every server has the same shape, and the substrate writes it once
(`crates/hale-stdlib/hl/io_tcp_serve.hl`): a **listener** that only
binds, accepts and relays each connection over the bus as an owned
handle, and **consumers** that take a handle and serve its connection.
The syscalls are stitched by placement: a listener placed `pinned`
blocks in `accept(2)` on its own thread, one on a `where async_io` pool
parks there; a consumer on an `async_io` pool serves each connection on
a coroutine of its own, and anywhere else one at a time.

- **`std::io::tcp::Acceptor { host, port, path, name, shards,
  max_accepts, ready_signal }`** — the listener. Binds at birth (a socket
  it cannot bind is its `listen_failed` violation; a restart binds
  afresh), enters its name (`name`, or `host:<bound port>`, or
  `unix:<path>`; one taken is `name_taken`) for `stop`,
  prints `ready_signal`, and accepts until
  stopped or `max_accepts`, relaying each connection. It serves
  nothing. **Two address families, one substrate:** with `path` set it
  binds a Unix stream socket there in place of `host:port` (a stale
  socket file is replaced, a path a live process holds is
  `listen_failed`) and removes the file when it dissolves — only while
  the path still names the file it bound (`std::io::unix::__release_path`):
  a listener stopped by name keeps its fd until it goes, a shut Unix
  socket refuses connects, so a replacement may take the path as stale
  and bind its own file there, and the old listener's cleanup leaves that
  file alone; accept, read,
  write and close are the same calls for both, so the relay, the handle
  and the consumers do not know which family a connection is. A failed
  accept nobody asked for (out of descriptors, a connection that failed
  between the handshake and the accept: `__accept_one`'s `-2`) ends that
  connection and never the listener, which accepts again 20 ms later; a
  requested wake (`stop`, `shutdown()`, the pool's shutdown) is `-1` and
  ends the loop. `shards: K` (default 1) deals connections round-robin over K
  routes. Place it `pinned` or `where async_io`; on a classic pool its
  accept holds the pool, and consumers on that pool are never served.
  It is on the checker's long-running list, like the Server.
- **`std::io::tcp::Accepted { listener, route, fd, seq }`** — the
  handle: the listener's name, the route it was sent on
  (`<listener>#<shard>`), the descriptor and the listener's count from
  1. **Whoever takes it owns `fd`** and closes it. The handle itself
  lives for its delivery: a long-lived consumer keeps nothing of the
  connections it takes (spec/memory.md § "Every wire delivery takes
  the wire cell"; it used to keep the two names, ~34 B a
  connection).
- **`__TcpAcceptedT`** — the substrate's topic, on the wire subject
  `__tcp.accepted`, `keyed_by route`, `on_unmatched: fallback`. A
  consumer subscribes `where key == <its route>`; every listener holds
  the topic's catch-unmatched subscription (`where key == _`) and
  closes a handle that is its own. **An accepted connection is served
  or closed — never leaked, never dropped in silence**, so a program
  may state it: `require no_silent_loss(topic __TcpAcceptedT)` holds
  (`spec/verification.md`), and `cover keys` is not available (the key
  is a String).
- **`std::io::tcp::ConnHandler`** — `fn serve(c: std::io::tcp::Accepted)`:
  what serves one connection, on its consumer's thread.
- **`std::io::tcp::Connections { listener, shard, handler,
  max_connections }`** — the consumer: subscribes to the route
  `<listener>#<shard>` and serves each connection with `handler` in a
  child of its own — a flow child on an `async_io` pool (concurrent,
  bounded and drained as above, reclaimed as its `run()` returns), an
  inline child anywhere else (one at a time). `refused` counts the
  connections closed unserved past its bound. Two consumers on one
  route would both take every handle, so routes are one consumer each:
  replicas share a listener through its shards.
- **`std::http::Responder { handler }`** — the HTTP `ConnHandler`: reads
  one request (reassembled, 1 MiB cap, 5 s recv timeout), asks
  `handler`, writes the response (takeover included; for a streamed
  body, only its last chunk).
- **`std::io::tcp::stop(name)`** — above.
- **`std::io::tcp::Listener { host, port, max_accepts, on_connection,
  max_connections }`** — the m83 listener, on the substrate: one locus
  holding both ends, as the Server does, whose consumer's handler calls
  `on_connection: fn(std::io::tcp::Stream)` with a `Stream` over each
  connection and closes the connection when the call returns.
  `max_accepts` (default 1; negative = until stopped) bounds the accepts,
  and `run()` returns once each accepted connection has been served.
  Placed `where async_io` it serves its connections concurrently, a
  coroutine each (at most `max_connections`, 256); anywhere else one at a
  time. Its name is its bound address (`listener`, `host:<bound port>`),
  for `stop`. A socket it cannot bind is no violation, as before the
  substrate: it accepts nothing and `run()` returns. Birth and dissolve
  print `__StdIoTcpListener.birth host=… port=…` and
  `__StdIoTcpListener.dissolve host=… port=…`; the default
  `on_connection` prints `__default_on_connection fd=<n>`.

Two replicas behind one pinned listener, each on an `async_io` pool of
its own, the listener stopped by name from the main locus:

```hale
main locus App {
    params {
        zero: std::io::tcp::Connections = std::io::tcp::Connections {
            listener: "web",
            shard: 0,
            handler: std::http::Responder { handler: Api { } },
        };
        one: std::io::tcp::Connections = std::io::tcp::Connections {
            listener: "web",
            shard: 1,
            handler: std::http::Responder { handler: Api { } },
        };
        acceptor: std::io::tcp::Acceptor = std::io::tcp::Acceptor {
            port: 8080,
            name: "web",
            shards: 2,
        };
    }
    placement {
        zero: cooperative(pool = zero) where async_io;
        one: cooperative(pool = one) where async_io;
        acceptor: pinned;
    }
    claims { handed_over: require no_silent_loss(topic __TcpAcceptedT); }
    run() {
        while !self.draining { std::time::sleep(20ms); }
        std::io::tcp::stop("web");
    }
}
```

Consumers are declared before the listener, so they are subscribed
before it accepts. `std::http::Server` is one locus that holds both
ends: its `relay` (the listener's end of the bus) and its `conns` (a
`Connections` on its own route, `handler: std::http::Responder { … }`).

**The api runtime's socket transports are substrate listeners** (spec/api.md
§ The Unix JSON transport, § The HTTP transport, § gRPC,
§ Streams). `std::api::unix::Listener`, `std::api::http::Listener` (the
latter `mcp::Rpc`'s too), `std::api::ws::Listener` (a `ws::Hub`'s) and
`std::io::h2::Listener` (`grpc::Rpc`'s) hold both ends as the Server does: a `relay` that accepts and relays, and a `conns` on
the route of the listener's name (`unix:<path>`, `host:<bound port>`),
built in `birth()` once the name is entered, whose `ConnHandler` starts
the transport's connection locus (the peer that reads requests, publishes
them as the exposure's ingress and writes the replies; the hub's WebSocket
peer, which upgrades the connection and carries its subscriptions and
calls; the HTTP/2 `Conn`, which owns the connection's nghttp2 session) as
a flow child of its own — accepted and released, on the listener's
`async_io` pool, and reclaimed with its region when the connection ends. A
per-connection locus that subscribes to the bus has to be a flow child
rather than a local of `serve`: a locus born in a method body and gone at
its end has its deliveries queued for the main thread, which would answer
the connection from another thread than the one reading it. What is the
api's own is the policy around the substrate: an address it cannot bind
or a name it cannot enter is the boot's failure (`api: <transport> could
not listen on <address>`, exit 2), nothing is accepted until the exposure
(the hub's core, the gRPC transport) is subscribed, and the exposure's
`stop()` shuts the socket (the accept returns and the listener releases
it, the Unix socket file with it) and ends its connections as its
transport does (a hang-up; the hub's `closed` frame; HTTP/2's `GOAWAY`
once the streams in flight are answered). Every api listener closes a
handle on its own name's route that no consumer takes, and `stop(name)`
reaches it: the socket is shut and released, and the connections it
accepted are served on until they end or the exposure stops (a drain,
never a cut).

The substrate is source the program carries only when it names it:
`std::http::Server`, `std::http::Responder` or one of
`std::io::tcp::{Listener, Acceptor, Connections, Accepted, ConnHandler}`
appends it (and its topic) to the program before the check, as the api
runtime is (`hale_stdlib::TCP_SERVE_SOURCE`), and
`std::io::tcp::Listener` appends the Listener after it
(`hale_stdlib::TCP_LISTENER_SOURCE`, `io_tcp_listener.hl`); every other
program carries none of it. It is stdlib source in every judgment: its
loci start where the program names them, a scope of it holds dynamic
sites in the placement table only once the program reaches it, the
cooperative-pool blocking advisory does not read its bodies, the
resource budget counts none of its descriptor sites, and a capability
it needs is refused at the program's literal (`--target wasm32`), with
the witness down into it.

**Not yet.** The peer address: `accept` is called without one, so a
consumer cannot be keyed by peer. The api listeners' consumers start
each connection's locus and return, so `max_connections` does not
bound them: an api listener's connections are as many as clients
open, as they were before the move.

## `Server.ready_signal` — synchronization for piped oracles

`std::http::Server` accepts an optional `ready_signal: String = ""`
param. When non-empty, the server emits it via `println` from
`birth()` immediately after `listen_socket` succeeds and before
the accept loop begins. Test harnesses, oracles, and shell
scripts that pipe the server's stdout (`./bin | grep -m1 READY`)
key off this line:

```hale
std::http::Server {
    port: 8080,
    handler: Routes { },
    ready_signal: "READY"
};
```

Pair with the line-buffered stdout setup (the prelude installs
`setvbuf(stdout, NULL, _IOLBF, 0)`) so a single `println` is
flushed even under pipes.

## `std::json::Builder` — streaming output API

`Builder` is a `@form(...)`-less locus that accumulates a JSON
document into an internal buffer. It tracks scope state in a
single context stack (one char per open scope: `O`/`A` for
object/array with at least one value already emitted, `o`/`a`
for empty). The Builder inserts separators (`,` between
siblings, `:` between key and value) automatically.

Methods, grouped:

- **Scopes:** `begin_object()`, `end_object()`,
  `begin_array()`, `end_array()`.
- **Object entries (key + value in one call):** `field(name,
  value)` for the common string case; `string_field`,
  `int_field`, `bool_field`, `null_field` for explicit typing.
- **Array entries / bare values:** `value(v)` (string), plus
  `string_value`, `int_value`, `bool_value`, `null_value`.
- **Nested scopes inside an object:** `begin_object_field(name)`
  / `begin_array_field(name)` — emit `"name":` then open the
  sub-scope.
- **Finish:** `result() -> String` returns the assembled buffer.

```hale
let b = std::json::Builder { };
b.begin_object();
b.field("name", "alice");
b.int_field("age", 30);
b.begin_array_field("tags");
b.value("admin"); b.value("ops");
b.end_array();
b.end_object();
let out = b.result();   // {"name":"alice","age":30,"tags":["admin","ops"]}
```

The flat-object readers (`find_*_field`, `array_first/next`)
are the input side of the same v1 commitment: JSON is a wire
format, not a tree value type, and the API surface reflects
that.

### String escaping — `escape_string` / `unescape_string`

`escape_string(s)` emits RFC 8259 §7: `\\` and `\"`, the four
short forms `\b \f \n \r \t`, and `\u00XX` for the remaining
control bytes. Every other byte passes through, so UTF-8 in the
input is UTF-8 in the output.

`unescape_string(s)` is the inverse, and decodes `\uHHHH` to
the scalar it names (GH #709 — before 2026-09-19 every `\uHHHH`
decoded to `?`, so an escaped literal never compared equal to
its plain form):

| input | result |
| --- | --- |
| a scalar, `\u0031` / `\u00e9` / `\u4e2d` | its UTF-8 encoding, 1–3 bytes |
| a surrogate pair, `\ud83d\ude00` | the astral scalar it denotes, 4 bytes |
| an unpaired surrogate, `\ud83d` or `\udc00` | U+FFFD |
| `\u0000` | U+FFFD |
| four digits that are not all hex, `\uZZZZ` | the six source bytes, verbatim |
| any other unknown escape, `\q` | the two source bytes, verbatim |

The two refusals are deliberate. A native `String` is
NUL-terminated, so U+0000 has no representation in one: writing
the raw byte would truncate the value and let a prefix compare
equal to the whole, which is worse than a visibly wrong
character. An unpaired surrogate is not a Unicode scalar and
has no UTF-8 encoding at all. Both answer U+FFFD rather than
failing, because these helpers read untrusted wire bytes and
the rest of the document is still worth decoding.

### Bounded memory

`escape_string`, `unescape_string` and every `Builder` emitter
accumulate into one growing byte buffer and materialize a
`String` once, so each is linear in the bytes it produces and
the transient cost is bounded by the output size (GH #708).

This is a contract, not an optimization: a `String` is
immutable, so the earlier `out = out + piece` form copied the
whole accumulated prefix per write AND left every prefix behind
in arena storage until the call returned — quadratic in both
time and retained memory. A downstream handoff generating a
document with a 600,000-byte shared value and 512 references
reached 23–24 GiB of scratch and exhausted swap.

`result()` copies the buffer, so a snapshot taken mid-document
is a stable `String`: the Builder keeps accumulating after it,
and the earlier snapshot does not change.

### Validation — `valid` / `valid_object`

`valid(text)` is true when `text` is one well-formed RFC 8259
value and nothing else: objects, arrays, strings, numbers,
`true` / `false` / `null`, with JSON whitespace (space, tab, LF,
CR) permitted around every token. Leading and trailing
whitespace is fine; trailing anything else is not, and `""` is
not a document. The number rules are the grammar's, so `01`,
`1.`, `.5`, `1e` and `+1` are all refused while `-0`, `1e5` and
`-1.25e-3` are values.

`valid_object(text)` is `valid` plus: the top-level value is an
object, its keys are unique, and none of them is written with an
escape. That is the shape a gate in front of a record wants. A
duplicate key is syntactically valid JSON — last-wins is a
convention, not the spec — so `valid` accepts it and two readers
of the same bytes can disagree about the record; an escaped key
spelling (`"\u0061"` for `"a"`) compares unequal to the plain
spelling in every `find_*` lookup in this module, so admitting
one lets a field be present and unfindable at the same time.
Keys of NESTED objects are checked for syntax only.

Three refusals go past the grammar. Each is a value this runtime
cannot carry, and each is the same call the escape helpers make:

| refused | why |
| --- | --- |
| text that is not well-formed UTF-8 | including a surrogate encoded directly as three bytes (ED A0..BF) and anything above U+10FFFF |
| a lone `\uXXXX` surrogate | denotes no scalar; `unescape_string` answers U+FFFD for it, and an admission gate should refuse rather than hand on a replacement character |
| `\u0000` | a native `String` is NUL-terminated, so U+0000 would truncate the value and let a prefix compare equal to the whole |

`valid` is linear in the input. Both are **bounded**:

- **Depth 64.** Nesting deeper than that is refused rather than
  scanned, so a hostile `[[[[…` costs a constant walk. The walk
  is iterative — one `Int` holds a bit per open level saying
  whether that level is an object — and 64 is where one machine
  word of that mask runs out. The bound is also the reason the
  scan is not a recursive descent: a coroutine stack is 64 KiB,
  and 64 frames of one is a real fraction of it.
- **64 top-level fields, for `valid_object` only.** Uniqueness is
  decided by walking the object's earlier members and comparing
  keys where they already live, so the check needs no table and
  no accumulator — and the width bound is what keeps that walk
  from being the quadratic cost it would otherwise become
  (uniqueness is at most 64 passes over the object). A wider
  object answers false. `valid` has no width bound, so it is
  still checkable as JSON — only the unique-key guarantee stops
  at 64, and a caller that needs both on a wider object can
  `valid` it and walk the keys with `object_first` /
  `obj_key_eq`.

Both are classified **pure**, and both allocate nothing at all:
bytes are read with `std::str::byte_at_unchecked` after explicit
bounds checks, whitespace runs skip through the SIMD
`next_non_ws`, and nothing intermediate is materialized — no
substring, no key set, no accumulator. That is a contract, not an
accident: these fns are meant to run on every inbound message, so
a per-call scratch buffer would land in the caller's arena and be
retained there until the caller returned. Cost is one pass for
UTF-8, one for the grammar, and for `valid_object` up to one per
field over the top-level keys — measured at ~10 ms per MiB,
against 10.6 s for the per-character checked-slice shape
(`text[p..p + 1]`, which allocates per byte) that hand-rolled
validators reach for first (GH #720).

String bodies are deliberately NOT scanned with the sibling
`next_quote_or_bs` jump: it cannot see the raw control bytes
RFC 8259 §7 forbids inside a string, and accepting those would
make `valid` weaker than the validators it exists to replace.

These are the strict counterpart to the `find_*` family, which
stays permissive on purpose (`find_string_field("{\"x\":1",
"x")` answers `1`). Validate at the boundary, then scan.

### Typed field access

`find_string_field(json, name)` answers with a `String` whatever
the value was. That is its contract and it does not change — but
it means eight different documents give one answer, and an
optional field cannot be read safely:

| document | `find_string_field` | `string_field(…).kind` |
| --- | --- | --- |
| `{"owner": "Ada"}` | `"Ada"` | `"string"` |
| `{"owner": null}` | `"null"` | `"null"` |
| `{"owner": "null"}` | `"null"` | `"string"` |
| `{"owner": ""}` | `""` | `"string"` |
| `{}` | `""` | `"missing"` |
| `{"owner": 7}` | `"7"` | `"number"` |
| `{"owner": true}` | `"true"` | `"bool"` |
| `{"owner": []}` | `"[]"` | `"array"` |
| `{"owner": {"a": 1}}` | `{"a": 1}` | `"object"` |
| `[1, 2]` (not an object) | `""` | `"invalid"` |

`string_field(json, name) -> JsonString` is the typed read (GH
#719). `JsonString` is `{ kind: String; text: String; }`:

- `kind` is one of `"string"`, `"null"`, `"missing"`, `"number"`,
  `"bool"`, `"array"`, `"object"`, `"invalid"`.
- `text` is the DECODED string content — surrounding quotes
  stripped, escapes resolved through the same `\uHHHH` path as
  `unescape_string` — and only when `kind` is `"string"`. Every
  other kind carries `""`, so a caller that ignores `kind` gets
  an empty name rather than a plausible wrong one.
- `"missing"` means `json` IS an object with no such member;
  `"invalid"` means `json` is not an object at all, or the
  member's value is not a JSON value the scanner can name (an
  unterminated string, a bare token like `NaN`).
- A key repeated within one object resolves to the FIRST member
  with that name — the same answer `find_string_field` gives,
  since both walk the top-level object cursor and stop at the
  first match.

The reader is not string-specific: an `Int` or `Bool` caller can
gate the coercion on `string_field(json, name).kind == "number"`
/ `== "bool"` before trusting `find_int_field` /
`find_bool_field`, which coerce the same way
`find_string_field` does.

The result types in this namespace — `JsonString`,
`JsonFieldRange`, `ArrayIter`, `ArrayIterSpan`, `ObjectIterSpan` —
are nominal types the CHECKER knows, not opaque values (GH #771).
`string_field(body, "owner").knd` is a type error naming
`std::json::JsonString` with a did-you-mean for `kind`; so are a
wrong arity, a swapped `(json, it)` argument pair on the span
cursors, and handing one of these structs to a `String`
parameter. `let r: std::json::JsonFieldRange = …` is the same
type as the call's inferred one, so the annotation is optional
rather than lossy.

```hale
let owner = std::json::string_field(body, "owner");
if owner.kind == "string" {
    node.adopt(owner.text);
} else if owner.kind == "null" || owner.kind == "missing" {
    // a root: parentless, and NOT a node named `null`
} else {
    return "owner must be a string or null";
}
```

## `read_file` for synthesized files

`std::io::fs::read_file(path)` uses a growing buffer internally
(4 KiB initial, doubling, 64 MiB cap) rather than pre-sizing
from `fstat`. Synthesized files under `/proc` and `/sys` report
`st_size=0` from `fstat`, so a fstat-then-read approach would
return an empty String for `/proc/self/statm`, FIFOs, sockets,
and similar synthetic sources. The growing-buffer shape reads
real bytes from all of them.

The 64 MiB cap is a runaway guard, not a memory budget — real
`/proc` / config files are 4–64 KiB. Callers hitting the cap
probably want a streaming API; the cap surfaces as
`IoError { kind="io", errno=EFBIG }`.

## What's not in stdlib (third-party territory)

- ML / learning libraries
- Database drivers (Postgres, etc.)
- Web frameworks beyond basic HTTP
- Image / audio / video processing
- Cloud SDKs (AWS, GCP, etc.)
- GUI / TUI frameworks
- Cryptography beyond TLS + SHA-2 basics
- Compression formats beyond ones used in stdlib

These live in the Hale package ecosystem (per
[`packages.md`](./packages.md)).

## Open decisions

1. **Module organization** — flat (`std::collections`,
   `std::string`) vs hierarchical (`std::collections::Map`).
   Probably the Go-style middle ground (`std/collections/map.go`).
2. **What's exported by default vs deep-imported.** `import
   std;` for everything? `import std::time;` only? Probably
   the latter: explicit per-module imports.
3. **API stability commitments.** Go's stdlib is famously
   stable. v0 stdlib is unstable; v1 marks specific APIs as
   `stable`; only stable APIs survive long-term.
4. **Versioning.** Stdlib versioned with the language? Or
   independently? Probably with the language for v0; consider
   independent versioning when stable.

## Why batteries-included

- **Lower adoption barrier.** New users don't need to evaluate
  third-party packages for table-stakes functionality.
- **Discipline propagation.** Stdlib uses framework primitives
  correctly; new code following stdlib examples inherits the
  discipline.
- **Ecosystem trust.** When the language ships a `std::crypto`,
  it's vetted; trust transfers to programs that use it.
- **Cross-language consistency.** Programs from different teams
  share the same vocabulary because they share the same stdlib.

Cost: stdlib is permanently load-bearing once shipped. Bad
decisions are hard to undo. Discipline at design time matters
more here than in third-party.
