# Standard library

Bundled with the toolchain, no separate install required, but
explicit `import std::...` is needed (Go-style: batteries
included but you say which battery).

This document scopes the v0 stdlib. Each module gets a one-paragraph
description, an export sketch, and an open-decisions list. Full
APIs are specified per-module in the `stdlib/` directory of the
compiler repo (when that exists).

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

Bus transports for the language's bus-block declarations:
- `nats::Adapter` — connects bus subjects to NATS
- `udp_multicast::Adapter` — UDP multicast (for grease)
- `unix_socket::Adapter`
- `in_memory::Adapter` — for tests
- `Adapter` trait that user code can implement for custom
  transports

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
  generic primitives; team-specific bindings (e.g. grease's
  typed messages) live in their own packages.

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
