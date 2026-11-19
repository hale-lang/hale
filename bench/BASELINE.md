# Performance baseline

Today's numbers for the v1 gates (GH #1950): build time, runtime and editor
latency, measured on one machine at one revision. **This page sets no
targets.** It records where the toolchain is so that targets can be chosen
against measurements and a later release can be compared with this one.

- Revision: the toolchain built at `8a3b1857b` (hale 0.35.0) with
  `cargo build --release`; the run's own record names the bench branch's
  head, which adds only `bench/`.
- Machine: AMD Ryzen 7 9800X3D, 8 cores / 16 threads, 31 GiB, Linux 6.18,
  otherwise idle (load average about 1).
- Programs are built by `hale build` with its defaults: release profile,
  tuned for the host CPU (`--target-cpu native`).
- Raw data, every run: [`baseline-2026-11-19.json`](./baseline-2026-11-19.json).

## Rerun

From the repository root:

```sh
cargo build --release
python3 bench/run.py --hale target/release/hale --out bench/results.json
```

It needs python3 and `rustc` (to build the HTTP load generator), and takes
about 12 minutes. It prints the table below and writes every run to
`--out`. `--only build,bus,json,http,memory,lsp` runs a subset; `--reps`
(default 5) sets the repeats of the runtime measurements, and
`--memory-seconds` (default 120) the memory run's length. Compare like
with like: the same machine, an idle one, and the same build profile.

## Results

Median of the repeats, with the range across them. Builds are repeated 3
times, runtime measurements 5 times.

| Area | Measure | Median | Range |
|---|---|---|---|
| build | `hale build` 92-build-an-api, cold | 5.94 s | 5.94 to 6.00 |
| build | 92-build-an-api, warm, unchanged | 1.83 s | 1.83 to 1.83 |
| build | 92-build-an-api, after a one-line edit | 1.83 s | 1.82 to 1.84 |
| build | 92-build-an-api, cold, peak RSS | 213 MiB | 213 to 214 |
| build | `hale build dna/host`, cold | 20.78 s | 20.73 to 20.81 |
| build | dna/host, warm, unchanged | 15.80 s | 15.75 to 15.86 |
| build | dna/host, after a one-line edit | 15.82 s | 15.80 to 15.83 |
| build | dna/host, cold, peak RSS | 1050 MiB | 1049 to 1050 |
| bus | round trip, same thread | 34 ns | 33 to 37 |
| bus | round trip, cross pool | 1514 ns | 1129 to 1528 |
| json | encode, `Order::to_json` (27 allocs/op) | 978 ns/op | 966 to 1014 |
| json | decode, `Order::from_json` (44 allocs/op) | 1937 ns/op | 1932 to 1974 |
| http | 1 connection: requests/s | 46,604 | 46,368 to 46,671 |
| http | 1 connection: p50 / p99 | 21 / 28 µs | p99 27 to 29 |
| http | 16 connections: requests/s | 80,860 | 80,148 to 81,455 |
| http | 16 connections: p50 / p99 | 193 / 243 µs | p99 240 to 247 |
| http | 64 connections: requests/s | 82,136 | 81,213 to 82,499 |
| http | 64 connections: p50 / p99 | 771 / 866 µs | p99 838 to 878 |
| http | errors / dropped attempts, every run | 0 / 0 | |
| memory | HTTP server RSS: idle, at 10 s, at 120 s of load | 4.7, 4.7, 4.7 MiB | |
| memory | RSS growth from 10 s to 120 s (9.7 M requests, 0 errors, 0 dropped) | -7 KiB/min (flat) | |
| editor | small seed: open, to diagnostics | 85 ms | |
| editor | small seed: edit, to diagnostics, p50 / p95 | 97 / 101 ms | 20 edits |
| editor | small seed: server peak RSS | 91 MiB | |
| editor | large seed (`dna/core`): open, to diagnostics | 721 ms | |
| editor | large seed: edit, to diagnostics, p50 / p95 | 751 / 758 ms | 20 edits |
| editor | large seed: server peak RSS | 303 MiB | |

## What each measures

**Build** (`hale build <target> -o <tmp>`, wall time). The references are one
representative example, `crates/hale-codegen/tests/fixtures/examples/92-build-an-api`
(an API with a surface, a handler and records), and the largest DNA program,
`dna/host` (the host's own seed with the core and the API it imports).

- *Cold* runs with an empty `XDG_CACHE_HOME`, so the runtime's C objects are
  compiled, then cached.
- *Warm* is the same build again with that cache.
- *After a one-line edit* appends a comment to the target's `main.hl` and
  builds again.

`hale build` has no incremental compilation beyond the runtime-object cache:
an unchanged rebuild costs what a rebuild after an edit does. The cold
build's extra cost is the runtime cache (about 4 s for the small program,
5 s for the large one). Peak RSS is the `hale build` process's own, from
`wait4`.

**Bus round trip** (`bench/bus_rt_same.hl`, `bench/bus_rt_xpool.hl`). main's
`run()` publishes a `Ping`, an `Echo` locus answers with a `Pong`, main's
handler counts it, and main yields until it is in before the next `Ping`: one
round trip in flight, timed over 200,000 (same thread) or 100,000 (cross
pool) after 1,000 of warm-up. *Same thread* places `Echo` on main's pool;
*cross pool* places it on a cooperative pool of its own (`io`, a second OS
thread), so each round trip is two cross-thread posts and their wake-ups. The
figure is wall time per round trip.

**JSON** (`bench/json_bench.hl`, run by `hale bench`). The generated struct
codecs on one order record with a nested list of three lines: `to_json`
encoding, and `from_json` decoding the encoded text. `hale bench` calibrates
the batch size and reports ns/op and allocs/op.

**HTTP** (`bench/http_serve.hl`, loaded by `bench/tools/httpload.rs`). The
book's minimal `std::http::Server`: a handler that counts its hits and
answers a short text body. The load generator is closed-loop: each
connection sends `GET /` as soon as the last response is in, for 10 s after
2 s of warm-up, at 1, 16 and 64 connections. The server answers each request
with `Connection: close`, so every request includes a TCP connect, and the
latency is measured from the request's send to the end of its response,
reconnect included. A non-200 response is an *error*; a connection that
fails before its response is complete is a *dropped* attempt, counted and
sent again, so a server that loses requests cannot read as error-free. The server is a single handler loop, so throughput
levels off at about 81,000 requests/s from 16 connections up, and further
connections only queue (p50 rises in proportion).

**Memory** (`bench/http_serve.hl` under load). The same server under 16
connections of constant load for 120 s, with its RSS (`/proc/<pid>/status`
VmRSS) read every second. Growth is the least-squares slope from 10 s to
the end. It stays at 4.7 MiB across 9.7 million requests.

**Editor** (`bench/tools/lsp_latency.py`). A client drives `hale lsp` over
stdio: it opens one file of a seed and times the diagnostics that follow,
then makes 20 ordinary edits (a comment line added at the end of the file,
then removed, alternately; full-document sync). Each time runs from the
client's write to the last `publishDiagnostics` of the pass (the server
silent for 1 s after it). The time to the edited file's first publication
is also recorded, and in this run it equals the time to the last one. The
small seed is `92-build-an-api/main.hl`; the large seed is `dna/core`,
opened at `assembly.hl`, which the server checks whole on every event. Peak
RSS is the server's VmHWM at the end. No editor baseline was recorded under
GH #1326 before this one.

## Not measured here

Cold-start of a built binary, cross-machine bus transports, HTTPS, and the
DNA organism's own services. Each needs a harness of its own; none is a v1
gate yet.
