# Standard library

Aperio's stdlib ships bundled with every binary — no separate
install, no manual import for stdlib namespaces (just inline
`std::*` paths in your code). This page indexes the shipped
surface. The authoritative phase-by-phase history lives at
[`spec/stdlib.md`](https://github.com/aperio-lang/aperio/blob/main/spec/stdlib.md).

## Two shapes

The stdlib comes in two structurally distinct shapes, with a
clear rule for which is which:

### Path-call dispatch

Inline calls through `std::*` paths that route directly to C
runtime primitives. No `.ap` source backing them — they're
extern bridges into `lotus_*` C functions:

```aperio
let pid     = std::process::pid();
let content = std::io::fs::read_file("config.toml") or "";
let n       = std::str::parse_int("42") or 0;
```

Namespaces with path-call shape:

| Namespace | Surface |
|---|---|
| `std::process` | `pid()`, `exit(code)`, `run(argv: String) -> ProcessOutput fallible(IoError)` — synchronous fork/exec; `argv` is newline-separated (e.g. `"git\nstatus\n"`), output captured up to 16 MiB/stream, exec failures surface as IoError (`kind="not_found"` / `"permission_denied"` / `"invalid"`). Lifecycle-bound subprocess uses the `Child` namespace lotus (see below). |
| `std::env` | `args_count()`, `arg(i)`, `arg_or(i, default)`, `var(name)`, `var_exists(name)` |
| `std::time` | `monotonic()` → Duration, `sleep(d)`, `now() -> Int` (wall-clock seconds since the Unix epoch — observation only, not for scheduling; backed by `clock_gettime(CLOCK_REALTIME)`) |
| `std::str` | `parse_int(s) -> Int fallible(ParseError)`, `parse_float(s) -> Float fallible(ParseError)` (`ParseError { kind, input }`), `can_parse_int`, `can_parse_float` (non-fallible predicates), `index_of`, `lower` / `upper`, `trim`, `substring(s, lo, hi)`, `replace`, `repeat`, `pad_left` / `pad_right`, `from_bytes`, `builder_new` / `builder_append` / `builder_len` / `builder_finish` |
| `std::bytes` | `at(b, i) -> Int fallible(IndexError)`, `slice(b, lo, hi)`, `from_string(s)`, `from_int(b)`, `concat(a, b)`, `builder_new` / `builder_append(b, chunk)` / `builder_len(b)` / `builder_finish(b)` — binary-safe sibling of the `std::str::builder_*` family; the append reads each chunk's length prefix so embedded NULs survive, finish emits a length-prefixed Bytes blob without a trailing NUL. Literal syntax: `b"..."` (added 2026-05-17) — same escapes as String, `\xNN` accepts full 0x00..0xFF; NUL-safe; length carried alongside the buffer. |
| `std::text` | byte-class predicates `is_alpha`, `is_digit`, `is_alnum`, `is_whitespace`, `is_word_char` (`fn(Int) -> Bool`); `tokenize_words_into(s, target_vec)` populates a `@form(vec) of String` with lowercased word tokens |
| `std::io::fs` | `read_file`, `write_file`, `write_file_append`, `read_bytes`, `file_size`, `mkdir`, `rename(src, dst)`, `unlink(path)`, `mktemp(prefix, suffix) -> String` (race-free mkstemps(3) wrapper — caller owns cleanup), `list_dir_count`, `list_dir_at` — all return `fallible(IoError)` (`kind: String`, `errno: Int`, `path: String`). `file_exists(path) -> Bool` is the only non-fallible predicate. Iterate directories via the index API (`for i in 0..count { let name = at(i); ... }`); the older newline-joined `list_dir(path) -> String` was removed 2026-05-16. |
| `std::io::stdin` | `read_line() -> String`, `read_line_status() -> Int` |
| `std::io::tcp` | path-call entry points `listen_socket(host, port) -> Int fallible(IoError)`, `connect(host, port) -> Int fallible(IoError)` (accepts dotted-quad hosts directly and falls back to hostname resolution via `getaddrinfo` (AF_INET) when the host isn't numeric — see [`spec/stdlib.md`](https://github.com/aperio-lang/aperio/blob/main/spec/stdlib.md) C6 row), `accept_one(listen_fd) -> Int fallible(IoError)`, `close_fd(fd)` (infallible) |
| `std::io::udp` | `bind(host, port) -> Int fallible(IoError)` (SOCK_DGRAM; host="" → INADDR_ANY), `send(fd, host, port, msg) -> () fallible(IoError)`, `recv(fd, max_bytes) -> Bytes fallible(IoError)`, `close(fd)` (infallible). Datagram boundaries preserved by the kernel. **Not a bus transport**: UDP doesn't satisfy atomic delivery; cross-host bus over UDP needs a user adapter layering retry / framing on top. |
| `std::io::tls` | Client-side TLS via system OpenSSL (`-lssl -lcrypto`). `connect(host, port) -> Int fallible(IoError)` opens a TCP connection, performs a TLS 1.2+ handshake with SNI + system-trust-store cert verification, returns an opaque handle. `send_bytes(handle, b: Bytes) -> Int` and `recv_bytes(handle, max: Int) -> Bytes` operate on the handshaked connection (return 0/-1 and empty Bytes on error respectively, mirroring `std::io::tcp`'s `send_bytes`/`recv_bytes` shape). `close(handle) -> Int` shuts the TLS layer down and closes the socket. |
| `std::bus` | `__StdBusAdapter` interface (single method `fn send(subject: String, bytes: Bytes)` — the contract for user-supplied transport adapters). `__local_dispatch(subject: String, bytes: Bytes)` primitive for adapter inbound: looks up the subject's registered deserializer, reconstructs the in-memory payload, fans into local subscribers. See [The bus → Writing your own adapter](../concepts/the-bus.md#writing-your-own-adapter). |
| `std::math` | `sqrt`, `exp`, `log`, `floor`, `ceil`, `pow`, `tanh`, `nan()`, `inf()`, `is_nan(f)` (IEEE 754 sentinels + classification — all non-fallible) |
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
stdlib provides a **namespace lotus**: an Aperio-sourced locus
under `runtime/stdlib/`. You instantiate it the same way you
instantiate any other locus:

```aperio
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
| `std::json` | `Builder` for JSON output; free-fn helpers `escape_string` / `unescape_string` (RFC 8259), `find_string_field` / `find_int_field` / `find_bool_field` (flat-object field lookup), `ArrayIter` + `array_first` / `array_next` (flat-array iteration). No nested-tree shape at v1 |
| `std::lang` | `Morpheme`, `Vocabulary`, etc. for language utilities |
| `std::log` | `Logger`, `LogEvent`, `StdoutSink` (subscribes to `log.**`) |
| `std::yaml` | YAML parsing surface |
| `std::test` | `assert(cond, msg)`, `assert_eq_int`, `assert_eq_str` |

Source for namespace-lotus stdlib lives at
[`crates/aperio-codegen/runtime/stdlib/`](https://github.com/aperio-lang/aperio/tree/main/crates/aperio-codegen/runtime/stdlib).
Read it directly — it's idiomatic Aperio that exercises every
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
[`spec/forms.md`](https://github.com/aperio-lang/aperio/blob/main/spec/forms.md)
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

Aperio's stdlib follows Go's batteries-included approach:
table-stakes functionality ships. Specifically *not* in
stdlib (and intended for the
[`aperio-lang/pond`](https://github.com/aperio-lang/pond)
contrib monorepo or third-party):

- ML / learning libraries
- Database drivers (Postgres, MySQL, ...)
- Web frameworks beyond basic HTTP
- Image / audio / video processing
- Cloud SDKs (AWS, GCP, ...)
- GUI / TUI frameworks beyond what `std::io::tcp` enables
- Cryptography beyond TLS basics
- Compression formats beyond gzip (used internally by HTTP)

Aperio also doesn't have parametric collection types in
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
   ([`spec/stdlib.md`](https://github.com/aperio-lang/aperio/blob/main/spec/stdlib.md))
   for the namespace you need; it's the authoritative
   surface.
3. **Read the namespace-lotus source** for any lotus you'll
   use — it's a few hundred lines per namespace, and it's
   the clearest documentation of how the surface composes.
