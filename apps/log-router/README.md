# log-router

A small Aperio program that demonstrates `std::log` with a
custom file-writing sink subscribing to a sub-tree pattern,
running alongside the default `std::log::StdoutSink`.

## What it does

1. Instantiates two sinks:
   - `std::log::StdoutSink` — subscribes to `log.**` and prints
     every event as `[LEVEL path] msg`.
   - `DbAuditSinkL` (defined here) — subscribes to
     `log.app.db.**` only, buffers each matching `LogEvent`
     into `self.buf`, and writes them to a file in its
     `dissolve()` lifecycle method.
2. Wires three `Logger`s in a tree:
   - `app`           (publishes on `log.app`)
   - `app.db`        (publishes on `log.app.db`)
   - `app.api`       (publishes on `log.app.api`)
3. Emits five events from those Loggers. Stdout shows all five;
   the audit file shows only the two `app.db` events.

This exercises:

- Custom sink locus with a typed bus subscription.
- Sub-tree wildcard subscription (`log.app.db.**`) — m94 +
  m95 working in concert.
- File I/O composed with `std::log` (audit file written via
  `std::io::fs::write_file`).
- Locus-state buffering across bus-handler calls plus a single
  flush at `dissolve()`.

## Run

```
target/debug/aperio build apps/log-router/main.ap
./apps/log-router/main                     # writes ./db-audit.log
./apps/log-router/main /tmp/my-audit.log   # custom path via argv
```

`aperio run` is not used: the interpreter does not yet support
qualified-name struct/locus literals like `std::log::Logger { }`
(per the m95 test-fixture commentary), so `aperio build` plus
the produced native binary is the only way to exercise this
program.

## Sample stdout

```
[INFO app] starting
[INFO app.db] connected
[WARN app.api] upstream slow
[ERROR app.db] query failed
[INFO app] shutting down
log-router: wrote audit log to db-audit.log
```

## Sample db-audit.log

```
INFO app.db connected
ERROR app.db query failed
```

The `app` and `app.api` events are *not* present in the file —
the `log.app.db.**` pattern excludes them. The pattern matches
`log.app.db` itself (zero trailing segments) and any descendant
(`log.app.db.query`, `log.app.db.cache`, ...) but not `log.app`
(parent) or `log.app.api` (peer).

## What it does not do

- No log-level filtering at the sink. `DbAuditSinkL` writes every
  level it sees; if you only wanted ERROR-and-above you would
  filter inside `on_db` with `if e.level >= 3`.
- No append semantics. `std::io::fs::write_file` truncates, so
  the sink buffers in `self.buf` and flushes once at dissolve.
  A long-running sink that needs incremental writes is blocked
  on a `write_file_append` (or open/write/close) primitive.
- No backpressure or rotation. Buffer grows unbounded for the
  lifetime of the sink.
- No structured fields beyond `level`, `path`, `msg`. The
  `LogEvent` payload is fixed in `std::log`.

## Friction

See `FRICTION.md` for the per-app friction log. One entry —
about a stale `aperio` CLI binary silently producing a working
build that drops user-defined bus subscriptions — is the only
real surprise this app hit.
