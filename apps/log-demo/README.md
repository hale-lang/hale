# log-demo

Bus-based structured logging in Aperio, end-to-end on currently
shipped primitives.

> **Historical note.** This demo was the **first dogfood test**
> of the agent-onboarding brief at
> `notes/agent-onboarding/app-dev-brief.md`. The friction it
> generated drove the m94 (bus subject wildcards) and m95
> (`std::log`) milestones the same session. **The pattern this
> file shows is now provided by the stdlib** —
> `std::log::Logger`, `std::log::LogEvent`,
> `std::log::StdoutSink` give you the same shape with cascading
> namespaces and sub-tree subscribers. New apps should reach
> for `std::log` directly (see `docs/std/src/log.md`); this
> file is kept as the "before" artifact and a regression
> baseline.

## What it does

Wires two loci through a single bus subject, `log.event`, of
type `LogEvent { level: Int; msg: String }`:

- `GreeterServiceL` is a fake service. During `birth()` it
  publishes five events on `log.event` (a mix of INFO, WARN,
  ERROR levels) modelling a service that emits log messages
  during its lifecycle.
- `StdoutSinkL` subscribes to `log.event`. For each event it
  receives, it prints `[LEVEL] msg` to stdout. The level prefix
  is computed by an `if`-chain over `e.level` since variant
  payload pattern-matching is not shipped.

`main()` instantiates the sink first, then the service, so the
subscription is registered before the publisher's `birth()`
fires (the v0 ordering rule documented in
`examples/05-bus/main.ap`).

## How to run

From the repo root:

```
cargo run -p aperio-cli --bin aperio -- run apps/log-demo/main.ap
```

or, if the binary is already built:

```
./target/debug/aperio run apps/log-demo/main.ap
```

Expected output:

```
[INFO] starting
[INFO] did the thing
[WARN] things look slow
[ERROR] imaginary failure
[INFO] shutting down
```

## What it doesn't do yet

- **No enum levels.** `level` is an `Int` (1 = INFO, 2 = WARN,
  3 = ERROR) because variant-payload pattern-matching on enums
  is not shipped (counter-hallucination row: "match patterns on
  enum variants"). Switching to `enum Level { Info, Warn, Error }`
  is a one-line edit at the type and three-line edit at the
  sink once that lands.
- **No timestamp / source / fields map.** A real structured
  logger would carry a timestamp and arbitrary key/value
  metadata. `Time` is a primitive, but `std::time::monotonic`
  yields a `Time` and a generic key/value map type does not
  exist (`Map<K, V>` is blocked). Until then the payload is
  just `level + msg`.
- **No sink filtering / level threshold.** The sink prints
  everything; there is no "only print >= WARN" mode. Trivial to
  add as an `if e.level >= threshold` once a config-param
  pattern is in scope.
- **No multi-process bus.** The bus is in-memory in v0; the
  TCP transport exists in the runtime (`lotus_tcp_*`) but no
  language-level wire-up appears in the example corpus. A
  future variant could publish across processes.
- **No drain ordering for in-flight messages.** Because
  `birth()` does the publishing synchronously and `main()`
  exits immediately after the second instantiation, all events
  flush before process exit. A locus that published in `run()`
  with `time::sleep` between events would expose drain ordering
  questions this app sidesteps.

## Friction

See `FRICTION.md`. One entry logged: a documentation gap in
the example corpus where `examples/01-locus-with-run/main.ap`
(and a handful of others) use `import "std/time";`, a syntax
the brief and the modern bus example explicitly say doesn't
exist. It didn't bite this app, but a cold-start agent reading
example 01 before the brief would absorb a syntax that
contradicts the brief's counter-hallucination rules.
