# Standard library

Bundled with the toolchain, no separate install required.

> **Status (m95):** Phases 1-5 of the v1.x stdlib roadmap are
> **sealed**. Phase 1 (substrate floor) at m76, Phase 3 (HTTP)
> at m86, Phase 2 v0.1 (assertions) at m88, Phase 4 v0.1
> (markdown) at m91, Phase 5 (doc-server capstone) at m92,
> stdlib organization (per-domain `.ap` files) at m93. **Phase 6
> (substrate for the IDE) underway:** m94 bus subject wildcards,
> m95 `std::log` namespace. See `docs/src/std/roadmap.md` for
> the v1.x plan and the per-phase tables below for what shipped
> under each. The aspirational "v0 module map" section near the
> bottom of this file was sketched pre-rename; treat the
> per-phase tables as authoritative for what's actually shipped.

## Phase 1 — what shipped (sealed m76)

The first arc of the v1.x stdlib build-out: importable I/O
substrate plus a working capstone example.

| Milestone | What it shipped |
|-----------|-----------------|
| m71 | Magic `std::*` path resolver in codegen + `std::process::pid()` proof symbol. No general module system; `std::*` is the only recognized prefix. |
| m72 | `lotus_tcp_*` C substrate. AF_INET sibling adapter to the m57 AF_UNIX SEQPACKET transport. Internal 8-byte LE length-prefix framing preserves the bus's atomic-message contract over `SOCK_STREAM`. |
| m73 | `std::io::tcp::Listener` stdlib locus. Bundled-source mechanism (`runtime/stdlib/`) + path-rewrite at qualified struct literals. Real birth/run/dissolve lifecycle wired through `listen_socket` / `accept_one` / `close_fd` path-call primitives. Single-accept shape (resolved in m83). |
| m74 | `lotus_fs_*` C substrate: `read_file`, `write_file`, `file_size`, `file_exists`. POSIX wrappers, no buffering, one-shot synchronous shape. (`read_dir` resolved in m90.) |
| m75 | `std::io::fs::*` Aperio surface. Functional path-call shape (mirrors `std::process::pid`), not locus-wrapped — one-shot file ops don't need lifetime-of-a-stream. `read_file` allocates from the m70 lazy global payload arena so the returned `String` outlives the call frame. |
| m76 | `examples/io-demo/` capstone exercising both surfaces. Reads optional config, listens, accepts one connection, writes a log. Integration test in `tests/io_demo.rs` drives it under CI. |

## Inter-phase cleanup (m77 → m81)

Bridge milestones between Phase 1 and Phase 3 — argv/env
plumbing and the language additions Phase 3 needed.

| Milestone | What it shipped |
|-----------|-----------------|
| m77 | `std::env::args_count` / `arg` / `var` / `var_exists`. Lifted main's signature to `i32 @main(i32, ptr)` so codegen captures argc/argv into a runtime stash via `lotus_env_init` in main's prelude. |
| m78 | `std::str::parse_int` / `can_parse_int`. strtoll-based, base 10, strict trailing-char check. **2026-05-17: flipped to `Int fallible(ParseError)`** — see the "Fallible-flipped paths" entry below. |
| m79 | `std::time::sleep` / `monotonic` aliases under `std::*` namespace; `std::process::exit(code)`. |
| m80 | Function-pointer language addition. `CodegenTy::FnPtr`, parser support for `fn(T) -> R` types, codegen lowering of fn names as values + indirect calls through fn-pointer fields. The Phase 3 prerequisite. |
| m81 | Stream locus + non-self method calls + `send` / `recv` / `connect` primitives. New `lower_external_method_call` for `obj.method(args)`. Bundled `Stream` declaration. |

## Language addition driven by m81 — m82 (locus-all-the-way-down)

m81's Stream test surfaced an Aperio v0 lifecycle issue:
custom `dissolve()` on a let-bound locus literal fired
eagerly at the end of the struct-literal expression, not at
the binding's scope exit. m82 fixes it: let-bound locus
literals defer dissolve to the enclosing fn's scope-exit
flush. The user-visible binding is the handle; the locus
instance lives until the binding's scope ends. One construct,
one mental model — no parallel "handle type" needed. See
`spec/semantics.md` for the operational rule.

## Phase 3 — HTTP (sealed m86)

Multi-accept Listener + request parser + response writer +
end-to-end working server.

| Milestone | What it shipped |
|-----------|-----------------|
| m83 | Multi-accept Listener with `on_connection: fn(Stream)` callback. Composes m80 + m81 + m82. Per-connection Stream lifecycles owned by a free-fn helper (`handle_one_connection`) whose scope-exit flush closes the fd between iterations. |
| m84 | `std::http::Request` + parser. Request and Response are `type` records (no lifecycle). Adds `std::str::index_of` substring-search primitive. STDLIB_PATH_RENAMES generalized to cover both loci and types. |
| m85 | `std::http::write_response`. Builds the HTTP/1.1 wire format (status line + Content-Type + Content-Length + Connection: close + body) via String concatenation, ships through `Stream.send`. |
| m86 | `examples/http-hello/` — Phase 3 capstone, real curl-able HTTP server in ~70 lines of Aperio. |

## Phase 2 v0.1 — testing primitives (sealed m88)

Bedrock assertion library, written purely in Aperio.

| Milestone | What it shipped |
|-----------|-----------------|
| m87 | `std::test::assert(cond, msg)`, `std::test::assert_eq_int(actual, expected, msg)`, `std::test::assert_eq_str(actual, expected, msg)`. Implementations compose `std::process::exit`. Pass = silent + exit 0; fail = "ASSERTION FAILED: <msg>" on stdout + exit 1. |
| m88 | Aperio self-tests on top of std::test (`tests/aperio_self_test.rs`). Six `.ap` programs assert on real Aperio behavior using the new layer. |

What's NOT in v0.1 (each a future-milestone arc): `aperio
test` CLI runner, `assert_rejects` (compile-time-error tests),
`assert_closure` (closure-test introspection), benchmarks,
property-based testing.

## Phase 4 v0.1 — markdown (sealed m91)

Block-level markdown → HTML rendering, written purely in
Aperio. Plus the Phase 5 prerequisites that landed alongside.

| Milestone | What it shipped |
|-----------|-----------------|
| m89 | `Bytes` codegen — the binary-safe sibling of String. Memory layout `[i64 len][u8 data[len]]`; same single-pointer ABI as String. `len(b)`, `std::io::fs::read_bytes`, `std::io::tcp::send_bytes`, `Stream.send_bytes` method. Embedded NUL bytes preserved across all three. |
| m90 | `std::io::fs::list_dir(path) -> String`. Newline-separated entry names (skipping `.` / `..`). v0 shape; the index-API sibling `list_dir_count` / `list_dir_at` (Phase 2e) is the structured alternative — no parametric `List<T>` needed. |
| m91 | `std::text::md_to_html(md) -> String`. ATX headings, multi-line paragraphs, fenced code blocks, HTML escaping. Inline formatting (bold/italic/code/links) deferred. |

## Phase 5 — the capstone (sealed m92)

| Milestone | What it shipped |
|-----------|-----------------|
| m92 | `examples/docs-server/main.ap`. Real HTTP server in ~200 lines of Aperio that lists and renders markdown files from a configured directory. Composes Listener (m83) + parse_request (m84) + write_response (m85) + read_file (m74) + the `list_dir_count` + `list_dir_at` index API (m90, post-2026-05-16 cleanup) + md_to_html (m91) + Stream lifecycle (m82) + function pointers (m80). |

## Stdlib organization — m93

The bundled stdlib was a single ~530-line `runtime/stdlib.ap`
through m92. m93 split it into per-domain files under
`runtime/stdlib/`:

| File | Contents |
|------|----------|
| `core.ap`   | Helpers used across the stdlib (`replace_all`, `html_escape`). |
| `io_tcp.ap` | `Stream` + `Listener` loci + `handle_one_connection` + `default_on_connection`. |
| `http.ap`   | `Request` + `Response` types + `parse_http_request` + `write_http_response` + `status_phrase`. |
| `text.ap`   | `md_to_html` + line tokenization helpers. |
| `test.ap`   | `test_assert` + `assert_eq_*` variants. |
| `log.ap`    | (m95) `LogEvent` + `Logger` + `StdoutSink`. |

`STDLIB_AP_SOURCE` in codegen is now
`concat!(include_str!("core.ap"), "\n", include_str!("io_tcp.ap"), ...)`.
Order matters — `core.ap` must precede `text.ap` (markdown
depends on `core`'s helpers); `io_tcp.ap` must precede
`http.ap` (HTTP signatures reference `Stream`). Each file
header documents its constraint.

## Phase 6 — substrate (underway, m94+)

| Milestone | What it shipped |
|-----------|-----------------|
| m94 | Bus subject wildcards. Trailing `**` matches zero+ remaining dot-separated segments. Subscribe-side: `subscribe "log.app.**"` catches a sub-tree. Publish-side: declaring `publish "log.**" of type T` authorizes runtime-computed subjects of that type. Three implementations (`lotus_subject_match` C, `subject_match` Rust runtime, `wildcard_match` typechecker) agree on identical semantics. |
| m95 | `std::log` namespace. `Logger` (cascading namespace via `name` + `parent_path`), `LogEvent` (level / msg / path), `StdoutSink` (subscribes `log.**`). Levels are Int constants pending enum-variant patterns. First Phase-6 user surface; written purely in Aperio composing m94. |

## What landed but isn't yet a phase capstone

- ~~**Errno surface.** Errors still collapse to `-1` / `false` /
  empty string. Disambiguation between "missing" and "permission
  denied" needs an error-introspection follow-up.~~
  **Closed 2026-05-16** by the `IoError` flip — `std::io::fs::*`
  and `std::io::tcp::*` path-calls now return
  `fallible(IoError)`; the agent addresses failures with
  `or raise` / `or fallback(err)`. See "IoError + fallible I/O"
  below.
- **Inline markdown.** Bold, italic, inline code, links — m92+.
- **HTTP keep-alive, custom headers, large bodies.** v0 hardcodes
  `Connection: close`, no header-map type, single-recv assumption
  for the request line + headers. All Phase 3 v1.0 follow-ups.
- **`aperio test` CLI runner.** Phase 2 v1.0 — currently the
  Rust integration harness fills the role.

## v1.x followups — language + stdlib (2026-05-11)

Driven by the v1.x followup list — items shipped in this session
as form-extending, parameter-populating, or substrate-tied
additions on top of F.22 v1. Each entry maps a v1.x item to its
surface.

| Add | v1.x item | Surface |
|---|---|---|
| F.22 interpreter parity | v1.x-1 | `pool X of T;` / `heap Y of T;` slots work under `aperio run` with the same `self.X.acquire/release/alloc/free` shape as codegen. |
| Cell content I/O (struct cells) | v1.x-2 | `cell.field = v` writes; `cell.field` reads. Primitive cells (`Cell<Int>` etc.) reject field access with focused diagnostic — primitive-cell content access is a later v1.x follow-up. |
| `as_parent_for Child` slot clause | v1.x-4 (surface) | Parser + typecheck accept `pool X of T as_parent_for Child;`. Runtime mechanic (borrow-mask + skip-destroy-on-borrowed) is v1.x-4b. |
| Slot-of-origin tracking on `Cell<T>` | v1.x-5 | Releasing a cell into the wrong slot is a hard error at codegen + runtime. |
| Type records hold `fn(...)` fields | v1.x-8 | `type Cmd { name: String; run: fn(); }` parses + dispatches. `c.run()` GEPs the field, loads the fn pointer, indirect-calls. |
| F-string interpolation | v1.x-10 | `f"hello {name}"` lowers to `Lit + to_string(expr) + Lit + ...`. Plain `"..."` strings keep `{` and `}` as ordinary characters (back-compat). |
| Explicit Float → Int narrowing | v1.x-11 | `Int(f)` truncates toward zero (fptosi); `Int(n)` is the identity; other types rejected. No implicit narrowing. |
| String-builder primitive | v1.x-15 | `std::str::builder_new() -> Bytes`, `builder_append(b, s)`, `builder_len(b) -> Int`, `builder_finish(b) -> String`. Doubling-realloc malloc buffer; N appends amortized O(N). Resolves reader-list_item-quadratic-concat. |
| `parse_float` + `can_parse_float` | v1.x-16 | `std::str::parse_float(s) -> Float fallible(ParseError)` (flipped 2026-05-17 alongside parse_int). Paired bool predicate `can_parse_float(s)` stays non-fallible for predicate use. |
| Bytes-builder (binary-safe sibling) | C10 (pond follow-up) | Originally shipped as a `std::bytes::builder_*` free-fn family — superseded 2026-05-19 by the `std::bytes::BytesBuilder` locus (see the BytesBuilder-locus row below). The free-fn surface is gone; the C primitives (`lotus_str_builder_t`-backed buffer, `[i64 len]`-prefixed binary-safe append, fresh-blob-in-payload-arena finish) remain as the locus methods' externs. Driver consumers (`pond/http/client/wire.ap`, `pond/agent/llm/wire.ap`) migrate to the locus form. Interpreter parity preserved in `aperio-runtime::builtins`. |
| Bytes-builder in-place ops (Phase 0) | 2026-05-19 (pond/websocket follow-up) | Long-lived recv-loop accumulator ops: `shift_front(n)` (drop first n bytes via memmove, capacity preserved), `clear()` (len=0, capacity preserved), `snapshot() -> Bytes` (copy `[0..len)` into a fresh `[i64 len][u8 data[len]]` Bytes blob in the bus payload arena, builder unchanged). Released the per-iteration `concat`/`slice` churn that pond/websocket's recv loop hit; the buffer now stabilizes at max-frame-size. Originally shipped as free fns alongside the C10 family — folded into the `std::bytes::BytesBuilder` locus method surface on 2026-05-19. The "leak unless finish" hazard that motivated the original `builder_free` is closed by the locus's dissolve cascade — no explicit free is needed. |
| recv_into family (Phase 1) | 2026-05-19 (pond/websocket follow-up) | `std::io::tcp::recv_into(fd: Int, buf: std::bytes::BytesBuilder, max_bytes: Int) -> Int` and tls / udp siblings. `buf` is a `BytesBuilder` locus instance; codegen extracts the internal `handle` slot at the call boundary and writes into the builder's tail, growing on insufficient headroom, bumping len by the count read. Returns POSIX read(2) semantics: `> 0` bytes appended, `= 0` peer closed cleanly (TCP) / zero-length datagram (UDP), `< 0` fatal error. Non-fallible — the return value carries error state, mirroring read(2). Eliminates the per-recv allocation in `g_bus_payload_arena` that the prior `recv_bytes` surfaces produce. The typecheck — `buf` must be a `BytesBuilder` locus, not a raw `Bytes` — is the load-bearing enforcement that closes the prior silent-misread footgun: with the builder lifted out of the `Bytes` type, calling `std::bytes::at(buf, i)` on a builder now fails at typecheck rather than reading the cap/len/buf header. The `recv_bytes` surfaces remain for one-shot reads. |
| BytesBuilder locus (`std::bytes::BytesBuilder`) | 2026-05-19 | Lifts the entire `std::bytes::builder_*` family out of the free-fn surface and into a stdlib locus. Construction: `let b = std::bytes::BytesBuilder { initial_cap: N };` (defaults to 64 if elided). Method surface: `b.append(chunk: Bytes)`, `b.len() -> Int`, `b.snapshot() -> Bytes`, `b.shift_front(n: Int)`, `b.clear()`, `b.finish() -> Bytes`, `b.view() -> BytesView` (Phase-2 (1), 2026-05-19; return type promoted from `Bytes` to `BytesView` on 2026-05-20 per F.30 — non-owning view aliasing the builder's `[i64 len][u8 data]` region; zero allocation, zero copy; lifetime valid until the next mutation on the source builder; the load-bearing primitive for "parser reads via `std::bytes::at`/`len`/`slice` over an rx_buf accumulator without snapshotting per peel attempt" — the dominant residual leak source after Phase 1). Lifecycle: birth allocates the underlying `lotus_str_builder_t` header + buffer via `lotus_bytes_builder_new(initial_cap)`; dissolve frees on scope exit. **The motivation is type discrimination at design time.** The builder header (`{cap, len, buf*}` separately-malloc'd body) is NOT layout-compatible with a regular `Bytes` blob (`[i64 len][u8 data]` contiguous) — the prior surface conflated the two under the `Bytes` static type, so `std::bytes::at(builder, i)` silently misread the cap field as the length and ran off the end. With the builder typed as its own locus, that confusion fails at typecheck. The Aperio axiom (everything with lifecycle + invariants is a locus, types are pure data) makes this the principled shape — the prior free-fn surface was a workaround for not having a locus to anchor the buffer's flow. Internal C-primitive bridges live behind `std::bytes::builder::__*` paths (not for user code; the locus's method bodies are the only legitimate callers). Failure routing: `append()` routes realloc-NULL (and null-handle from a failed birth) through `violate alloc_failed` per F.27 — the locus declares `closure alloc_failed { captures: initial_cap; epoch inline; }` and `append` violates it on the failure path. Owners of the BytesBuilder bind an `on_failure` policy to handle (restart / drain / bubble); an unhandled violation bubbles past `main` and exits the process non-zero. **Phase-3 zero-alloc additions (2026-05-19):** `b.append_slice(src: Bytes, lo: Int, hi: Int)` copies `src[lo..hi)` directly into the builder's tail — eliminates the `slice + append` pair's intermediate Bytes wrapper that otherwise lands in g_bus_payload_arena. Routes alloc-fail (and out-of-range indices) through `violate alloc_failed` per F.27. `b.text_view() -> StringView` (return type promoted from `String` to `StringView` on 2026-05-20 per F.30) returns a non-owning view aliasing the builder's buffer; lifetime tied to "no mutation while view is in use" (same documented-and-trusted contract as `view()`). The builder's buffer reserves one trailing-NUL slot past `cap` and maintains `buf[len] == '\0'` after every mutation, so the returned C string is well-formed for the `lotus_str_*` surface — zero allocation, zero copy. Use case: a recv loop publishes a text-typed message field referencing the accumulator without per-message `lotus_str_from_bytes` allocation. **Birth-time violate (F.27 extension, 2026-05-19):** `birth()` now routes `lotus_bytes_builder_new(initial_cap) → NULL` (malloc fail) through `violate alloc_failed` at construction. The codegen restriction on lifecycle-body violate was lifted; the same divergent-return + parent-on_failure machinery fires from birth as from method bodies. Construction-time fail surfaces immediately rather than waiting for first method use. **`snapshot()` / `finish()` violate routing (2026-05-19):** payload-arena alloc failures (handle-NULL, arena create fail, lotus_bytes_create fail) now route through `violate alloc_failed` per F.27. The C primitives use a dedicated alloc-fail sentinel pointer on every failure path while success paths emit a fresh arena-allocated blob (even for len=0). The locus method body discriminates via `std::bytes::__is_alloc_fail(out)` before returning. The previous "empty-on-success aliases empty-on-fail" hazard is closed; the channels meet at the `violate` boundary, never as a silent zero-length blob. |
| `base64::decode` | v1.x-16 | `std::text::base64::decode(s) -> Bytes`. Standard alphabet, whitespace tolerated, non-alphabet / wrong padding → empty Bytes. Inverse of `base64::encode`. |
| `std::str::lower` / `std::str::upper` | (follow-up) | ASCII case folding. One-pass C-runtime primitives — non-ASCII bytes pass through. Used internally by `std::http::header` for RFC 7230 case-insensitive lookup; `apps/cli`'s `upper()` helper now delegates here too. |
| `std::str::trim` / `std::str::replace` | (follow-up) | `trim(s)` strips ASCII whitespace (space / tab / CR / LF) from both ends. `replace(s, needle, replacement)` does greedy non-overlapping substring replace; empty needle is a no-op (avoids the infinite-replace footgun). Both C-runtime primitives; both anchor results in the bus payload arena. |
| `std::str::repeat` / `pad_left` / `pad_right` | (follow-up) | `repeat(s, n)` returns n concatenated copies (n <= 0 → empty). `pad_left(s, width, pad)` and `pad_right(s, width, pad)` align to a target width using the first char of `pad` as the fill byte (empty pad → space). No truncation — if `len(s) >= width`, returns `s` unchanged. Common for separator lines, column-aligned table output, and right-aligned numeric formatting. |
| F-string interpolation supports nested string literals | (follow-up) | The interpolation-capture loop tracks quote state via `\"` toggles, so `f"got: {func(\"abc\")}"` parses cleanly. `{` / `}` inside the interpolated string don't perturb depth counting. Limitation: a `"` inside the nested string can't be re-escaped (would need triple-backslash) — flagged in the lexer source. |
| `std::time::now() -> Int` wall-clock seconds | C7 (pond follow-up) | Wraps `clock_gettime(CLOCK_REALTIME, &ts)` via the new `lotus_time_now_seconds` C primitive; returns `ts.tv_sec` as `Int`. Drives `pond/sessions` cookie expiries that must survive a process restart (the monotonic origin resets, the wall-clock origin does not). Observation only — NTP slewing / leap seconds can warp the value, so `std::time::monotonic` stays the basis for scheduling. The `today` shape called out in the pond handoff was deferred until a consumer surfaces a concrete date-shape need. |
| `std::math::{tanh, nan, is_nan, inf}` IEEE 754 surface | C8 (pond follow-up) | `tanh(Float) -> Float` is a direct LLVM extern into libm `tanh` (same shape as `sqrt` / `exp` / `log` / `floor` / `ceil` / `pow`). `nan() -> Float` / `inf() -> Float` are nullary and return platform-quiet-NaN / +infinity via `lotus_math_nan` / `lotus_math_inf` (backed by C's `NAN` / `INFINITY` macros, so they don't reference libm at link time — keeps the test helper binaries that compile `lotus_arena.c` without `-lm` happy). `is_nan(Float) -> Bool` is the canonical IEEE 754 `f != f` test via `lotus_math_is_nan` (returns C `int`, truncated to `i1` at the call site). All four are non-fallible — libm domain errors return NaN naturally, which is the caller's contract. Drives `pond/ml/neural` (was hand-rolling tanh from `exp`) and `pond/math/matrix` (was synthesizing `nan_sentinel()` as `0.0 / 0.0` and `is_nan(f)` as `f != f`). NaN-printing is platform-dependent (`nan` / `NaN` / `-nan` via printf %g); tests assert correctness via `is_nan(x)`, not by comparing the printed value of NaN itself. |
| `std::io::fs::rename(src, dst) -> () fallible(IoError)` | C9 (pond follow-up) | POSIX `rename(2)`; atomic on the same filesystem (EXDEV cross-fs). Backs `pond/logfmt`'s log-rotation policy — the standard "shift `.N-1` → `.N`, overwrite oldest, truncate active" cycle was previously a `read_file → write_file` chain because no rename existed. `IoError.path` is anchored to `dst` (destination is more diagnostic on the common failure modes: target dir missing, target already a non-empty dir, cross-fs). |
| `std::io::fs::unlink(path) -> () fallible(IoError)` | C9 (pond follow-up) | POSIX `unlink(2)` — removes regular files / symlinks (directories require a future `rmdir` sibling). Pairs with rename for `pond/logfmt`'s rotation; also the natural cleanup primitive for tempfiles created by `mktemp` below. Spec name `unlink` per the `unlink(2)` syscall; `pond/logfmt/FRICTION.md` proposes the synonym `remove_file` which can grow as a sibling later if friction surfaces. |
| `std::io::fs::mktemp(prefix, suffix) -> String fallible(IoError)` | C9 (pond follow-up) | Race-free temp-path allocator wrapping `mkstemps(3)`. Assembles `prefix + "XXXXXX" + suffix`, atomically open+creates the file (mode 0600), closes the fd immediately, returns the resulting path string anchored in the global payload arena. Caller owns cleanup. Backs `pond/agent/sandbox` (per-tool scratch dirs) and is the right shape for any lib needing scratch space (future `pond/agent/embeddings`, `pond/data/*`). The close-then-return-path shape inherits standard `mktemp(3)` discipline — race-free path allocation, not race-free lifecycle; an attacker with write-access to the parent dir could in principle unlink + replace between our close and the caller's reopen. A `mkstemp_fd` sibling that hands back a held-open fd can grow later if that becomes operative. `IoError.path` is the assembled template (prefix + "XXXXXX" + suffix) so the agent sees which directory failed. |
| `std::http::Response.headers` field + symmetric `header(resp, name)` lookup | C11 (pond follow-up) | Response gains a `headers: String = ""` field with the same CRLF-joined shape as `Request.headers` (header lines joined by `\r\n`, no trailing CRLF). `write_response` splices these lines in after the fixed `Connection: close\r\n` and before the blank-line separator; an empty `headers` field reproduces the prior wire bytes byte-for-byte. The path-call `std::http::header(receiver, name)` now dispatches on receiver type — Request receivers route to `__http_request_header`, Response receivers route to `__http_response_header` — so consumers read attached headers back off the same shape they wrote. Drives `pond/sessions` (Set-Cookie attachment without a custom Stream writer) and `pond/http/client` (lifts the duplicate `__find_header` walker into the stdlib). Both wrappers delegate to a shared `__http_find_header_in_block` walker to avoid duplicate scan logic. |
| `std::crypto::sha256(b) -> Bytes` + `std::crypto::hmac_sha256(key, msg) -> Bytes` | C3 (pond follow-up) | FIPS 180-4 SHA-256 (32-byte digest) and RFC 2104 HMAC-SHA256 (32-byte tag) over the sha256 primitive. Stand-alone pure-C implementation in `lotus_arena.c` (`lotus_crypto_sha256` / `lotus_crypto_hmac_sha256`) — no libcrypto / OpenSSL link dep, same shape as `lotus_crypto_sha1`. Both non-fallible; results anchored in the bus payload arena. Drops `pond/crypto`'s ~140-line pure-Aperio `sha256.ap` (which composed digests via `O(N²)` `std::bytes::concat` chains) and its `hmac.ap` wrapper. Interpreter parity in `aperio-runtime::builtins`. Verified against FIPS 180-2 vectors B.1 / B.2 / empty-string and RFC 4231 test case 1. The `sha512` / `hmac_sha512` shapes called out in `pond/crypto/FRICTION.md` were deferred — no current consumer surfaces a concrete 64-byte digest need. |
| `std::os::getrandom(n: Int) -> Bytes fallible(IoError)` | C4 (pond follow-up) | Cryptographically-strong random bytes via the Linux `getrandom(2)` syscall, with `/dev/urandom` as a transparent fallback on `ENOSYS` (kernels too old to expose the syscall) and on non-Linux platforms. EINTR retries in-place; short reads are looped until `n` bytes are filled. `n <= 0` returns an empty Bytes (no error); `n > 8192` errors with `IoError.kind="invalid"` — the per-call cap is an ergonomics floor (key material is 16-64 bytes; session tokens 16-32) and callers wanting more can loop. `IoError.path` is anchored to the static label `"std::os::getrandom"` since there's no caller-supplied path. The returned Bytes lives in the bus payload arena (same lifetime as `std::io::fs::read_bytes`). Resolves the `pond/crypto` `no-csprng-getrandom` friction note — `random_bytes` can now flip from the xorshift64 placeholder to a real CSPRNG. |
| `std::io::tcp::connect` DNS fallback | C6 (pond follow-up) | Lifts the IPv4-only restriction surfaced by `pond/http/client` (`FRICTION.md` § "No DNS"). `lotus_tcp_connect` still fast-paths `inet_pton(AF_INET, host, ...)` (byte-for-byte identical to the pre-C6 path for numeric hosts) and on `inet_pton == 0` falls back to `getaddrinfo(host, port_str, hints = {AF_INET, SOCK_STREAM})` and connects to the first returned address. The signature stays `connect(host: String, port: Int) -> Int fallible(IoError)` — no API surface change, only richer resolution. gai errors map onto the existing `IoError` taxonomy without a new kind: `EAI_NONAME` → `errno = ENOENT` → kind `"not_found"`; everything else (DNS server failure, transient, no-address-for-family) → `errno = EHOSTUNREACH` → kind `"host_unreachable"`. `gai_strerror` is logged to stderr for diagnostics. IPv6 stays out: `hints.ai_family = AF_INET` is the deliberate v1 choice (callers wanting `::1` would need an AF_UNSPEC pass + connect-then-fallback, which is its own design call). Libc-only: no new linker dep. |
| Three friction-lift surface adds (parse_decimal, find_field_raw, time_from_unix) | 2026-05-20 (fathom follow-up) | Three small surface adds bundled from fathom's `lib/venues/kraken/FRICTION.md` asks. **`std::str::parse_decimal(s: String) -> Decimal fallible(ParseError)`** — backed by new `lotus_str_parse_decimal` / `lotus_str_can_parse_decimal` C primitives (i128 mantissa at fixed scale 9, split into two i64 halves over the C ABI to match the `lotus_decimal_to_string` convention). Closes the lossy `parse_float` workaround for venue book qtys reaching 8 decimal places (`0.00005100`) where IEEE 754 rounded the trailing zeros. **`std::json::find_field_raw(json: String, name: String) -> String`** — promotes the underscored stdlib internal to the public surface AND extends the walker to be bracket-balanced over nested objects (`{...}`) and arrays (`[...]`), respecting embedded string contents. The prior implementation walked-until-separator and silently broke on commas inside nested objects (the comment claimed nested support; the code didn't deliver). Recursive descent primitive for wrapped payloads (Kraken sub-ack's `"result":{...}`, `"data":[{...}]` channel state). **`std::time::time_from_unix(n: Int) -> Time`** — `gmtime_r` + `strftime` ISO 8601 UTC format in the caller arena; round-trips with `std::time::now()` so gateways can stamp `recv_ts: Time` from a monotonic-derived Int instead of falling through to the compile-time literal default. Interpreter parity for all three in `aperio-runtime::builtins`. |
| `std::process::run(argv) -> ProcessOutput fallible(IoError)` + `std::process::{spawn, wait, kill, write_stdin, read_stdout, read_stderr}` over `Child` | C2 (pond follow-up) | Synchronous + async subprocess. **`run`** forks, execs argv[0] with argv[1..] as args, drains stdout and stderr via `poll()` (interleaved so the child can write to either stream without deadlocking), waits for exit, and returns `ProcessOutput { code: Int; signal: Int; stdout: String; stderr: String; }`. argv is newline-separated String (`"git\nstatus\n"`) — Aperio's statically-sized arrays can't carry dynamic command-lines, so the newline blob is the v1 ergonomic compromise (mirrors `cli.ap`'s `argv_keys`). Exec failures surface as IoError: `kind="not_found"` for ENOENT, `"permission_denied"` for EACCES, `"invalid"` for E2BIG / empty argv; the parent decodes child-side `_exit(127)` with no stderr as ENOENT so the agent sees the typed signal. **`Child`** is the lifecycle-bound async handle: `spawn(argv) -> Child fallible(IoError)` forks+execs and returns; `wait(c)` blocks; `kill(c)` does SIGTERM → 100ms grace → SIGKILL → waitpid; `write_stdin(c, s)` blocking write (SIGPIPE ignored globally so closed-pipe writes return EPIPE via IoError); `read_stdout(c)` / `read_stderr(c)` non-blocking 64 KiB reads (empty String on EAGAIN or EOF — disambiguate via `wait`). `Child.dissolve()` closes all three pipe fds and calls `kill_escalate or discard` so an unwaited child doesn't leak zombies on scope exit; `kill_escalate` is idempotent (ESRCH → success, ECHILD → success). **Process group.** Every child gets `setpgid(0, 0)` in the post-fork prelude so a parent crash leaves the children in their own group (no orphans on shared session resources, and a future "kill the whole group" surface is one syscall away). We deliberately chose `setpgid` over `prctl(PR_SET_PDEATHSIG, SIGKILL)`: setpgid is POSIX (macOS/BSD work identically), prctl is Linux-only and overzealous — a controlled `dissolve()` already covers the orderly-shutdown path, and hard-parent-crash is a higher-layer concern (systemd/cgroups). **SIGPIPE** is globally ignored once at `lotus_io_init` startup so any write to a closed pipe (subprocess or otherwise) surfaces as EPIPE through the IoError channel instead of killing the parent. Output capped at 16 MiB per stream against runaway children. Resolves `pond/subprocess` (the existing custom fork/exec wrapper hand-rolled in pond goes away) and `pond/agent/sandbox` (supervised tool execution). |

Shipped after the table above:

- v1.x-3 (recognition projection class proper backing) — SHIPPED
  2026-05-12. Four sub-modes (`fixed_cell`, `shared_slab`,
  `spillover`, `summary_only`); v1 ships `fixed_cell` and
  `shared_slab`. `lotus_recpool_fixed_*` and
  `lotus_recpool_slab_*` extern surfaces in
  `crates/aperio-codegen/runtime/lotus_arena.c`. See
  `spec/memory.md` § "Recognition sub-modes (v1.x-3)" for the
  per-sub-mode commitment table and `spec/runtime.md`
  § "Recognition pool allocators (v1.x-3)" for the C ABI.

Deferred (gated on design):

- v1.x-9 (closures with capture) — MS2 invariant says every
  quantity assignable to one locus tower; naive lexical capture
  lets values float. Wait for closure-design pass.
- v1.x-FORM-4 (`@form(hashmap)`) — shipped 2026-05-13 end-to-end
  (PR1–7). `spec/forms.md` carries the full contract. FORM-3 perf
  work (lazy fallible-payload construction + subregion elision
  for non-allocating fn bodies) shipped alongside; the
  `form_vec_push` 10% (band (a)) gate is met. `@form(ring_buffer)`
  followed in v1.x-FORM-5 (fixed-capacity FIFO; push returns Bool,
  pop is fallible(EmptyError); spec at `spec/forms.md`
  § `@form(ring_buffer)`).

Cut from roadmap (2026-05-12 design pass):

- v1.x-6 (Result + `?` operator). `?` is pure desugaring for
  `if r.is_err { return r.err; }`; the load-bearing part is
  `Result<T, E>` as a discipline. Aperio already has
  failure-propagation-upward at the **locus** level via
  `bubble` / `on_failure` — that's the Design's
  failure-propagation-upward mechanic expressed structurally.
  Adding value-level `Result` would create a second, parallel
  mechanism for the same thing (parametric option for what is
  already covered structurally — exactly what The Design
  counsels against). The Aperio idiom is `fallible(E)` plus
  required `or` addressing at the call site — see
  `spec/types.md` § Fallible typing and the per-call list
  below. (Historical note: pre-`fallible` the surface used a
  sentinel-with-discriminator pair like `parse_int(s) -> Int`
  + `can_parse_int(s) -> Bool`; that surface was flipped
  2026-05-16/17 as `fallible(IoError)` / `fallible(ParseError)`
  flips landed.)
- v1.x-12 (Map as parametric stdlib type) / v1.x-13 (Vec as
  parametric stdlib type) / v1.x-14 (Rope) — replaced by the
  `@form(...)` machinery. Aperio source code never writes
  `Map<K, V>` or `Vec<T>` parametrically; collections are loci
  with form annotations. `@form(vec)` shipped via v1.x-FORM-2;
  `@form(hashmap)` is v1.x-FORM-4 forward content. Rope is
  superseded by v1.x-15 string-builder for the immediate
  driver workload (`reader-list_item-quadratic-concat`).
- v1.x-17 (machine-sized defaults) — runtime-queried page-size /
  cache-line constants for F.22 chunk sizing.
- `std::io::stdin::read_line` (2026-05-15) — closes the
  interactive-input gap. POSIX `getline` under the hood with a
  payload-arena copy; trailing newline (+ optional CR) stripped.
  Returns the empty-string sentinel on EOF / IO error. Paired
  with `std::io::stdin::read_line_status() -> Int` so callers
  can distinguish "empty input line" (status 0, len 0) from
  "EOF" (status -1, len 0). Both runtimes implement the surface
  (`builtins::resolve_path` in `aperio-runtime`; `lower_std_io_
  stdin_*` in `aperio-codegen`).

## Ergonomics arc — small wins (2026-05-11)

Driven by friction-log triage; bundled because each is one
primitive at the C-runtime + codegen seams. None capstones; each
resolves a specific friction-log entry.

| Add | Resolves | Surface |
|---|---|---|
| `std::io::fs::mkdir(path) -> () fallible(IoError)` | `apps/ssg` `no-mkdir` | Single-level mkdir, mode 0755. Wraps libc `mkdir`; not recursive. Flipped to `fallible(IoError)` 2026-05-16; pre-flip shape returned `Int` (0 / -1). |
| `std::io::fs::write_file_append(path, content) -> () fallible(IoError)` | `apps/log-router` `write-file-truncates-no-append` | Companion to `write_file`. Opens with `O_WRONLY \| O_CREAT \| O_APPEND` (no truncate). Flipped to `fallible(IoError)` 2026-05-16; pre-flip returned `Int` (0 / -1). |
| `eprintln(args...)` / `eprint(args...)` builtins | `apps/log-router` `no-eprintln-cant-isolate-debug-output` | Bare-name builtins like `print` / `println`. Route through `dprintf(2, ...)` to avoid the cross-libc `stderr` FILE* macro. Same compose-many-args shape as `println`. |
| `String + <printable>` auto-coerce | `apps/tcp-echo` `to_string-int-via-concatenation` | Mixed-type `+` where one side is `String` and the other is `Int` / `Float` / `Bool` / `Decimal` / `Duration` / `Time` / enum auto-coerces the non-String side via `value_to_string`. Symmetric (`port + " is the port"` works) and chained. |
| `approx` / `within` contextual narrowing | `lotus-harness` `closure-keyword-shadows-helper-ident` | The closure-assertion long-form spellings `approx` and `within` now lex as ordinary idents; the parser recognizes them as assertion vocabulary only inside `closure { ... }` bodies (F.10-style narrowing). Frees `approx`/`within` as fn / variable / field names everywhere else. (Phase 2a) |
| `if` and block as expression | `lotus-harness` `if-needs-block-value` | `Block` carries `tail: Option<Box<Expr>>`. A block's last item without a trailing `;` is the block's value when the block is used in expression position. `if cond { i } else { j }` produces a value via phi-merge of the arm tails; the else branch is required for the value form, and arm types must match. Composes with let-bindings inside arms. (Phase 2b) |
| Int → Float widening + `std::math::*` libm primitives | `lotus-harness` `float-surface-gaps` | Codegen widens Int → Float (via `sitofp`) at let-binding type ascriptions and fn-arg sites where the parameter is `Float`; one-way only, `Float → Int` and `Decimal` mixes still reject. `std::math::{sqrt, exp, log, floor, ceil}` (unary) + `std::math::pow` (binary) ship as path-call dispatches into libm. (Phase 2c) |
| `[val; N]` array-literal repetition | `lotus-harness` `float-surface-gaps` (sub-bullet 3) | New `Expr::ArrayRepeat { val, count }`. `val` is evaluated once; the result is broadcast to N slots of an arena-allocated `[N x T]`. N is a non-negative Int literal at v0. (Phase 2d) |
| Binary-safe TCP recv + Bytes/String surface | `apps/ws-echo` `tcp-recv-string-strlen-truncates-binary` | `Stream.recv_bytes(max) -> Bytes` (length-prefixed; embedded NULs survive) backed by `lotus_tcp_recv_bytes`. Companions: `std::bytes::from_string(s) -> Bytes`, `std::str::from_bytes(b) -> String`, `std::bytes::at(b, i) -> Int fallible(IndexError)` (flipped 2026-05-16; pre-flip returned -1 sentinel), `std::bytes::slice(b, lo, hi) -> Bytes`. All anchored in the global payload arena. Together they make a WebSocket frame parser straight-line Aperio. (Phase 2g) |
| `bin/aperio` stable publish bundles `libaperio_ts_shim.a` | 2026-05-20 (Form I) | `scripts/publish-stable.sh` now copies `target/release/libaperio_ts_shim.a` to `bin/libaperio_ts_shim.a` alongside the binary. The codegen's staticlib lookup (`locate_ts_shim_staticlib`) gains a second probe — the directory of the running aperio binary (`current_exe().parent()`) — between the env-var override and the workspace target/ fallback. Before this fix, an app team pinning to `bin/aperio` failed to link any user program that touched `std::io::fs` (`__StdSourceWalk` transitively references `lotus_ts_*` symbols, so even `read_file` callers needed the shim). The fix keeps the workspace fallback for `cargo run` / `cargo test` flows; the new probe just adds the bundled shape for published-binary distribution. |
| Fixed-size array bus-payload fields (`[T; N]`) | 2026-05-20 (Form H) | m70's wire codec previously rejected any array-typed payload field. This commit carves out the fixed-size case: a payload field declared `[T; N]` where T is a primitive (Int / Float / Decimal / Bool / Duration / Time) or a TypeRef (nested user-struct whose leaves are supported) now serializes / deserializes through `emit_array_field_serialize` / `emit_array_field_deserialize`. Primitive-element arrays take a single-memcpy fast path (`N * sizeof(elem)` bytes); TypeRef-element arrays loop N times, walking each nested struct inline. Arrays of String / Bytes and arrays-of-arrays remain deferred — the friction's "fixed-size case" framing. Closes fathom's SymbolBook (`[BookLevel; 10]`) shape ask. Field storage and indexed read/write already worked pre-Form-H; only the wire codec was missing. |
| `BytesBuilder.append_slice` OOB closure split | 2026-05-20 | The C primitive's return contract now distinguishes 1=ok / 0=alloc-fail / -1=out-of-range (was 1=ok / 0=any-fail). The Aperio-side `append_slice` method branches on the three states and violates either the existing `alloc_failed` closure or the new `index_oob` closure (`captures: lo, hi`). Pre-split both routed through `alloc_failed`, which misled fathom's production on_failure handlers — they read `captures.initial_cap` and concluded "memory exhausted" when the real cause was a caller-supplied bad index. Closes fathom FRICTION on append_slice routing. |
| View coercion at user-fn fallible + method-call arg sites; String/Bytes literal → View at storage defaults | F.30b follow-ons (5a, 5b) — 2026-05-20 | F.30 shipped the `view_coerces_to` predicate but only consulted it at non-fallible free-fn arg sites. **5a** extends the consultation to user-defined fallible-fn arg sites (the `fallible(E)` shape pond/websocket's `peek_header` uses), self-method + external-method + interface-dispatch + fn-pointer + monomorphized-generic arg sites. The unpack-and-check helpers from F.30b fire at every consultation. **5b** adds a literal-only coercion at storage-site defaults: a `String` / `Bytes` literal initializer for a `StringView` / `BytesView` field is wrapped in a view struct with `builder = NULL` (the unpack helper's NULL branch returns the data pointer without an epoch check, since the literal lives in the global string table at program-lifetime). Non-literal expressions still reject — those values might not have program-lifetime and shouldn't bypass the F.30 owned-vs-view distinction. Closes fathom's `WsMessage.text: StringView = ""` shape ask. |
| BytesView / StringView mutation-while-live runtime guard | F.30b (2026-05-20) | `lotus_bytes_builder_t` gains a monotonic `mutation_epoch` field bumped by every mutating op (`append` / `append_slice` / `shift_front` / `clear` / `advance`). `view()` and `text_view()` return a small fat-pointer struct (`{data, builder, stamped_epoch}` — 24 bytes, allocated in the caller arena). Read-site coercions (`view_coerces_to(BytesView, Bytes)` / `(StringView, String)`, the `len(view)` builtin, the println view arms) emit a call to `lotus_bytes_view_data` / `lotus_str_view_data` which compares the stamped epoch against the live epoch; mismatch → `lotus_view_stale_panic` (stderr diagnostic + `_exit(1)`). Catches "future maintainer added `b.append(...)` between `b.view()` and the consumer's read" loudly at the read site. Not closure-routed: stale-view is a programmer error, not a recoverable runtime condition — same shape as out-of-bounds array reads. Does NOT catch view-held-across-finish/dissolve (separate UAF concern; same shape as today's lifetime contract). |
| `std::bytes::clone(v) -> Bytes` and `std::str::clone(v) -> String` | F.30 follow-up (2026-05-20) | Deep-copy a `BytesView` / `StringView` into the caller's arena (via Task 8's TLS allocator), producing an owned blob that survives subsequent mutations on the source builder. The explicit upgrade path BytesView signals when storage sites reject the read-only coercion — `let owned: Bytes = std::bytes::clone(b.view());` is the canonical form. Both accept the matching owned type as a no-op same-type clone, so generic callers (e.g. "clone whatever was passed in") don't branch on view-ness. Backed by `lotus_bytes_clone` (new) and `lotus_str_clone` (existing m49). |
| `list_dir` index API | `apps/ssg` `list_dir-newline-string` | `std::io::fs::list_dir_count(path) -> Int fallible(IoError)` + `std::io::fs::list_dir_at(path, i) -> String fallible(IoError)`. Both walk the same global-arena cache, so iteration becomes a 4-line `let n = count; while i < n { name = at(i); i = i + 1; }` — no manual newline-scanning. **2026-05-16 cleanup:** the older newline-joined `list_dir(path) -> String` shape was removed; the index API is the only iteration form. (Phase 2e) |
| `read_file` errno status | `apps/ssg` `read_file-empty-vs-error` | The Phase-2f legacy companion `read_file_status(path) -> Int` was **removed 2026-05-16**. Use `read_file(path) -> String fallible(IoError)` and address the err path with `or raise` / `or substitute` / `or handler(err)`; the `IoError` payload carries errno + kind tag. Distinguishes empty-file vs missing-file via the err arm rather than a paired status call. |
| Stale-CLI rebuild check | `apps/log-router` `stale-cli-silent-drops-subscribers` | `crates/aperio-cli/build.rs` hashes `codegen.rs`, `lotus_arena.c`, and every `runtime/stdlib/*.ap` file at CLI-build time, bakes the hash + crate path into the binary via `cargo:rustc-env`. On every `aperio build` invocation, `check_stale_cli()` in main.rs recomputes from disk and emits a four-line warning when they diverge — catches the "edit codegen, run cargo test, forget to rebuild CLI" footgun without forcing an automatic rebuild. Skipped silently for installed binaries or when `APERIO_SKIP_STALE_CHECK=1`. (Phase 2i) |

## F.19 — per-directory seed model (2026-05-11)

`aperio build <dir>` accepts a directory and bundles every `.ap`
file in the directory as one seed (one binary). Top-level decls
in any file are visible to every file in the same directory, in
one shared scope — same shape Go gets from per-package
visibility. `aperio build <file.ap>` keeps working for
single-file apps.

File order in the merged bundle is alphabetical; resolution is
order-free (the typechecker flattens before name lookup). Binary
defaults to the directory's basename (`myapp/` → `myapp/myapp`).

Resolves the single-file-app-monolith friction. Spec entry: F.19
in `spec/design-rationale.md`. Example fixture:
`crates/aperio-codegen/tests/fixtures/examples/multi-file-seed/`.
Regression test: `crates/aperio-codegen/tests/multi_file_build.rs`.

## F.20 — structural interfaces, Phase A + Phase B (2026-05-11)

`interface Name { fn ...; ... }` declares a structural interface.
Any locus whose method set is a superset structurally satisfies
it; satisfaction is implicit (no `impl I for L`).

**Phase A (shipped):** parser, AST, resolver, typechecker.
Structural impl-check fires at every call site where a fn
declares an interface-typed param (missing-method / arity /
type / return-type diagnostics).

**Phase B (shipped):** codegen vtable dispatch. Interface values
lower as fat pointers `{data, vtable}` allocated in the current
arena; the data slot is the underlying locus pointer (same
ABI as `LocusRef`), the vtable slot points at a per-(locus,
interface) static global of fn pointers indexed by interface-
method declaration order. A locus flowing into an interface
slot coerces at the call site; method calls on an interface
value lower as indirect calls through `vtable[i]` with the data
pointer passed as the implicit self arg. End-to-end coverage
in `crates/aperio-codegen/tests/interface_dispatch.rs`.

**Phase B follow-ups:**
- Interface values in locus param/field — **shipped 2026-05-16**
  (`Server { handler: MyHandler { } }` where `handler:
  std::http::Handler`). Codegen coerces locus → interface at the
  struct/locus literal field-store site; field reads through the
  fat pointer dispatch via vtable. Typechecker resolves
  `self.field.method()` against the interface's method set when
  the field's declared type is an interface.
- Interface values as `@form(vec)` cell elements — **shipped
  2026-05-17** (A10). Locus → interface coercion fires on
  `vec.push` / `vec.set`; the stored fat pointer aliases the
  underlying locus so mutations through the vec write through to
  the original locus's region.
- Returning an interface value from a fn — **shipped 2026-05-18**
  (this milestone). The return site inserts an implicit
  locus → interface coercion and the m90 locus-instantiation
  routing is extended to fire when the fn declares
  `-> Interface(I)` and a satisfying locus is instantiated in
  the body. The fat-pointer struct is deep-copied into the
  caller's arena by `emit_return_value_deep_copy`. Also fixes a
  pre-existing typecheck reject where interface → same-interface
  pass-through was wrongly flagged as a structural-impl failure.
  Coverage in `crates/aperio-codegen/tests/interface_return.rs`.
- Interface elements inside tuples / fixed arrays — still
  deferred. The gap is the broader composite-construction
  coercion shape (recursive coercion at composite-construction
  sites plus locus-routing across nested return positions); the
  same gap governs tuple-of-`LocusRef` escape today.

The `std::text::Sink` stdlib migration (split into `StdoutSink` /
`StringSink` / `FileSink` loci behind one `Sink` interface)
shipped 2026-05-11 as a separate commit — see `std::text` in
`spec/stdlib.md` and the `sink-as-tagged-locus` friction log
entry. The `std::http::Handler` interface (2026-05-16) is the
second canonical use: stateful HTTP loci flow into the Server
locus's `handler` field without needing closures.

Resolves (partial) `notes/aperio-friction.md` 2026-05-10
sink-as-tagged-locus. Spec entry: F.20 in
`spec/design-rationale.md`.

## Path resolution (m71)

`.ap` source references stdlib symbols by fully-qualified path:

```aperio
let p = std::process::pid();
let contents = std::io::fs::read_file("config.toml");
std::io::tcp::Listener { host: "127.0.0.1", port: 8080 };
```

The parser tokenizes `::` as a path separator and the type checker
punts namespaced paths to `Ty::Unknown`; the codegen layer resolves
`std::*` paths against a hardcoded namespace dispatcher.

There is **no general module system** in Phase 1 — no `use`
statements, no user-defined modules, no multi-file `.ap` projects.
`std::*` is the only recognized prefix. Adding a new stdlib function
means: declare its libc backer in `aperio-codegen` (the Phase 1
stdlib section of `declare_builtins`), add a match arm to
`lower_stdlib_path_call_expr` (or its statement sibling), and
implement one `lower_std_*` method.

Adding a real module system is deferred until something forces it
(probably Phase 3+ when the HTTP server or Phase 5 doc-server
example pushes against single-file organization). The
import-mechanism choice is recorded in
`docs/src/std/roadmap.md` and the project memory.

## Design principles

- **Batteries included.** Go's approach: if a typical Aperio
  program needs it, it ships. A new Aperio user shouldn't
  need third-party packages for trading-system or coordinator-
  system work.
- **One canonical implementation.** Per Go's "one obvious way":
  one `std::collections::Map`, not seven. Multiple options live
  in third-party.
- **Framework-aware.** Stdlib types use the language's projection
  classes, modes, and closure tests where appropriate. The
  stdlib is itself disciplined.
- **Replaceable.** Anything in stdlib can be replaced by a
  third-party module; nothing in stdlib is tied into the
  compiler.

## Shipped module surface

The phase tables above are the authoritative list of what's in
tree. Quick reference grouped by `std::*` namespace prefix:

| Namespace | Surface (shipped) | Source |
|---|---|---|
| `std::process` | `pid() -> Int`, `exit(code: Int)` | path-call dispatch in `aperio-codegen` |
| `std::env` | `args_count()`, `arg(i)`, `arg_or(i, default)`, `var(name)`, `var_exists(name)` | path-call dispatch + main-prelude argv stash |
| `std::time` | `monotonic() -> Duration`, `sleep(d: Duration)`, `now() -> Int`, `time_from_unix(n: Int) -> Time` | `clock_gettime(CLOCK_MONOTONIC)` + EINTR-retrying `clock_nanosleep`; `now()` calls `clock_gettime(CLOCK_REALTIME)` via `lotus_time_now_seconds`; `time_from_unix` formats `gmtime_r` + `strftime` ISO 8601 UTC via `lotus_time_from_unix` |
| `std::str` | `parse_int(s) -> Int fallible(ParseError)`, `parse_float(s) -> Float fallible(ParseError)`, `parse_decimal(s) -> Decimal fallible(ParseError)`, `can_parse_int(s) -> Bool`, `can_parse_float(s) -> Bool`, `can_parse_decimal(s) -> Bool`, `index_of`, `lower` / `upper`, `trim`, `substring(s, lo, hi)`, `replace`, `repeat`, `pad_left` / `pad_right`, `from_bytes`, `builder_new` / `builder_append` / `builder_len` / `builder_finish` | `lotus_str_*` C runtime primitives |
| `std::bytes` | `at(b, i) -> Int fallible(IndexError)`, `slice(b, lo, hi) -> Bytes`, `from_string(s) -> Bytes`, `from_int(v) -> Bytes`, `concat(a, b) -> Bytes`. Growing-buffer accumulator surface lives on the `BytesBuilder` locus (`std::bytes::BytesBuilder` — see § _Builders vs Bytes_ below): `b.append`, `b.len`, `b.snapshot`, `b.view` (zero-copy alias; Phase-2 (1)), `b.shift_front`, `b.clear`, `b.finish`. The locus shape is what enforces builder/Bytes type discrimination — `std::bytes::at(b, i)` on a `BytesBuilder` fails at typecheck rather than misreading the runtime header. | `lotus_bytes_*` C runtime primitives |
| `std::text` | `md_to_html(md) -> String`, `base64::encode` / `base64::decode`, `Sink` interface + `StdoutSink` / `StringSink` / `FileSink` loci, byte-class predicates `is_alpha` / `is_digit` / `is_alnum` / `is_whitespace` / `is_word_char`, `tokenize_words_into(s, target_vec)` | `runtime/stdlib/text.ap` + C runtime |
| `std::io::fs` | `read_file`, `write_file`, `write_file_append`, `read_bytes`, `file_size`, `mkdir`, `rename`, `unlink`, `mktemp`, `list_dir`, `list_dir_count`, `list_dir_at` — all `fallible(IoError)`. `file_exists(path) -> Bool` (predicate, not failable). One-shot path-call surface: each call opens, does the op, closes. For held-open shapes use `std::io::file::File`. | `lotus_fs_*` C runtime primitives |
| `std::io::file` | `File` locus (held-open fd with auto-dissolve close). `open(path, mode) -> File fallible(IoError)`; `read_line(f) -> String` (returns "" at EOF or read error — pair with `at_eof`); `at_eof(f) -> Bool`; `write_bytes(f, b)`, `write_line(f, s)`, `seek(f, offset)` all `fallible(IoError)`. Mode strings `"r"` / `"w"` / `"a"` / `"r+"` / `"w+"`. Returned Strings live in the bus payload arena (program-lifetime). | `lotus_file_*` C runtime primitives + `runtime/stdlib/file.ap` |
| `std::io::udp` | Raw UDP networking primitives. `bind(host, port) -> Int fallible(IoError)` (creates SOCK_DGRAM bound to addr; host=""→INADDR_ANY); `send(fd, host, port, msg) -> () fallible(IoError)` (best-effort datagram send); `recv(fd, max_bytes) -> Bytes fallible(IoError)` (one datagram); `recv_into(fd, buf: Bytes, max_bytes) -> Int` (caller-provided builder destination — Phase 1; see feature table for return semantics); `close(fd) -> Int`. Datagram boundaries preserved by the kernel — no framing needed at this layer. **NOT a bus transport**: UDP doesn't satisfy the bus's atomic-delivery contract. Cross-host bus over UDP would require a user-written adapter (Wave B) layering retry / framing on top. | `lotus_udp_*` C runtime primitives |
| `std::io::stdin` | `read_line() -> String`, `read_line_status() -> Int` | `lotus_stdin_*` C runtime primitives (POSIX `getline` + payload-arena copy) |
| `std::io::tcp` | `Listener` locus, `Stream` locus, `send`, `send_bytes`, `recv_bytes`, `recv_into(fd, buf: Bytes, max_bytes) -> Int` (caller-provided builder destination — Phase 1). Path-calls `listen_socket`, `connect`, `accept_one` are `fallible(IoError)`. `connect` accepts dotted-quad hosts directly and falls back to hostname resolution via `getaddrinfo` (AF_INET) when the host isn't numeric. | `lotus_tcp_*` C runtime primitives |
| `std::io::tls` | Client-side TLS via system OpenSSL. `connect(host: String, port: Int) -> Int fallible(IoError)` opens a TCP connection, performs a TLS 1.2+ handshake with SNI + system-trust-store cert verification (`SSL_VERIFY_PEER` + `SSL_set1_host`), and returns an opaque handle. `send_bytes(handle, bytes: Bytes) -> Int` and `recv_bytes(handle, max: Int) -> Bytes` operate on the handshaked connection; both non-fallible at the language level (return 0/-1 or empty Bytes on error). `recv_into(handle, buf: Bytes, max_bytes) -> Int` reads directly into a caller-provided builder (Phase 1; same return semantics as the tcp/udp siblings). `close(handle) -> Int` shuts the TLS layer down + closes the underlying socket. The `lotus_tls.c` translation unit is compiled separately so helper tests that link `lotus_arena.c` directly don't drag in libssl/libcrypto; the main `aperio build` link adds `-lssl -lcrypto`. | `lotus_tls_*` C runtime primitives in `runtime/lotus_tls.c` |
| `std::http` | `Request` + `Response` types (`Response.content_type` defaults to `"text/plain"`; `Response.headers: String = ""` carries CRLF-joined user-supplied headers — no trailing CRLF — for Set-Cookie / CORS / custom-header use), `parse_request`, `write_response`, case-insensitive symmetric `header(receiver, name)` lookup that works on both Request and Response receivers, `Handler` interface (`fn handle(req: Request) -> Response`), `Server` locus (accept loop dispatches each request to a `Handler`-typed locus's `handle` method — state lives on the handler's params; `handler:` is a required field on `Server`, no default; optional `ready_signal: String = ""` prints a sync line to stdout after `listen()` succeeds) | `runtime/stdlib/http.ap` |
| `std::json` | `Builder` locus (streaming output assembly: `begin_object` / `end_object` / `begin_array` / `end_array`, `field` / `string_field` / `int_field` / `bool_field` / `null_field`, `value` / `string_value` / `int_value` / `bool_value` / `null_value`, `begin_object_field` / `begin_array_field`, `result() -> String`), `escape_string` / `unescape_string` (RFC 8259 string escaping), `find_string_field` / `find_int_field` / `find_bool_field` (flat-object field lookup), `find_field_raw(json, name) -> String` (raw value-token substring; bracket-balanced over nested objects/arrays — the recursive-descent primitive for wrapped payloads), `ArrayIter` + `array_first` / `array_next` (flat-array iteration). No nested-tree shape at v1 (no typed walker over the structure; `find_field_raw` returns the substring so callers can re-feed it into the same surface). | `runtime/stdlib/json.ap` |
| `std::test` | `assert(cond, msg)`, `assert_eq_int`, `assert_eq_str` | `runtime/stdlib/test.ap` |
| `std::log` | `Logger`, `LogEvent`, `StdoutSink` (subscribes `log.**`) | `runtime/stdlib/log.ap` |
| `std::math` | `sqrt`, `exp`, `log`, `floor`, `ceil`, `pow`, `tanh`, `nan`, `is_nan`, `inf` | path-call dispatch into libm (`nan` / `inf` / `is_nan` are IEEE 754 sentinels / classification) |
| `std::crypto` | `sha1(b) -> Bytes` (20-byte), `sha256(b) -> Bytes` (32-byte), `hmac_sha256(key, msg) -> Bytes` (32-byte). All non-fallible; results anchored in the bus payload arena. | `lotus_crypto_*` C runtime primitives (stand-alone — no libcrypto / OpenSSL link dep) |
| `std::os` | `getrandom(n: Int) -> Bytes fallible(IoError)` (CSPRNG; `getrandom(2)` with `/dev/urandom` fallback) | `lotus_os_getrandom` C runtime primitive |
| `std::bus` | `__StdBusAdapter` interface (contract for user-supplied bus transports — a single `fn send(subject: String, bytes: Bytes)` method). `__local_dispatch(subject: String, bytes: Bytes)` primitive (m105) lets an adapter relay received wire-bytes into the local handler set: the runtime looks up the subject's registered deserialize fn, reconstructs the struct-layout bytes, and fans into subscribers via `lotus_bus_local_dispatch` (same shape the unix reader thread uses). No concrete adapter implementations ship in std — protocol-layer transports (NATS, MQTT, raw-TCP-with-framing) live in user code or downstream packages. The binding-site wiring (`bindings { T: MyAdapter { ... }; }`) shipped 2026-05-18 as Wave B of the bus-transport redesign; codegen instantiates the adapter into the program-lifetime payload arena, resolves its `send` fn pointer, and the runtime dispatches outbound payloads through the pointer. With `__local_dispatch` in place, adapters can now implement both halves (outbound `send`, inbound run-loop). | `runtime/stdlib/bus.ap` |

Aperio doesn't use parametric stdlib collection types (`Map<K,
V>`, `Vec<T>`, `Set<T>`, etc.). Storage is locus-shaped via the
`@form(...)` annotation machinery — see `spec/forms.md`. v1
ships `@form(vec)` (contiguous-buffer; v1.x-FORM-2),
`@form(hashmap)` (intrusive open-addressing; v1.x-FORM-4), and
`@form(ring_buffer)` (fixed-capacity FIFO; v1.x-FORM-5).

### Builders vs Bytes — the recv-loop pattern

`Bytes` and `std::bytes::BytesBuilder` are deliberately
distinct types because their runtime ABIs are **incompatible**:

- **`Bytes` blob.** Single contiguous allocation: `[i64 len]
  [u8 data[len]]`. The handle points at the length prefix.
  `lotus_bytes_len(b)` reads `*(int64_t*)b`. `lotus_bytes_at(b, i)`
  reads `((u8*)b)[8 + i]`. Stable across the value's lifetime.
- **`BytesBuilder` locus.** Owns a `lotus_str_builder_t`
  header `{ size_t cap; size_t len; char *buf; }` whose body
  lives in a separately malloc'd region pointed to by `buf`
  and can move on realloc (the header pointer is stable, the
  body pointer is not). `cap` starts at the locus's
  `initial_cap` param (default 64) and doubles on grow.

The two ABIs cannot be unified without giving up stable handles
(the body has to be relocatable; the Bytes blob layout doesn't
tolerate that). They used to share a static type — `builder_*`
free fns returned `Bytes`, so `std::bytes::at(builder, i)`
silently read the cap field as the length and ran off the end
of the heap. Lifting the builder into its own locus type
turned that silent footgun into a typecheck error.

The discipline that follows:

1. **Pick one role per binding.** A binding is either a
   `BytesBuilder` (long-lived growable buffer, methods
   `append` / `len` / `shift_front` / `clear` / `snapshot` /
   `finish`) or a `Bytes` (immutable length-prefixed blob,
   functions `at` / `slice` / `len` / `concat`). The
   typechecker enforces this — there's no implicit coercion.
2. **Cross between roles via explicit calls.** `BytesBuilder
   → Bytes` is either `b.snapshot()` (copies into the bus
   payload arena — stable across mutations) or `b.view()` (no
   copy, aliases the builder's buffer — valid until the next
   mutation; the right choice for read-then-discard idioms
   like parser passes). `Bytes → BytesBuilder` is `let b =
   std::bytes::BytesBuilder { ... };` followed by
   `b.append(bytes)` (copies). The Builder → Bytes direction
   has a zero-cost path via `view()`; the reverse does not.
3. **Long-lived accumulators live as locus state.** Either a
   method-body `let`-binding (dissolves at scope exit) or a
   param-typed field on the owning service locus (dissolves
   via cascade at the parent's dissolve — see F.29). Method-
   body `let` is simpler when the accumulator's lifetime fits
   inside one method invocation; field-typed is needed when
   the buffer must outlive a single method call (e.g., state
   held across bus subscription callbacks).
4. **Read syscalls write directly into the builder.** The
   `recv_into` family takes a `BytesBuilder` as `buf` and
   writes into its tail. Codegen extracts the internal handle
   slot at the call boundary. Combined with `b.shift_front`
   after each peeled frame (streaming case) or `b.clear()` at
   message boundaries (per-message accumulator case), the
   steady-state recv loop is zero-alloc against
   `g_bus_payload_arena`.

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
multiple method calls (bus callbacks, message-state held
between handler firings, etc.):

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
    // via self.last_msg.clear() + .append(...) between
    // message boundaries — no per-frame allocation.
  }
  // No dissolve() needed for rx_buf / last_msg — the cascade
  // fires their dissolve when WsClient itself dissolves.
}
```

Consumer reads `self.last_msg` via a contract that exposes
`b.view()` — zero-copy across the F.14 interface.

### ~~`std::panic`~~ — not a thing

Aperio doesn't have `panic(msg)`, `assert(cond)`, or any other
value-level "bail from this function" primitive. Failure is
structural, not parametric:

1. Declare a **closure** in the relevant locus asserting the
   invariant you want enforced.
2. When the assertion fails at the closure's epoch, the
   runtime constructs a `ClosureViolation` and routes it to
   the parent's `on_failure` handler per **F.9**.
3. The parent picks one of `restart` / `restart_in_place` /
   `quarantine` / `reorganize` / `bubble`, or absorbs the
   violation by returning without calling any of them.
4. A violation that bubbles past `main` exits the process
   non-zero with the violation report on stderr.

That covers every legitimate use of `panic`. "Impossible state"
becomes "a closure asserting state is possible." "Bail from
this function" is a category error in Aperio — functions return
values, failure lives at the locus level. The earlier
speculative `panic(msg)` / `catch_panic` surface here was
inherited from Rust convention and doesn't match Aperio's
Design-aligned failure shape; cut from the roadmap on
2026-05-12 alongside `Result + ?`.

See [closures/index](../docs/src/reference/closures/index.md)
and [recovery/index](../docs/src/reference/recovery/index.md)
for the operational details.

## What's not in stdlib (third-party territory)

- ML / learning libraries
- Database drivers (Postgres, etc.)
- Web frameworks beyond basic HTTP
- Image / audio / video processing
- Cloud SDKs (AWS, GCP, etc.)
- GUI / TUI frameworks
- Cryptography beyond TLS basics
- Compression formats beyond ones used in stdlib (gzip for HTTP)

These are the kinds of things that should live in the Aperio
package ecosystem (TBD: how packages work, where they live).

## Open decisions

1. **Module organization** — flat (`std::collections`,
   `std::string`) vs. hierarchical (`std::collections::Map`).
   The Go-style middle ground (`std/collections/map.go`) is
   probably right.
2. **What's exported by default vs. what's deep-imported.**
   `import std;` for everything? `import std::time;` only?
   Probably the latter: explicit per-module imports.
3. **API stability commitments.** Go's stdlib is famously
   stable. We'd want similar. v0 stdlib is *unstable*; v1
   marks specific APIs as `stable`; only stable APIs survive.
4. **Versioning.** Stdlib is versioned with the language? Or
   independently? Probably with the language for v0; consider
   independent versioning when stable.

## Form-synthesized types (v1.x-FORM-1)

Beyond the explicit `std::*` namespace, the resolver injects
form-specific error/payload types into the top scope when any
locus in the bundle uses the corresponding form. These behave
like ordinary user types after injection — they can be the
target of `fallible(...)`, declared as fn parameters / fields,
or pattern-matched in `match`. They are NOT importable via
`std::*` (they are not in a namespace); their names live at
the top level.

| Form                  | Synthesized type | Fields |
|-----------------------|------------------|--------|
| `@form(vec)`          | `IndexError`     | `kind: String`, `index: Int`, `len: Int` |
| `@form(hashmap)`      | `KeyError`       | `kind: String` |
| `@form(ring_buffer)`  | `EmptyError`     | `kind: String` |
| `std::io::fs` / `std::io::tcp` | `IoError` | `kind: String`, `errno: Int`, `path: String` |
| `std::str::parse_int` / `parse_float` | `ParseError` | `kind: String`, `input: String` |

Idempotency: if a user / library declares a type with the same
name, the user declaration wins. The injection only runs if the
target name is not already in scope.

### `IoError` — unified I/O failure payload (2026-05-16)

`std::io::fs::*` (except `file_exists`) and the path-call surface
of `std::io::tcp::*` (`listen_socket`, `connect`, `accept_one`)
return `fallible(IoError)`. Agents address failures uniformly:

```aperio
let s = std::io::fs::read_file(path) or raise;
let n = std::io::fs::file_size(path) or 0;
std::io::fs::mkdir(out_dir) or show(err);
```

The `kind` tag is errno-derived via `lotus_io_error_kind` —
`"not_found"`, `"permission_denied"`, `"is_dir"`,
`"already_exists"`, `"would_block"`, `"connection_refused"`,
`"timeout"`, `"host_unreachable"`, `"broken_pipe"`,
`"interrupted"`, etc., with `"io"` as the catch-all for unmapped
codes. `errno` carries the raw platform errno for callers that
want it; `path` carries the file path / connection target /
empty string for socket-fd ops without a useful path label.

`Stream.send` / `Stream.recv_bytes` / `Stream.send_bytes` are
*locus methods*, not path-calls, and per the two-channel rule
(`spec/semantics.md` § "Fallible call semantics") locus methods
cannot declare `fallible(E)`. They keep the legacy sentinel
shape (returning -1 / 0 on failure). The same is true of
`std::io::stdin::read_line` (path-call but pairs with
`read_line_status` for the EOF-vs-error distinction; EOF is a
natural non-error terminator in the typical loop).

The interpreter and codegen runtimes both wire failures through
`Value::FallibleErr` / sret-path-indicator respectively; both
construct the same `IoError { kind, errno, path }` shape.

Closes the v1 errno-disambiguation follow-up. Before the flip,
agents reaching for the modern shape (`read_file(path) or raise`)
were blocked twice: (a) `IoError` didn't exist, (b) `or` over a
Path callee didn't codegen. Both gaps closed together — `or` now
accepts Path callees and the IoError synth wraps every flipped
path-call's sentinel return into a typed payload.

### `ParseError` — string→number failure payload (2026-05-17)

`std::str::parse_int(s)` and `std::str::parse_float(s)` return
`fallible(ParseError)`. The non-fallible siblings were removed —
every call site must address the failure with `or`. `ParseError`
carries:

- `kind: String` — `"empty"` (s was `""`), `"trailing_chars"` (s
  parsed a prefix and had junk after), `"invalid"` (no leading
  numeric prefix), `"overflow"` (parse_int only — magnitude exceeds
  Int range).
- `input: String` — the original `s` (truncated to a reasonable
  preview if very long), for diagnostic surfaces.

```aperio
let n = std::str::parse_int(s) or 0;
let n = std::str::parse_int(s) or raise;
let n = std::str::parse_int(s) or self.report(err);
```

Reach for the predicate sibling `can_parse_int(s) -> Bool` only
when you genuinely want to branch *before* parsing rather than
parse-and-substitute. In most cases `or` is shorter.

### `Server.ready_signal` — synchronization for piped oracles (2026-05-17)

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

Pair with the line-buffered stdout setup (`spec/runtime.md` §
"stdout buffering") — the prelude installs `setvbuf(stdout, NULL,
_IOLBF, 0)` so a single `println` is flushed even under pipes.
Without that, the READY line would sit in libc's full-buffer
queue while `accept()` blocked, and the oracle would hang.

### `std::json::Builder` — streaming output API (2026-05-17)

`Builder` is a `@form(...)`-less locus that accumulates a JSON
document into an internal `buf: String` via append. It tracks
scope state in a single `ctx: String` stack (one char per open
scope: `O`/`A` for object/array with at least one value already
emitted, `o`/`a` for empty). The Builder inserts separators
(`,` between siblings, `:` between key and value) automatically
based on context.

Methods, grouped:

- **Scopes:** `begin_object()`, `end_object()`, `begin_array()`,
  `end_array()`.
- **Object entries (key + value in one call):** `field(name, value)`
  for the common string case; `string_field`, `int_field`,
  `bool_field`, `null_field` for explicit typing.
- **Array entries / bare values:** `value(v)` (string), plus
  `string_value`, `int_value`, `bool_value`, `null_value`.
- **Nested scopes inside an object:** `begin_object_field(name)`
  / `begin_array_field(name)` — emit `"name":` then open the
  sub-scope, so the caller can recurse without juggling
  separators.
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

The Builder pairs with `escape_string` for raw-untyped writes
(`b.buf = b.buf + std::json::escape_string(s)` is permitted but
defeats the separator tracking — prefer the typed setters). The
flat-object readers (`find_*_field`, `array_first/next`) are
the input side of the same v1 commitment: JSON is a wire format,
not a tree value type, and the API surface reflects that.

## Why batteries-included

The user's note: "I like the batteries included approach of Go."
Concretely, batteries-included gives:

- **Lower adoption barrier.** New users don't need to evaluate
  third-party packages for table-stakes functionality.
- **Discipline propagation.** Stdlib uses framework primitives
  correctly; new code following stdlib examples inherits the
  discipline.
- **Ecosystem trust.** When the language ships a `std::stat`
  with statistical primitives, they're vetted; trust transfers
  to programs that use them.
- **Cross-language consistency.** Programs from different teams
  share the same vocabulary because they share the same stdlib.

Cost: stdlib is permanently load-bearing once shipped. Bad
decisions are hard to undo. Discipline at design time matters
more here than in third-party.
