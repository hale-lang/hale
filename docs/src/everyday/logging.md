# Logging

> **Coming from Python / Node?** Instead of a global `logger`
> object you configure at import time, Hale logging is built on
> the message bus: a `Logger` *publishes* typed log events, and a
> *sink* subscribes to them and decides what to do (print, write
> a file, ship to a collector). It's your first look at the bus —
> the mechanism the whole next tier is built on.

## The minimal setup

Two pieces: something that emits log events, and something that
consumes them.

```hale
fn main() {
    // The sink must exist before anything logs to it.
    let sink = std::log::StdoutSink { };

    let log = std::log::Logger { name: "app" };
    log.info("starting up");
    log.warn("disk almost full");
    log.error("connection refused");
}
```

`StdoutSink` subscribes to all log events and prints them, one
**record** per event on stdout;
`Logger` emits them. The ordering matters — instantiate the sink
*first*, because a subscriber has to exist before a publisher
sends, or the early events have nowhere to go.

## Levels

Loggers carry the usual levels: `trace`, `debug`, `info`,
`warn`, `error`. Call the matching method:

```hale,fragment
log.debug(f"cache size = {n}");
log.error(f"request {id} failed: {reason}");
```

### Turning the volume down

`HALE_LOG` sets a threshold — `error`, `warn`, `info`, or
`debug` — and anything below it is dropped (unset, or a value it
doesn't recognise, drops nothing):

```sh
HALE_LOG=warn ./myapp      # warn and error only
```

The filtering happens at the **logger**, not at the sink, so a
`log.trace(...)` below the threshold publishes nothing at all: no
payload, no fanout, one integer compare. Pin a level in code with
`std::log::Logger { name: "app", min_severity: 3 }` to ignore the
environment (0 = trace, 1 = debug, 2 = info, 3 = warn, 4 = error).

Sinks accept the same `min_severity`, which is what you want when
two sinks should see different amounts — a file at `debug` and a
console at `warn`.

## Structured fields

A message is for a human; **fields** are for whatever reads the
logs afterwards. The `_kv` variant of each level takes them:

```hale,fragment
log.info_kv("order settled", std::log::kv("order", id));

log.warn_kv(
    "retrying",
    std::log::kv("attempt", to_string(n)) + " "
        + std::log::kv("after", "2s")
);
// → time=2026-10-19T06:56:00Z level=warn logger=app msg=retrying attempt=2 after=2s
```

`std::log::kv(key, value)` renders one `key=value` pair in
[logfmt](https://brandur.org/logfmt), quoting the value when it
contains a space, a quote, an `=`, a backslash or a control
character. Join pairs with a space.

Fields ride on the event as text rather than as a map, because a
map in Hale is a *locus* and a locus cannot be a payload. The
practical consequence is a good one: the record stays flat, so it
crosses every transport the bus supports, and the fields keep the
order you wrote them in.

Every event also carries `ts` — unix seconds, stamped where the
event was **published**, not where it was rendered. Under a
queued sink, a sink bridged to another process, or `hale replay`,
those are different times, and the one worth printing is the
event's.

## Records: the format a sink writes

Every line a `StdoutSink` or a `FileSink` writes is a **record**: the
event's time (RFC 3339, UTC), its level, the logger's path, the
message and the fields. By default that is one logfmt line, every
level on stdout, so a platform's log collector reads them as data:

```text
time=2026-10-19T06:56:00Z level=info logger=app msg="starting up"
time=2026-10-19T06:56:00Z level=warn logger=app.db msg=retrying attempt=2 after=2s
```

`HALE_LOG_FORMAT` picks another:

```sh
HALE_LOG_FORMAT=json ./myapp   # {"time":"…","level":"warn","logger":"app.db","msg":"retrying","attempt":"2","after":"2s"}
HALE_LOG_FORMAT=text ./myapp   # [WARN app.db] retrying attempt=2 after=2s  (warn and error on stderr)
```

The runtime writes its own diagnostics the same way, on stderr: a
socket that failed, a TLS handshake, a drain that ran out of time each
come out as a record whose `logger` names the part (`lotus.tcp`,
`lotus.tls`, `lotus.drain`, …), in the format `HALE_LOG_FORMAT` names,
so one collector reads the program's logs and the runtime's alike:

```text
time=2026-10-19T07:31:08Z level=error logger=lotus.tcp msg="accept failed" op="lotus_tcp_accept_one: accept" errno=9 error="Bad file descriptor"
```

`text` is the format before records, kept for a terminal you read
yourself (`ConsoleSink` is the colored one). A sink's own `format:`
param (`"logfmt"`, `"json"`, `"text"`) pins it in code. An unknown
name is logfmt, so a typo never turns records back into free text. A
message or a value that holds a space, a quote, an `=`, a
backslash or a control character is quoted, its line breaks, tabs and
other control characters escaped (`\n`, `\t`, `\u0001`), so a record
is always one line that every logfmt reader splits the same way. (Before October 2026 the default was `text`.)

## Per-component loggers, one sink

Each `Logger` has a `name`, which becomes the event's topic
(`log.app`, `log.db`, `log.http`). You can give every component
its own named logger and still have a single sink see everything:

```hale
fn main() {
    let sink = std::log::StdoutSink { };

    let app_log = std::log::Logger { name: "app" };
    let db_log  = std::log::Logger { name: "db" };

    app_log.info("ready");
    db_log.warn("slow query");
}
```

A custom sink subscribes to a *subtree* — `log.db.**` to capture
only database logs, `log.**` to capture all of them — without the
loggers knowing who's listening. Publisher and subscriber never
reference each other; they only share the topic name.

## Files and pretty consoles

`StdoutSink` is the minimal sink; two richer drop-ins subscribe to
the same `log.**`, so swapping is a one-line change at the wiring
site:

```hale,fragment
std::log::FileSink { path: "logs/app.log" };       // append + rotate
std::log::ConsoleSink { };                          // colored badges
```

`FileSink` appends every event and rotates by size — `app.log` →
`app.log.1` → … up to `keep_files`, oldest evicted, all atomic
renames. I/O failures never crash your program; they land in the
sink's `last_error_kind()` / `last_error_errno()` /
`last_error_path()` accessors. `ConsoleSink` renders
`14:02:07 WARN  app.db retry 1/3` with colored level badges —
automatically disabled when output isn't a terminal (and `NO_COLOR`
always wins). `ConsoleSink`, like `StdoutSink`, sends WARN/ERROR to
stderr so shell pipelines and CI capture keep the signal lane
separate; `FileSink` writes every level to its file. Run several sinks at once
— they're all just subscribers.

## You just used the bus

That decoupling — emitters publish, sinks subscribe, neither
holds a reference to the other — is the **bus**, Hale's typed
publish/subscribe channel. Logging is a small, friendly instance
of it: `Logger` publishes a `LogEvent` on a topic, `StdoutSink`
subscribes. The same mechanism carries any typed message between
any two loci in your program.

At this level you've used the bus without declaring one. The
[services tier](../services/bus.md) makes it first-class: you
declare your own `topic`s, `subscribe` and `publish` them in a
locus's `bus { }` block, and use them to wire concurrent
components together. Everything you just saw — emit, subscribe to
a subtree, no direct references — is exactly how it works at
scale.

## The runtime's events, to a collector

Your logs say what your code did. The runtime can also tell a
collector what happened to the program's loci — every birth, dissolve,
restart and failure — as OpenTelemetry log records:

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

Leave `endpoint` out and the standard variables fill it in
(`OTEL_EXPORTER_OTLP_LOGS_ENDPOINT`, or `OTEL_EXPORTER_OTLP_ENDPOINT`
plus `/v1/logs`), along with `OTEL_SERVICE_NAME` and
`OTEL_RESOURCE_ATTRIBUTES` — the ones your platform already sets. A
failure arrives as a `WARN` record naming the locus and the closure that
broke; every export carries the build's model hash, so a record can be
tied back to the exact program that wrote it.

Like a scrape, the backend is a param of the main locus: one per
program, placed by `placement` (`pinned` gives it a thread of its own),
and it never makes the program wait for the collector. Each send has a deadline (`timeout_ms`, two
seconds unless you say otherwise); a batch the collector doesn't take
in time is dropped and counted, and when you also hold a
scrape, that count is `hale_otlp_dropped_total` on the scrape page. A
program that holds no `Otlp` keeps no event ring at all.

For all the backends in one place, see [Observing a program](./observing.md).

---

That's the everyday tier. With loci, collections, files, JSON,
HTTP, config, and logging, you can build real applications —
CLIs, web services, data tools. The next tier is for programs
that *run over time and coordinate*: long-lived services, a typed
bus you design, concurrency, and supervision.

Next: [The lifecycle](../services/lifecycle.md).
