# Standard library

Bundled with the toolchain, no separate install required. This
document describes the **current** stdlib surface. Milestone /
phase history lives in [`../CHANGELOG.md`](../CHANGELOG.md).

## Path resolution

`.ap` source references stdlib symbols by fully-qualified path:

```aperio
let p = std::process::pid();
let contents = std::io::fs::read_file("config.toml");
std::io::tcp::Listener { host: "127.0.0.1", port: 8080 };
```

The parser tokenizes `::` as a path separator and the type checker
punts namespaced paths to `Ty::Unknown`; the codegen layer
resolves `std::*` paths against a hardcoded namespace dispatcher.

There is **no general module system** at v1 — no `use`
statements, no user-defined modules, no multi-file `.ap`
packages via the std-style mechanism. `std::*` is the only
recognized prefix. Adding a new stdlib function means: declare
its libc backer in `aperio-codegen`'s `declare_builtins`, add a
match arm to `lower_stdlib_path_call_expr` (or its statement
sibling), and implement one `lower_std_*` method.

Cross-binary user code uses the F.25 cross-seed-imports mechanism
(`import "path/to/lib" as alias;`) — distinct from the `std::*`
magic path; see [`design-rationale.md` § F.25](./design-rationale.md).

## Design principles

- **Batteries included.** Go's approach: if a typical Aperio
  program needs it, it ships. A new Aperio user shouldn't need
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

## Module surface

| Namespace | Surface | Source |
|---|---|---|
| `std::process` | `pid() -> Int`, `exit(code: Int)`, `rss_bytes() -> Int`, `dump_arena_residency() -> Int` (no-op unless `LOTUS_ARENA_RESIDENCY=1`; writes per-arena residency snapshot to stderr), `run(argv) -> ProcessOutput fallible(IoError)`, `spawn` / `wait` / `kill` / `write_stdin` / `read_stdout` / `read_stderr` over `Child` | path-call dispatch + C primitives |
| `std::env` | `args_count()`, `arg(i)`, `arg_or(i, default)`, `var(name)`, `var_exists(name)` | path-call dispatch + main-prelude argv stash |
| `std::time` | `monotonic() -> Duration`, `monotonic_ns() -> Int`, `sleep(d: Duration)`, `now() -> Int`, `time_from_unix(n: Int) -> Time` | `clock_gettime` + EINTR-retrying `clock_nanosleep`; `now()` is `CLOCK_REALTIME`; `time_from_unix` formats `gmtime_r` + `strftime` ISO 8601 UTC |
| `std::decimal` | `to_float(d: Decimal) -> Float` | Direct i128 → f64 conversion at scale 9 (`mantissa × 10^-9`) — skips an ASCII round-trip |
| `std::str` | `parse_int(s) -> Int fallible(ParseError)`, `parse_float(s) -> Float fallible(ParseError)`, `parse_decimal(s) -> Decimal fallible(ParseError)`; predicate siblings `can_parse_int` / `can_parse_float` / `can_parse_decimal`; `index_of`, `lower` / `upper`, `trim`, `substring(s, lo, hi)`, `replace`, `repeat`, `pad_left` / `pad_right`, `from_bytes`, `clone(v) -> String` (deep-copy a `StringView` to an owned blob; identity on a `String` for generic callers); `builder_new` / `builder_append` / `builder_len` / `builder_finish` (String-builder primitives — for binary-safe accumulator use `std::bytes::BytesBuilder`) | `lotus_str_*` C runtime primitives |
| `std::bytes` | `at(b, i) -> Int fallible(IndexError)`, `slice(b, lo, hi) -> Bytes`, `from_string(s) -> Bytes`, `from_int(v) -> Bytes`, `concat(a, b) -> Bytes`, `clone(v) -> Bytes` (deep-copy a view to an owned blob). Growing-buffer accumulator surface lives on the `BytesBuilder` locus — see [§ Builders vs Bytes](#builders-vs-bytes--the-recv-loop-pattern) | `lotus_bytes_*` C runtime primitives |
| `std::text` | `md_to_html(md) -> String`, `base64::encode` / `base64::decode`, `Sink` interface + `StdoutSink` / `StringSink` / `FileSink` loci, byte-class predicates (`is_alpha` / `is_digit` / `is_alnum` / `is_whitespace` / `is_word_char`), `tokenize_words_into(s, target_vec)` | `runtime/stdlib/text.ap` + C runtime |
| `std::io::fs` | `read_file`, `write_file`, `write_file_append`, `read_bytes`, `file_size`, `mkdir`, `rename`, `unlink`, `mktemp`, `list_dir`, `list_dir_count`, `list_dir_at` — all `fallible(IoError)`. `file_exists(path) -> Bool` is a predicate (non-fallible). One-shot path-call surface: each call opens, does the op, closes. For held-open shapes use `std::io::file::File`. | `lotus_fs_*` C runtime primitives |
| `std::io::file` | `File` locus (held-open fd with auto-dissolve close). `open(path, mode) -> File fallible(IoError)`; `read_line(f) -> String` (returns "" at EOF or error — pair with `at_eof`); `at_eof(f) -> Bool`; `write_bytes(f, b)`, `write_line(f, s)`, `seek(f, offset)` all `fallible(IoError)`. Mode strings `"r"` / `"w"` / `"a"` / `"r+"` / `"w+"`. Returned Strings live in the bus payload arena. | `lotus_file_*` C primitives + `runtime/stdlib/file.ap` |
| `std::io::stdin` | `read_line() -> String`, `read_line_status() -> Int` (status `-1` = EOF/IO error; `0` = OK including empty-line) | POSIX `getline` + payload-arena copy |
| `std::io::tcp` | `Listener` locus, `Stream` locus, `send`, `send_bytes`, `recv_bytes`, `recv_into(fd, buf: Bytes, max_bytes) -> Int` (caller-provided builder destination). Path-calls `listen_socket`, `connect`, `accept_one` are `fallible(IoError)`. `connect` accepts dotted-quad hosts directly and falls back to hostname resolution via `getaddrinfo(AF_INET)`. | `lotus_tcp_*` C primitives |
| `std::io::udp` | `bind(host, port) -> Int fallible(IoError)` (`host=""` → INADDR_ANY); `send(fd, host, port, msg)`, `recv(fd, max_bytes)`, `recv_into(fd, buf: Bytes, max_bytes)`, `close(fd)`. Datagram boundaries preserved. **NOT a bus transport** — UDP doesn't satisfy the bus's atomic-delivery contract. | `lotus_udp_*` C primitives |
| `std::io::tls` | Client-side TLS via system OpenSSL. `connect(host, port) -> Int fallible(IoError)` does the TCP connection + TLS 1.2+ handshake with SNI + system-trust-store cert verification. `send_bytes` / `recv_bytes` / `recv_into` / `close` over the handshaked connection. Process-global `SSL_CTX` runs with `SSL_MODE_RELEASE_BUFFERS` — OpenSSL releases its read/write buffers between records so long-running TLS clients don't accumulate ~32 KiB per idle connection. The `lotus_tls.c` TU compiles separately so helper tests linking `lotus_arena.c` directly don't drag in libssl/libcrypto. | `lotus_tls_*` in `runtime/lotus_tls.c` |
| `std::http` | `Request` + `Response` types (`Response.headers: String` carries CRLF-joined user-supplied headers — no trailing CRLF — for Set-Cookie / CORS / custom headers); `parse_request`, `write_response`; case-insensitive symmetric `header(receiver, name)` lookup; `Handler` interface (`fn handle(req: Request) -> Response`); `Server` locus with `shutdown()` (cross-thread safe — see [§ Server.shutdown](#servershutdown--interruptible-accept-loop)) and optional `ready_signal: String` for piped oracles. | `runtime/stdlib/http.ap` |
| `std::json` | `Builder` locus (streaming output assembly — see [§ json::Builder](#stdjsonbuilder--streaming-output-api)); `escape_string` / `unescape_string` (RFC 8259); `find_string_field` / `find_int_field` / `find_bool_field` (flat-object lookup); `find_field_raw(json, name) -> String` (bracket-balanced raw substring over nested objects/arrays — the recursive-descent primitive); `ArrayIter` + `array_first` / `array_next`. No nested-tree shape at v1 — re-feed substrings into the same surface for nested walks. | `runtime/stdlib/json.ap` |
| `std::test` | `assert(cond, msg)`, `assert_eq_int`, `assert_eq_str` | `runtime/stdlib/test.ap` |
| `std::log` | `Logger`, `LogEvent`, `StdoutSink` (subscribes `log.**`) | `runtime/stdlib/log.ap` |
| `std::math` | `sqrt`, `exp`, `log`, `floor`, `ceil`, `pow`, `tanh`, `nan`, `is_nan`, `inf` | path-call dispatch into libm (`nan`/`inf`/`is_nan` are IEEE 754 sentinels) |
| `std::crypto` | `sha1(b) -> Bytes` (20-byte), `sha256(b) -> Bytes` (32-byte), `hmac_sha256(key, msg) -> Bytes` (32-byte). All non-fallible. | `lotus_crypto_*` C primitives (no libcrypto link dep) |
| `std::os` | `getrandom(n: Int) -> Bytes fallible(IoError)` (CSPRNG; `getrandom(2)` with `/dev/urandom` fallback) | `lotus_os_getrandom` C primitive |
| `std::bus` | `__StdBusAdapter` interface (contract for user-supplied bus transports — a single `fn send(subject: String, bytes: Bytes)` method); `__local_dispatch(subject, bytes)` primitive lets an adapter relay received wire-bytes into the local handler set. No concrete adapter implementations ship — protocol-layer transports (NATS, MQTT, raw-TCP-with-framing) live downstream. | `runtime/stdlib/bus.ap` |

Aperio doesn't use parametric stdlib collection types (`Map<K,
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
   `append` / `len` / `shift_front` / `clear` / `snapshot` /
   `finish` / `view` / `text_view`) or a `Bytes` (immutable
   length-prefixed blob with functions `at` / `slice` / `len`
   / `concat`). The typechecker enforces this; no implicit
   coercion between them.
2. **Cross between roles via explicit calls.** `BytesBuilder →
   Bytes` is either `b.snapshot()` (copies into the bus
   payload arena — stable across mutations) or `b.view()` (no
   copy, aliases the builder's buffer — valid until the next
   mutation; the right choice for parser passes). `Bytes →
   BytesBuilder` is `let b = std::bytes::BytesBuilder { ... };
   b.append(bytes)` (copies). The Builder → Bytes direction has
   a zero-cost path via `view()`; the reverse does not.
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

```aperio
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

```aperio
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

## `~~std::panic~~` — not a thing

Aperio doesn't have `panic(msg)`, `assert(cond)`, or any other
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
function" is a category error in Aperio — functions return
values; failure lives at the locus level.

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
| `std::io::fs` / `std::io::tcp` | `IoError` | `kind: String`, `errno: Int`, `path: String` |
| `std::str::parse_int` / `parse_float` / `parse_decimal` | `ParseError` | `kind: String`, `input: String` |

Idempotency: if a user / library declares a type with the same
name, the user declaration wins. The injection only runs if the
target name isn't already in scope.

### `IoError`

`std::io::fs::*` (except `file_exists`) and the path-call surface
of `std::io::tcp::*` (`listen_socket`, `connect`, `accept_one`)
return `fallible(IoError)`. Agents address failures uniformly:

```aperio
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
ops without a useful path label.

`Stream.send` / `Stream.recv_bytes` / `Stream.send_bytes` are
*locus methods*, not path-calls, and per the two-channel rule
([`semantics.md` § Fallible call semantics](./semantics.md))
locus methods cannot declare `fallible(E)`. They use the legacy
sentinel shape (returning -1 / 0 on failure). The same is true
of `std::io::stdin::read_line` (path-call but pairs with
`read_line_status` for EOF-vs-error distinction).

### `ParseError`

`std::str::parse_int(s)` / `parse_float(s)` / `parse_decimal(s)`
return `fallible(ParseError)`. The non-fallible siblings exist
only as `can_parse_*` predicate spellings; every parsing call
site must address the failure with `or`. `ParseError` carries:

- `kind: String` — `"empty"` (s was `""`), `"trailing_chars"`
  (s parsed a prefix and had junk after), `"invalid"` (no
  leading numeric prefix), `"overflow"` (`parse_int` only —
  magnitude exceeds Int range).
- `input: String` — the original `s` (truncated to a reasonable
  preview if very long), for diagnostic surfaces.

```aperio
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

```aperio
let bytes = std::process::rss_bytes();
println("rss=", to_string(bytes));
```

For the *current* RSS, parse `/proc/self/statm`'s line one
field two via `read_file` (size-tolerant for synthesized
files). Both surfaces ship; pick by use case (peak for
alarms, current for heartbeat gauges).

## `Server.shutdown()` — interruptible accept loop

`std::http::Server` exposes a `shutdown()` method that calls
`shutdown(SHUT_RDWR)` on the listen socket, forcing any thread
blocked in `accept()` to return immediately with an error. The
accept loop in `run()` detects the negative return and breaks;
`dissolve()` does the actual `close()`.

`shutdown()` is **safe to call from any thread, including
cross-scheduler** — that's the whole point. A cooperative-
scheduled Server can't pump its own `shutdown()` call because
the scheduler is parked in `accept()`, so the wake-up must come
from outside. Typical pattern (e.g. a pinned gateway with a
duration-bounded recv loop, sharing a process with a
cooperative metrics endpoint):

```aperio
locus App {
    params {
        gateway: Gateway = Gateway { duration_s: 60 };
        metrics: std::http::Server = std::http::Server {
            port: 9100, handler: MetricsHandler { }
        };
    }
    run() {
        // gateway is pinned — runs on its own thread.
        // metrics is cooperative — its run() blocks in accept.
        // When gateway's run() finishes, signal metrics to
        // wind down from the pinned thread.
        self.metrics.shutdown();
    }
}
```

The accept loop treats any negative `accept_one` return as a
clean shutdown signal, so even degenerate fd closes (external
fd-closes, etc.) terminate gracefully.

## `Server.ready_signal` — synchronization for piped oracles

`std::http::Server` accepts an optional `ready_signal: String = ""`
param. When non-empty, the server emits it via `println` from
`birth()` immediately after `listen_socket` succeeds and before
the accept loop begins. Test harnesses, oracles, and shell
scripts that pipe the server's stdout (`./bin | grep -m1 READY`)
key off this line:

```aperio
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

```aperio
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

These live in the Aperio package ecosystem (per
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
