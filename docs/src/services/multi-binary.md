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
type Match { size: Int; }
topic MatchReady { payload: Match; }

locus Matchmaker {
    params { target_size: Int = 2; }
    bus { publish MatchReady; }
    run() { MatchReady <- Match { size: self.target_size }; }
}

main locus App {
    bindings {
        MatchReady: unix("/tmp/matches.sock");
    }
    run() {
        Matchmaker { target_size: 4 };
    }
}

fn main() { App { }; }
```

A binding may name a topic the seed imports — `bindings {
core::Verdict: unix("/tmp/verdicts.sock", role: listen); }` — so a
library's own declarations are what cross the socket, not copies
of them.

A listen binding serves up to 64 connected publishers at once,
and a `keyed_by` topic keeps its routing across the socket: the
receiving side derives the key from the payload, so a
`subscribe T as h where key == self.k` behind a binding hears
exactly what it would hear in-process. The same holds for bytes an
adapter hands to `std::bus::__local_dispatch`: they carry no key,
so the runtime decodes them once — through the binding's codec,
when it has one — and routes by the key the payload holds.

Such bytes are a publisher the program's `bus { }` blocks do not
show, so the check counts the call as one: a topic that only ever
arrives through `__local_dispatch` is not "subscribed but never
published". A call that writes its subject as a literal publishes
that subject; one that computes it — a broker connection
dispatching whatever arrives — may publish any, so no subscribed
topic of that program is reported as unpublished.

A topic can also be bound to a **hub**, a listener that streams it to
subscribers (a CLI, a dashboard) rather than to another binary, and a
program offers operations to the same callers with `api::serve`; that is
its own chapter, [The API surface](./api.md).

`bindings { }` is legal only on a `main` locus. The publisher's
`MatchReady <- m;` and the subscriber's `subscribe MatchReady
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
- **`shm_ring("/name", on_overflow: …)`** — a POSIX shared-memory
  ring for routes between processes on one machine; the
  [systems tier](../systems/zero-copy-bus.md) covers it.
- **A user adapter** — any locus you write that satisfies the
  `__StdBusAdapter` interface (a single `send(subject, bytes)`
  method). This is how NATS, MQTT, a raw-TCP framing, or a
  custom JSON-over-WebSocket transport plug in — as ordinary loci
  in your code, not language features:
  ```hale,fragment
  bindings {
      BrokerEvt: MyNatsAdapter { url: "nats://prod:4222" };
  }
  ```
  The adapter is built once, before the `main` locus that names it,
  so its literal is checked as an expression in a free function is: it
  cannot read that locus's params (`self.url` there is refused at
  `hale check`) or act on it (`terminate`, a publish). Write the value in the literal, or give the
  adapter's own field a default.
  An adapter a library ships is named through its import alias,
  like any imported name — and so is a codec:
  ```hale,fragment
  bindings {
      BrokerEvt: nats::NatsAdapter { url: "nats://prod:4222" } codec(nats::JsonCodec { });
  }
  ```

The substrate stays neutral on protocol semantics — reliability,
ordering, retries, backpressure all live in the adapter body,
where they belong.

`udp://host:port` — datagrams, IPv4 multicast included, lossy by
nature, right for tick streams and telemetry where
stale-is-worthless — is not a `bindings { }` constructor. It is a
route in the `LOTUS_BUS_CONFIG` file the runtime reads at startup
(the peer and route table `hale node` writes), beside `unix://`
routes. The route lives in the operator's layer, so the source
names no transport for it at all.

An adapter is pinned: its `run()` — the receive loop — has a thread
of its own, and every topic it subscribes to is delivered there.
`send` is called on the publisher's thread, so an adapter that wants
every socket write on its own thread has `send` publish the message
onto a topic it subscribes to itself, and writes it from that
handler:

```hale,fragment
locus BrokerAdapter {
    bus { publish Outgoing; subscribe Outgoing as on_outgoing; }
    fn send(subject: String, bytes: Bytes) {
        Outgoing <- Frame { subject: subject, data: bytes };
    }
    fn on_outgoing(f: Frame) { /* write to the socket */ }
    run() { while !self.draining { /* read the socket */ } }
}
```

Being pinned, an adapter is held to the same restrictions as a
locus placed `pinned` (placement rule 6): it declares no
`accept(...)` and no closure whose epoch is `birth` or `dissolve`
(dissolve is the default), because the lifecycle cascade that runs
those cannot cross onto its thread. `hale check` refuses either one
at the binding entry, with notes at the adapter's declaration and at
the member that conflicts.

## What each binding promises

A send succeeding means the broker accepted the message — and
what "accepted" obligates the broker to depends on the binding:

| Binding      | "Accepted" means                                          |
|--------------|-----------------------------------------------------------|
| in-process   | dispatched to every born subscriber in this binary        |
| `unix(...)`  | handed to the peer connection, message boundaries intact  |
| `shm_ring(...)` | slot claimed and committed, under the declared `on_overflow` policy |
| adapter      | whatever the adapter locus's own contract says            |
| `udp://` route | handed to the local IP stack — lossy from there, by design |

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
a real locus, built before any statement you wrote, whose lifecycle
opens the transport at birth and tears it down at dissolve —
the same shape as a custom adapter. Its dissolve is the *last*
one your program runs; see the teardown order below.)

The *connect* side is the one that can genuinely lose its link —
the peer it sends to goes away mid-run. That loss is structural:
by default the process exits with a diagnostic naming the
subject, because a broker that kept "accepting" messages it can
no longer deliver would be lying to you. If you'd rather
reconnect, say so — as a supervision decision on `main`:

```hale
type Reading { v: Int; }
topic Evt { payload: Reading; }

main locus App {
    bindings { Evt: unix("/tmp/evt.sock", role: connect); }
    bus { publish Evt; }
    on_failure(t: std::bus::UnixTransport, err: ClosureViolation) {
        restart(t);      // re-run the connect-with-retry
    }
    run() { Evt <- Reading { v: 1 }; }
}

fn main() { App { }; }
```

`restart` re-dials with the same retry window the boot connect
uses, and publishing resumes on success. No hidden retry loops
in the transport, no policy kwargs on the binding — the same
`on_failure` + recovery-primitive vocabulary you already use for
child loci. (Messages published while the link is down are
dropped, and the drop is visible in the supervision flow — the
broker never pretends they were delivered.)

If a publish would rather not be dropped during that window, it
can say so at the send site:

```hale,fragment
Evt <- reading or wait;
```

`or wait` parks the publisher until the reconnect lands, then
sends on the re-armed link — the third option next to
counted-drop and structural exit. It's a delivery-mode choice,
not error handling: the send is still infallible, and each send
site of the same topic picks its own behavior (a hot path can
keep the default drop while a must-arrive path waits). Two
honest edges: the send that *discovers* a dead link still fails
(that's how the loss is detected — `or wait` prevents the
window drops that follow, not the detection casualty), and a
wait that can never be satisfied — the reconnect fails, or the
program is already tearing down — raises instead of hanging.
Parks are visible as a `waits` counter next to `dropped_lost`
in the `LOTUS_BUS_COUNTERS_DUMP=1` line.

The same rule covers routes added at deploy time through the
`LOTUS_BUS_CONFIG` file: a route that's asked for but can't be
opened refuses the boot.

The *listen* side makes a delivery promise of its own: **wire
data the kernel accepted is delivered, even at the awkward
edges**. A peer that connects and publishes in the instant
between the socket appearing and this binary's subscribers
finishing registration doesn't lose that traffic: it's buffered
(bounded — 64 messages / 1 MiB per binding — and visible as
`buffered_early` in the counters dump) and delivered the moment
the registration lands. And a program
whose `main` returns while messages still sit undrained in a
socket queue quiesces first: listeners stop accepting new
connections, what already arrived is drained and handled while
every subscriber is still alive, then teardown proceeds. The
quiesce is bounded (`LOTUS_BUS_QUIESCE_MS`, default 500ms) so a
silent peer holding its connection open can't stall your exit.
What you should *not* read into this: it is not durability — a
message still in the publisher when either process dies is gone,
and `udp://` remains lossy by declaration.

The binding locus itself goes last. It is set up in `fn main`'s
prelude, before any statement you wrote, so it is the first thing
`fn main` owns and the last it tears down — the teardown order at
exit is: quiesce the listeners, join the cooperative
pools, dissolve your loci innermost-first, and only then
dissolve the bindings, closing their sockets and joining their
serve threads. That ordering is what makes a publish from a
`dissolve()` body still reach the wire, and it is why a binding
is the one child whose lifetime spans your whole program rather
than the scope it was written in.

## Talking to other languages: codecs

By default the bus uses Hale's internal wire format, which is
fine Hale-to-Hale but opaque to a consumer in another language.
When you need JSON over a socket or protobuf to a Python peer, a
binding names a `codec` — a locus that owns encode/decode:

```hale,fragment
bindings {
    Tick: unix("/tmp/ticks.sock") codec(TickJsonCodec { });
}
```

A codec rides any binding, including an adapter's: the adapter's
`send` receives the codec's bytes, and bytes it hands to
`std::bus::__local_dispatch` — from its own pinned receive loop —
are decoded by the same codec before any subscriber sees them.

```hale,fragment
bindings {
    Tick: BrokerAdapter { url: "broker://localhost" } codec(TickJsonCodec { });
}
```

The codec is structurally typed against the topic's payload
(`encode` takes the payload type, `decode` returns it) and must
be *pure* — no hidden state — because it runs on transport
threads. Different bindings on the same topic can carry different
codecs; the publisher's send site doesn't know which.

## Checking the whole deployment

Each binary checks its own claims in its own closed world. That
leaves one question none of them can answer: what happens once they
are wired together? A component that is individually correct can
still be deployed into an arrangement that isn't — two publishers
where the law says one, or a path from a strategy to a gateway that
was supposed to go through risk.

`hale fleet` answers that by composing the **artifacts**, never the
source.

### A fleet in source

Declare the deployment as a seed of its own holding a `fleet locus`.
Its params are the other seeds' mains, its placement a default host or
replica count per instance, its bindings the topics that wire them,
and its claims the law across them:

```hale
import "../desk" as desk;
import "../venue" as venue;
import "../shared" as t;

fleet locus Prod {
    params {
        orders: desk::Orders;
        gw: venue::Gateway;
    }
    placement {
        orders: host("box-a");
        gw: replicas(2);
    }
    bindings {
        t::OrderRequest: unix("/run/orders.sock");
    }
    claims {
        one_desk: count publishers(topic t::OrderRequest) == 1;
        no_return: forbid reaches(gw, orders);
    }
}
```

A fleet is a locus kind, not a param of some main: it is checked as one
graph and never runs, so it holds no code, `hale check` judges it and
`hale build` refuses it. `hale fleet check fleets/prod` cuts each
instance's artifact from its seed and derives the routes — you write
the subject, never the endpoints:

```text
component `orders` runs `desk::Orders` from …/desk (shape …), on host `box-a`: orders-0
  publishes svc.order.request as `t::OrderRequest`, bound unix
component `gw` runs `venue::Gateway` from …/venue (shape …), 2 replicas: gw-0, gw-1
  subscribes svc.order.request as `t::OrderRequest`, bound unix
route `t::OrderRequest` (svc.order.request, unix): orders-0 -> gw-0, gw-1
ok: fleet `Prod` composed — 3 instance(s), 1 route(s), fleet_shape_hash …
```

The fleet's binding is the route's configuration. An end whose own main
binds the subject too must bind it over the same transport, and a binding
nobody publishes (or nobody subscribes) wires nothing; both are refused.

Why an *instance* rather than an application: `oms` is a program,
`oms-0` is a process. Cardinality, witnesses, and running two copies
of one binary all need the distinction, so an instance's id is
`<param>-<i>`, one per replica or named host.

Why artifacts rather than source: merging the programs would invent
edges that no deployed route creates (an unbound topic is in-process
by default) and turn messaging into ordinary call reachability. So the
fleet seed is never merged with the seeds it imports: `hale check`
judges it over its own source and what each import declares, and
`hale fleet check` checks each instance's seed as it cuts its artifact.
Matching wire identities establish *compatibility*; only a binding
creates an edge.

A violation names the path across binaries — each hop, the route
carrying it, and the file each vertex lives in:

```
fleet claim `orders_pass_oms` violated — witness:
  prober-0::Probe::submit  [prober/main.hl]
  -(route `t::OrderRequest`)->
  gw-0::Gateway::on_order  [gw/main.hl]
```

Declare your deployments in `hale.toml` and check them all at once;
each entry names a fleet seed:

```toml
[fleets]
production = "fleets/prod"
staging    = "fleets/staging"
```

Whoever wants the old plan document gets it as an export of the rows —
`hale fleet export fleets/prod` writes the instances, their seeds and
hosts, the derived routes and the claims as JSON. Nothing reads one
back.

### Signing the certificate

A composition proves a world against the rows it derives. When the
fleet travels — a deploy box that didn't write it, a CI stage that
didn't run the checks — a signature over the rows proves they are the
ones you meant:

```sh
hale fleet keygen ops                    # ops.pem + ops.pub.pem
hale fleet sign fleets/prod --key ops.pem
hale fleet check fleets/prod --trust ops.pub.pem
```

The signature (`fleets/prod/fleet.sig`) covers the export's exact bytes:
each instance's seed, host and component shape, the routes and the
claims. A component whose model changes after signing changes the rows,
and the check refuses them.

Trust is **strict when declared**: with `--trust` given (or
`[fleet_trust] keys = [...]` in `hale.toml`), unsigned rows are
refused, not skipped. The signature is ES256 — the same suite as
`std::crypto`, so a Hale program can verify it itself.

### What it does not tell you

The honest boundary matters more here than at any other tier,
because a green result is easy to over-read.

It certifies **topology** — which routes exist, which instances they
connect, what each carries. It does not prove a message will
*arrive*: delivery is a property of the protocol and the peer, not of
your code, and no amount of static analysis changes that. Runtime
observation is what measures delivery.

Nor is it a rolling-upgrade proof. A clean old fleet and a clean new
fleet say nothing about the arrangement that exists midway between
them.

Where the model is uncertain it says so rather than guessing. If a
component reaches a call the compiler cannot resolve, a prohibition
past that point comes back `uncertified` — not `holds`. An absence
nobody could see is not an absence.


## Health a platform can read: probes

A supervisor (compose's healthcheck, a Kubernetes probe) needs to ask a
process whether it is alive and whether it is ready, and a program that
serves no API still has to answer. Declare it as a param of the main
locus:

```hale
main locus App {
    params {
        probes: std::observe::Probes = std::observe::Probes { port: 9100 };
    }
    run() {
        while !self.draining { std::time::sleep(100ms); }
    }
}
fn main() { App { }; }
```

The runtime then serves, on that port, from a thread of its own (not a
pool you place, so a stuck pool cannot keep it from answering):

- `GET /healthz`: `200 ok`, or `503` naming each queue (main's, a
  pool's) that has held work its worker has not touched for ten seconds
  (`LOTUS_PROBES_STALL_MS` changes the window). A pool running one long
  handler with nothing waiting is still alive; one with work piling
  behind a stuck handler is not.
- `GET /readyz`: `503 starting` until the main locus's births are done
  (its children born, its bindings open), then `200 ready`, and
  `503 draining` once the process drains, so a supervisor stops sending
  it work before it stops.
- `GET /capacity`: each queue's waiting cells, its capacity, and how long
  it has stalled, as JSON.

The main locus declares one, and builds it by its default: the port can
read another param (`std::observe::Probes { port: self.probe_port }`),
but a construction of the main locus cannot supply the probes. Nothing
is served, or measured, in a program without the param.

## Rendering the deployment

`hale deploy render` turns what a program already declares into a
platform description, and stops there: it never builds or deploys.

```sh
hale deploy render fleets/prod --env prod            # backend from hale.toml
hale deploy render fleets/prod --backend compose --placement alloc.json --out deploy/
hale deploy render ./svc --backend kubernetes --out deploy/
```

```toml
[environments.prod]
source_only = true
backend = "compose"

[environments.prod.env]
LOG_LEVEL = "info"
```

Given a seed it renders one workload. Given a fleet seed it renders one
description **per host** — the habitat applies each host on its own — with
one service per instance there (compose) or one Deployment per program
with `replicas` its instance count there (Kubernetes), and `depends_on`
where a route joins two instances on the host. Each instance runs on the
host its placement names, unless the allocation you pass overrides it:

```json
[{"instance": "gw-0", "host": "box-a"}, {"instance": "gw-1", "host": "box-b"}]
```

A route between hosts is declared in each description it touches, by its
endpoints (`gw-1@box-b`): compose's `x-hale-routes`, a comment for
Kubernetes. With `--out`, each host's file goes into `<dir>/<host>/`. The probes above become a compose `healthcheck` and the
Kubernetes liveness and readiness probes; a `std::observe::Scrape`
becomes a published port and `prometheus.io/*` annotations. A port read
from another param (`port: self.probe_port`) renders as that param's
default, the port the program runs on when nothing overrides it. A program
that holds no `Probes` gets no probe section (a note on stderr says so),
and no autoscaler is rendered, since nothing declares a scale policy.
The compose output mounts the built binaries (`hale build` each seed
first) rather than building an image. Only a program you name is
rendered: a repository with no `.hl` source of its own is refused.

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
