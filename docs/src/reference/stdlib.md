# Standard library

Hale's stdlib ships bundled with every binary — no separate
install, no manual import for stdlib namespaces (just inline
`std::*` paths in your code). This page indexes the shipped
surface. The authoritative phase-by-phase history lives at
[`spec/stdlib.md`](https://github.com/hale-lang/hale/blob/main/spec/stdlib.md).

`std::*` is the curated bundled-with-the-compiler surface; the
link floor is libc + OpenSSL only. For C-ABI bindings to other
libraries (raylib, sqlite, curl, SDL, ...), Hale exposes a
user-extensible FFI mechanism — library authors land bindings
in pond / vendored libs without compiler changes. See
[Bind a C library](../how-tos/ffi-bindings.md) for the how-to
and [`spec/ffi.md`](https://github.com/hale-lang/hale/blob/main/spec/ffi.md)
for the substrate contract.

## Two shapes

The stdlib comes in two structurally distinct shapes, with a
clear rule for which is which:

### Path-call dispatch

Inline calls through `std::*` paths that route directly to C
runtime primitives. No `.hl` source backing them — they're
extern bridges into `lotus_*` C functions:

```hale
let pid     = std::process::pid();
let content = std::io::fs::read_file("config.toml") or "";
let n       = std::str::parse_int("42") or 0;
```

Namespaces with path-call shape:

| Namespace | Surface |
|---|---|
| `std::process` | `pid()`, `exit(code)`, `run(argv: String) -> ProcessOutput fallible(IoError)` — synchronous fork/exec; `argv` is newline-separated (e.g. `"git\nstatus\n"`), output captured up to 16 MiB/stream, exec failures surface as IoError (`kind="not_found"` / `"permission_denied"` / `"invalid"`). Lifecycle-bound subprocess uses the `Child` namespace lotus (see below). |
| `std::env` | `args_count()`, `arg(i)`, `arg_or(i, default)`, `var(name)`, `var_exists(name)` |
| `std::time` | `monotonic()` → Duration, `sleep(d)`, `now() -> Int` (wall-clock seconds since the Unix epoch — observation only, not for scheduling; backed by `clock_gettime(CLOCK_REALTIME)`), `time_from_unix(n: Int) -> Time` (construct a Time from epoch seconds — ISO 8601 UTC; round-trips with `now()` for stamping `recv_ts`-shaped fields at runtime) |
| `std::str` | `parse_int(s) -> Int fallible(ParseError)`, `parse_float(s) -> Float fallible(ParseError)`, `parse_decimal(s) -> Decimal fallible(ParseError)` (`ParseError { kind, input }`; fixed scale-9 i128 mantissa — survives the trailing-zero precision that IEEE 754 doubles round off, the right shape for venue book qtys and money values), `can_parse_int`, `can_parse_float`, `can_parse_decimal` (non-fallible predicates). Range-bounded variants (2026-05-26) operate on a byte range of an existing `String` without materializing a substring: `range_eq(s, start, end_exclusive, expected) -> Bool`, `range_parse_int(s, start, end_exclusive) -> Int fallible(ParseError)`, `range_parse_decimal(...) -> Decimal fallible(ParseError)`. Paired with `std::json::iter_find_*_range` for allocation-free JSON walks. `byte_at_unchecked(s, i) -> Int` (2026-05-26) is the direct-pointer byte access for stdlib scan helpers — no bounds check, caller-guaranteed `0 ≤ i < len(s)`, misuse → UB. Plus `index_of`, `lower` / `upper`, `trim`, `substring(s, lo, hi)`, `replace`, `repeat`, `pad_left` / `pad_right`, `from_bytes`, `clone(v: StringView) -> String` (F.30, 2026-05-20 — deep-copy a non-owning text view into the caller's arena), `builder_new` / `builder_append` / `builder_len` / `builder_finish` |
| `std::bytes` | `at(b, i) -> Int fallible(IndexError)`, `slice(b, lo, hi)`, `from_string(s)`, `from_int(b)`, `concat(a, b)`, `clone(v: BytesView) -> Bytes` (F.30, 2026-05-20 — deep-copy a non-owning view into the caller's arena). Growing-buffer accumulator surface lives on the `BytesBuilder` locus (see below), not as free fns: `let buf = std::bytes::BytesBuilder { initial_cap: N };` then `buf.append(chunk)`, `buf.append_slice(src, lo, hi)` (F.30, zero-alloc range copy), `buf.len()`, `buf.snapshot() -> Bytes` (copies; stable across mutations), `buf.view() -> BytesView` (zero-copy non-owning alias; reads through the view check the builder's mutation epoch and panic with a clear stderr diagnostic if the builder was mutated between view() and read — F.30b runtime guard, 2026-05-20), `buf.text_view() -> StringView` (NUL-terminated companion, same F.30b epoch guard), `buf.shift_front(n)`, `buf.clear()`, `buf.finish() -> Bytes`. The locus shape enforces builder-vs-Bytes type discrimination at design time — `std::bytes::at(buf, i)` on a `BytesBuilder` fails at typecheck rather than silently misreading the runtime header (the two have incompatible ABIs: a `Bytes` blob is `[i64 len][u8 data]` contiguous; a builder is a `{cap, len, buf*}` header with a separately-malloc'd body). Literal syntax: `b"..."` (added 2026-05-17) — same escapes as String, `\xNN` accepts full 0x00..0xFF; NUL-safe; length carried alongside the buffer. |
| `std::text` | byte-class predicates `is_alpha`, `is_digit`, `is_alnum`, `is_whitespace`, `is_word_char` (`fn(Int) -> Bool`); `tokenize_words_into(s, target_vec)` populates a `@form(vec) of String` with lowercased word tokens |
| `std::io::fs` | `read_file`, `write_file`, `write_file_append`, `read_bytes`, `file_size`, `mkdir`, `rename(src, dst)`, `unlink(path)`, `mktemp(prefix, suffix) -> String` (race-free mkstemps(3) wrapper — caller owns cleanup), `list_dir_count`, `list_dir_at` — all return `fallible(IoError)` (`kind: String`, `errno: Int`, `path: String`). `file_exists(path) -> Bool` is the only non-fallible predicate. Iterate directories via the index API (`for i in 0..count { let name = at(i); ... }`); the older newline-joined `list_dir(path) -> String` was removed 2026-05-16. |
| `std::io::stdin` | `read_line() -> String`, `read_line_status() -> Int` |
| `std::io::tcp` | path-call entry points `listen_socket(host, port) -> Int fallible(IoError)`, `connect(host, port) -> Int fallible(IoError)` (accepts dotted-quad hosts directly and falls back to hostname resolution via `getaddrinfo` (AF_INET) when the host isn't numeric — see [`spec/stdlib.md`](https://github.com/hale-lang/hale/blob/main/spec/stdlib.md) C6 row), `accept_one(listen_fd) -> Int fallible(IoError)`, `recv_into(fd, buf: std::bytes::BytesBuilder, max_bytes) -> Int` (added 2026-05-19; `buf` is a `BytesBuilder` locus instance; the recv writes into the builder's tail with zero arena allocation — POSIX read(2) semantics: > 0 bytes appended, 0 peer closed, < 0 error; the `BytesBuilder` typecheck is what closes the silent-misread footgun a raw `Bytes` would carry), `close_fd(fd)` (infallible) |
| `std::io::udp` | `bind(host, port) -> Int fallible(IoError)` (SOCK_DGRAM; host="" → INADDR_ANY), `send(fd, host, port, msg) -> () fallible(IoError)`, `recv(fd, max_bytes) -> Bytes fallible(IoError)`, `recv_into(fd, buf: std::bytes::BytesBuilder, max_bytes) -> Int` (added 2026-05-19; same shape as tcp `recv_into`, single datagram per call, zero arena allocation), `close(fd)` (infallible). **Multicast surface (P1, 2026-05-26)**: `join_group(fd, group, iface)`, `leave_group(fd, group, iface)` (`iface=""` → INADDR_ANY), `set_multicast_ttl(fd, ttl)` (0..255), `set_multicast_loop(fd, enabled: Bool)`, `set_multicast_iface(fd, addr)`. **Transparent setsockopt (P2, 2026-05-26)**: `set_option_int(fd, level, name, value)`, `set_option_bool(fd, level, name, enabled)`, `get_option_int(fd, level, name) -> Int` — paired with `std::io::sockopt::<NAME>()` for `level` / `name` constants (see row below). **Source-bearing recv + timeouts (P4, 2026-05-26)**: `recv_with_source(fd, max_bytes) -> Bytes` populates a thread-local source-IP/port cache; `last_source_host() -> String` / `last_source_port() -> Int` read the cache (errno-style; read immediately after recv). `set_recv_timeout(fd, d: Duration)` / `set_send_timeout(fd, d: Duration)` wrap `SO_RCVTIMEO` / `SO_SNDTIMEO` (struct timeval; `d = 0` blocks). Datagram boundaries preserved by the kernel. **Not a bus transport**: UDP doesn't satisfy atomic delivery; cross-host bus over UDP needs a user adapter layering retry / framing on top. |
| `std::io::sockopt` | Named-constant getters returning the platform's numeric value for each setsockopt `level` / `name`. Use as args to `std::io::udp::set_option_*` / `get_option_int`. Each is a zero-arg fn (e.g. `std::io::sockopt::SO_RCVBUF()`) so the value tracks the kernel headers (`SOL_SOCKET` is 1 on Linux, 0xffff on macOS — no hardcoding). Shipped names (2026-05-26): `SOL_SOCKET`, `IPPROTO_IP`, `IPPROTO_IPV6`, `IPPROTO_TCP`, `IPPROTO_UDP`, `SO_REUSEADDR`, `SO_REUSEPORT`, `SO_RCVBUF`, `SO_SNDBUF`, `SO_RCVTIMEO`, `SO_SNDTIMEO`, `SO_BROADCAST`, `SO_KEEPALIVE`, `SO_LINGER`, `SO_PRIORITY`, `SO_BINDTODEVICE`, `IP_TTL`, `IP_TOS`, `IP_MULTICAST_TTL`, `IP_MULTICAST_LOOP`, `IP_MULTICAST_IF`, `IP_ADD_MEMBERSHIP`, `IP_DROP_MEMBERSHIP`, `IP_PKTINFO`. |
| `std::io::tls` | Client-side TLS via system OpenSSL (`-lssl -lcrypto`). `connect(host, port) -> Int fallible(IoError)` opens a TCP connection, performs a TLS 1.2+ handshake with SNI + system-trust-store cert verification, returns an opaque handle. `send_bytes(handle, b: Bytes) -> Int` and `recv_bytes(handle, max: Int) -> Bytes` operate on the handshaked connection (return 0/-1 and empty Bytes on error respectively, mirroring `std::io::tcp`'s `send_bytes`/`recv_bytes` shape). `recv_into(handle, buf: std::bytes::BytesBuilder, max_bytes) -> Int` (added 2026-05-19) reads directly into a `BytesBuilder` locus instance with zero arena allocation — same POSIX read(2) semantics as the tcp/udp siblings. `close(handle) -> Int` shuts the TLS layer down and closes the socket. |
| `std::bus` | `__StdBusAdapter` interface (single method `fn send(subject: String, bytes: Bytes)` — the contract for user-supplied transport adapters). `__local_dispatch(subject: String, bytes: Bytes)` primitive for adapter inbound: looks up the subject's registered deserializer, reconstructs the in-memory payload, fans into local subscribers. **Substrate transports** wired by `LOTUS_BUS_CONFIG=<file>` at startup: `unix://<path>` (AF_UNIX framed-byte; m58) and `udp://<host>:<port>` (2026-05-26 — IPv4 UDP, single scheme covers unicast and multicast based on the destination's address class; lossy delivery). See [The bus → Writing your own adapter](../concepts/the-bus.md#writing-your-own-adapter) and [Multi-binary bus → UDP transport](../how-tos/multi-binary-bus.md#udp-transport-unicast--multicast). |
| `std::math` | `sqrt`, `exp`, `log`, `floor`, `ceil`, `pow`, `tanh`, `nan()`, `inf()`, `is_nan(f)` (IEEE 754 sentinels + classification — all non-fallible). Trig surface (2026-05-23): `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`. All libm pass-through; non-fallible. |
| `std::crypto` | `sha1(b) -> Bytes` (20-byte), `sha256(b) -> Bytes` (32-byte), `hmac_sha256(key, msg) -> Bytes` (32-byte). All non-fallible; stand-alone pure-C impl (no libcrypto / OpenSSL link dep); digests anchored in the bus payload arena. Verified against FIPS 180-2 + RFC 4231 vectors. |
| `std::os` | `getrandom(n: Int) -> Bytes fallible(IoError)` — cryptographically-strong random bytes via the Linux `getrandom(2)` syscall with `/dev/urandom` fallback. `n <= 0` returns empty Bytes (no error); `n > 8192` errors with `IoError.kind="invalid"`. Use for session tokens, nonces, key material. |
| `std::ts` | tree-sitter bindings (Go grammar shipped) |

Path-call surfaces are appropriate for *value-shaped*
operations that don't need lifecycle. A file read returns
bytes; a math op returns a number; argv access returns a
string. No locus required.

### Namespace lotus

When the operation has a lifetime — a stream that's open
across multiple reads, a sink that has setup and teardown — the
stdlib provides a **namespace lotus**: an Hale-sourced locus
under `runtime/stdlib/`. You instantiate it the same way you
instantiate any other locus:

```hale
let l = std::io::tcp::Listener {
    host: "127.0.0.1",
    port: 8080,
    on_connection: my_handler,
};
```

Namespaces with namespace-lotus shape:

| Namespace | Loci / interfaces shipped |
|---|---|
| `std::io::tcp` | `Listener` (multi-accept loop, dispatch via `on_connection: fn(Stream)`), `Stream` (per-connection handle with `send` / `send_bytes` / `recv` / `recv_bytes` methods) |
| `std::process` | `Child` (async subprocess handle obtained via `spawn(argv: String) -> Child fallible(IoError)`); methods `wait(c)` (blocking), `kill(c)` (SIGTERM → 100ms grace → SIGKILL → reap), `write_stdin(c, s)`, `read_stdout(c)`, `read_stderr(c)` (non-blocking 64 KiB; empty String on EAGAIN or EOF — disambiguate via `wait`). Every spawned child gets its own process group (`setpgid(0,0)` post-fork); SIGPIPE is globally ignored at runtime init so writes to closed pipes surface as IoError EPIPE. `Child.dissolve()` closes pipes and kill-escalates idempotently to prevent zombies on scope exit. |
| `std::http` | `Request` and `Response` types (`Response.content_type` defaults to `"text/plain"`; `Response.headers: String = ""` carries CRLF-joined user-supplied headers — no trailing CRLF — for Set-Cookie / CORS / custom-header use; emitted on the wire after the fixed Content-Type / Content-Length / Connection-close lines), `parse_request`, `write_response`, case-insensitive symmetric `header(receiver, name)` lookup that works on both Request and Response receivers, `Handler` interface (`fn handle(req: Request) -> Response`), `Server` locus (wraps accept-recv-parse-dispatch-write; `handler:` is a required field typed by `Handler`, takes any locus with a `handle` method — state lives in the handler-locus's params) |
| `std::text` | `md_to_html`, `base64::encode` / `decode`, `Sink` interface with `StdoutSink` / `StringSink` / `FileSink` implementations (note: the byte-class predicates + `tokenize_words_into` are path-call surface, listed in the previous table) |
| `std::cli` | `Resolver` for argv parsing |
| `std::iter` | `Lines` iterator over text |
| `std::json` | `Builder` for JSON output; free-fn helpers `escape_string` / `unescape_string` (RFC 8259), `find_string_field` / `find_int_field` / `find_bool_field` (flat-object field lookup), `find_field_raw(json, name) -> String` (raw value-token substring; bracket-balanced over nested objects/arrays — the recursive-descent primitive for wrapped payloads where the real fields live inside a `"result":{...}` or `"data":[{...}]`), `ArrayIter` + `array_first` / `array_next` (flat-array iteration). **Allocation-free walk surface (2026-05-26)**: `array_first_span` + `array_next_span` + `iter_find_field_range(it, json, name) -> JsonFieldRange` + `iter_find_string_field_range` return `{ok, start, end_pos}` tracking byte positions in the source `json` instead of copying substrings; paired with `std::str::range_eq` / `range_parse_int` / `range_parse_decimal` the full per-element walk runs without per-field allocations (the fathom-class workload where per-field `String` returns dominated arena pressure). No typed walker over nested trees at v1 — `find_field_raw` returns the substring so callers can re-feed it into the same surface. |
| `std::lang` | `Morpheme`, `Vocabulary`, etc. for language utilities |
| `std::log` | `Logger`, `LogEvent`, `StdoutSink` (subscribes to `log.**`) |
| `std::yaml` | YAML parsing surface |
| `std::test` | `assert(cond, msg)`, `assert_eq_int`, `assert_eq_str` |

Source for namespace-lotus stdlib lives at
[`crates/hale-codegen/runtime/stdlib/`](https://github.com/hale-lang/hale/tree/main/crates/hale-codegen/runtime/stdlib).
Read it directly — it's idiomatic Hale that exercises every
pattern Concepts covers.

## Built-in identifiers (no path needed)

A handful of functions and types are always in scope without
any `std::*` qualification:

| Name | Purpose |
|---|---|
| `print`, `println`, `eprint`, `eprintln` | stdout / stderr output |
| `len(x)` | length of String / Bytes / array |
| `to_string(x)` | format any printable value to String |
| `min(a, b)`, `max(a, b)`, `abs(x)` | numeric helpers |
| `starts_with(s, prefix)`, `contains(s, needle)` | string predicates |
| `sum(expr)`, `prod(expr)` | reductions (also closure-test primitives) |
| `Int(x)` | explicit Float → Int narrowing (truncate toward zero) |

Primitive types (`Int`, `Uint`, `Float`, `Decimal`, `String`,
`Bool`, `Time`, `Duration`, `Bytes`) are valid only in type
position.

## Form-synthesized types

When any locus in your program uses `@form(...)`, the
resolver injects companion error types into the top scope:

| Form | Synthesized type | Fields |
|---|---|---|
| `@form(vec)` | `IndexError` | `kind: String`, `index: Int`, `len: Int` |
| `@form(hashmap)` | `KeyError` | `kind: String` (also surfaces `IndexError` for `key_at` / `entry_at`) |
| `@form(ring_buffer)` | `EmptyError` | `kind: String` |
| `std::io::*` | `IoError` | `kind: String`, `errno: Int`, `path: String` |

The form-method surface synthesizes more than fallibility — see
[`spec/forms.md`](https://github.com/hale-lang/hale/blob/main/spec/forms.md)
for the full per-form table. Quick reference for what's on each:

| Form | Synthesized methods |
|---|---|
| `@form(vec)` | `push`, `get`, `set`, `pop`, `len`, `is_empty`, `sort`, `sort_by`, `sort_desc_by` |
| `@form(hashmap)` | `set`, `get`, `has`, `remove`, `len`, `is_empty`, `key_at`, `entry_at`, `bump` |
| `@form(ring_buffer)` | `push -> Bool`, `pop`, `len`, `is_full` |

You can reference these as ordinary types — pattern-match
them in `match`, declare fn parameters typed by them,
construct them in fallback expressions.

## What's NOT in stdlib

Hale's stdlib follows Go's batteries-included approach:
table-stakes functionality ships. Specifically *not* in
stdlib (and intended for the
[`hale-lang/pond`](https://github.com/hale-lang/pond)
contrib monorepo or third-party):

- ML / learning libraries
- Database drivers (Postgres, MySQL, ...)
- Web frameworks beyond basic HTTP
- Image / audio / video processing
- Cloud SDKs (AWS, GCP, ...)
- GUI / TUI frameworks beyond what `std::io::tcp` enables
- Cryptography beyond TLS basics
- Compression formats beyond gzip (used internally by HTTP)

Hale also doesn't have parametric collection types in
stdlib — no `Vec<T>` / `Map<K, V>` / `Set<T>` / `Option<T>` /
`Result<T, E>` as user-facing tagged enums. Storage is
locus-shaped via `@form(...)`. See
[Capacity & storage](../concepts/capacity-storage.md) for
the rationale.

## Reading order

If you're writing application code and want to discover
what's available, the productive order is:

1. **Skim this page** to know what namespaces exist.
2. **Read the spec section**
   ([`spec/stdlib.md`](https://github.com/hale-lang/hale/blob/main/spec/stdlib.md))
   for the namespace you need; it's the authoritative
   surface.
3. **Read the namespace-lotus source** for any lotus you'll
   use — it's a few hundred lines per namespace, and it's
   the clearest documentation of how the surface composes.
