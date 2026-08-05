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

## Order matters: a blocking `run()` stops later siblings

Children are born **in declaration order**, one at a time, and a
child's `run()` runs *right there* — on the same thread, before the
next sibling is touched. So a child whose `run()` never returns
means the params after it are never born.

```hale
main locus App {
    params {
        server:  Server  = Server { };    // run() { while true { ... } }
        metrics: Metrics = Metrics { };   // never born
    }
}
```

`Metrics` doesn't just fail to *run* — its `birth()` never happens,
so whatever it would have subscribed to or opened simply doesn't
exist. Nothing errors. The program boots and goes quiet, and the
symptom ("my metrics handler never fires") points at the bus rather
than at the params block.

Two fixes. Declare the never-returning child **last**:

```hale,fragment
params {
    metrics: Metrics = Metrics { };
    server:  Server  = Server { };       // last
}
```

Or give it a thread of its own, which frees the main thread to
finish births:

```hale,fragment
params {
    server:  Server  = Server { };
    metrics: Metrics = Metrics { };
}
placement { server: pinned; }            // or cooperative(pool = io)
```

Only the *blocking* child's placement matters. Pinning the later
one doesn't help — the construction itself happens on the main
thread regardless.

The compiler warns when it can prove the shape (a `while` loop with
no `break`, on a child that runs on the main thread, with something
declared after it). It can't prove every loop, so treat the absence
of a warning as "not detected", not "correct".

And this is why work that needs its siblings to exist belongs in
the **main locus's** `run()`, not a child's — main's `run()` starts
only once every child has been born.

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

**The scope is the enclosing function, not the enclosing block.** A
`let` inside a loop body therefore doesn't dissolve per iteration — the
whole run accumulates and releases at once when the function returns:

```hale,fragment
while i < steps {
    let m = zeros(rows, cols);   // a fresh arena every iteration…
    i = i + 1;
}                                // …none of them reclaimed yet
```

That's fine for a bounded loop and a real problem for a long-running
one. Both spellings behave identically here — a factory call allocates
just as a `Matrix { }` literal would — and the compiler warns about
each of them. The fixes are to hoist one instance out of the loop and
refill it, or, if the value is only passed straight on, to drop the
binding: an unbound result is reclaimed at the end of its statement.

### Replacing a locus held in a field

If a locus holds another locus in a field — say a server that
keeps its current connection in `self.conn` — assigning a fresh
one **replaces a live thing**, so it's a lifecycle event, not a
plain store:

```hale,fragment
self.conn = Connection { url: next };   // reconnect
```

Hale tears the old `self.conn` down first (drain → dissolve, so
its socket and any children are released), *then* builds the new
one into this locus's arena and points the field at it. The old
and new never overlap, and the new instance lives until the
parent dissolves — no manual close, no leak. This is
**break-before-make**: if you need make-before-break (hold the old
connection open while the new one warms up), keep both in
separate fields and swap explicitly.

To *reconfigure the same instance* instead of replacing it, mutate
in place — `self.conn.url = next;` — which keeps the connection
and triggers no teardown.

One rule comes with this: the right-hand side has to be a **literal**.

```hale,fragment
self.conn = Connection { url: next };   // fine — built in place
self.conn = make_connection(next);      // error
```

The second line looks reasonable and is rejected on purpose. Two
things would claim that connection — the field, which tears it down
when this locus dissolves, and the function that built it, which
tears down what it made. The language has no way to pick, so it asks
you to. Build it here with a literal, or if the thing genuinely
belongs to you as a child rather than a field, take it through
[`accept`](./parents-children.md).

This is the same idea as a method not being allowed to *return* a
locus: a locus is structure, not a value you pass around. Plain
`let`-bound loci are unaffected — a factory result you bind and use
locally is owned by that binding, and that's fine.

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
