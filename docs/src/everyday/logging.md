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

```hale
log.debug(f"cache size = {n}");
log.error(f"request {id} failed: {reason}");
```

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
