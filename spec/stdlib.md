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

The parser tokenizes `::` as a path separator and the type checker
punts namespaced paths to `Ty::Unknown`; the codegen layer
resolves `std::*` paths against a hardcoded namespace dispatcher.

There is **no general module system** at v1 — no `use`
statements, no user-defined modules, no multi-file `.hl`
packages via the std-style mechanism. `std::*` is the only
recognized prefix. Adding a new stdlib function means: declare
its libc backer in `hale-codegen`'s `declare_builtins`, add a
match arm to `lower_stdlib_path_call_expr` (or its statement
sibling), and implement one `lower_std_*` method.

Cross-binary user code uses the F.25 cross-seed-imports mechanism
(`import "path/to/lib" as alias;`) — distinct from the `std::*`
magic path; see [`design-rationale.md` § F.25](./design-rationale.md).

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
| `std::process` | `pid() -> Int`, `exit(code: Int)`, `rss_bytes() -> Int`, `dump_arena_residency() -> Int` (no-op unless `LOTUS_ARENA_RESIDENCY=1`; writes per-arena residency snapshot to stderr), `dump_pool_residency() -> Int` (F.35, 2026-05-28; writes one stderr line per cooperative pool — name, async_io vs blocking mode, parked-coro count, pending cell-queue depth — for ops embedding in heartbeat ticks), `run(argv) -> ProcessOutput fallible(IoError)`, `spawn` / `wait` / `kill` / `write_stdin` / `read_stdout` / `read_stderr` over `Child` | path-call dispatch + C primitives |
| `std::env` | `args_count()`, `arg(i)`, `arg_or(i, default)`, `var(name)`, `var_exists(name)` | path-call dispatch + main-prelude argv stash |
| `std::cli` | `Resolver` locus — layered config resolution with precedence **CLI argv > env var > fallback**. Params: `env_prefix: String = "HALE_"` (each `get(key, …)` looks up `<prefix><UPPER(key)>` in the process env — `prefix="HALE_"`, `key="dir"` → `HALE_DIR`) and `argv_keys: String = ""` (newline-separated positional keys — first line maps to `argv[1]`, second to `argv[2]`, …; a blank line doesn't shift positions; a key absent from `argv_keys` skips the CLI layer). Methods: `get(key, fallback) -> String` (the highest *populated* layer wins; empty/unset at a layer falls through) and `get_int(key, fallback) -> Int` (same precedence; a non-parseable value falls through to `fallback` rather than crash). No birth/run/dissolve lifecycle — the params *are* the configuration, so re-prefixing the Resolver retargets it without touching the body. | `runtime/stdlib/cli.hl` |
| `std::time` | `monotonic() -> Duration`, `monotonic_ns() -> Int`, `sleep(d: Duration)`, `now() -> Int`, `time_from_unix(n: Int) -> Time` | `clock_gettime` + EINTR-retrying `clock_nanosleep`; `sleep` slices the request into ≤100ms intervals and folds in a cooperative bus drain after each slice, so a long keep-alive sleep doesn't starve main-pool handlers (see `spec/runtime.md` § "`time::sleep` drain semantics"); `now()` is `CLOCK_REALTIME`; `time_from_unix` formats `gmtime_r` + `strftime` ISO 8601 UTC |
| `std::decimal` | `to_float(d: Decimal) -> Float` | Direct i128 → f64 conversion at scale 9 (`mantissa × 10^-9`) — skips an ASCII round-trip |
| `std::str` | `parse_int(s) -> Int fallible(ParseError)`, `parse_float(s) -> Float fallible(ParseError)`, `parse_decimal(s) -> Decimal fallible(ParseError)`; predicate siblings `can_parse_int` / `can_parse_float` / `can_parse_decimal`; range-bounded variants `range_eq(json, start, end_exclusive, expected) -> Bool` / `range_parse_int(json, start, end_exclusive) -> Int fallible(ParseError)` / `range_parse_decimal(json, start, end_exclusive) -> Decimal fallible(ParseError)` (2026-05-26 — operate on byte ranges within an existing `String` without materializing a substring, paired with `std::json::iter_find_*_range` for allocation-free JSON walks); `byte_at_unchecked(s, i) -> Int` (2026-05-26 — direct byte access at offset i with NO bounds check; caller must guarantee 0 ≤ i < len(s); used by stdlib scan helpers (JSON walkers) where the bound is externally known and a per-access strlen / bytes_from_string would tank perf; misuse → UB); `index_of`, `lower` / `upper`, `trim`, `substring(s, lo, hi)`, `replace`, `repeat`, `pad_left` / `pad_right`, `from_bytes`, `clone(v) -> String` (deep-copy a `StringView` to an owned blob; identity on a `String` for generic callers); `builder_new` / `builder_append` / `builder_len` / `builder_finish` (String-builder primitives — for binary-safe accumulator use `std::bytes::BytesBuilder`) | `lotus_str_*` C runtime primitives |
| `std::bytes` | `at(b, i) -> Int fallible(IndexError)`, `slice(b, lo, hi) -> Bytes`, `from_string(s) -> Bytes`, `from_int(v) -> Bytes`, `concat(a, b) -> Bytes`, `clone(v) -> Bytes` (deep-copy a view to an owned blob). **Word-scan + masked-XOR** (2026-06-13, fast-protocol-I/O #4): `find_byte(b, off, needle) -> Int` returns the first index `>= off` whose byte equals `needle` (low 8 bits) or `-1` (non-fallible; `memchr` word-at-a-time scan — the length/delimiter-framing primitive for HTTP CRLF etc.). `read_*` / `at` / `find_byte` also accept a **`BytesMut`** raw `{ptr,len}` window (a `MirrorRing.readable()` window or a `Topic.write` slot) — read directly via their `_raw` siblings (length is the window length, no `[i64 len]` prefix), so a mirror-ring parse is zero-copy. `BytesBuilder.xor_mask(src: Bytes, key: Int)` (and its primitive `std::bytes::builder::__xor_mask_into(handle, src, key) -> Int`) appends `src` XOR'd with a repeating 4-byte key (`masked[i] = src[i] ^ key[i % 4]`, key bytes packed little-endian) in one reserve + word-at-a-time pass — the WebSocket masking primitive, replacing a per-byte `from_int` + `append` loop. **Binary-pack readers** (2026-06-06, shm-ring-interop Proposal A): `read_u8` / `read_u16_{le,be}` / `read_u32_{le,be}` / `read_u64_{le,be}` and the signed `read_i8` / `read_i16_{le,be}` / `read_i32_{le,be}` / `read_i64_{le,be}` (sign-extended), each `(b, off) -> Int fallible(IndexError)`; plus `read_f32_le` / `read_f64_{le,be}` `-> Float fallible(IndexError)`. Fixed-width scalar reads at a byte offset, bounds-checked (`[off, off+width)` past the buffer raises `IndexError { kind: "out_of_bounds", index: off, len }`, same error as `at`). Endianness is explicit (`_le` is the x86-native common case); a `u64` with the top bit set wraps to a negative `Int` (i64). **Binary-pack writers** (2026-06-08, shm-ring-interop A1): the mirror `write_u8` / `write_u16_{le,be}` / `write_u32_{le,be}` / `write_u64_{le,be}`, signed `write_i8` / `write_i16_{le,be}` / `write_i32_{le,be}` / `write_i64_{le,be}`, and `write_f32_le` / `write_f64_{le,be}`, each `(buf, off, val) -> Int fallible(IndexError)` — a fixed-width scalar write at a byte offset into a **`BytesMut`** raw window (a `Topic.write` slot or a `MirrorRing.writable()` window), bounds-checked identically to the readers (`[off, off+width)` past the window raises `IndexError`), returning the offset past the write. These back the `Topic.write(max) { … }` zero-copy ring producer and the `repr:`-tagged `Type::set_field`. Growing-buffer accumulator surface lives on the `BytesBuilder` locus — see [§ Builders vs Bytes](#builders-vs-bytes--the-recv-loop-pattern) | `lotus_bytes_*` C runtime primitives |
| `std::text` | `md_to_html(md) -> String`, `base64::encode` / `base64::decode` / `base64::url_encode` (RFC 4648 §5 URL-safe, unpadded — for JWT/JWS, OAuth, webhooks), `Sink` interface + `StdoutSink` / `StringSink` / `FileSink` loci, byte-class predicates (`is_alpha` / `is_digit` / `is_alnum` / `is_whitespace` / `is_word_char`), `tokenize_words_into(s, target_vec)` | `runtime/stdlib/text.hl` + C runtime |
| `std::io::fs` | `read_file`, `write_file`, `write_file_append`, `read_bytes`, `file_size`, `mkdir`, `rename`, `unlink`, `mktemp`, `list_dir`, `list_dir_count`, `list_dir_at` — all `fallible(IoError)`. `file_exists(path) -> Bool` is a predicate (non-fallible). One-shot path-call surface: each call opens, does the op, closes. For held-open shapes use `std::io::file::File`. | `lotus_fs_*` C runtime primitives |
| `std::io::file` | `File` locus (held-open fd with auto-dissolve close). `open(path, mode) -> File fallible(IoError)`; `read_line(f) -> String` (returns "" at EOF or error — pair with `at_eof`); `at_eof(f) -> Bool`; `write_bytes(f, b)`, `write_line(f, s)`, `seek(f, offset)` all `fallible(IoError)`. Mode strings `"r"` / `"w"` / `"a"` / `"r+"` / `"w+"`. Returned Strings live in the bus payload arena. | `lotus_file_*` C primitives + `runtime/stdlib/file.hl` |
| `std::io::stdin` | `read_line() -> String`, `read_line_status() -> Int` (status `-1` = EOF/IO error; `0` = OK including empty-line). `read_byte(timeout_ms: Int) -> Int` — `poll` up to `timeout_ms` then a 1-byte `read`; returns `0..255` = the byte, `-1` = timeout, `-2` = EOF/error (sentinel; a timeout is a control outcome, not an error). `timeout_ms <= 0` is a pure poll. For interactive raw-mode input paired with `std::term::RawMode`. | POSIX `getline` / `poll`+`read` + payload-arena copy |
| `std::io::stdout` | `write_bytes(s: String) -> Int` — `fflush(stdout)` then a raw `write(1, ...)` that bypasses the prelude's `_IOLBF` line-buffering, so a multi-line frame isn't flushed per newline. Returns bytes written, `-1` on error (sentinel). The `fflush` keeps output ordered consistently with buffered `println`. | `lotus_term_write_stdout` C primitive |
| `std::term` | `is_tty(fd: Int) -> Bool` (POSIX `isatty` — probe whether an fd is a terminal, e.g. so a logger can decide on color without vendoring an FFI shim). `RawMode` — an RAII guard locus: `let r = std::term::RawMode { };` puts stdin in raw mode (no echo/canon/ISIG — unbuffered byte input, Ctrl-C as byte `0x03`) at birth and restores the saved termios at dissolve (scope exit). Soft-fails (runs unstyled) when stdin isn't a tty. `RawMode` also registers a runtime `atexit` termios restore, so a panic / unhandled error (which `exit()` through `atexit`) restores the terminal too — no FFI glue needed for terminal hygiene. `size() -> TermSize` (record `{ cols: Int; rows: Int }` via `ioctl(TIOCGWINSZ)`; `{ 0, 0 }` when stdout isn't a tty — poll per frame, no SIGWINCH handling). POSIX-only; non-tty / non-POSIX degrades soft. | `lotus_term_*` C primitives + `runtime/stdlib/term.hl` |
| `std::io::tcp` | `Listener` locus, `Stream` locus, `send`, `send_bytes`, `recv_bytes`, `recv_into(fd, buf: Bytes, max_bytes) -> Int` (caller-provided builder destination). Path-calls `listen_socket`, `connect`, `accept_one` are `fallible(IoError)`. `connect` accepts dotted-quad hosts directly and falls back to hostname resolution via `getaddrinfo(AF_INET)`. Send/recv timeouts (2026-05-27): `set_recv_timeout(fd, d: Duration) -> () fallible(IoError)` wraps `SO_RCVTIMEO`; `set_send_timeout(fd, d: Duration)` wraps `SO_SNDTIMEO`. `d = 0` disables (blocking default). After `set_recv_timeout(fd, 100ms)` a `recv_bytes` on a quiet socket returns ~100ms instead of blocking forever — unblocks recv loops that need periodic silence-detection / heartbeat / watchdog work. Shares the `sock_set_timeout_ns` helper with the udp siblings (P4, 2026-05-26). Nagle control (2026-06-13): `set_nodelay(fd, on: Bool) -> () fallible(IoError)` sets `TCP_NODELAY` on a connected fd — `on = true` disables Nagle so small writes hit the wire immediately instead of waiting up to ~40ms to coalesce, the first thing a latency-sensitive request/response or market-data socket reaches for. Previously impossible from Hale (`std::io::tcp` had no setsockopt surface); the generic udp `set_option_*` / `get_option_int` work on any fd if a less-common TCP option is needed. Kernel RX timestamps (2026-06-13): `set_rx_timestamps(fd, on: Bool) -> () fallible(IoError)` enables `SO_TIMESTAMPNS` once at setup; `recv_stamped_into(fd, buf: Bytes, max_bytes) -> Int` is the timestamped sibling of `recv_into` (identical `>0` / `0` EOF / `-1` fatal / `-2` retryable contract) that issues one `recvmsg(2)` capturing the kernel's wire-arrival timestamp alongside the bytes — no extra syscall on the hot path. Read the stamps with `last_recv_kernel_ns() -> Int` / `last_recv_user_ns() -> Int` immediately after the call (errno-style thread-local, same idiom as `udp::last_source_*`). `last_recv_user_ns` is a `CLOCK_REALTIME` stamp taken at `recvmsg` return; `last_recv_kernel_ns` is the kernel's `SCM_TIMESTAMPNS` value, or **`0` when the platform/path delivered none** — notably loopback TCP and any NIC without RX software/hardware timestamping (it never returns garbage, so `>= 0` is the contract). The cmsg is parsed defensively (first control message only, length-validated, no `CMSG_NXTHDR` walk — some libcs loop forever on a zero-length cmsg). **Bus-routed I/O observability** (2026-05-27): `Stream` gains a `log_subject: String = ""` param and a `bus { publish "io.tcp.**" of type std::io::tcp::LogEvent; }` declaration. When `log_subject` is set, every send / recv / close on that Stream publishes a `LogEvent { phase, detail, bytes, fd }` on the configured subject. Empty `log_subject` (the default) gates the publish with a single `len(s) > 0` branch — zero hot-path cost. Users wire any subscriber locus they want (`subscribe "io.tcp.**" as on_evt of type std::io::tcp::LogEvent`) — stderr sink, structured log, metrics, ring buffer; the bus is the indirection so no `Logger` interface or per-Stream sink locus is needed. Closes the "I/O lib is silent by default with no hook" friction. | `lotus_tcp_*` C primitives |
| `std::io::udp` | `bind(host, port) -> Int fallible(IoError)` (`host=""` → INADDR_ANY); `send(fd, host, port, msg)`, `recv(fd, max_bytes)`, `recv_into(fd, buf: Bytes, max_bytes)`, `close(fd)`. Multicast (2026-05-26 P1): `join_group(fd, group, iface) -> () fallible(IoError)` (iface=`""` → INADDR_ANY), `leave_group(fd, group, iface)`, `set_multicast_ttl(fd, ttl)` (0..255), `set_multicast_loop(fd, enabled: Bool)` (whether the sender receives its own packets), `set_multicast_iface(fd, addr)`. Transparent setsockopt pass-through (P2): `set_option_int(fd, level, name, value)`, `set_option_bool(fd, level, name, enabled)`, `get_option_int(fd, level, name) -> Int` — paired with `std::io::sockopt::<NAME>()` named constants below for the `level` / `name` args. Source-bearing recv + timeouts (2026-05-26 P4): `recv_with_source(fd, max_bytes) -> Bytes fallible(IoError)` populates a thread-local source cache; `last_source_host() -> String` / `last_source_port() -> Int` read the cache from the most-recent recv_with_source on the current thread (errno-style; read immediately after recv). `set_recv_timeout(fd, d: Duration) -> () fallible(IoError)` / `set_send_timeout(fd, d: Duration)` wrap `SO_RCVTIMEO` / `SO_SNDTIMEO` (they take a struct timeval so can't ride `set_option_int`); `d = 0` disables (blocking default). Datagram boundaries preserved. **`std::io::udp` is the raw-socket primitive, not a bus transport** — its `recv` / `send` calls don't carry the bus's typed-payload-dispatch contract. For UDP-as-bus see the `std::bus` row's `udp://host:port` substrate transport (shipped 2026-05-26): single URL scheme covers IPv4 unicast and multicast, dispatch goes through the same `LOTUS_BUS_CONFIG` route as `unix://`, lossy delivery (publisher-side "sendto returned" durability; subscribers best-effort). | `lotus_udp_*` C primitives |
| `std::io::sockopt` | Named-constant getters returning the platform's numeric value for each setsockopt level / name. Use as the `level` / `name` args to `std::io::udp::set_option_*` / `get_option_int`. Each is a zero-arg fn (`std::io::sockopt::SO_RCVBUF()` etc.) so the value tracks the kernel headers; cross-platform without hardcoding. Shipped (2026-05-26): `SOL_SOCKET`, `IPPROTO_IP`, `IPPROTO_IPV6`, `IPPROTO_TCP`, `IPPROTO_UDP`, `SO_REUSEADDR`, `SO_REUSEPORT`, `SO_RCVBUF`, `SO_SNDBUF`, `SO_RCVTIMEO`, `SO_SNDTIMEO`, `SO_BROADCAST`, `SO_KEEPALIVE`, `SO_LINGER`, `SO_PRIORITY`, `SO_BINDTODEVICE`, `IP_TTL`, `IP_TOS`, `IP_MULTICAST_TTL`, `IP_MULTICAST_LOOP`, `IP_MULTICAST_IF`, `IP_ADD_MEMBERSHIP`, `IP_DROP_MEMBERSHIP`, `IP_PKTINFO`. PMTU surface added 2026-05-27: `IP_MTU_DISCOVER` + `IP_PMTUDISC_DONT` / `IP_PMTUDISC_WANT` / `IP_PMTUDISC_DO` / `IP_PMTUDISC_PROBE` (Linux-only; returns -1 on platforms missing the constant — caller can detect and skip). TCP option added 2026-06-13: `TCP_NODELAY` (use with `IPPROTO_TCP` as `level`; or reach for the `std::io::tcp::set_nodelay` convenience). | `lotus_sockopt_<NAME>` C getters |
| `std::io::tls` | Client-side TLS via system OpenSSL. `connect(host, port) -> Int fallible(IoError)` does the TCP connection + TLS 1.2+ handshake with SNI + system-trust-store cert verification. `send_bytes` / `recv_bytes` / `recv_into` / `close` over the handshaked connection. Send/recv timeouts (2026-06-12): `set_recv_timeout(handle, d: Duration) -> () fallible(IoError)` / `set_send_timeout(handle, d: Duration)` wrap `SO_RCVTIMEO` / `SO_SNDTIMEO` on the connection's underlying socket fd (the handle-aware siblings of the `std::io::tcp` ones, which take a raw fd). With a recv timeout set, `recv_into` returns the `-2` "timed out, retryable" sentinel (not `-1`/fatal) when `SSL_read` yields `SSL_ERROR_WANT_READ` — so a long-lived client can bound a blocking read and run connection-liveness work (ping/pong) instead of hanging forever on a half-open connection. **Fast-path siblings** (2026-06-14): `set_nodelay(handle, on: Bool) -> () fallible(IoError)` (TCP_NODELAY on the underlying fd — reuses the tcp primitive) and `recv_stamped_into(handle, buf: Bytes, max) -> Int` with `last_recv_kernel_ns() -> Int` / `last_recv_user_ns() -> Int`, the TLS siblings of the `std::io::tcp` versions. `set_rx_timestamps(handle, on: Bool) -> () fallible(IoError)` enables `SO_TIMESTAMPNS`. The kernel timestamp rides the *socket* recvmsg but `SSL_read` sits in front of the socket, so the TLS connection uses a custom BIO whose read does `recvmsg` + the defensive `SCM_TIMESTAMPNS` cmsg walk, capturing the stamp on the socket fill that feeds `SSL_read` — `last_recv_kernel_ns` is the last segment's kernel RX stamp (0 when the path delivered none, e.g. no NIC RX timestamping), `last_recv_user_ns` is a `CLOCK_REALTIME` stamp at `SSL_read` return. (The recvmsg cmsg is the path that carries `SO_TIMESTAMPNS`; the `SIOCGSTAMPNS` ioctl reads `sk_stamp`, which that option does not populate.) Process-global `SSL_CTX` runs with `SSL_MODE_RELEASE_BUFFERS` — OpenSSL releases its read/write buffers between records so long-running TLS clients don't accumulate ~32 KiB per idle connection. **Allocation (audit, fast-protocol-I/O #6):** `recv_into` is zero-alloc on the Hale side — `SSL_read` decrypts straight into the caller's reserved buffer, no per-record malloc in the binding (the `tcp`/`udp` `recv_into` siblings likewise; pinned by `crates/hale-codegen/tests/recv_zero_alloc.rs` via the `std::diag` counter). The only per-record TLS allocation is OpenSSL-internal: with `SSL_MODE_RELEASE_BUFFERS` set, a released read buffer is re-malloc'd on the next record — a deliberate memory-vs-malloc tradeoff. An always-busy latency-critical connection that prefers zero per-record malloc over frugal idle memory would clear that mode (retain the buffers); that knob isn't exposed yet (no consumer). The `lotus_tls.c` TU compiles separately so helper tests linking `lotus_arena.c` directly don't drag in libssl/libcrypto. | `lotus_tls_*` in `runtime/lotus_tls.c` |
| `std::io::MirrorRing` | Double-mmap "magic ring" wrap-free buffer (2026-06-13, fast-protocol-I/O #3). `MirrorRing { cap: Int }` (power-of-two, page-multiple) mmaps a `memfd` of `cap` bytes twice back-to-back, so any window ≤ `cap` that straddles the physical seam is one contiguous slice — a parser (or a recv) never copies or special-cases the wrap. `readable() -> BytesMut` / `writable() -> BytesMut` hand out a raw `{ptr,len}` window over the live / free region (read it zero-copy with `std::bytes::{read_*, at, find_byte}`, which accept a `BytesMut` window directly); `commit(n)` advances the write cursor, `skip(n)` the read cursor; `len()` is the buffered byte count. `recv_from(fd, max) -> Int` recvs straight into the contiguous free window and auto-commits (`>0` bytes / `0` EOF-or-full / `-1` fatal / `-2` retryable, like `recv_into`). Opt-in (2× address space + a memfd per ring — wrong as a default recv buffer; for the common case use a `BytesBuilder` accumulator). Linux only (`memfd_create`); birth fails (`alloc_failed`) elsewhere or on a non-power-of-two / non-page-multiple `cap`. | `lotus_mirror_ring_*` in `runtime/lotus_arena.c` |
| `std::shm` | In-band record-header field delivery for a foreign ring (2026-06-13, shm-ring-interop). When a `layout:`-bound subscriber's `ring_layout` declares `record_header_bytes` with in-band header scalars (a per-record fixed header before the payload — e.g. a sequence number and a producer wire-arrival timestamp), `last_record_seq() -> Int` / `last_record_kernel_ns() -> Int` / `last_record_user_ns() -> Int` read the header fields of the record currently being delivered, called from inside the handler (errno-style thread-local, the same read-immediately idiom as `tcp::recv_stamped` — the value is the most-recent record's, valid for the duration of the handler). Each returns `0` when the bound layout declares no corresponding header field. The names map to the layout's declared header scalars by role; the layout's `recheck post_copy` guard ensures a torn header isn't surfaced. | `lotus_shm_*` in `runtime/lotus_shm_ring.c` |
| `std::diag` | Test-time gate counters (2026-06-13). `heap_alloc_count() -> Int` returns the cumulative number of heap allocations (malloc / realloc / calloc / mmap) the runtime has made; `syscall_count(name: String) -> Int` returns the cumulative count of a wrapped I/O syscall (`"recv"`, `"recvmsg"`, `"read"`, `"write"`, `"send"`, `"sendto"`). Read a counter before and after a steady-state region and assert the delta — the runtime/test-time complement to compile-time `--warn-unbounded-alloc` ("this loop did zero heap allocs" / "exactly one read per poll"). Both return `-1` when the counting shim is absent (sanitizer builds — TSan/ASan interceptors collide with the `-Wl,--wrap` shim), so a caller can distinguish "gate unavailable in this build" from a real `0`. Counters are process-wide and monotonic; `syscall_count` returns `-1` for an unrecognized name. The wrap shim is compiled into every default (`-O2`) build at the cost of one relaxed-atomic increment per allocation / wrapped syscall — only the runtime's own calls are routed (libc- and libssl-internal I/O is untouched). | `lotus_diag_*` + `__wrap_*` in `runtime/lotus_arena.c` |
| `std::http` | `Request` + `Response` types (`Response.headers: String` carries CRLF-joined user-supplied headers — no trailing CRLF — for Set-Cookie / CORS / custom headers); `parse_request`, `write_response`; case-insensitive symmetric `header(receiver, name)` lookup; `Handler` interface (`fn handle(req: Request) -> Response`); `Server` locus with `shutdown()` (cross-thread safe — see [§ Server.shutdown](#servershutdown--interruptible-accept-loop)) and optional `ready_signal: String` for piped oracles. **Bus-routed observability** (2026-05-27): `Server` gains a `log_subject: String = ""` param and a `bus { publish "io.http.**" of type std::io::tcp::LogEvent; }` declaration. When `log_subject` is set, listen-start / accept / listen-close events publish on the configured subject; empty (default) keeps the hot path at a single `len > 0` branch per event. Reuses the `std::io::tcp::LogEvent` type so one subscriber can observe both TCP and HTTP layers. | `runtime/stdlib/http.hl` |
| `std::json` | `Builder` locus (streaming output assembly — see [§ json::Builder](#stdjsonbuilder--streaming-output-api)); `escape_string` / `unescape_string` (RFC 8259); `find_string_field` / `find_int_field` / `find_bool_field` (flat-object lookup); `find_field_raw(json, name) -> String` (bracket-balanced raw substring over nested objects/arrays — the recursive-descent primitive); `ArrayIter` + `array_first` / `array_next`, and the span-bearing `ArrayIterSpan` cursor `array_first_span(json, start) -> ArrayIterSpan` / `array_next_span(it) -> ArrayIterSpan` (carry the element's byte range rather than an owned substring — the allocation-free array-walk sibling of the object cursor below). Range-bearing iter family (2026-05-26): `iter_find_field_range(it, json, name) -> JsonFieldRange` and `iter_find_string_field_range(it, json, name) -> JsonFieldRange` return `{ok, start, end_pos}` instead of an owned-String substring; paired with the `std::str::range_*` family for fully allocation-free per-element walks on large arrays (the fathom-class workload where per-field allocation dominates arena pressure). No nested-tree shape at v1 — re-feed substrings into the same surface for nested walks. Single-pass object member cursor (2026-06-09): `object_first(json) -> ObjectIterSpan` / `object_next(it, json)` walk `{...}` members once, with `obj_key_eq(it, json, name) -> Bool` / `obj_key_len(it) -> Int` for key dispatch and `obj_value_int` / `obj_value_bool` / `obj_value_string` / `obj_value_raw(it, json)` reading the current value from its source range (no per-field rescan; nested objects/arrays on unmatched keys are skipped whole by the depth scan). This is the substrate a compiler-generated, schema-specialized parser drives — and the seam a future SIMD structural index slots under. | `runtime/stdlib/json.hl` |
| `std::test` | `assert(cond, msg)`, `assert_eq_int`, `assert_eq_str` | `runtime/stdlib/test.hl` |
| `std::log` | `Logger`, `LogEvent`, `StdoutSink` (subscribes `log.**`) | `runtime/stdlib/log.hl` |
| `std::math` | `sqrt`, `exp`, `log`, `floor`, `ceil`, `pow`, `tanh`, `nan`, `is_nan`, `inf`, `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`, `int_to_float`, `float_to_int`, `round`, `trunc` | path-call dispatch into libm (`nan`/`inf`/`is_nan` are IEEE 754 sentinels; trig added 2026-05-23 for spatial / animation code). `int_to_float`/`float_to_int` (2026-06-11) are named `sitofp`/`fptosi` conversions — round-toward-zero, callable in any expression position; see `spec/types.md` § Explicit numeric conversions. `round(Float) -> Int` / `trunc(Float) -> Int` are the Float→Int siblings that return `Int` directly: `round` is round-half-away-from-zero (`3.7 → 4`, `2.5 → 3`, `-2.5 → -3`), `trunc` is round-toward-zero (an alias of `float_to_int`). Both lower to pure LLVM (`fptosi`, plus a compare/select half-shift for `round`) — **no libm symbol**, so unlike `floor`/`ceil` (which stay libm and return `Float`) they need no host import on the `wasm32` target. |
| `std::crypto` | `sha1(b) -> Bytes` (20-byte), `sha256(b) -> Bytes` (32-byte), `hmac_sha256(key, msg) -> Bytes` (32-byte), `sha512(b) -> Bytes` (64-byte) / `hmac_sha512(key, msg) -> Bytes` (64-byte) (the 64-bit-word SHA-2 sibling — FIPS 180-4 SHA-512 + RFC 2104 HMAC over a 128-byte block; same non-fallible shape as `hmac_sha256`, hand-rolled, no libcrypto; added 2026-06-25 for Kraken / Gate.io venue order-entry auth, which sign with HMAC-SHA512), `crc32(b) -> Int` (4-byte IEEE 802.3 checksum returned as Int; reversed polynomial `0xEDB88320`, init `0xFFFFFFFF`, final XOR `0xFFFFFFFF` — the zlib / Python `binascii.crc32` variant; added 2026-05-27). `ecdsa_p256_sign(key, message) -> Bytes` / `ecdsa_p256_verify(pubkey, message, sig) -> Bool` (ES256 — ECDSA over NIST P-256 + SHA-256; `key` is a PEM EC private key, SEC1 or PKCS#8; `pubkey` is PEM SPKI; signature is raw `r‖s`, 64 bytes, the JWS/COSE form JWT wants; added 2026-06-03 for venue/JWT auth). `ecdsa_p256_sign` has two forms: the bare call returns an empty Bytes on failure (the `base64::decode` convention — `len(sig) == 0` ⇒ failed), and in an `or` context it is `Bytes fallible(CryptoError)` (2026-06-04), so `let sig = std::crypto::ecdsa_p256_sign(key, msg) or raise;` propagates a structured `CryptoError { kind: String, detail: String }` (`kind` = the op tag `"ecdsa_p256_sign"`; `detail` = the failure reason) — read it via `or handler(err)` / `or fail err` / `or <substitute>` exactly like `IoError` / `ParseError`. The hashes + crc32 are hand-rolled (no libcrypto); ECDSA is OpenSSL-backed (rides the libssl/libcrypto link TLS already pulls). | `lotus_crypto_*` (hashes in `runtime/lotus_arena.c`; ECDSA in `runtime/lotus_tls.c`) |
| `std::os` | `getrandom(n: Int) -> Bytes fallible(IoError)` (CSPRNG; `getrandom(2)` with `/dev/urandom` fallback) | `lotus_os_getrandom` C primitive |
| `std::rand` | `next_int(max: Int) -> Int` — a uniform-ish integer in `[0, max)` drawn from a shared xorshift64\* generator; `seed_from_time()` re-seeds that generator from the wall clock. **Not cryptographic** (deterministic PRNG, process-shared state) — for security-sensitive randomness use `std::os::getrandom`. | `lotus_rand_*` C runtime |
| `std::ts` | Tree-sitter parse substrate (m96 — the `std::ts::*` routes back the higher-level `Lang` locus). `parse_go(src: String) -> Int` parses Go source and returns an opaque **tree handle** (`Int`); `root_node(tree) -> Int` returns the root **node handle**. Node navigation (all handles are `Int`): `node_child_count(node)` / `node_named_child_count(node)`, `node_child(parent, i)` / `node_named_child(parent, i)`, `node_is_named(node) -> Int` (`0`/`1`). Kind, text, spans: `node_kind(node) -> String`, `node_text(node) -> String`, `node_start_byte(node) -> Int` / `node_end_byte(node) -> Int`. Go is the only bundled grammar at v1; the tree-sitter shim staticlib is linked into the build (gating the link on actual `std::ts` use is future work). | `lotus_ts_*` + tree-sitter shim |
| `std::bus` | `__StdBusAdapter` interface (contract for user-supplied bus transports — a single `fn send(subject: String, bytes: Bytes)` method); `__local_dispatch(subject, bytes)` primitive lets an adapter relay received wire-bytes into the local handler set. **Substrate transport URL schemes** (resolved at runtime by `lotus_bus_load_config` from `LOTUS_BUS_CONFIG=<file>`): `unix://<path>` (AF_UNIX SEQPACKET; m58); `udp://<host>:<port>` (2026-05-26 — IPv4 UDP, single scheme covers unicast and multicast: addresses in `224.0.0.0/4` trigger `IP_ADD_MEMBERSHIP` on the subscribe side, everything else takes the plain unicast bind/sendto path; lossy delivery — publishers get "sendto returned" durability, subscribers best-effort; gap recovery is a deployment concern via app-layer repeaters, MoldUDP-style). Each LOTUS_BUS_CONFIG line: `subject = <url>:<role>` where role is `listen` or `connect`. Other protocol-layer transports (NATS, MQTT, raw-TCP-with-framing) come in via user adapters (the `__StdBusAdapter` route above). **Payload size** (2026-05-27): the substrate handles bus payloads up to ~64 KB (`LOTUS_PAYLOAD_MAX`, sized to UDP datagram max). Mailbox cells inline payloads ≤ 512 B (`LOTUS_PAYLOAD_INLINE` — zero malloc on the hot path); larger payloads route through a per-cell `malloc` that the drain path frees after the handler returns. The UDP transport's wire buffer is sized at LOTUS_PAYLOAD_MAX; the kernel-side socket receive buffer takes its size from `SO_RCVBUF` (default ~208 KB on Linux, raise via `LOTUS_BUS_UDP_RCVBUF=<bytes>` env). UDP `sendto` failures log once per errno class to stderr (`EMSGSIZE` typically means path-MTU mismatch; see `std::io::sockopt::IP_MTU_DISCOVER` for the DF-bit knob). | `runtime/stdlib/bus.hl` + `lotus_bus_*` C runtime |

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
ops without a useful path label.

`Stream.send` / `Stream.recv_bytes` / `Stream.send_bytes` are
*locus methods* on `std::io::tcp::Stream`. They use the
legacy sentinel shape (returning -1 / 0 on failure) for
historical reasons — they predate open-question #24
(2026-05-25) lifting the "no fallible on locus methods"
restriction. Migrating them to `fallible(IoError)` is
follow-on work; the sentinel surface stays load-bearing for
the existing stdlib consumers. The same is true of
`std::io::stdin::read_line` (path-call but pairs with
`read_line_status` for EOF-vs-error distinction).

**`Stream` fd ownership** (`owns_fd: Bool = true`, 2026-05-29).
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

```hale
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
