# Parents & children

> **Coming from Go?** This is structured concurrency — closer to
> an `errgroup` or a supervised tree than to bare goroutines. A
> parent locus *accepts* child loci; the children live inside the
> parent's scope, the parent sees their progress through a typed
> contract, and when the parent shuts down its children shut down
> first. No detached goroutine outliving the thing that spawned
> it.

## A parent accepts children

A locus declares it can parent a child type by implementing
`accept`:

```hale
locus GameSession {
    params { players: [Player]; tick: Int = 0; }
}

locus Room {
    accept(g: GameSession) {
        // runs before g's region is allocated — the gatekeeper.
        // return normally to admit; route through on_failure to reject.
    }

    fn on_join(p: Player) {
        // instantiating a child inside a parent method attaches it
        GameSession { players: [p] };
    }
}
```

When `GameSession { ... }` is evaluated inside `Room`'s body, the
runtime runs `Room.accept(g)` first, then allocates the child's
region *inside* the parent's, then births and runs it. The
parent's `self.children` holds its accepted children (with
`self.children.count` and `self.children.is_empty` for quick
summaries).

## The contract: what crosses the boundary

A child decides what its parent may see by declaring a
`contract`:

```hale
locus GameSession {
    params { tick: Int = 0; state: SessionState; }
    contract {
        expose tick: Int;          // parent may read this
        expose state: SessionState;
        consume clock: Time;       // parent must provide this
    }
}

locus Room {
    contract { consume clock: Time; }
    accept(g: GameSession) {
        if g.tick > 1000 { /* ... */ }     // reading an exposed field
    }
}
```

`expose` is what the child lets the parent read; `consume` is
what the child needs the parent to provide. Anything not in the
contract is invisible across the boundary — the compiler rejects
reads of un-exposed fields. You don't write hiding logic; the
structural boundary does it.

## Flow is vertical only

The rule the whole tower rests on: **a locus talks up to its
parent and down to its children — never sideways.** Two sibling
sessions don't reference each other; if they need to coordinate,
they route through their shared parent (the `Room` is exactly the
place that should know how sessions relate), or over the
[bus](./bus.md). No sibling pointer, no cousin back-channel.

This is what makes cleanup sound: a child's memory is a
sub-region of its parent's, no pointer ever crosses sideways, so
when a locus dissolves its whole subtree frees wholesale — no
garbage collector, no per-object bookkeeping.

## Flow children vs residents

Here's the piece that matters for any long-running parent — a
server that accepts one child per connection. By default an
accepted child lives until its *parent* dissolves. For a daemon
whose parent never dissolves, that means per-connection children
pile up forever. Two shapes fix it:

```hale
locus Conn {
    params { conn_fd: Int = -1; }
    run() {
        let stream = std::io::tcp::Stream { conn_fd: self.conn_fd, owns_fd: false };
        loop {
            let chunk = stream.recv(4096);
            if len(chunk) == 0 { return; }   // client closed → run() ends
            // ... handle chunk
        }
    }
}

locus Server {
    accept(c: Conn)  { }
    release(c: Conn) { }   // ← declaring release marks Conn a *flow*
}
```

- Declaring **`release(c: Conn)`** on the parent marks `Conn` a
  **flow**: its `run()` *is* its lifetime. When `run()` returns
  (the recv loop ends on close), the runtime reclaims the child
  right then — drains it, calls the parent's `release` for a
  final look, dissolves it, frees its region — while the server
  keeps running. The connection's memory ends with the
  connection.
- A child no parent `release`s is a **resident**: its `run()`
  returning means "ready," and it lives until the parent
  dissolves. That's the right shape for a fixed cohort of
  long-lived workers spun up at boot.
- A locus can also end *itself* early with **`terminate;`** —
  the locus analogue of `return`. It exits the method and lets
  the runtime tear the locus down.

The same "`run()` returned" event means "reclaim me" for a flow
and "I'm ready" for a resident — disambiguated by whether the
parent declared `release`, never guessed. If you accept a child
per connection and memory climbs with connection count, you have
a resident that should be a flow.

Next: what happens when a child breaks — [When things
fail](./failure.md).
