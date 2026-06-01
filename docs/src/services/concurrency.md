# Concurrency & placement

> **Coming from Go?** Concurrency isn't `go f()` scattered through
> the code. Loci run concurrently by default; *where* each one
> runs — a shared cooperative pool (like a scheduler's worker) or
> its own dedicated OS thread — is declared in one place, the
> `placement { }` block on `main`. It's a deployment decision, not
> something baked into the locus. And there's no `async`/`await`:
> the lifecycle and the bus already give you what coloring
> functions would.

## Two ways a locus can run

Hale's concurrency is deliberately **bimodal** — two choices, no
third:

- **Cooperative** — the locus shares an OS thread with other
  cooperative loci on the same *pool*. It yields between units of
  work (after a handler, on a bus dispatch, on `time::sleep`, on
  an explicit `yield`). Handler bodies run to completion without
  interruption, so within one cooperative locus there's no
  data race to worry about. This is the default.
- **Pinned** — the locus owns its own OS thread and doesn't yield
  to neighbors. For latency-critical or CPU-bound work that
  shouldn't share.

## Placement lives on `main`

You declare placement once, against the top-level loci, in
`main`:

```hale
main locus App {
    params {
        gateway: Gateway       = Gateway { };
        metrics: MetricsServer = MetricsServer { port: 9100 };
        ui:      Renderer      = Renderer { };
    }
    placement {
        gateway: pinned(core = 1);          // own thread, pinned to core 1
        metrics: cooperative(pool = io);    // shares the "io" pool
        ui:      cooperative(pool = render);
        // anything unlisted defaults to cooperative(pool = main)
    }
}
```

- `cooperative(pool = X)` puts the locus on pool `X`'s thread.
  The runtime spawns one OS worker per pool name it sees.
- `pinned` / `pinned(core = N)` gives the locus its own thread,
  optionally pinned to a CPU core.
- Unmentioned top-level loci default to `cooperative(pool =
  main)` — the program's main thread.

Placement keys on the *field name*, not the locus type, so two
instances of the same locus type can live on different threads —
the parallelism case (one gateway per core, say).

Why on `main` and not on the locus? Because where something runs
is a property of the *deployment*, not the code. The same
`Gateway` locus is pinned in production and cooperative in a
test, with no edit to `Gateway` itself. Library authors say what
a locus *is*; the binary author says *where it runs*.

## Nested loci inherit their pool

Placement entries apply only to top-level `main` loci. A locus
instantiated inside another locus's body runs on its parent's
pool. To put a component on its own pool, hoist it to a top-level
sibling in `main` and give it a placement entry. (This is the
canonical fix for "my long-running child starved its parent" —
make it a sibling, not a nested child.)

## The bus crosses threads for you

When a cooperative locus on one pool publishes to a subscriber on
another pool — or to a pinned locus on its own thread — the
runtime handles the hand-off: it copies the payload across the
thread boundary and wakes the destination. The sender never
blocks. From your code's point of view, `Topic <- value;` is the
same line whether the subscriber is on the same thread or a
different one. The substrate adapts; the source doesn't.

## High-concurrency I/O: `where async_io`

A single pinned thread handles one blocking connection at a time.
To serve *many* concurrent connections on one thread without a
thread-per-connection explosion, tag a cooperative pool with
`where async_io`:

```hale
placement {
    workers: cooperative(pool = ws) where async_io;
}
```

The pool's worker runs an event loop (epoll under the hood), and
blocking I/O calls inside loci on that pool — `recv`, `accept`,
`send` — *park and resume* instead of holding the thread. Your
locus code stays synchronous-shaped: `stream.recv(4096)` is the
same call either way; the substrate picks the parking lowering at
the syscall boundary. This is how you get async-style throughput
without async-style function coloring.

Next: how loci nest and own each other — [Parents &
children](./parents-children.md).
