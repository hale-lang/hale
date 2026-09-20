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

- **Statement position** (`Ticker { };` — no binding, and the
  value is discarded): the locus runs its whole lifecycle right
  there and tears down at the end of the statement.
  Fire-and-forget.
- **`let`-bound** (`let t = Ticker { };`): it's born and runs,
  but **dissolve is deferred to the end of the enclosing
  function's scope**. The binding stays usable for method calls
  until then.
- **A literal you *use*** — as a method receiver
  (`Ticker { }.tick()`), as an argument (`serve(Ticker { })`),
  for a field read (`Ticker { }.every_ms`): the expression that
  consumes it is the handle, so it behaves exactly like the
  `let` form — born before the expression runs, dissolved at the
  end of the enclosing function. Writing it inline and naming it
  with `let` first are the same program, whichever position it
  is in. That is what lets you hand a fresh locus to a service
  that keeps it: `serve(Provider { })` is as safe as
  `let p = Provider { }; serve(p);`.
- **A locus a function hands back** (`let t = make_ticker();`, or
  the call used directly — `serve(make_ticker())`): whatever
  consumes the handle owns it, so it behaves like the two above.
  A **fallible** factory is reached through `or`, and that changes
  nothing: `let c = std::process::spawn(argv) or raise;` closes
  the child's pipes and reaps it when the scope exits, just as a
  literal would. The two places the frame stays out of it are the
  places the handle is *handed on* rather than consumed — written
  straight into another locus's field (`Router { quick: make("q") }`,
  which the router owns and reclaims with itself, exactly as it
  would a `Quick { }` written there) and `return`ed to your caller
  (who owns it).
- **Long-lived** (the locus subscribes to the bus, or its `run()`
  hasn't returned): it stays alive until its scope exits,
  regardless of binding — it has to, to keep receiving messages.

So `let` keeps a locus alive for the scope; statement position is
fire-and-forget. When several `let`-bound loci share a scope,
they dissolve in reverse order of creation (the later one, which
may depend on the earlier, goes first).

Whichever line you're on, the timing is the timing of the **whole
tree** the locus owns. A locus you write as another locus's param
field has no teardown moment of its own — it's the owner's, and so
is the one *it* holds, all the way down. That holds however you
wrote it: a nested literal (`Mid { leaf: Leaf { } }`), a factory
call (`Mid { leaf: make_leaf() }`, including `make_leaf() or
raise`), and a param whose **default** is one, are the same
program. When the
owner goes, every level's `drain()` has run (deepest first), every
level's `dissolve()` body has run (outermost first) and every
level's arena is gone. The exception is a handle you pass *in* —
`Mid { leaf: shared }` borrows `shared`, so the cascade steps over
it at whatever depth it sits, and `shared` is released once, by the
scope that made it. A call that hands back a locus it didn't build
is the same borrow, written as a call.

**How the field is declared doesn't change the answer.** A param
typed by an `interface` the child satisfies, or by a
`perspective(P)` it serves, holds an owned child on the same terms
as a locus-typed param — the cascade reaches it and everything
under it:

```hale,fragment
locus Queries {
    params { j: Counter = Churner { }; }   // an interface slot
}
locus Gateway {
    params { router: perspective(Router) = RouterV1 { }; }
}
```

Both children go when their holder goes, and so does whatever they
hold. Designating a different impl at the literal (`Gateway { router:
RouterV2 { } }`) reclaims the one you actually built.

One shape that is *not* a transfer: a locus written inside the
initializer of a param that can't hold a locus. In `Lonely { n:
Queries { }.total() }` the `Queries` is just an expression — it has
no field to live in — so it belongs to the enclosing function's
scope, exactly as if you had written it on a line of its own.

**The scope is the enclosing function, not the enclosing block** — a
`let` is readable for the rest of the function, including after the
loop that bound it. But a locus created **in a loop** is reclaimed
when the next iteration reaches the same line:

```hale,fragment
while i < steps {
    let m = zeros(rows, cols);   // a fresh arena every iteration…
    i = i + 1;
}                                // …each one released as the next
                                 //    replaces it; the last at return
```

Coming back round to that line is the end of the previous `m`'s life:
it runs its `drain()` and `dissolve()` and gives back its arena before
the new one takes its place, and the function's exit releases the last
one — so `m` is still readable after the loop. The loop holds one
instance at a time, not `steps` of them. Both spellings behave
identically — a factory call, a `Matrix { }` literal and a literal you
merely *use* (`Matrix { }.trace()`, `sum(Matrix { })`) all get the
same boundary.

Each line reclaims its own previous instance, where it stands, so two
loci bound in one iteration are released in the order they were
written — the reverse of the newest-first order at function exit. They
are independent either way; it only shows if one's `dissolve()` reads
a handle it borrowed from the other, which would see the next
iteration's instance.

The compiler still warns about the shape, because a fresh arena per
iteration is real work in a hot loop even when it is reclaimed. To
spend nothing, hoist one instance out of the loop and refill it.

Dropping the binding changes *where* the reclaim lands, not whether
one happens: a bare `Matrix { };` statement is reclaimed at the
statement, while `Matrix { }.trace()` and `sum(Matrix { })` both *use*
the literal, so each lives to the top of the next iteration — and the
last of them to the end of the function — exactly as the binding did.

### Early `return` is an exit, not a shortcut

"The end of the enclosing function" means *whichever* way the
function ends. A guard that returns early tears down everything
alive at that point, and it takes nothing away from the ordinary
exit:

```hale,fragment
let store = Store { path: dir };
if std::env::args_count() < 2 {
    println("usage: report <name>");
    return 2;                     // dissolves `store`, exits 2
}
let report = Report { store: store };
                                  // ordinary exit: `report`, then `store`
```

Both endings are complete. The guarded one dissolves `store`; the
one taken when the guard does *not* fire dissolves `report` and
then `store`, newest first as always. A locus bound only inside
the branch that returns dissolves on that branch alone — the other
endings never built it, so there is nothing for them to release.

This holds in `fn main` too, which matters because `main` is where
usage checks and flag guards live.

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
