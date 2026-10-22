# Observing a program

Hale programs expose four observability surfaces: structured application and runtime logs, Prometheus metrics (`std::observe::Scrape`), HTTP health and queue-capacity probes (`std::observe::Probes`), and OpenTelemetry lifecycle events (`std::observe::Otlp`). The three network backends are declared as `params` on your `main locus` and incur zero binary or runtime cost when omitted.

```hale
main locus App {
    params {
        probe_port: Int = 9090;
        reg: std::metrics::Registry = std::metrics::Registry { namespace: "app" };
        metrics: std::observe::Scrape = std::observe::Scrape { port: 9100 };
        probes: std::observe::Probes = std::observe::Probes { port: self.probe_port };
        events: std::observe::Otlp = std::observe::Otlp { };
    }
    placement { metrics: pinned; events: pinned; }
    run() {
        let hits = std::metrics::counter(self.reg, "hits",
            std::metrics::labels_empty());
        hits.inc();
        while !self.draining { std::time::sleep(100ms); }
    }
}

fn main() { App { }; }
```

---

## Observability Surfaces Summary

| Surface / Backend | Activation | Endpoints / Output | Purpose |
|---|---|---|---|
| **Application & Runtime Logs** | `std::log` sinks + automatic runtime `stderr` | `stdout` / `stderr` / `FileSink` (`HALE_LOG`, `HALE_LOG_FORMAT`) | Structured `logfmt`, `json`, or `text` records for app events and `lotus.*` runtime diagnostics |
| **Local Shared Memory** | `LOTUS_OBS=1` or `hale run --observe` | Shared-memory segment read by [Iris](../systems/iris.md) | Live inspection of the locus tree and bus traffic with zero code changes |
| **`std::observe::Scrape`** | `main locus` param | `GET /metrics` (default `127.0.0.1:<port>`) | Prometheus text exposition of all `std::metrics::Registry` instances and `hale_*` runtime metrics |
| **`std::observe::Probes`** | `main locus` param | `GET /healthz`, `/readyz`, `/capacity` (`0.0.0.0:<port>`) | Liveness (queue stall detection), readiness, and queue utilization JSON |
| **`std::observe::Otlp`** | `main locus` param | `POST <endpoint>` (OTLP/HTTP JSON logs) | Exports `hale.start`, `locus.birth`, `locus.dissolve`, `locus.restart`, and `locus.failure` |

---

## Default Behavior (With Nothing Declared)

When a program declares no `std::observe::*` parameters on its `main locus`, no network ports are opened and no telemetry is sent over the network:

- **Application logs**: Any configured `std::log` sink writes one [logfmt](./logging.md#output-formats-hale_log_format) record per event to `stdout` (`HALE_LOG_FORMAT=json` or `text` changes the format; `HALE_LOG` filters severity).
- **Runtime diagnostics**: Failed sockets, TLS handshake errors, and drain timeouts are written to `stderr` in the format selected by `HALE_LOG_FORMAT`, with `logger` identifying the runtime subsystem (`lotus.tcp`, `lotus.tls`, `lotus.drain`).
- **Local shared-memory telemetry (`LOTUS_OBS=1`)**: Setting `LOTUS_OBS=1` (enabled automatically by `hale run --observe`) publishes the live locus tree and bus traffic to a local shared-memory segment that the [local observer](../systems/iris.md) attaches to.

---

## Main-Locus Observation Backends

`std::observe::Scrape`, `std::observe::Probes`, and `std::observe::Otlp` are declared as `params` on the `main locus` and scheduled via `placement { }`. The compiler recognizes each backend by its type:
- A program may declare **at most one** parameter of each backend type.
- Callers constructing the `main locus` cannot pass an override for a backend parameter.

### Prometheus Metrics Scrape (`std::observe::Scrape`)

```hale
main locus App {
    params {
        metrics: std::observe::Scrape = std::observe::Scrape { port: 9100 };
    }
    placement { metrics: pinned; }
}

fn main() { App { }; }
```

- Serves `GET /metrics` on `127.0.0.1:9100` (`port` is required; `host` defaults to `"127.0.0.1"` and `path` defaults to `"/metrics"`).
- Renders the Prometheus text exposition of every live `std::metrics::Registry` in the program alongside the runtime's own `hale_*` series (resident memory, drain status, live loci count, restarts, failures, per-pool queue depth, and per-topic traffic).
- Place `Scrape` with `pinned` to run its listener on a dedicated thread. For full details, see [Metrics § Process-Wide Scrape Server](./metrics.md#process-wide-scrape-server-stdobservescrape) and `spec/runtime.md` § Observe backends.

### Health and Capacity Probes (`std::observe::Probes`)

```hale
main locus App {
    params {
        probes: std::observe::Probes = std::observe::Probes { port: 9090 };
    }
    run() {
        while !self.draining { std::time::sleep(100ms); }
    }
}

fn main() { App { }; }
```

`Probes` binds `<port>` on all network interfaces (`0.0.0.0`) and runs on its own dedicated runtime thread (no `placement` entry is needed or applied):

| Path | HTTP Status & Payload | Description |
|---|---|---|
| `/healthz` | `200 ok` or `503` | Returns `503` naming any queue that has held unprocessed work for more than 10 seconds (configurable via `LOTUS_PROBES_STALL_MS`) |
| `/readyz` | `503 starting`, `200 ready`, or `503 draining` | Returns `503 starting` until all `main locus` births complete, `200 ready` during normal execution, and `503 draining` once drain begins |
| `/capacity` | `200` (`application/json`) | Reports waiting messages, total capacity, and stalled duration for every runtime queue |

`Probes` does not serve `/metrics`. For multi-process supervision details, see [Multi-binary systems](../services/multi-binary.md) and `spec/semantics.md` § Probes.

### Lifecycle Events Exporter (`std::observe::Otlp`)

```hale
main locus App {
    params {
        events: std::observe::Otlp = std::observe::Otlp {
            endpoint: "http://otel-collector:4318/v1/logs",
        };
    }
    placement { events: pinned; }
}

fn main() { App { }; }
```

- Exports runtime lifecycle events (`locus.birth`, `locus.dissolve`, `locus.restart`, and `locus.failure`, preceded by an initial `hale.start` record in the first batch) as OTLP/HTTP JSON log records every `interval` (`1s` default). Every batch carries the build's model hash in its OpenTelemetry resource attributes.
- If `endpoint` is empty, `Otlp` resolves `OTEL_EXPORTER_OTLP_LOGS_ENDPOINT`, or `OTEL_EXPORTER_OTLP_ENDPOINT` + `/v1/logs`. If no endpoint is configured, batches are dropped and counted.
- Exports never block application execution: if the internal ring fills or the collector fails/times out (`timeout_ms`, default 2000 ms), records are dropped and counted in `hale_otlp_dropped_total` on the `Scrape` page. See [Logging § Exporting Runtime Lifecycle Events](./logging.md#exporting-runtime-lifecycle-events-stdobserveotlp).

---

## Combining Backends & Zero-Overhead Guarantee

- **Independent composition**: Declare any subset of `Scrape`, `Probes`, and `Otlp` in any order. When both `Scrape` and `Otlp` are present, `hale_otlp_dropped_total` is automatically exposed on `/metrics`.
- **No unintended shared memory**: Declaring `Scrape` or `Otlp` enables internal runtime counters without creating the local shared-memory segment; external local tools cannot attach unless `LOTUS_OBS=1` is also set.
- **Separation of logs and lifecycle events**: `std::log` records remain on `std::log` sinks; `Otlp` carries only structural locus lifecycle transitions.
- **Zero cost when unused**: If a program declares no `std::observe::*` backend parameters, backend implementations, global registry tables, and the lifecycle event ring are excluded from compilation and linking (`spec/runtime.md` § Observe backends). A program declaring only `Probes` samples its queues once per second on the probe thread with zero instrumentation on hot paths.
