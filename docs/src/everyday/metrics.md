# Metrics

> **Coming from Python / Go?** `std::metrics` is shaped like the
> Prometheus client libraries you already know: register a
> counter / gauge / histogram once, mutate it from the hot path,
> and expose everything on a `/metrics` endpoint in the standard
> text format. No collector protocol, no push gateway — a scraper
> pulls the text page.

## A registry and a counter

Everything hangs off a `Registry`. It owns its own storage, so
constructing one is a single line:

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

The `namespace` becomes the `app_` prefix on every series — set
it once, forget it.

Two things to notice about the factory call:

- **It's idempotent.** Calling `counter(reg, "hits", ...)` again
  with the same name and labels returns a handle to the *same*
  series — nothing resets. Registration is safe to repeat.
- **The handle is the hot-path object.** It references the
  storage slot directly, not the registry. Resolve it once at
  boot, cache it as a field, and call `inc()` from wherever the
  work happens:

```hale
locus OrderHandler {
    params {
        processed: std::metrics::Counter;
    }
    fn on_order(o: Order) {
        // ... do the work ...
        self.processed.inc();
    }
}
```

## Gauges and histograms

A gauge goes up *and* down — current temperature, queue depth,
open connections:

```hale,fragment
let depth = std::metrics::gauge(reg, "queue_depth",
    std::metrics::labels_empty());
depth.set(0.0);
depth.inc();        // also: dec(), add(v), sub(v)
```

A histogram buckets observations — latencies, sizes. You declare
the bucket upper bounds as a space-separated string, parsed once
at registration:

```hale,fragment
let lat = std::metrics::histogram(reg, "latency_seconds",
    "0.005 0.01 0.05 0.1 0.5", std::metrics::labels_empty());
lat.observe(0.023);
```

Rendering follows the Prometheus histogram convention: cumulative
`_bucket{le="..."}` counts, an implicit `+Inf` bucket that
catches everything, and `_sum` / `_count` series. Bounds must be
ascending, and there's room for 32 of them — more than any
scrape dashboard wants to look at anyway.

## Labels

Labels distinguish series that share a name:

```hale,fragment
let l = std::metrics::labels_two("route", "/api", "method", "GET");
let l2 = std::metrics::labels_append(l, "status", "200");
```

`labels_empty()` / `labels_one(k, v)` / `labels_two(...)` cover
the common arities; `labels_append` extends any of them. The same
name with different labels is a different series — that's the
whole point.

## The /metrics endpoint

`std::metrics::Endpoint` is a ready-made `std::http::Handler`
that answers every request with the current rendering:

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

Point Prometheus (or anything that speaks the text exposition
format) at it and you're done.

Returning the registry from a builder function like this works
because the registry *owns* its storage: the metric map and
histogram list are its children and live exactly as long as it
does. If you ever pass storage in explicitly (the `store:` /
`histograms:` params accept overrides), construct it in a scope
that outlives every scrape — a let-bound override inside a
builder function dissolves when the builder returns.

## Scraping from another pool

The canonical production shape puts the scrape server on its own
pool while handlers write metrics from theirs. The metric map is
`sync = serialized` for exactly this reason — one mutex around a
map touched a few times per request and once per scrape is the
right discipline, and you don't have to think about it.
