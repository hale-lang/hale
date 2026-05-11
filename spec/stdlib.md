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
| m73 | `std::io::tcp::Listener` stdlib locus. Bundled-source mechanism (`runtime/stdlib/`) + path-rewrite at qualified struct literals. Real birth/run/dissolve lifecycle wired through `__listen_socket` / `__accept_one` / `__close_fd` path-call primitives. Single-accept shape (resolved in m83). |
| m74 | `lotus_fs_*` C substrate: `read_file`, `write_file`, `file_size`, `file_exists`. POSIX wrappers, no buffering, one-shot synchronous shape. (`read_dir` resolved in m90.) |
| m75 | `std::io::fs::*` Aperio surface. Functional path-call shape (mirrors `std::process::pid`), not locus-wrapped — one-shot file ops don't need lifetime-of-a-stream. `read_file` allocates from the m70 lazy global payload arena so the returned `String` outlives the call frame. |
| m76 | `examples/io-demo/` capstone exercising both surfaces. Reads optional config, listens, accepts one connection, writes a log. Integration test in `tests/io_demo.rs` drives it under CI. |

## Inter-phase cleanup (m77 → m81)

Bridge milestones between Phase 1 and Phase 3 — argv/env
plumbing and the language additions Phase 3 needed.

| Milestone | What it shipped |
|-----------|-----------------|
| m77 | `std::env::args_count` / `arg` / `var` / `var_exists`. Lifted main's signature to `i32 @main(i32, ptr)` so codegen captures argc/argv into a runtime stash via `lotus_env_init` in main's prelude. |
| m78 | `std::str::parse_int` / `can_parse_int`. strtoll-based, base 10, strict trailing-char check. |
| m79 | `std::time::sleep` / `monotonic` aliases under `std::*` namespace; `std::process::exit(code)`. |
| m80 | Function-pointer language addition. `CodegenTy::FnPtr`, parser support for `fn(T) -> R` types, codegen lowering of fn names as values + indirect calls through fn-pointer fields. The Phase 3 prerequisite. |
| m81 | Stream locus + non-self method calls + `__send` / `__recv` / `__connect` primitives. New `lower_external_method_call` for `obj.method(args)`. Bundled `__StdIoTcpStream` declaration. |

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
| m83 | Multi-accept Listener with `on_connection: fn(Stream)` callback. Composes m80 + m81 + m82. Per-connection Stream lifecycles owned by a free-fn helper (`__handle_one_connection`) whose scope-exit flush closes the fd between iterations. |
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
| m89 | `Bytes` codegen — the binary-safe sibling of String. Memory layout `[i64 len][u8 data[len]]`; same single-pointer ABI as String. `len(b)`, `std::io::fs::read_bytes`, `std::io::tcp::__send_bytes`, `Stream.send_bytes` method. Embedded NUL bytes preserved across all three. |
| m90 | `std::io::fs::list_dir(path) -> String`. Newline-separated entry names (skipping `.` / `..`). v0 shape; sibling `[String]` API waits on a generic `List<T>` type. |
| m91 | `std::text::md_to_html(md) -> String`. ATX headings, multi-line paragraphs, fenced code blocks, HTML escaping. Inline formatting (bold/italic/code/links) deferred. |

## Phase 5 — the capstone (sealed m92)

| Milestone | What it shipped |
|-----------|-----------------|
| m92 | `examples/docs-server/main.ap`. Real HTTP server in ~200 lines of Aperio that lists and renders markdown files from a configured directory. Composes Listener (m83) + parse_request (m84) + write_response (m85) + read_file (m74) + list_dir (m90) + md_to_html (m91) + Stream lifecycle (m82) + function pointers (m80). |

## Stdlib organization — m93

The bundled stdlib was a single ~530-line `runtime/stdlib.ap`
through m92. m93 split it into per-domain files under
`runtime/stdlib/`:

| File | Contents |
|------|----------|
| `core.ap`   | Helpers used across the stdlib (`__replace_all`, `__html_escape`). |
| `io_tcp.ap` | `Stream` + `Listener` loci + `__handle_one_connection` + `__default_on_connection`. |
| `http.ap`   | `Request` + `Response` types + `__parse_http_request` + `__write_http_response` + `__status_phrase`. |
| `text.ap`   | `__md_to_html` + line tokenization helpers. |
| `test.ap`   | `__test_assert` + `assert_eq_*` variants. |
| `log.ap`    | (m95) `__StdLogEvent` + `__StdLogLogger` + `__StdLogStdoutSink`. |

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

- **Errno surface.** Errors still collapse to `-1` / `false` /
  empty string. Disambiguation between "missing" and "permission
  denied" needs an error-introspection follow-up.
- **Inline markdown.** Bold, italic, inline code, links — m92+.
- **HTTP keep-alive, custom headers, large bodies.** v0 hardcodes
  `Connection: close`, no header-map type, single-recv assumption
  for the request line + headers. All Phase 3 v1.0 follow-ups.
- **`aperio test` CLI runner.** Phase 2 v1.0 — currently the
  Rust integration harness fills the role.

## Ergonomics arc — small wins (2026-05-11)

Driven by friction-log triage; bundled because each is one
primitive at the C-runtime + codegen seams. None capstones; each
resolves a specific friction-log entry.

| Add | Resolves | Surface |
|---|---|---|
| `std::io::fs::mkdir(path) -> Int` | `apps/ssg` `no-mkdir` | Single-level mkdir, mode 0755. Returns 0 / -1. Wraps libc `mkdir`; not recursive. |
| `std::io::fs::write_file_append(path, content) -> Int` | `apps/log-router` `write-file-truncates-no-append` | Companion to `write_file`. Opens with `O_WRONLY \| O_CREAT \| O_APPEND` (no truncate). Returns 0 / -1. |
| `eprintln(args...)` / `eprint(args...)` builtins | `apps/log-router` `no-eprintln-cant-isolate-debug-output` | Bare-name builtins like `print` / `println`. Route through `dprintf(2, ...)` to avoid the cross-libc `stderr` FILE* macro. Same compose-many-args shape as `println`. |
| `String + <printable>` auto-coerce | `apps/tcp-echo` `to_string-int-via-concatenation` | Mixed-type `+` where one side is `String` and the other is `Int` / `Float` / `Bool` / `Decimal` / `Duration` / `Time` / enum auto-coerces the non-String side via `value_to_string`. Symmetric (`port + " is the port"` works) and chained. |

## F.19 — per-directory seed model (2026-05-11)

`aperio build <dir>` accepts a directory and bundles every `.ap`
file in the directory as one seed (one binary). Top-level decls
in any file are visible to every file in the same directory, in
one shared scope — same shape Go gets from per-package
visibility. `aperio build <file.ap>` keeps working for
single-file apps.

File order in the merged bundle is alphabetical; resolution is
order-free (the typechecker flattens before name lookup). Binary
defaults to the directory's basename
(`apps/ferryman/` → `apps/ferryman/ferryman`).

Resolves `notes/aperio-friction.md` 2026-05-10
single-file-app-monolith. Spec entry: F.19 in
`spec/design-rationale.md`. Example: `examples/multi-file-seed/`.
Regression test: `crates/aperio-codegen/tests/multi_file_build.rs`.

## F.20 — structural interfaces, Phase A (2026-05-11)

`interface Name { fn ...; ... }` declares a structural interface.
Any locus whose method set is a superset structurally satisfies
it; satisfaction is implicit (no `impl I for L`).

**Phase A (shipped):** parser, AST, resolver, typechecker.
Structural impl-check fires at every call site where a fn
declares an interface-typed param (missing-method / arity /
type / return-type diagnostics).

**Phase B (deferred):** codegen vtable dispatch. Currently a
locus passed where an interface is expected errors at codegen
with a friendly Phase-B-pending message. The `std::text::Sink`
migration waits for Phase B.

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

- **Batteries included.** Go's approach: if a typical lotus
  program needs it, it ships. A new lotus user shouldn't
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

## v0 module map

### `std::collections`

Common containers. Built atop the language's projection-class
generics so the same API works across N=10, N=10K, N=10M.

- `Map<K, V>`        — hash map
- `Set<T>`           — hash set
- `List<T>`          — growable array
- `Deque<T>`         — double-ended queue
- `RingBuffer<T>`    — fixed-size circular buffer (recognition-class)

### `std::string`

String manipulation: `split`, `join`, `replace`, `trim`,
`startswith`, `endswith`, `format`, etc. Uses the language's
built-in `string` type.

### `std::math`

Beyond the language-native `sum` and `prod`:
- `min`, `max`, `mean`, `median`, `mode`, `stddev`
- `pow`, `sqrt`, `log`, `exp`
- `floor`, `ceil`, `round`, `abs`
- Constants: `pi`, `e`, etc.

### `std::stat`

Statistics needed by the framework's discipline:
- `correlate(x, y) -> float`
- `covariance(x, y) -> float`
- `regression(x, y) -> LinearFit`
- `convergence(perspectives, tolerance) -> bool`
- `perspective_distance(p1, p2) -> float`

### `std::numerical`

Numerical primitives for analyst-side curve fitting:
- `LinAlg`: matrices, vectors, common operations
- `Solve`: linear systems, optimization
- `FFT`: frequency-domain transforms (relevant to harmonic-mode
  projections)
- `Decimal`: extensions to the built-in `decimal` type
  (e.g., financial-rounding rules)

### `std::time`

Beyond the language-native `time` and `duration`:
- `now()`, `monotonic()`
- `sleep(d)`, `tick(d)`
- `format`, `parse` (ISO-8601 and common formats)
- `Calendar` for trading-day arithmetic
- `mock_clock(d)` for tests

### `std::io`

File and basic I/O:
- `read_file`, `write_file`, `append_file`
- `Reader`, `Writer` traits / interfaces
- `BufferedReader`, `BufferedWriter`
- stdin / stdout / stderr (richer interface than runtime
  builtins)

### `std::net`

Networking:
- TCP, UDP, Unix domain sockets
- `Listener`, `Connection`
- HTTP client + server (basic; not a full framework)
- TLS

### `std::bus`

The framework's view: **a transport is the bus kernel projected
through a parameter regime.** NATS and UDP multicast (and Unix
sockets, and shared memory, and TCP) are the same primitive
— typed pub-sub between loci — operating at different
(B, c, σ, φ) values. The `std::bus` module exposes this directly:
one `Adapter` interface; multiple implementations, each with its
declared parameter envelope.

```
trait Adapter {
    // Identifies the parameter regime this adapter operates in.
    // Used by the runtime to bind channels to transports based
    // on the channel's mode and declared envelope.
    fn envelope() -> TransportEnvelope;

    fn subscribe(subject: string, handler: ...) -> Subscription;
    fn publish(subject: string, msg: T) -> Result<(), Error>;
    // request_response is optional; some envelopes don't support it.
    fn request(subject: string, msg: T, timeout: duration) -> Result<R, Error>;
}

struct TransportEnvelope {
    latency_typical: duration;       // wire latency under load
    throughput_messages_per_sec: int;
    reliability: Reliability;        // BestEffort | AtLeastOnce | ExactlyOnce
    request_response: bool;
    fanout_max: int;                 // 1 for unicast, >1 for multicast / broker
    ordering: Ordering;              // None | PerSubject | Total
}
```

Provided implementations:

- `bus::tcp::Adapter` — typed pub-sub over TCP. Ordered, reliable,
  unicast or many-to-many via broker.
- `bus::nats::Adapter` — broker-mediated; reliable; supports
  request-response; per-subject ordering. Higher latency.
- `bus::udp_multicast::Adapter` — best-effort; line-rate
  fanout; no request-response. Sub-microsecond at LAN scale.
- `bus::unix_socket::Adapter` — same-host, ordered, reliable.
- `bus::in_memory::Adapter` — for tests; deterministic ordering.

Channels declared in Aperio source bind to transports at
deployment time. The locus's `bus { subscribe "..." as h; }`
declaration carries the channel's mode (bulk / harmonic /
resolution); the deployment config maps mode + subject pattern
to a transport whose envelope satisfies the requirement.

A bulk-mode market-data channel binds to UDP multicast; a
resolution-mode RFQ-quote channel binds to NATS; a closure-
test reporting channel binds to TCP or Unix socket. Same
source code, different transport per channel — chosen by
parameter fit, not by name.

### `std::encoding`

Serialization:
- `json::encode`, `json::decode`
- `protobuf::encode`, `protobuf::decode`
- `msgpack::encode`, `msgpack::decode`
- `binary::encode`, `binary::decode` (raw little/big endian)

### `std::perspective`

Infrastructure beyond what the runtime provides:
- `Versioned<T>` — wrap a perspective with version metadata
- `serialize<T: Perspective>(p)` — wire format
- `deserialize<T: Perspective>(bytes) -> Result<T>`
- `commit_when(condition)` — declarative commit policy

### `std::observability`

Metrics, logs, tracing:
- `Metric` (counter, gauge, histogram)
- `Logger` (structured, level-based)
- `Span` (tracing context propagation)
- Built on the bus interface so observability events flow as
  typed messages, not magic side-channels

### `std::test`

Testing primitives (referenced in `testing.md`):
- `assert`, `assert_eq`, `assert_neq`, `assert_rejects`
- `assert_closure(name, tolerance)` — runs the named closure
  test and asserts within band
- `mock_locus<T>(...)` — substitute a locus with a mock
- `bench_iter(n, f)` — controlled benchmark inner loop

### `std::ffi`

Foreign function interface — generic, language-agnostic. No
specific external runtime is favored.

- `extern fn` declaration syntax (TBD: grammar extension)
- `c::Callable` for calling into C libraries
- Marshalling helpers for common types
- Adapters for other runtimes (Go, Rust, etc.) live as
  third-party modules, not stdlib. Lotus stdlib provides the
  generic primitives; team-specific bindings (e.g.
  domain-specific typed messages) live in their own packages.

### `std::random`

Pseudo-random generation:
- `Rng` (defaults to a cryptographically-strong source for
  production; `mock_rng(seed)` for tests)
- Distributions: uniform, normal, etc.

### `std::sync`

Synchronization primitives at the locus boundary:
- `Channel<T>` — typed channel (Go-shaped)
- `Mutex<T>` — locks; rarely needed because of locus structure
- `WaitGroup` — coordination across multiple sub-loci

### `std::panic`

Panic introspection (rare; usually you use `on_failure`
instead):
- `catch_panic(f) -> Result<T, Panic>`
- `panic(msg)` — explicit panic

## What's not in stdlib (third-party territory)

- ML / learning libraries
- Database drivers (Postgres, etc.)
- Web frameworks beyond basic HTTP
- Image / audio / video processing
- Cloud SDKs (AWS, GCP, etc.)
- GUI / TUI frameworks
- Cryptography beyond TLS basics
- Compression formats beyond ones used in stdlib (gzip for HTTP)

These are the kinds of things that should live in the lotus
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
