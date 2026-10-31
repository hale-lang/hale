# Metrics

`std::metrics` provides Prometheus-compatible counters, gauges, and histograms backed by a thread-safe `Registry`. Register metric handles at startup, update them directly on the hot path, and expose them in the standard Prometheus text exposition format via `std::observe::Scrape` or `std::metrics::Endpoint`.

```hale
fn main() {
    let reg = std::metrics::Registry { namespace: "app" };

    let hits = std::metrics::counter(reg, "hits",
        std::metrics::labels_one("route", "/api"));
    hits.inc();
    hits.add(2.0);

    print(reg.render());
}
```

```text
# TYPE app_hits counter
app_hits{route="/api"} 3
```

---

## API Summary

| Type / Function | Signature / Parameters | Description |
|---|---|---|
| `std::metrics::Registry` | `namespace: String`, `render() -> String` | Owns metric storage and prefixes all series names with `<namespace>_` |
| `std::metrics::counter` | `fn(Registry, String, Labels) -> Counter` | Registers or retrieves a monotonically increasing counter (`inc()`, `add(Float)`) |
| `std::metrics::gauge` | `fn(Registry, String, Labels) -> Gauge` | Registers or retrieves a gauge (`set(Float)`, `inc()`, `dec()`, `add(Float)`, `sub(Float)`) |
| `std::metrics::histogram` | `fn(Registry, String, String, Labels) -> Histogram` | Registers or retrieves a bucketed histogram (`observe(Float)`) |
| `std::metrics::labels_empty` | `fn() -> Labels` | Creates an empty label set |
| `std::metrics::labels_one` | `fn(String, String) -> Labels` | Creates a single `k=v` label set |
| `std::metrics::labels_two` | `fn(String, String, String, String) -> Labels` | Creates a two-pair label set |
| `std::metrics::labels_append` | `fn(Labels, String, String) -> Labels` | Appends an additional `k=v` pair to an existing label set |
| `std::observe::Scrape` | `port: Int`, `host: String`, `path: String` | Main-locus HTTP server exposing all program registries and runtime `hale_*` series |
| `std::metrics::Endpoint` | `registry: Registry` | `std::http::Handler` serving a single `Registry` on an existing `std::http::Server` |

---

## Registries, Counters & Hot-Path Handles

A `std::metrics::Registry` owns its metric map and histogram storage, applying its `namespace` as a `<namespace>_` prefix on every metric family.

### Factory Semantics & Hot-Path Caching

- **Idempotent registration**: Calling `counter(reg, "hits", labels)` multiple times with the same metric name and label set returns a handle to the **same** underlying series without resetting its value.
- **Direct slot handles**: `Counter`, `Gauge`, and `Histogram` handles reference their storage slot directly without performing a map lookup per update. Resolve handles once at startup, store them in locus `params`, and mutate them on the hot path:

```hale
type Order { id: Int; }

locus OrderHandler {
    params {
        processed: std::metrics::Counter;
    }
    fn on_order(o: Order) {
        self.processed.inc();
    }
}
```

---

## Gauges, Histograms & Labels

### Gauges (`std::metrics::gauge`)

Use a `Gauge` for values that increase and decrease (such as queue depth, active connections, or temperature):

```hale,fragment
let depth = std::metrics::gauge(reg, "queue_depth",
    std::metrics::labels_empty());
depth.set(0.0);
depth.inc();        // also supports: dec(), add(v), sub(v)
```

### Histograms (`std::metrics::histogram`)

Use a `Histogram` to record distributions (such as request latencies or payload sizes). Bucket upper bounds are passed as a space-separated string of ascending floats and parsed once at registration:

```hale,fragment
let lat = std::metrics::histogram(reg, "latency_seconds",
    "0.005 0.01 0.05 0.1 0.5", std::metrics::labels_empty());
lat.observe(0.023);
```

- **Prometheus exposition**: Renders cumulative `_bucket{le="..."}` counters, an implicit `+Inf` bucket catching all observations, and `<name>_sum` / `<name>_count` series.
- **Bucket limits**: Bounds must be listed in ascending order. Up to **32 buckets** are supported per histogram (bounds beyond the 32nd are ignored).

### Labels (`labels_*`)

Labels partition a metric name into distinct time series grouped under one `# TYPE` family header:

```hale,fragment
let l  = std::metrics::labels_two("route", "/api", "method", "GET");
let l2 = std::metrics::labels_append(l, "status", "200");
```

Use `labels_empty()`, `labels_one(k, v)`, or `labels_two(k1, v1, k2, v2)` for common arities, and chain `labels_append(l, k, v)` for three or more labels.

---

## Process-Wide Scrape Server (`std::observe::Scrape`)

To expose every `Registry` in your program along with the runtime's built-in `hale_*` metrics on a dedicated `/metrics` HTTP listener, declare `std::observe::Scrape` as a param on your `main locus`:

```hale
main locus App {
    params {
        reg: std::metrics::Registry = std::metrics::Registry { namespace: "app" };
        metrics: std::observe::Scrape = std::observe::Scrape { port: 9100 };
    }
    placement { metrics: pinned; }
    run() {
        let hits = std::metrics::counter(self.reg, "hits",
            std::metrics::labels_empty());
        hits.inc();
    }
}

fn main() { App { }; }
```

```console
$ curl localhost:9100/metrics
# TYPE app_hits counter
app_hits 1
# TYPE hale_draining gauge
hale_draining 0
# TYPE hale_failures_total counter
hale_failures_total 0
# TYPE hale_loci_live gauge
hale_loci_live 4
# TYPE hale_process_cpu_seconds_total counter
hale_process_cpu_seconds_total 0.031416
# TYPE hale_process_resident_memory_bytes gauge
hale_process_resident_memory_bytes 10670080
# TYPE hale_process_start_time_seconds gauge
hale_process_start_time_seconds 1793260800.25
# TYPE hale_restarts_total counter
hale_restarts_total 0
```

### `Scrape` Configuration & Guarantees

- **Automatic discovery**: Every live `std::metrics::Registry` in the process is automatically included alongside runtime series (`hale_process_resident_memory_bytes`, `hale_process_cpu_seconds_total` (user and system CPU time), `hale_process_start_time_seconds` (when the process started, unix seconds: `time() - hale_process_start_time_seconds` is its uptime), `hale_draining`, `hale_loci_live`, `hale_restarts_total`, `hale_failures_total`, per-pool queue depths and parked coroutines, and per-topic bus traffic).
- **Host, port, and path**: `port` is required. `host` defaults to `"127.0.0.1"` and `path` defaults to `"/metrics"`. Both can reference other locus params or environment variables:

```hale,fragment
params {
    metrics_port: Int = std::str::parse_int(std::env::var("METRICS_PORT")) or 9100;
    metrics: std::observe::Scrape = std::observe::Scrape { host: "0.0.0.0", port: self.metrics_port };
}
```

- **Single-instance rule & zero cost when unused**: A program may declare at most one `std::observe::Scrape` param on its `main locus` (declaring a second or passing one at `main locus` construction is a compile error). If a program declares no `Scrape`, registries do not register globally and zero scrape runtime overhead is linked into the binary.
- **Separate from health probes**: `std::observe::Scrape` and [`std::observe::Probes`](./observing.md#health-and-capacity-probes-stdobserveprobes) are separate listeners on separate ports; `Probes` does not serve `/metrics`.

---

## Serving a Single Registry (`std::metrics::Endpoint`)

To mount a single `Registry` on an existing [`std::http::Server`](./http.md) instead of running `std::observe::Scrape`, use `std::metrics::Endpoint` (which satisfies `std::http::Handler`):

```hale
fn build_registry() -> std::metrics::Registry {
    let reg = std::metrics::Registry { namespace: "app" };
    let hits = std::metrics::counter(reg, "hits",
        std::metrics::labels_empty());
    hits.inc();
    return reg;
}

fn main() {
    std::http::Server {
        port: 9090,
        handler: std::metrics::Endpoint { registry: build_registry() }
    };
}
```

```console
$ curl localhost:9090/metrics
# TYPE app_hits counter
app_hits 1
```

### Storage Lifetime & Cross-Pool Synchronization

- **Owned storage**: `Registry` owns its internal metric map and histogram list children, so returning a `Registry` by value from a helper function (`build_registry()`) keeps its storage alive as long as the `Registry` lives. (If you explicitly override the `store:` or `histograms:` params on `Registry`, construct those overrides in a scope that outlives every scrape.)
- **Cross-pool thread safety**: The metric map inside `Registry` uses `sync = serialized`, allowing worker pools to mutate counters, gauges, and histograms concurrently while a scrape server renders `/metrics` on a separate pool or `pinned` thread.

Next: [Observing a program](./observing.md).
