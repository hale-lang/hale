# Parents & Children

Hale organizes loci into a strict parent-child ownership hierarchy where every child's memory region is nested inside its parent's arena. Parents can hold static children in `params` or dynamically admit children at runtime via `accept(c: Child)`, inspect child state through `contract { expose; consume; }`, and automatically reclaim per-connection children upon completion via `release(c: Child)`.

```hale
type Player { name: String; }

locus GameSession {
    params { host: Player; tick: Int = 0; }
}

locus Room {
    accept(g: GameSession) {
        // runs after g's params are initialized, before g.birth()
    }

    fn on_join(p: Player) {
        GameSession { host: p };   // attaches to Room via accept()
    }
}
```

## Child Ownership Modes

| Child Kind | Declaration on Parent | Lifetime & Arena Reclamation | Typical Use Case |
|---|---|---|---|
| **Static child** | `params { db: Pool = Pool { }; }` | Constructed with parent; lives until replaced (`self.db = Pool { }`) or parent dissolves. | Fixed subsystem components, connection pools, co-located counters. |
| **Dynamic resident** | `accept(c: Worker) { }` *(no `release`)* | Constructed dynamically; stays alive after `c.run()` returns until `c` calls `terminate;` or parent dissolves. | Long-lived dynamic entities (rooms, actors, subscribed shards). |
| **Dynamic flow** | `accept(c: Conn) { }` + `release(c: Conn) { }` | `c.run()` *is* its lifetime; when `c.run()` returns, the runtime immediately drains, calls `release(c)`, dissolves `c`, and frees its arena. | Per-connection TCP/WebSocket handlers, short-lived request tasks. |

## Accepting Dynamic Children (`accept`)

A locus declares that it can parent dynamic instances of `Child` by implementing `accept(c: Child)`:

1. When `GameSession { host: p };` is evaluated inside `Room` (or a descendant of `Room`), the runtime allocates the child's arena inside `Room`'s arena and initializes `g`'s `params`.
2. The runtime invokes `Room.accept(g)` before `g.birth()` runs. `accept` can inspect `g`'s initialized `params` and update parent state, but cannot reject the child.
3. The runtime executes `g.birth()` and `g.run()`.

### `self.children` and Single-Clause Rule

- **Built-in `self.children` tracking**: Every locus exposes `self.children.count` and `self.children.is_empty` to query currently live accepted children.
- **Reserved locus identifiers**: `children`, `k_max` (the displacement bound), and `draining` (the shutdown flag) are synthetic fields on every locus. Naming a locus parameter, method, or capacity slot `children` is a compile error (plain data `type` structs may still declare a field named `children`).
- **One `accept` clause per parent**: A locus can declare at most one `accept` clause. Multiple parent locus types may declare `accept` for the same child type; each child instance's lifecycle hooks (`accept` and `release`) dispatch to the specific parent instance that admitted it.

### Polymorphic Acceptance via an Interface

To parent multiple child locus types under a single supervisor, declare `accept(j: Interface)` (and optionally `release(j: Interface)`) using an [interface](./perspectives.md#interface-or-perspective):

```hale
interface Job { fn weight() -> Int; }

locus Small { params { w: Int = 1; } fn weight() -> Int { return self.w; } }
locus Big   { params { w: Int = 9; } fn weight() -> Int { return self.w * 2; } }

locus Pool {
    params { load: Int = 0; }
    accept(j: Job) { self.load = self.load + j.weight(); }
    fn fill() {
        Small { w: 3 };   // accepted via Job
        Big { w: 5 };     // accepted via Job
    }
}
```

- **Compile-time specialization**: The compiler specializes `accept(j: Job)` for each concrete child type (`Small` and `Big`), turning `j.weight()` into a direct call.
- **Interface-only member access**: Inside `accept(j: Job)`, only methods declared on `Job` are accessible (`j.w` is a compile error even if every concrete child has a `w` field).
- **Collection queries**: `self.children.count` and `self.children.is_empty` count all accepted children across concrete types, whereas iterating with `for c in self.children` is rejected when `accept` takes an interface because the children do not share a single concrete type.

## Ancestor Bubbling

When a locus instantiates a child whose type the immediate enclosing locus does not `accept`, ownership **bubbles up the locus tower to the nearest ancestor that accepts it**:

```hale
locus Ship { params { hull: Int = 0; } }

locus World {
    accept(s: Ship) { }          // top-level owner of all Ship instances
}

locus Fleet {
    fn spawn() {
        Ship { hull: 100 };      // Fleet has no accept(s: Ship), so Ship bubbles up to World
    }
}
```

### Bubbling Rules

- **Nearest accepting ancestor wins**: If multiple ancestors accept `Ship` (either directly or via an interface `Ship` satisfies), the innermost ancestor takes ownership.
- **Transient local fallback**: If no ancestor in the enclosing tower accepts the type, the instance behaves as an unowned local scoped to the statement or function.
- **Typed generic defaults**: `params { cache: Cache<Int, String> = Cache { cap: 7 }; }` constructs the requested specialization as a fresh child. Its fields must match the specialized types, even when some type arguments do not appear in any field.
- **Shared and generic intermediaries**: Two owners can use the same producer type. Each producer retains its own nearest accepting owner, including the owner's concrete generic specialization. An instance created outside an accepting subtree keeps transient children. A local choice constructs on its owner's thread; a cross-pool choice can post to a program-lifetime singleton owner. If any choice posts, write the birth as a bare statement rather than using its value. Other unresolved or dynamic cross-pool alternatives remain refused.
- **Bubbling during parent `params` initialization**: If a parameter of `World` (such as `Fleet`) spawns `Ship` instances during its inline `run()` while `World` is still initializing its `params`, `World.accept(s)` runs normally because `World`'s state and `self.children` are initialized before its first parameter is built. Declare any parent parameter mutated by `accept` (such as a counter) *before* the parameter that spawns children so the counter's default initializer does not overwrite the updated value.
- **Cross-pool bubbling**: When a worker on one thread/pool spawns a child owned by an ancestor on a different thread/pool, the child is constructed on the owner's thread. Consequently, cross-pool spawns must be written as bare fire-and-forget statements (`Ship { hull: 100 };`). `hale check` rejects binding a cross-pool spawn to a local variable (`let s = Ship { ... };`), storing it via a `params` default (`s: Ship = Ship { ... }`), or constructing it in a default function/method argument (`fn take(s: Ship = Ship { ... })`), `const`, or `type` field default that executes off the owner's thread. If a worker locus has one instance on the owner's thread and another on a separate pool, bare `Ship { ... };` works in both; keeping the value or spawning toward an owner with multiple instances is rejected.

### Bus Subscribers Created in Handlers Require an Owner

Because a bus handler returns after processing each message, an unowned locus instantiated inside a bus handler dissolves immediately when the handler exits—meaning any bus subscriptions on that locus would never fire for future messages. `hale check` rejects unowned bus subscribers instantiated inside handlers:

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
        Watcher { };   // error: bus subscriber instantiated without an accepting owner in handler
    }
}

main locus App {
    params { h: Hub = Hub { }; }
    run() { }
}
```

To fix this error, declare `accept(w: Watcher)` on `Hub` or on an ancestor of `Hub` that encloses `Hub` on **every** construction path:

- Type aliases (`type W = Watcher`) resolve to the underlying locus declaration, whereas a similarly named locus from another seed (`lib::Watcher`) or a different generic specialization (`Cell<Int>` vs. `Cell<String>`) does not match.
- If `App` accepts `Watcher` and holds one `Hub` in `params`, but `fn main()` also constructs a standalone `Hub { }` outside `App`, `hale check` still rejects `Watcher { };` and points to the unparented `Hub` in `fn main()`.
- Subscribers instantiated inside `birth()`, `run()`, or non-handler methods (or in library seeds with no entry point, or with `--allow-unowned-subscriber`) are permitted.

## Parent-Child Contracts (`contract { expose; consume; }`)

A child locus declares which parameters its parent may inspect in a `contract { expose ...; }` block, and the parent declares which fields it reads in `contract { consume ...; }`:

```hale
type SessionState = enum { Lobby, Playing, Over };

locus GameSession {
    params { tick: Int = 0; state: SessionState = SessionState::Lobby; }
    contract {
        expose tick: Int;
        expose state: SessionState;
    }
}

locus Room {
    contract { consume tick: Int; }
    accept(g: GameSession) {
        if g.tick > 1000 { /* ... */ }
    }
}
```

`hale check` verifies that the parent declares an `accept` clause and that every field listed in `consume` is listed in the accepted child's `expose` block with the exact same type. (Note: `contract` checks the declared `expose`/`consume` pairs; use `@sealed` on a locus when you want the compiler to forbid all external field access.)

## Flow Is Vertical Only

In Hale's ownership hierarchy, references flow strictly vertically—from parent to child and child to parent—never sideways between siblings. When sibling loci need to coordinate, they route through their shared parent or publish over the [bus](./bus.md).

Because child memory arenas are sub-regions of their parent's arena and no lateral pointers exist across siblings, tearing down a parent locus safely tears down every accepted child, deregisters all child bus subscriptions, and frees the entire memory subtree wholesale without garbage collection.

## Flow Children vs. Resident Children (`release` and `terminate;`)

By default, an accepted child is a **resident**: when its `run()` returns, the child stays alive in the parent's arena (for example, to continue handling bus messages) until the parent dissolves.

For a long-lived server that accepts a new child per client connection, keeping children as residents until the server exits would accumulate memory indefinitely. Declaring **`release(c: Conn)`** on the parent marks `Conn` as a **flow**:

```hale
locus Conn {
    params { conn_fd: Int = -1; }
    run() {
        let stream = std::io::tcp::Stream { conn_fd: self.conn_fd, owns_fd: false };
        while true {
            let chunk = stream.recv(4096) or "";
            if len(chunk) == 0 { return; }   // client closed connection -> run() returns
        }
    }
}

locus Server {
    accept(c: Conn)  { }
    release(c: Conn) { }   // marks Conn as a flow child
}
```

### Flow vs. Resident Semantics

- **Automatic flow reclamation (`release`)**: For a flow child, `run()` *is* its lifetime. As soon as `c.run()` returns, the runtime drains `c`, invokes the parent's `release(c)` hook, runs `c.dissolve()`, and frees `c`'s memory arena immediately while `Server` continues running.
- **Program-wide flow classification (`hale check --flows`)**: Declaring `release(c: T)` on *any* locus in the program (including uninstantiated loci or imported seeds) classifies `T` as a flow type everywhere in the program. Run `hale check --flows` to inspect every `release(c: T)` declaration across your project and its imports.
- **Passing handler data to residents**: When a bus handler spawns or updates a resident child, scalar values, `String`s, and record fields are deep-copied into the resident's arena so they remain valid after the handler returns. Passing a handler-local locus collection (`@form(vec)`) by reference is rejected at compile time because the handler's frame dissolves on return; instead, copy the elements into the resident's own `@form(vec)` inside `birth()`.
- **Self-termination (`terminate;`)**: Any child (resident or flow) can end its own lifecycle from inside a method or bus handler by executing `terminate;`. On a resident child, `terminate;` exits the current method and runs `drain() → dissolve()` upon handler return without invoking `release`.

Next: [Failure & Supervision](./failure.md).
