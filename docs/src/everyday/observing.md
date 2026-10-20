# Observing a program

A running program can be seen four ways: its logs, its metrics, its
health, and the events of its own structure. Each is its own chapter's
detail; this one says what comes out with nothing declared, how to turn
each backend on, and what they cost.

## With nothing declared

- **Logs.** A `std::log` sink writes one [logfmt](./logging.md#records-the-format-a-sink-writes)
  record per event on stdout (`HALE_LOG_FORMAT=json` or `text` picks
  another; `HALE_LOG` sets the threshold).
- **The runtime's diagnostics.** A socket that failed, a TLS handshake,
  a drain that ran out of time come out on stderr as records of the same
  format, their `logger` naming the part (`lotus.tcp`, `lotus.tls`,
  `lotus.drain`). One collector reads the program's logs and the
  runtime's alike.
- **Shared memory.** `LOTUS_OBS=1` makes any binary publish its locus
  tree and traffic to a segment the local [observer](../systems/iris.md)
  attaches to (`hale run --observe`). It is the local default and needs
  no code.

Nothing is served on a port, and nothing is sent anywhere.

## Backends are params of main

The three network backends are params of the main locus, placed with
`placement { }` as any other param. The compiler recognizes each by its
type: there is no syntax, a program holds at most one of each kind, and
a construction of the main locus cannot supply one.

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

Each section below shows one of them alone.

## The metrics scrape

```hale
main locus App {
    params {
        metrics: std::observe::Scrape = std::observe::Scrape { port: 9100 };
    }
    placement { metrics: pinned; }
}

fn main() { App { }; }
```

`GET /metrics` on `127.0.0.1:9100` answers the Prometheus text
exposition of every live `std::metrics::Registry` and of the runtime's
own `hale_*` series (memory, the drain, live loci, restarts, failures,
each pool's queue, each topic's traffic). `port` is required; `host` and
`path` have defaults. A scrape is a listener of its own, and `pinned`
gives it a thread. Detail: [Metrics](./metrics.md#the-programs-scrape),
`spec/runtime.md` § Observe backends.

## Health and capacity probes

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

The runtime serves, on that port and every interface, from a thread of
its own that no placement applies to:

| path | answers |
|---|---|
| `/healthz` | `200 ok`, or `503` naming a queue that has held work its consumer has not touched for ten seconds (`LOTUS_PROBES_STALL_MS`) |
| `/readyz` | `503 starting` until the main locus's births are done, `200 ready`, then `503 draining` |
| `/capacity` | JSON: each queue's waiting cells, capacity and stalled time |

The probes do not serve `/metrics`. Detail:
[Multi-binary systems](../services/multi-binary.md), `spec/semantics.md`
§ Probes.

## Lifecycle events to a collector

```hale
main locus App {
    params {
        events: std::observe::Otlp = std::observe::Otlp { endpoint: "http://otel-collector:4318/v1/logs" };
    }
    placement { events: pinned; }
}

fn main() { App { }; }
```

Every locus birth, dissolve, restart and failure is sent as an OTLP/HTTP
JSON log record (`locus.birth`, `locus.dissolve`, `locus.restart`,
`locus.failure`), a batch every `interval` (one second), under a
resource carrying the build's model hash. The first batch opens with a
`hale.start` record. An empty `endpoint` is read from
`OTEL_EXPORTER_OTLP_LOGS_ENDPOINT`, else `OTEL_EXPORTER_OTLP_ENDPOINT`
plus `/v1/logs`; with none at all, every batch is dropped and counted.
Nothing waits on the collector: a full ring or a refused batch drops
events and counts them (`hale_otlp_dropped_total` on a scrape the
program also holds). Detail: [Logging](./logging.md#the-runtimes-events-to-a-collector).

## Combining them

They are independent. Hold any subset, in any order of declaration; a
program with a scrape and an `Otlp` shows the exporter's drops on its
page. Holding a scrape or an `Otlp` runs the runtime's counters without
the shared-memory segment, so nothing can attach and nothing is
published unless `LOTUS_OBS=1` also asks for it. Logs stay on the
sinks; the OTLP backend carries lifecycle events, not `std::log` records.

## Observing nothing costs nothing

A program that holds no backend param does not get the backends' source,
registries never register anywhere, the runtime's series table and event
ring are not linked, and the binary is the one it would be without the
feature (`spec/runtime.md` § Observe backends). A program with `Probes`
alone samples its queues once a second on the probe thread and nothing
on its own paths.
