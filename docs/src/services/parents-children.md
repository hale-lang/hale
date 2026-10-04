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
type Player { name: String; }

locus GameSession {
    params { host: Player; tick: Int = 0; }
}

locus Room {
    accept(g: GameSession) {
        // runs once g's params are built, before g's birth().
        // It admits g; it cannot turn it away.
    }

    fn on_join(p: Player) {
        // instantiating a child inside a parent method attaches it
        GameSession { host: p };
    }
}
```

When `GameSession { ... }` is evaluated inside `Room`'s body, the
runtime allocates the child's region *inside* the parent's, builds
its params, runs `Room.accept(g)`, then births and runs it.
`accept` sees the child's params but not its running state, and
whatever it does, the child is admitted. The
parent's `self.children` holds its accepted children (with
`self.children.count` and `self.children.is_empty` for quick
summaries).

`children` is one of three names every locus already carries —
the others are `k_max` (the displacement bound) and `draining`
(the drain flag) — and `self.<name>` always means the built-in
one. So they're reserved: naming a params field, a method, or a
capacity slot `children` is an error at the declaration ("rename
it"), not a confusing type mismatch wherever you read it back.
A `type`'s struct field may still be called `children` — only
loci carry the synthetic members.

A locus accepts **one** child type. Writing a second `accept` is a
compile error that points at both clauses; if a parent needs to own
two kinds of children, one of them belongs under a different owner
(or the same owner one level down). The other direction is fine:
several parent types may accept the *same* child type, and each
child's `release` fires on the parent that actually accepted it,
running that parent's own `release` body.

## Bubbling: the nearest accepting ancestor collects the child

`accept` isn't limited to *direct* children. If you instantiate a
child where the enclosing locus doesn't accept its type, the child
doesn't become a detached throwaway — it **bubbles up to the
nearest ancestor that does** accept it.

```hale
locus Ship { params { hull: Int = 0; } }

locus World {
    accept(s: Ship) { }          // a top-level registry of ships
}

locus Fleet {
    fn spawn() {
        Ship { hull: 100 };      // Fleet doesn't accept Ship...
    }                            // ...so this Ship bubbles to World
}
```

`World` collects every `Ship` spawned anywhere beneath it — through
a `Fleet` that never mentions ships — with no manual registration.
It's the structural counterpart to the [bus](./bus.md): the bus
carries ephemeral *messages*; this carries ephemeral *ownership* —
a live collection the ancestor holds and cleans up.

A few rules keep it predictable:

- **Nearest wins.** If several ancestors accept the type, the
  innermost one gets the child. A direct parent that accepts it is
  the nearest of all — so nothing about ordinary parent/child
  attachment changes; bubbling only fills the gap where a child
  *had* no owner.
- **No owner is fine.** A child whose type no ancestor accepts is
  just a transient local — bubbling is opt-in via `accept`, and the
  absence of an owner is never an error.
- **Still vertical.** Bubbling travels *up* the tower to an
  ancestor; it never reaches sideways. The child's region still
  lives inside its owner's, so the whole "[flow is vertical
  only](#flow-is-vertical-only)" cleanup story holds — the owner is
  just possibly a grandparent, not always the direct parent.

When the owner lives on a **different thread** — a `main locus`
registry collecting entities that workers spawn on their own pools —
the child is created over on the owner's thread, so the spawning
side can't hold onto it. There a cross-pool spawn is
**fire-and-forget**: write it as a bare statement, not
`let s = Ship { ... }`. `hale check` points at the literal if you
try to keep the value in the spawning locus's own code. (A `Ship`
built in another locus's `params` default, which that locus's
instantiation carries across, is refused only by `hale build`, and
without a location.)

"A different thread" counts instances, not types. A worker nested
inside a pinned or pool-placed locus runs on that locus's thread,
so its spawns are cross-pool even though the worker itself has no
placement entry. And when one worker type has an instance on the
owner's thread and another off it, the same `Ship { ... };` line
does the right thing in each: born directly where it can be, handed
over where it can't. Keeping the value at such a line, or spawning
toward an owner that has several instances, is refused with a
message that lists where each instance runs.
### A subscriber born in a handler needs an owner

There is one place where "no owner is fine" stops being true. A bus
handler returns after every message, so a locus it creates and nobody
owns dissolves at that return — and if that locus subscribes to the
bus itself, its subscription can never fire for a later message. The
compiler refuses that shape:

```hale,refused
type Ping { n: Int = 0; }
topic Tick { payload: Ping; }

locus Watcher {
    bus { subscribe Tick as on_tick; }
    fn on_tick(t: Ping) { }
}

locus Hub {
    bus { subscribe Tick as on_msg; }
    fn on_msg(p: Ping) {
        Watcher { };   // error: instantiated unowned in a bus handler
    }
}

main locus App {
    params { h: Hub = Hub { }; }
    run() { }
}
```

Owning it fixes it: `accept(w: Watcher)` on `Hub`, or on any ancestor
of `Hub` — the nearest accepting ancestor collects it, exactly as
bubbling does anywhere else. What counts is the *declaration* the
`accept` names, not the spelling. `accept(w: W)` with `type W =
Watcher` owns it; an `accept(w: lib::Watcher)` that names another
seed's `Watcher` doesn't own yours, however alike the last segment
reads. For a generic subscriber the specialization must match:
`accept(c: Cell<Int>)` owns a birth declared `Cell<Int>`, not one
declared `Cell<String>`. If two loci share a name, the compiler judges
the first one declared and says which it judged.

The ancestor has to be there on *every* path that builds `Hub`. If
`App` accepts `Watcher` and holds a `Hub` as a field, but `fn main`
also builds a `Hub { }` of its own, that second `Hub` has no parent to
collect what it births — so the compiler still refuses the birth, and
its note points at the `Hub` that `main` builds. The same goes for a
`Hub` built somewhere the compiler can't follow: inside a plain
function, or in a locus whose placement it can't work out. An owner it
can't see is not an owner it can count on.

Where the compiler can't tell who owns the child, it stays quiet
rather than guess. A library seed with no entry point can't be judged,
because its consumer may supply the owner. Neither can a generic
subscriber whose specialization nothing declares. A subscriber
created in `run()`, `birth()` or an ordinary method is fine too: it
lives for that scope and hears what's published meanwhile. And if you
manage its lifetime some other way, `--allow-unowned-subscriber` lets
the shape through.

## The contract: what crosses the boundary

A child declares what its parent may see in a `contract`, and a
parent declares what it reads:

```hale
type SessionState = enum { Lobby, Playing, Over };

locus GameSession {
    params { tick: Int = 0; state: SessionState = SessionState::Lobby; }
    contract {
        expose tick: Int;          // the parent may read this
        expose state: SessionState;
    }
}

locus Room {
    contract { consume tick: Int; }        // what Room reads from a session
    accept(g: GameSession) {
        if g.tick > 1000 { /* ... */ }     // reading an exposed field
    }
}
```

`expose` is what the child offers its parent; `consume` is what
the parent reads from the child type it accepts. The compiler
checks the two against each other: every name a parent consumes
must be one its child type exposes, with the same type, and a
parent that consumes with no `accept` to bind against is an
error. The check is on the declarations: a parent's direct read
of a field the child did not expose (`g.secret`) still compiles
today, so treat the contract as the boundary you have declared
and checked, not as a wall around the rest.

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
garbage collector, no per-object bookkeeping. A parent that
accepts children tracks every one of them for exactly this moment,
whether or not it ever reads `self.children`: when the parent goes
— at shutdown, or because the field holding it was reassigned —
each accepted child is torn down first, its bus subscriptions
deregistered, before the memory it lived in is freed.

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
        while true {
            let chunk = stream.recv(4096) or "";
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
- Flow or resident is a **type-wide** fact. `release(c: T)`
  declared on *any* parent in the program — including one that is
  never instantiated, or one in an imported seed — makes every `T`
  a flow, whoever accepted it. Removing the clause from one owner
  does not make its children residents while another owner still
  declares it; if a child you meant to keep is reclaimed when its
  `run()` returns, `hale check --flows` lists every `release(c: T)`
  in the program, imported seeds included, with its file and line.
- What a handler hands a resident is copied or refused, never left
  dangling. The payload and anything the handler builds belong to
  that dispatch and are reclaimed when the handler returns. A
  String, a row or a payload field handed to the resident is copied
  into its own storage as it is stored. A container that is a locus
  (a `@form(vec)` the handler built) would be a borrow that dies
  with the dispatch, so `hale check` refuses it at the argument; the
  resident copies the rows into a `@form(vec)` of its own in
  `birth()` instead.
- A locus can also end *itself* early with **`terminate;`** —
  the locus analogue of `return`. It exits the method and lets
  the runtime tear the locus down. For a resident that is the
  handler-driven end (an `on_close` handler that `terminate`s):
  it drains and dissolves when the handler returns, and no
  `release` fires, since no parent releases its type. Otherwise a
  resident ends in its parent's dissolve cascade. Neither is a
  flow's completion.

The same "`run()` returned" event means "reclaim me" for a flow
and "I'm ready" for a resident — disambiguated by whether the
parent declared `release`, never guessed. If you accept a child
per connection and memory climbs with connection count, you have
a resident that should be a flow.

Next: what happens when a child breaks — [When things
fail](./failure.md).
