# The bus

Loci speak to each other in two ways. The first is the
**contract** introduced in [chapter
5](./05-contracts-and-parents.md) — a parent reading a
child's exposed surface through `accept`. The second is the
**bus** — typed pub-sub on named subjects, with no direct
reference between publisher and subscriber.

This chapter covers the bus surface: declaring publish /
subscribe on a locus, the `<-` send operator, what the
subscriber's handler receives, and the substrate's load-bearing
**F.8** — vertical-only-flow — which makes the bus's graph of
communication closed by construction.

## The bus block

A locus declares its bus interface in a `bus { ... }` block:

```aperio
type Greeting {
    text: String;
    sender: String;
}

type Acknowledgment {
    received: String;
}

locus EchoL {
    bus {
        subscribe "demo.greeting" as on_greeting of type Greeting;
        publish   "demo.ack"               of type Acknowledgment;
    }

    fn on_greeting(g: Greeting) {
        println("got: ", g.text, " from ", g.sender);
        "demo.ack" <- Acknowledgment { received: g.text };
    }
}
```

Two declaration forms appear here:

- `subscribe "demo.greeting" as on_greeting of type Greeting`
  — this locus listens on the subject `"demo.greeting"`. When a
  message of type `Greeting` is published on that subject, the
  locus's `on_greeting` method runs with the message as its
  argument.
- `publish "demo.ack" of type Acknowledgment` — this locus
  may publish messages of type `Acknowledgment` on the subject
  `"demo.ack"`. The compiler tracks which subjects a locus is
  permitted to publish on; `<-` on a subject the locus did not
  declare is a compile-time error.

A locus may declare any number of subscribe and publish entries,
in any combination. A locus that only publishes does not declare
subscriptions; a pure subscriber does not declare publishes.

## Subjects and types

A subject is a string-literal name like `"demo.greeting"` or
`"market.l1.book"`. Conventionally these are dotted hierarchical
names, but the language imposes no structure beyond *a string
literal*. The full UTF-8 string is the subject identifier.

Every subject carries exactly one `type`. The type is declared
on every subscribe and publish entry that names the subject; the
compiler verifies that all declarations agree. A subject named
`"demo.greeting"` cannot be `Greeting` in one locus and
`Acknowledgment` in another — that is a compile-time error. The
type checker treats the subject's typed signature as a global
fact.

The payload is always a [user-defined
`type`](./03-types-and-values.md#user-defined-records), not a
primitive — bus messages are records, by convention.

## `<-`: the send operator

Publishing is a statement, not an expression. The form is:

```aperio
"subject" <- payload;
```

with a string-literal subject on the left and any expression
producing the subject's declared type on the right. `<-`
produces no value and does not nest in expressions; it is
statement position only.

```aperio
"demo.greeting" <- Greeting {
    text: "hello",
    sender: "sender-1",
};
```

For each currently-active subscriber to `"demo.greeting"`, the
runtime delivers a copy of the `Greeting` payload into the
subscriber's arena and arranges for the subscriber's handler to
run. The publisher's locus does not block waiting for delivery;
publish-and-continue is the dispatch model.

## Subscriber handlers

A `subscribe` entry of the form
`subscribe "demo.greeting" as on_greeting of type Greeting`
binds the subject to a method named `on_greeting` on the same
locus. The method has the signature:

```aperio
fn on_greeting(g: Greeting) {
    println("got: ", g.text, " from ", g.sender);
    "demo.ack" <- Acknowledgment { received: g.text };
}
```

— exactly one parameter, of the subject's declared type. The
runtime calls it once per delivered message.

The handler runs **on the subscriber's arena**: the `Greeting`
argument it receives is a copy that lives in the subscriber's
own region of memory. When the subscriber's lifecycle
eventually dissolves, that arena is freed wholesale and the copy
goes with it — the publisher's original payload is independent.

A handler may publish on any subject the locus declared
`publish` for. The example above does exactly this: on
receiving a `Greeting`, `EchoL` publishes an `Acknowledgment`
on `"demo.ack"`.

## When subscriptions become active

Subscriptions register at the locus's `birth`. Until `birth`
runs, a locus is not a subscriber on any subject; messages
published on the subject before then are not delivered to it.

This matters for ordering at startup. In the bus example
above, `EchoL` and `AckLogL` are constructed before `SenderL`
in `main`:

```aperio
fn main() {
    EchoL { };       // subscribes to "demo.greeting"
    AckLogL { };     // subscribes to "demo.ack"
    SenderL { };     // publishes on "demo.greeting" in birth()
}
```

`SenderL`'s `birth` publishes a `Greeting`. By the time it
runs, the two subscribers' `birth` calls have already
registered their subscriptions, so the publish is delivered
correctly.

If `SenderL` were constructed first, its publish would happen
before any subscriber existed and the `Greeting` would simply
be dropped — there is nobody to deliver to. Subscription
ordering matters in v0; the sender must be born after its
subscribers are ready.

## F.8: vertical-only-flow

The bus is **vertical-only-flow**. That is the substrate's
load-bearing rule about how communication can shape itself, and
it has two parts.

### The graph of communication is closed

Every bus interaction passes through a declared subject. There
is no "message accidentally reaches a locus that didn't ask for
it"; subscription is opt-in by declaration. Consequently:

- The set of subjects a locus listens to is a static, declared
  property of its source code.
- The set of subjects a locus publishes on is a static,
  declared property of its source code.
- The graph "who-can-talk-to-whom" is the union of all
  subscribe / publish declarations across the program.

This graph is checked at compile time. A program does not need
runtime introspection to know its communication topology — the
compiler already does.

### No lateral failure routing

The other half of **F.8**: failures do not flow along the bus.
A locus's failure (`ClosureViolation`, panic) propagates *up* —
to its parent, who decides whether to absorb, restart, or
bubble it further. There is no "failure published as a message
to a peer" mechanism; siblings cannot accidentally absorb each
other's failures. (Recovery primitives are the topic of
[chapter 12](./12-recovery-and-supervision.md).)

The combination of these two — closed communication graph plus
no-lateral-failure — rules out an entire class of
distributed-systems bug where a peer silently absorbs another
peer's misbehavior. Every failure has a finite, visible
upward path.

## Transports

A subject's wire transport is bound at **deployment time**, not
in source. The same source can run with the bus over:

- **In-memory** — the v0 default. The router is a process-local
  data structure; publish copies the payload between arenas in
  the same process.
- **Unix sockets** — for cross-process bus on a single machine
  (m57+). Subjects with this transport land in
  [chapter 9](./09-cross-process.md).
- **NATS / UDP multicast / TCP** — the production transports.
  Implementations of `std::bus::Adapter`. Same source binds to
  any of them per `deployment.yaml`.

The locus's source declares *what* the subject's payload is; the
deployment declares *where* the subject travels. This split is
what lets the same Aperio source target single-process,
single-machine multi-process, and multi-machine production
without source-level modification.

## Memory: bus copy semantics

When `<-` runs, the payload is copied from the publisher's arena
into each subscriber's arena. The substrate calls this **bus
copy semantics**. A few consequences worth keeping straight:

- **The subscriber owns its copy.** The handler may keep a
  reference to (parts of) the payload for as long as the
  subscriber's lifecycle lasts; when the subscriber dissolves,
  the copy is freed alongside everything else in its arena.
- **The publisher does not block.** Once `<-` returns, the
  publisher is free to mutate or discard the original payload;
  the subscriber's copy is independent.
- **In-process and cross-process look identical at the source
  level.** The copy that crosses arenas in-process is the same
  observable shape as the byte-stream copy that crosses a Unix
  socket boundary.

For variable-length fields — most commonly `String` — the
runtime's payload arena (allocated lazily on first
cross-process publish) holds the string bytes in the
subscriber's reach. The mechanics are introduced in
[chapter 9](./09-cross-process.md); for in-process bus you do
not need to think about it.

## What this chapter does not cover

- **Cross-process bus** — `connect` / `listen` deployment
  roles, the wire format, the per-field serializer — see
  [chapter 9](./09-cross-process.md).
- **Closures** — the audit construct that uses `~~` and runs
  at compile-known epochs — see
  [chapter 7](./07-closures.md). Closures interact with the
  bus when an audit involves messages observed across a
  duration epoch.
- **Long-running children** — children whose lifecycles
  outlive a single statement in the parent's body. Bus
  subscribers are typically long-running because their `birth`
  registers a subscription that needs to remain active while
  the locus listens for messages.

The next chapter, **[Closures](./07-closures.md)**, introduces
the substrate's audit primitive — the rune by which a locus
watches its own structural invariants, evaluated at
compile-known epochs.
