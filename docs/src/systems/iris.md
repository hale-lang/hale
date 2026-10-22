# Iris: Live System Observer

**Iris** is Hale's built-in live topology and telemetry observer. When a Hale binary runs with `LOTUS_OBS=1` (or via `hale run --observe`), Iris attaches to its shared-memory observation rings, fuses single- or multi-process telemetry into a unified system graph, and serves a real-time web UI and JSON/SSE API over HTTP.

```sh
hale run --observe path/to/prog.hl
# Open the live topology view in a browser:
open http://127.0.0.1:8787
```

## Running with `--observe`

Passing `--observe` to `hale run` sets `LOTUS_OBS=1` on the target program and spawns `hale iris` alongside it:

| Flag | Behavior |
|---|---|
| `hale run --observe prog.hl` | Starts `prog.hl` with `LOTUS_OBS=1` and serves Iris on `http://127.0.0.1:8787`. |
| `hale run --observe=<port> prog.hl` | Serves Iris on `<port>` instead of `8787`. |
| `hale run --observe=0 prog.hl` | Binds an ephemeral OS-assigned port (recommended for parallel test suites and scripts). |

`--observe` is transparent to the observed program:
- **Clean `argv` and `stdout`**: `hale run` consumes the `--observe` flag before building the program's `argv`, and relays Iris's own diagnostic messages to `stderr` so piping (`hale run --observe prog.hl | cat`) captures only the program's `stdout`.
- **Coupled lifecycle**: The Iris process is bound to the `hale run` session. Terminating `hale run` (via `Ctrl-C`, `timeout`, or CI cancellation) automatically stops the observer.

## `hale iris` CLI Reference

You can also run `hale iris` standalone to attach to any running `LOTUS_OBS=1` processes registered under `$XDG_RUNTIME_DIR/hale/` (or `/tmp/hale-obs/`):

| Command | Description |
|---|---|
| `hale iris [port] [artifact.json]` | Attaches to all local `LOTUS_OBS=1` processes, fuses their observation rings, and serves HTTP on `port` (default `8787`). If a `--dump-topology` `artifact.json` is provided, overlays declared vs. live topology. |
| `hale iris --diff <a.topology> <b.topology>` | Computes `hale model diff` between two topology artifacts and renders the semantic diff over the live fleet (view key `4`), using `b.topology` as the target law artifact. |
| `hale iris --diff <diff.json>` | Renders a precomputed `hale model diff` JSON document over the live fleet. |
| `hale iris inspect <artifact.json> [url]` | Headless CLI check that compares `<artifact.json>` against a live `hale iris` URL and prints topology drift, declared-but-silent topics, and claim status. |
| `hale iris --where` | Prints the path to the cached Iris binary directory for the active toolchain and environment. |
| `hale iris --build-only` | Materializes and compiles the embedded Iris binary ahead of time (useful in Dockerfile build stages) and prints the binary path. |

### HTTP Endpoints

A running `hale iris` server exposes three endpoints:
- **`GET /`**: Interactive browser UI displaying the locus tree, message routing topology, transport edge metrics (throughput, queue depth, latency, drops), and live supervision events.
- **`GET /snapshot`**: Point-in-time JSON snapshot of the fused system graph.
- **`GET /events`**: Server-Sent Events (SSE) stream of live graph deltas.

### Live-Locus Table Bounds

To keep observer memory strictly bounded, Iris tracks at most **8,192 live loci** across all attached processes. Table slots are assigned first-come, first-served so long-lived root loci, servers, and worker pools remain tracked even if a child locus churns rapidly. If the table reaches capacity, additional locus births increment `loci_untracked` in `/snapshot` until existing loci dissolve and free table rows. If the observer process is killed by a signal, `hale iris` logs the signal name and exits with code `128 + <signal>`.

## Topology Diff & Fleet Review (`--diff`)

To review how an architectural change affects a running system, dump the topology before and after your edit and launch `hale iris --diff`:

```sh
hale check . --dump-topology=before.topology
# ... edit source files ...
hale check . --dump-topology=after.topology
hale iris --diff before.topology after.topology
```

Pressing `4` in the browser UI opens the review overlay:
- Displays semantic model changes (`+ locus EmailIntake`, `! fn Intake::on_mail gains publish`, `! locus Support subscribes: +Tickets`, `! claim iso: holds → violated`).
- Compares each attached process's compiled model hash against the target artifact, marking processes still running the older build as **stale** in the sidebar and ringing them on the graph canvas.

> [!NOTE]
> `hale dna run` attaches Iris to a DNA organism process just like any other Hale binary. Iris inspects low-level locus and bus execution; for DNA's human-facing operational board (`dna/face`), see [DNA](../dna/index.md).

## Build Caching & Protocol Guarantees

Iris is written in Hale (`fuse-hl` graph consumer, C shared-memory attach shim, and vanilla JavaScript renderer) and its source is embedded directly inside the `hale` binary. On first invocation, `hale` materializes and compiles Iris into:

```text
$XDG_CACHE_HOME/hale/iris/<toolchain-hash>/   (or ~/.cache/hale/iris/<toolchain-hash>/)
```

The `<toolchain-hash>` incorporates the compiler version, embedded Iris sources, and active build environment variables (`HALE_DEV`, sanitizers, `LOTUS_LTO`). Subsequent launches execute the cached binary directly (`hale iris --where` prints the cache path; delete the directory to force a rebuild). Because the same compiler builds both the observed binary and Iris against `crates/hale-codegen/runtime/obs_protocol.h` (enforced by compile-time `_Static_assert`s), the shared-memory wire protocol cannot drift between program and observer.

Iris is strictly read-only: attaching increments `observer_count` on the target's shared-memory control page—instructing the runtime probes to emit ring records in addition to counters—and never alters scheduling or mutates target memory. To capture a run to disk for deterministic re-execution, use `LOTUS_OBS_RECORD` and [`hale replay`](./replay.md).
