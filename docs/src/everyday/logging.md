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

`StdoutSink` subscribes to all log events and prints them;
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
`debug` — and anything below it is dropped:

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
// → [WARN app] retrying attempt=2 after=2s
```

`std::log::kv(key, value)` renders one `key=value` pair in
[logfmt](https://brandur.org/logfmt), quoting the value when it
contains a space, a quote or an `=`. Join pairs with a space.

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
always wins). Both send WARN/ERROR to stderr so shell pipelines and
CI capture keep the signal lane separate. Run several sinks at once
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

---

That's the everyday tier. With loci, collections, files, JSON,
HTTP, config, and logging, you can build real applications —
CLIs, web services, data tools. The next tier is for programs
that *run over time and coordinate*: long-lived services, a typed
bus you design, concurrency, and supervision.

Next: [The lifecycle](../services/lifecycle.md).
