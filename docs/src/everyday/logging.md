# Logging

`std::log` is a publish/subscribe logging system built on Hale's typed message bus. A `Logger` publishes `LogEvent` records to topic `log.<name>`, and one or more sinks (`StdoutSink`, `FileSink`, `ConsoleSink`) subscribe to `log.**` to format and write them.

```hale
fn main() {
    // Instantiate sinks before loggers so subscribers exist when events are published
    let sink = std::log::StdoutSink { };

    let log = std::log::Logger { name: "app" };
    log.info("starting up");
    log.warn_kv("retrying", std::log::kv("attempt", "2") + " " + std::log::kv("after", "2s"));
    log.error("connection refused");
}
```

---

## Logging Components & Environment Variables

| Component / Variable | Parameters / Values | Description |
|---|---|---|
| `std::log::Logger` | `name: String`, `min_severity: Int` | Publishes events to `log.<name>` via `trace`, `debug`, `info`, `warn`, `error` and `*_kv` methods |
| `std::log::kv` | `fn(String, String) -> String` | Formats and quotes a single `key=value` logfmt pair |
| `std::log::StdoutSink` | `min_severity: Int`, `format: String` | Subscribes to `log.**` and writes one record per line to stdout (`text` mode sends `warn`/`error` to stderr) |
| `std::log::FileSink` | `path: String`, `keep_files: Int`, `min_severity: Int`, `format: String` | Appends all levels to `path` and rotates files atomically by size (`app.log` -> `app.log.1`) |
| `std::log::ConsoleSink` | `min_severity: Int` | Human-readable terminal sink with colored level badges (`WARN`/`ERROR` on stderr; honors `NO_COLOR` and TTY detection) |
| `std::observe::Otlp` | `endpoint: String`, `timeout_ms: Int`, `interval: Duration` | Main-locus backend exporting runtime lifecycle events (`birth`, `dissolve`, `restart`, `failure`) via OTLP/HTTP |
| `HALE_LOG` | `error`, `warn`, `info`, `debug` | Sets the default minimum severity threshold at the logger |
| `HALE_LOG_FORMAT` | `logfmt` (default), `json`, `text` | Controls record formatting for sinks and runtime stderr diagnostics |

---

## Severity Levels & Filtering

Loggers provide five severity levels, each with a plain message method and a structured `_kv` method:

| Severity | `min_severity` Code | Plain Method | Structured Method |
|---|---|---|---|
| Trace | `0` | `log.trace(msg)` | `log.trace_kv(msg, kvs)` |
| Debug | `1` | `log.debug(msg)` | `log.debug_kv(msg, kvs)` |
| Info | `2` | `log.info(msg)` | `log.info_kv(msg, kvs)` |
| Warn | `3` | `log.warn(msg)` | `log.warn_kv(msg, kvs)` |
| Error | `4` | `log.error(msg)` | `log.error_kv(msg, kvs)` |

### Filtering with `HALE_LOG` and `min_severity`

Set `HALE_LOG` (`error`, `warn`, `info`, or `debug`) to drop events below that threshold:

```sh
HALE_LOG=warn ./myapp      # emits warn and error only
```

- **Zero-cost publisher filtering**: Severity filtering happens inside `Logger` before publishing. A `log.trace(...)` call below the threshold performs a single integer comparison and publishes nothing to the bus. (When `HALE_LOG` is unset or unrecognized, no levels are dropped.)
- **Code-pinned thresholds**: Pass `min_severity` (`0`..`4`) to `std::log::Logger { name: "app", min_severity: 3 }` to pin the threshold in code and ignore `HALE_LOG`.
- **Per-sink thresholds**: Sinks also accept `min_severity`, allowing you to run multiple sinks at different verbosities simultaneously (for example, `FileSink` at `min_severity: 1` (`debug`) and `ConsoleSink` at `min_severity: 3` (`warn`)).

---

## Structured Fields (`_kv` and `std::log::kv`)

Attach structured key-value fields to any log event using the `*_kv` methods and `std::log::kv(key, value)`:

```hale,fragment
log.info_kv("order settled", std::log::kv("order", id));

log.warn_kv(
    "retrying",
    std::log::kv("attempt", to_string(n)) + " "
        + std::log::kv("after", "2s")
);
// -> time=2026-10-19T06:56:00Z level=warn logger=app msg=retrying attempt=2 after=2s
```

- **Logfmt escaping**: `std::log::kv(key, value)` formats a `key=value` pair in [logfmt](https://brandur.org/logfmt), automatically quoting `value` and escaping `\n`, `\t`, quotes, backslashes, and control characters (`\u0001`) whenever `value` contains spaces, quotes, `=`, backslashes, or control characters. Join multiple pairs with `" "`.
- **Flat bus payloads**: Fields are carried on `LogEvent` as pre-formatted text rather than a map locus, preserving field order and allowing `LogEvent` to cross inter-process bus transports directly.
- **Publish-time timestamps (`ts`)**: Every event is stamped with `ts` (Unix seconds) at the moment it is **published**, not when the sink renders it. Queued sinks, cross-process sinks, and `hale replay` preserve the original emission timestamp.

---

## Output Formats (`HALE_LOG_FORMAT`)

Every line emitted by `StdoutSink` or `FileSink` is a single-line record containing the event timestamp (RFC 3339 UTC), severity level, logger name, message, and structured fields.

| Format | Example Output | Notes |
|---|---|---|
| `logfmt` (default) | `time=2026-10-19T06:56:00Z level=warn logger=app.db msg=retrying attempt=2 after=2s` | Default since October 2026; writes all levels to stdout on `StdoutSink` |
| `json` | `{"time":"2026-10-19T06:56:00Z","level":"warn","logger":"app.db","msg":"retrying","attempt":"2","after":"2s"}` | Single-line JSON object per event |
| `text` | `[WARN app.db] retrying attempt=2 after=2s` | Human-oriented format; routes `warn` and `error` to stderr |

Select a format via the `HALE_LOG_FORMAT` environment variable or pin it on a sink with `format: "logfmt" | "json" | "text"` (unrecognized format strings default to `"logfmt"`):

```sh
HALE_LOG_FORMAT=json ./myapp
HALE_LOG_FORMAT=text ./myapp
```

### Runtime Diagnostics on `stderr`

The Hale runtime emits its own operational diagnostics (such as TCP accept errors, TLS handshake failures, or drain timeouts) to `stderr` using the same record format selected by `HALE_LOG_FORMAT`, with `logger` set to the runtime subsystem (`lotus.tcp`, `lotus.tls`, `lotus.drain`, etc.):

```text
time=2026-10-19T07:31:08Z level=error logger=lotus.tcp msg="accept failed" op="lotus_tcp_accept_one: accept" errno=9 error="Bad file descriptor"
```

---

## Per-Component Loggers and Sinks

### Hierarchical Logger Names

Each `Logger`'s `name` determines its bus topic (`log.app`, `log.db`, `log.http`). Multiple components can each hold their own named `Logger` while a single sink captures all of them:

```hale
fn main() {
    let sink = std::log::StdoutSink { };

    let app_log = std::log::Logger { name: "app" };
    let db_log  = std::log::Logger { name: "db" };

    app_log.info("ready");
    db_log.warn("slow query");
}
```

Because logging uses the [typed message bus](../services/bus.md), emitters and sinks never hold references to each other. Custom sinks can subscribe to specific topic subtrees (such as `log.db.**` for database events or `log.**` for all logs).

### `FileSink` and `ConsoleSink`

Swap or combine sinks at the top of `main` without changing any `Logger` calls:

```hale,fragment
std::log::FileSink { path: "logs/app.log" };   // append + size-based rotation
std::log::ConsoleSink { };                      // colored terminal output
```

- **`FileSink`**: Appends all severity levels to `path` and rotates files by size (`app.log` -> `app.log.1` -> ... up to `keep_files`, evicting the oldest via atomic renames). File I/O errors never abort the program; inspect the most recent I/O failure via `last_error_kind()`, `last_error_errno()`, and `last_error_path()`.
- **`ConsoleSink`**: Formats human-readable lines (`14:02:07 WARN  app.db retry 1/3`) with ANSI color badges, routing `WARN` and `ERROR` to `stderr`. Colors are automatically disabled when output is not a TTY or when `NO_COLOR` is set.

---

## Exporting Runtime Lifecycle Events (`std::observe::Otlp`)

While `std::log` records application-level events, `std::observe::Otlp` exports runtime locus lifecycle transitions—every `birth`, `dissolve`, `restart`, and `failure`—to an OpenTelemetry collector as OTLP/HTTP JSON log records:

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

- **Environment configuration**: If `endpoint` is omitted, `Otlp` reads `OTEL_EXPORTER_OTLP_LOGS_ENDPOINT` (or `OTEL_EXPORTER_OTLP_ENDPOINT` + `/v1/logs`), along with `OTEL_SERVICE_NAME` and `OTEL_RESOURCE_ATTRIBUTES`.
- **Attribution & closure violations**: Closure violations are exported as `WARN` records naming both the failing locus and the violated closure. Every exported batch includes the binary's build model hash in its resource attributes.
- **Non-blocking delivery**: `Otlp` runs on its own placement (`pinned`) and bounds each HTTP export by `timeout_ms` (`2000` ms default). If the collector is slow or unreachable, batches are dropped rather than stalling application pools, and dropped records are counted in `hale_otlp_dropped_total` on the Prometheus scrape page. Programs that do not declare an `Otlp` param allocate no lifecycle event ring.

For a complete overview of runtime observability backends, see [Observing a program](./observing.md) and [Metrics](./metrics.md).

Next: [The lifecycle](../services/lifecycle.md).
