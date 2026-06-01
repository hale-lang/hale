# The lifecycle

> **Coming from Go?** A long-running locus is like a goroutine
> with structure: instead of `go func(){...}()` and a `context`
> you thread around for cancellation, a locus has named lifecycle
> methods the runtime drives — `birth → run → drain → dissolve` —
> and shutdown cascades through the tree automatically. You write
> the phases; the runtime sequences them.

Until now, loci have been object-like: state plus methods you
call. A locus can also *run over time*. When it does, it moves
through a fixed sequence of lifecycle states, and the runtime
guarantees the ordering.

## The five phases

```hale
locus Server {
    params { listen_fd: Int = -1; }

    birth()    { /* acquire: open sockets, files, buffers */ }
    run()      { /* steady-state work — the main loop */ }
    drain()    { /* stop taking new work; finish in-flight */ }
    dissolve() { /* release what birth acquired */ }
}
```

- **`birth()`** runs once, at construction, after the locus's
  state is initialized. Acquire resources here — open a socket,
  read a file, allocate a buffer. By the time it returns, the
  locus is live.
- **`run()`** is the steady-state body — typically a loop that
  serves requests, drains a queue, or ticks on a timer. It runs
  until it returns on its own or the locus is asked to shut down.
- **`drain()`** runs when shutdown begins: stop accepting new
  work, let in-flight work finish.
- **`dissolve()`** runs last: release what `birth` acquired. The
  locus's memory is freed wholesale right after.

There's also **`accept`** and **`release`** for parent/child
relationships — those belong to [Parents &
children](./parents-children.md). And **`on_failure`** for
recovery — [When things fail](./failure.md).

You only write the phases you need; the compiler supplies no-op
defaults for the rest. A locus with just `birth` and `run` is
completely normal.

> One rule: no `return` inside `birth` / `run` / `dissolve`
> bodies. These are driven by the runtime, not called by you, so
> "return a value" has no meaning. Factor any early-exit logic
> into a helper free function the body calls.

## A simple service

```hale
locus Ticker {
    params { count: Int = 0; limit: Int = 5; }

    run() {
        while self.count < self.limit {
            println("tick ", self.count);
            std::time::sleep(500ms);
            self.count = self.count + 1;
        }
    }
}

fn main() {
    Ticker { limit: 3 };     // runs to completion, then tears down
}
```

## When does a locus dissolve?

This is the one piece of bookkeeping worth internalizing,
because it's how Hale frees resources without a `defer` or a
`finally`:

- **Statement position** (`Ticker { };` — no binding): the locus
  runs its whole lifecycle right there and tears down at the end
  of the statement. Fire-and-forget.
- **`let`-bound** (`let t = Ticker { };`): it's born and runs,
  but **dissolve is deferred to the end of the enclosing
  function's scope**. The binding stays usable for method calls
  until then.
- **Long-lived** (the locus subscribes to the bus, or its `run()`
  hasn't returned): it stays alive until its scope exits,
  regardless of binding — it has to, to keep receiving messages.

So `let` keeps a locus alive for the scope; statement position is
fire-and-forget. When several `let`-bound loci share a scope,
they dissolve in reverse order of creation (the later one, which
may depend on the earlier, goes first).

## Shutdown cascades

`drain()` is always **depth-first cascading**. Calling it on a
locus first drains all of its children (and theirs, recursively),
waits for them, then drains itself, then dissolves. You never
write a manual teardown walk.

This is what makes Ctrl-C trivial: SIGINT calls `drain()` on the
program's root, the whole tree winds down in dependency order,
in-flight work finishes, resources release, the process exits
cleanly. "Press Ctrl-C and it shuts down properly" is the
default, not something you wire up.

The lifecycle is the skeleton of every long-running Hale program.
Next, the thing those programs use to talk to each other: [The
bus](./bus.md).
