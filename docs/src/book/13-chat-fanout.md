# Building chat-fanout

This is the capstone. **chat-fanout** is a multi-binary chat
program that exercises every substrate primitive introduced over
the previous twelve chapters — a websocket-facing server, a
terminal client, and the bus between them.

The shape: a *server* binary accepts websocket connections, holds
per-session state, fans out every message to every connected
client. A *client* binary opens one websocket, reads stdin,
prints incoming traffic. The two communicate only through
declared bus subjects, each subject typed against a shared
schema compiled into both binaries.

This chapter walks the program in full: the shared schema, the
server's source, the client's source, the deployment
configuration, and how the substrate primitives compose into a
working production-shaped artifact.

## Repository layout

```text
examples/chat-fanout/
    shared.ap         // types compiled into both binaries
    server.ap         // the server binary's main
    client.ap         // the client binary's main
    deployment.yaml   // intended deployment-time config
    README.md
```

Each `.ap` file is built independently:

```bash
aperio build examples/chat-fanout/server.ap
aperio build examples/chat-fanout/client.ap
```

Two ELF binaries land beside the source: `server` and `client`.
They run as separate operating-system processes, each opening
its own [lotus](../reference/glossary.md#lotus).

## The shared schema (`shared.ap`)

The shared file declares the wire-level types that travel on
the bus, plus a perspective for the session record:

```aperio
type Message {
    sender: String;
    body: String;
    ts: Time;
}

type Session {
    user: String;
    joined_at: Time;
    message_count: Int;
}

type Presence {
    user: String;
    status: String;     // "join" | "leave"
    ts: Time;
}

perspective SessionPerspective {
    params {
        session: Session;
        auth_token: String = "";
    }

    stable_when {
        return self.session.user != "" && self.auth_token != "";
    }

    serialize_as Session;
}
```

Per [chapter 9](./09-cross-process.md), schema agreement is *by
compilation, not by runtime negotiation*. Both binaries
`import "chat-fanout/shared";`. Each compiles the same struct
layouts; the wire format is exactly that in-memory layout, so
deserialization is exact.

The `SessionPerspective` ([chapter 11](./11-perspectives.md))
wraps the wire-shaped `Session` with a server-internal
`auth_token` and a `stable_when` predicate ("ship only once
authenticated"). The `serialize_as Session` annotation declares
that on the wire the perspective is a `Session` — the
`auth_token` is server-internal bookkeeping that does not cross
to the client.

## The server (`server.ap`)

The server's job: accept websocket connections, hold per-session
state, fan out every `chat.message` to every connected client
as a `chat.broadcast`.

```aperio
import "chat-fanout/shared";

locus ChatServerL {
    params {
        host: String = "127.0.0.1";
        port: Int = 8080;

        connected_count: Int = 0;
        messages_received: Int = 0;
        broadcasts_sent: Int = 0;
    }

    bus {
        subscribe "chat.message" as on_message of type Message;
        publish   "chat.broadcast"             of type Message;
        publish   "chat.presence"              of type Presence;
        publish   "chat.session"               of type Session;
    }

    birth() {
        std::io::ws::__listen(self.host, self.port,
                              self.__on_connection);
    }

    fn __on_connection(s: std::io::ws::Stream) {
        let sess = Session {
            user: s.handshake_user,
            joined_at: std::time::now(),
            message_count: 0,
        };
        let p = SessionPerspective {
            session: sess,
            auth_token: s.handshake_token,
        };
        if p.is_stable() {
            "chat.session" <- sess;
            "chat.presence" <- Presence {
                user: sess.user,
                status: "join",
                ts: sess.joined_at,
            };
            self.connected_count = self.connected_count + 1;
        }
    }

    fn on_message(m: Message) {
        self.messages_received = self.messages_received + 1;
        "chat.broadcast" <- m;
        self.broadcasts_sent = self.broadcasts_sent + 1;
    }

    closure broadcast_keeps_pace {
        self.broadcasts_sent ~~ self.messages_received within 1;
        epoch tick;
    }
}

fn main() {
    ChatServerL { };
}
```

The substrate primitives in play, all introduced earlier:

- **`params` block** with host/port plus running counters
  ([chapter 5](./05-contracts-and-parents.md)).
- **`bus` block** with one subscription and three publications
  ([chapter 6](./06-the-bus.md)).
- **A bus handler** (`on_message`) that mutates the locus's
  state and publishes onward ([chapter 6](./06-the-bus.md)).
- **A perspective construction** wrapping a `Session` with an
  `auth_token`, then `is_stable()` invoking the perspective's
  `stable_when` predicate ([chapter 11](./11-perspectives.md)).
- **A closure** auditing that broadcasts keep pace with received
  messages (within a small tolerance for in-flight dispatch)
  ([chapter 7](./07-closures.md)).

> **v0 caveat.** `std::io::ws` (websocket primitives) is a Phase
> 3 stdlib module; the substrate-level locus shape works today
> against a TCP listener (`std::io::tcp::Listener`). The chapter
> reads against the v1.x surface; the v0 example in
> `examples/chat-fanout/` substitutes the TCP form.

## The client (`client.ap`)

The client's job: open a websocket to the server, read stdin
into `chat.message` publishes, render every `chat.broadcast`
and `chat.presence` to stdout.

```aperio
import "chat-fanout/shared";

locus ChatClientL {
    params {
        server_host: String = "127.0.0.1";
        server_port: Int = 8080;
        user: String = "anonymous";

        sent_count: Int = 0;
        received_count: Int = 0;
    }

    bus {
        publish   "chat.message"                   of type Message;
        subscribe "chat.broadcast" as on_broadcast of type Message;
        subscribe "chat.presence"  as on_presence  of type Presence;
        subscribe "chat.session"   as on_session   of type Session;
    }

    birth() {
        std::io::ws::__connect(self.server_host, self.server_port,
                               self.user);
    }

    run() {
        let mut line = std::io::stdin::read_line();
        while line != "" {
            let m = Message {
                sender: self.user,
                body: line,
                ts: std::time::now(),
            };
            "chat.message" <- m;
            self.sent_count = self.sent_count + 1;
            line = std::io::stdin::read_line();
        }
    }

    fn on_broadcast(m: Message) {
        std::io::stdout::println(m.sender + ": " + m.body);
        self.received_count = self.received_count + 1;
    }

    fn on_presence(p: Presence) {
        std::io::stdout::println("[" + p.user + " " + p.status + "]");
    }

    fn on_session(s: Session) {
        std::io::stdout::println("[session for " + s.user + "]");
    }

    closure sent_received_balance {
        self.received_count ~~ self.sent_count within 5;
        epoch dissolve;
    }
}

fn main() {
    let mut user = "anonymous";
    if std::env::args_count() > 1 {
        user = std::env::arg(1);
    }
    ChatClientL { user: user };
}
```

A larger surface than the server, exercising:

- **Three subscriptions and one publication.** The client
  consumes `chat.broadcast` (every message from any client),
  `chat.presence` (join/leave notices), and `chat.session`
  (its own session record); it produces `chat.message` (its
  own outgoing lines).
- **An interactive `run()` loop.** Reading stdin until EOF
  publishes each line as a `Message`. The bus subscriptions
  fire concurrently with the loop, dispatched cooperatively.
- **An at-dissolve closure** auditing that received broadcasts
  approximately match what the client sent (within a
  tolerance of 5 for in-flight traffic at shutdown — the
  exact pacing depends on the number of other connected
  clients).

## The deployment

The chat-fanout `deployment.yaml` shows the *intended* production
transport binding:

```yaml
channels:
  "chat.message":
    transport: websocket
    url: "ws://localhost:8080"

  "chat.broadcast":
    transport: websocket
    url: "ws://localhost:8080"

  "chat.presence":
    transport: nats
    url: "nats://nats-control:4222"

  "chat.session":
    transport: nats
    url: "nats://nats-control:4222"
```

Each subject is bound to a transport appropriate to its traffic
shape:

- **`chat.message`** / **`chat.broadcast`** — websocket. Live
  bidirectional traffic between the server and every connected
  client; the substrate uses the websocket framing as the bus
  transport.
- **`chat.presence`** — NATS. Slower cadence, reliable
  delivery, and useful to control-plane observers that aren't
  themselves chat participants.
- **`chat.session`** — NATS. Session records persist beyond
  any single transport hop; reliable delivery matters.

For local testing, the YAML supports a wildcard swap:

```yaml
channels:
  "chat.*":
    transport: in_memory
```

This binds every `chat.*` subject to the in-memory transport —
the same source code, bound differently for a single-process
integration test.

> **v0 caveat.** The YAML form is the *intended* future surface.
> v0's actual cross-process bus consumes the simpler
> `LOTUS_BUS_CONFIG` line format from chapter 9.
> `deployment.yaml` parsing and richer transport selection are
> v1.x roadmap items.

## What the program does

Putting the parts together, the chat-fanout pipeline:

1. **The server births.** `birth()` opens a websocket listener
   on `host:port`; each accepted connection wraps a `Session`
   in a perspective, ships the session record if the
   handshake authenticated, and emits a `chat.presence` join.
2. **A client types.** Each stdin line publishes a `Message`
   on `chat.message`. The transport hops the message through
   the server's listener.
3. **The server fans out.** `on_message` mutates the server's
   counters and republishes the same `Message` on
   `chat.broadcast`. Every connected client's subscription
   fires.
4. **Each client renders.** `on_broadcast` prints
   `<sender>: <body>` to stdout. `on_presence` prints
   `[<user> join|leave]`.
5. **The closures audit.** The server's `broadcast_keeps_pace`
   fires at every tick and complains if outgoing broadcasts
   drift behind incoming messages. The client's
   `sent_received_balance` fires at dissolve and complains if
   the fanout math doesn't approximately balance.

Each binary's lotus is independently lifecycle-managed: `birth`
runs once when each process starts; `run` keeps the locus alive
(implicit on the server, explicit stdin-loop on the client);
`dissolve` fires when the process receives a shutdown signal
(SIGINT in the v0 substrate).

## What the substrate enforces

The chat-fanout is small in lines but exercises the full
substrate-up stance the language was built for. A reader
familiar with the previous twelve chapters can verify the
following at the source level:

- **No leaked allocations.** Every `Message`, every
  intermediate string, every `Presence` copy lives in its
  locus's arena and is freed when the locus dissolves. There
  is no available concept of escape across the boundary.
- **No lateral failure routing.** A `ClosureViolation` on the
  client's `sent_received_balance` reaches `ChatClientL`'s
  `on_failure` (which is unhandled in this version, so the
  process exits non-zero with the violation report).
  Sibling-to-sibling absorption is structurally impossible.
- **Schema agreement by compilation.** Both binaries compile
  the same `shared.ap`. There is no schema document to
  maintain separately; if the schema changes, both binaries
  recompile from the same source, and the deployment is a
  single rolling update.
- **Hot session updates via perspectives.** When the server
  re-authenticates a session, it ships a new `Session`
  through the `SessionPerspective`. The client swaps it in
  atomically. No code reload, no client restart, no special
  "config refresh" mechanism — the session is the value the
  system was built around.

## What v0 does not yet do

A few production-relevant pieces are roadmap, not v0:

- **`std::io::ws` (websocket stdlib).** The server and client
  reach for `std::io::ws::__listen` and `__connect`;
  `std::io::ws` is a Phase 3 module. The v0 example
  substitutes a TCP listener (`std::io::tcp::Listener`) with
  framing handled in user code.
- **`p.is_stable()` as a method.** The server calls
  `p.is_stable()` on the session perspective; for v0 the
  substrate treats the `stable_when` block as the body of an
  `is_stable()` method. Generalizing perspective methods
  beyond `stable_when` is post-v1.
- **`serialize_as TypeV1` rolling deployments.** Schema
  evolution with mixed-version producer/consumer pairs during
  a deployment window. Open-question #13; implemented when a
  workload demands it.

## Where to go next

Beyond this chapter:

- **The reference.** The
  [Aperio Reference](../reference/introduction.md) covers every
  construct in the language with formal grammar and semantics.
  Reach for it when a question this book left imprecise comes up.
- **The standard library.** The [Aperio Standard
  Library](../std/roadmap.md) catalogs the batteries — I/O,
  HTTP, text processing, the test framework — that overlay the
  substrate. (Phases 1–5 of the v1.x roadmap; many libraries
  are in active development.)
- **The example ladder.** `examples/` in the repository is the
  ladder this book has drawn from. Each rung is a runnable
  Aperio program with an annotated `main.ap` and a `README.md`
  walk-through.

You have read the substrate-up tour. Aperio's promise is that
programs of any shape, written against this substrate, behave
the way the substrate's invariants guarantee — by construction,
with the compiler enforcing the rules and the runtime upholding
them.

Open the wand. Cast.
