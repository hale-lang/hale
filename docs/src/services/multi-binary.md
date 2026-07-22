# Across binaries

> **Coming from Go?** Splitting a program into services usually
> means rewriting in-process calls as RPC or queue clients. In
> Hale the publisher and subscriber code *doesn't change* — a
> topic that was an in-process queue becomes a Unix socket or a
> broker by adding one line to `main`'s `bindings { }` block. The
> deployment seam is the only place that knows.

## A topic is in-process by default

When a topic isn't mentioned in any `bindings { }` block, it's
delivered by an in-process cooperative queue. Two loci in the
same binary just talk. Nothing to configure.

## Binding a topic to a transport

To carry a topic *between* binaries, name it in the `main`
locus's `bindings { }` block with a transport:

```hale
main locus App {
    bindings {
        MatchReady: unix("/tmp/matches.sock");
    }
    run() {
        Matchmaker { target_size: 4 };
    }
}
```

`bindings { }` is legal only on a `main` locus. The publisher's
`MatchReady <- info;` and the subscriber's `subscribe MatchReady
as ...` are *unchanged* — they don't know or care that delivery
now crosses a socket. The same locus source runs in a test
(in-memory), a single binary (in-memory), and a multi-binary
deployment (unix), chosen entirely at this seam.

## The transports that ship

- **In-process** — the default; absence of a binding.
- **`unix("/path")`** — an AF_UNIX framed-byte transport, owned
  by the runtime. The role (listen vs connect) is inferred from
  whether the binary publishes or subscribes the topic; specify
  `role: listen | connect` when one binary does both.
- **`udp://host:port`** — datagram transport, including IPv4
  multicast. Lossy by nature — right for tick streams and
  telemetry where stale-is-worthless.
- **A user adapter** — any locus you write that satisfies the
  `__StdBusAdapter` interface (a single `send(subject, bytes)`
  method). This is how NATS, MQTT, a raw-TCP framing, or a
  custom JSON-over-WebSocket transport plug in — as ordinary loci
  in your code, not language features:
  ```hale
  bindings {
      BrokerEvt: MyNatsAdapter { url: "nats://prod:4222" };
  }
  ```

The substrate stays neutral on protocol semantics — reliability,
ordering, retries, backpressure all live in the adapter body,
where they belong.

## What each binding promises

A send succeeding means the broker accepted the message — and
what "accepted" obligates the broker to depends on the binding:

| Binding      | "Accepted" means                                          |
|--------------|-----------------------------------------------------------|
| in-process   | dispatched to every born subscriber in this binary        |
| `unix(...)`  | handed to the peer connection, message boundaries intact  |
| `udp://...`  | handed to the local IP stack — lossy from there, by design |
| adapter      | whatever the adapter locus's own contract says            |

The one thing a broker may never do is accept a message it
already knows it can't handle. So a binding that can't be
*opened* — the socket path doesn't exist, the address won't
bind, the peer never answers the connect retry — is a **birth
failure**: the program prints a structural diagnostic naming the
subject and exits non-zero at startup, where your supervisor
(systemd, Kubernetes) sees it. There is no mode where a dead
binding lets publishers keep "succeeding" while every message is
dropped. Per-datagram loss on `udp://` is different — that
transport's guarantee is best-effort by declaration, so downstream
loss is within contract and won't kill the process.

And a peer *disconnecting* from a `unix(...)` listener isn't a
failure at all: the listener stays bound and simply accepts the
next connection. Restart the publishing binary and it reconnects
— the subscriber never notices. (Under the hood each binding is
a real locus, a child of your `main` locus, whose lifecycle
opens the transport at birth and tears it down at dissolve —
the same shape as a custom adapter.)

The *connect* side is the one that can genuinely lose its link —
the peer it sends to goes away mid-run. That loss is structural:
by default the process exits with a diagnostic naming the
subject, because a broker that kept "accepting" messages it can
no longer deliver would be lying to you. If you'd rather
reconnect, say so — as a supervision decision on `main`:

```hale
main locus App {
    bindings { Evt: unix("/tmp/evt.sock", role: connect); }
    on_failure(t: std::bus::UnixTransport, err: ClosureViolation) {
        restart (t);     // re-run the connect-with-retry
    }
    run() { /* ... */ }
}
```

`restart` re-dials with the same retry window the boot connect
uses, and publishing resumes on success. No hidden retry loops
in the transport, no policy kwargs on the binding — the same
`on_failure` + recovery-primitive vocabulary you already use for
child loci. (Messages published while the link is down are
dropped, and the drop is visible in the supervision flow — the
broker never pretends they were delivered.)

The same rule covers routes added at deploy time through the
`LOTUS_BUS_CONFIG` file: a route that's asked for but can't be
opened refuses the boot.

## Talking to other languages: codecs

By default the bus uses Hale's internal wire format, which is
fine Hale-to-Hale but opaque to a consumer in another language.
When you need JSON over a socket or protobuf to a Python peer, a
binding names a `codec` — a locus that owns encode/decode:

```hale
bindings {
    Tick: unix("/tmp/ticks.sock") codec(TickJsonCodec { });
}
```

The codec is structurally typed against the topic's payload
(`encode` takes the payload type, `decode` returns it) and must
be *pure* — no hidden state — because it runs on transport
threads. Different bindings on the same topic can carry different
codecs; the publisher's send site doesn't know which.

## The shape this gives you

A single source tree, decomposed into loci that coordinate over
topics. How those topics are delivered — same process, same
machine over a socket, across the network via a broker — is a
deployment decision living in `bindings { }`, separate from the
logic. You design the system once and deploy it many ways. The
[systems tier](../systems/zero-copy-bus.md) adds one more
transport for the highest-frequency same-machine routes:
shared-memory zero-copy.

---

That's the services tier: lifecycle, a typed bus, concurrency and
placement, supervised parent/child trees, structural failure, and
multi-binary deployment. You can build daemons, servers, and
distributed systems with this. The final tier goes under the
runtime — memory, layout, raw performance, and the C boundary —
for when you need that control.

Next: [Memory & lifetime](../systems/memory.md).
