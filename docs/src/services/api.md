# The API binding

A service that is authoritative over something ends up wanting a
surface that tools can plug into: a command line, a dashboard, an
MCP host. You could write an HTTP server, a JSON codec per message
and a routing table for it. You do not have to.

A program says what it exposes in rows: a **surface** is a table of
operations, each naming a handler and the roles a caller must hold
(`spec/api.md`). The compiler checks the rows, folds each surface into
a contract digest and prints what a caller can learn, without running
anything. Serving a surface is `api::serve`, described below: it runs
today over an in-process test transport, and the Unix socket and HTTP
transports are the next steps of the track. Until they land, the older
path, one `api:` binding entry that puts everything a program declares
on its bus on a Unix socket, is how a program is served over a socket,
and the rest of this chapter documents it. It is the path the surfaces
retire (step R4 of GH #1417).

## Surfaces

An `api` block names a surface and lists its rows. Each `rpc` line is
one row: a member fn of a locus, and the roles a caller needs.

```hale
role trader;

unit cent;
type Money = quantity Int in cent;
type OrderId = distinct Int;

type PlaceOrder { symbol: String; qty: Int; limit: Money; }
type OrderReceipt { order: OrderId; notional: Money; }
type CancelOrder { order: OrderId; }
type Cancelled { order: OrderId; was_open: Bool; }
type OrderError { code: String; reason: String; }

api Public {
    rpc Orders::place;
    rpc Orders::cancel requires: [trader];
}

locus Orders {
    params {
        next: Int = 41;
        open: Int = 0;
    }
    closure position_limit { captures: open; epoch inline; }

    fn place(o: PlaceOrder) -> OrderReceipt fallible(ClosureViolation) {
        if o.qty > 10000 { violate position_limit; }
        let id = OrderId(self.next);
        self.next = self.next + 1;
        self.open = self.open + 1;
        return OrderReceipt { order: id, notional: o.limit * o.qty };
    }

    fn cancel(c: CancelOrder, ctx: std::api::Context) -> Cancelled fallible(OrderError) {
        let n = Int(c.order);
        if n < 41 || n >= self.next {
            fail OrderError { code: "unknown_order", reason: "no order " + to_string(n) };
        }
        return Cancelled { order: c.order, was_open: true };
    }
}

fn main() { }
```

A handler takes its request as one parameter, and may take a trailing
`ctx: std::api::Context`, which is not part of the request. What it
returns is the response, and its error type decides what a caller sees
when it fails: `cancel` fails with an `OrderError` the caller receives
as the handler error, with that type's schema; `place` may violate a
closure, so (as any value-returning method that may violate) it is
`fallible(ClosureViolation)`, and its failure reaches a caller as the
server error, which carries nothing of the violation.

The same row can be written on the handler instead. `@rpc` contributes
it to the seed's **default surface**, named after the seed:

```hale,fragment
locus Orders {
    @rpc
    fn place(o: PlaceOrder) -> OrderReceipt fallible(ClosureViolation) { … }
    @rpc(requires: [trader])
    fn cancel(c: CancelOrder) -> Cancelled fallible(OrderError) { … }
}
```

Both spellings feed one table, and nothing else about the locus is
read: a handler is an operation because a row says so, not because it
subscribes a topic or returns a value. A handler can sit in several
surfaces, each with its own `requires`, and two versions of one API
over one set of handlers are two surfaces.

### What the check refuses

The rows are checked before anything is served. A row that names no
handler, a handler that takes two requests, a member listed twice in
one surface, a role no `role` declares, a type the JSON codec cannot
carry, and a handler returning nothing that may violate without saying
so are each an error at the row:

```text
rpc `Orders::plcae`: `Orders` declares no fn `plcae`; did you mean `place`?
rpc `Orders::cancel` requires `tradr`, which no `role` declares; did you mean `trader`?
rpc `Ledger::dump`: its response `Export` has a field `raw: Bytes`, which the JSON codec does not carry
rpc `Orders::flush` may violate (`violate stale`) and returns nothing: an rpc handler that may violate is `fallible(ClosureViolation)`, so its caller receives the server error instead of a result
```

### The digest

Each surface folds into a **contract digest**: its rows in member
order, each its member, the shape hashes of its request, response and
error types, and its roles. It moves when a member, a shape, an error
type or a role changes, and for nothing else: not the surface's name,
not where it is served, not the build. A shape hash is deep, so a field
changed inside a nested type moves it too (`spec/model.md` § The shape
of a type). `hale check --dump-model` prints the surfaces with their
digests and rows.

### What a caller can learn

`hale check --api` prints the program's **inventory** from the rows:
every surface with its digest and rows, every place the program serves
one, and the JSON Schema of every type they name.

```text
$ hale check --api desk.hl
{
  "inventory": 1,
  "app": "Desk",
  "surfaces": [
    {
      "name": "Public",
      "digest": "fnv1a64:a8930d6e7998e986",
      "members": [ … ]
    }
  ],
  …
}
```

A caller fetches a **description** scoped to the one exposure it
reaches, filtered by the roles it holds there. What a caller holds is
its role source's to say when the program runs, so the check takes it
as an input:

```text
$ hale check --api desk.hl --exposure public --caller bob
$ hale check --api desk.hl --exposure public --caller alice --holds trader
```

`bob` sees `Orders::place` alone; `alice`, holding `trader`, sees
`Orders::cancel` too. A row whose error type is `ClosureViolation`
lists the string `"ClosureViolation"` in place of an error schema, and
the check says so on stderr beside it.

The same rows project to the other forms a client is generated from,
one per surface, each carrying the digest:

```text
$ hale check --api desk.hl --surface Public --openapi
$ hale check --api desk.hl --surface Public --json-schema
$ hale check --api desk.hl --surface Public --mcp
```

### Serving

A serve site puts one surface on one transport for the instances that
answer it:

```hale,fragment
let public = api::serve(Public, self.fixture, as: "public", receivers: { Orders: self.orders }, bound: 64, on_full: refuse);
```

The transport is a locus that implements `std::api::Rpc`: it listens,
frames a request, and writes each outcome in its protocol's terms. The
standard library ships three today. `std::api::test::Rpc` has no socket: a
test hands it requests and reads the answers, through the same
admission, dispatch and completion every transport uses. `unix::Rpc` is
a Unix socket and `http::Rpc` an HTTP listener. The sources are fields of the
transport's literal, `principals:` for who a bearer token is and
`roles:` for who holds a role; grants belong to the source the serve
site names, never to a role's name, so one program can serve a surface
twice under two sources.

### Serving over a Unix socket

`unix::Rpc` takes a `path:` and a `roles:`. Who is calling is the peer's
kernel credentials, so it has no bearer source: the caller is `uid:1000`,
and a role source says what `uid:1000` holds. The desk of the witness
serves its `Admin` surface to its operator this way:

```hale,fragment
main locus Desk {
    params {
        admin_roles: Grants = Grants { operator: "uid:1000" };
        orders: Orders = Orders { };
        ledger: Ledger = Ledger { };
    }
    run() {
        let admin = api::serve(Admin, unix::Rpc { path: "/run/desk/admin.sock", roles: self.admin_roles }, as: "admin", bound: 16, on_full: refuse);
        while !self.draining { std::time::sleep(100ms); }
        admin.stop();
    }
}
```

The socket is bound when the program boots, so a path it cannot bind (a
missing directory, a socket another program is serving) stops the
program with a diagnostic instead of running without it. (An empty `path`
is not a path it cannot bind: it binds nothing, and the exposure has no
socket. A program that finds its path is another process's to hold passes
none.) Its accept and
read loops run on a pool of their own, which only the main locus can
place, so the serving locus is the main locus; the compiler adds the
listener to it. A client writes one JSON object per line and reads one
per reply, each carrying the `id` it was sent with, the `request_id` the
exposure assigned, and who the exposure took the caller to be:

```text
{"call": "Orders::cancel", "payload": {"order": 41}, "id": "c-1", "digest": "fnv1a64:40381db6685c9f75"}
{"request_id": 1, "id": "c-1", "ok": true, "value": {"order": 41, "was_open": true}, "caller": {"mode": "unix", "name": "uid:1000", "uid": 1000, "gid": 1000, "pid": 4242}}
{"describe": true}
```

A `describe` answers with the exposure's identity and the members this
caller may call: the same rows and the same sources the next request is
checked against, so what it lists is what it admits. A connection that
breaks (an EOF, a line that is not an object, a reply that cannot be
written) is closed alone: the listener and the other connections go on.
`admin.stop()` answers what is executing, refuses what is queued as
`shutting_down`, and closes the socket after the replies. A program that
ends without calling it releases its sockets too, but a caller with a
request in flight then sees the connection end instead of a reply.

The serve site makes an *exposure* when its locus is born, and the
call is the handle: `public.stop()` stops accepting, refuses what is
queued, lets what is executing finish and releases the listener, and
the locus's own teardown does the same if you never call it.
`receivers:` names the instance of each locus the surface's rows
call; when the serving locus holds exactly one instance of a type, it
is inferred, and two with no binding is an error naming both. A
request is checked in a fixed order before the handler's state can tell
it arrived (who is calling, the surface digest, the member, the roles
the row requires, the shape of the payload, then the exposure's
bound), and the caller sees one of five outcomes: the result, the
handler's own error, a refusal (which says whether the call ran: only
`unavailable` and `shutting_down` mean it did not), a server error when
a handler that may violate did, or a broken connection. A handler that
may violate is declared `fallible(ClosureViolation)`; the owner's
`on_failure` runs first and the caller is told only that the server
failed. A receiver that is draining, restarting or replaced is
`unavailable`, per receiver: the exposure's other members serve on. A
handler that wants to know which exposure a call came through, and which
generation of its receiver admitted it, declares `ctx:
std::api::ServedContext` where it would declare `std::api::Context`.

### Serving over HTTP

`http::Rpc` takes a `bind:` (`host:port`), a `codec:` (`json`, the one v1
has), a `principals:` and a `roles:`. A caller is a bearer: the request
carries `Authorization: Bearer <token>`, the `principals:` source says who
the token is (a name it does not know is nobody, and the request is
refused `unauthenticated`), and the `roles:` source says what that name
holds. The witness serves `Public` this way, twice, to two sets of people
on two ports:

```hale,fragment
locus Tokens {
    fn principal(token: String) -> std::api::Principal {
        if token == "t-alice" { return std::api::Principal { mode: "bearer", name: "alice" }; }
        return std::api::Principal { mode: "bearer", name: "" };
    }
    fn refused() -> String { return "no such token"; }
}

main locus Desk {
    params {
        bearer: Tokens = Tokens { };
        public_roles: Grants = Grants { trader: "alice" };
        partner_roles: Grants = Grants { trader: "carol" };
        orders: Orders = Orders { };
        partner_orders: Orders = Orders { next: 9001 };
    }
    run() {
        let public = api::serve(Public, http::Rpc { bind: "127.0.0.1:8080", codec: json, principals: self.bearer, roles: self.public_roles }, as: "public", receivers: { Orders: self.orders }, bound: 64, on_full: refuse);
        let partner = api::serve(Public, http::Rpc { bind: "127.0.0.1:8081", codec: json, principals: self.bearer, roles: self.partner_roles }, as: "partner", receivers: { Orders: self.partner_orders }, bound: 64, on_full: refuse);
        while !self.draining { std::time::sleep(100ms); }
        public.stop();
        partner.stop();
    }
}
```

The listener is bound when the program boots (an address it cannot bind
stops the program with a diagnostic) and, like the Unix socket's, runs on
a pool of its own, so the serving locus is the main locus. A call is
`POST /call/<member>` whose body is the payload, with the surface digest
you generated against in `Hale-Surface-Digest` (it is optional); the
outcome is the status and the body of the contract, one request to a
connection:

```text
POST /call/Orders::place HTTP/1.1
Authorization: Bearer t-alice
Content-Type: application/json
Hale-Surface-Digest: fnv1a64:a8930d6e7998e986

{"symbol": "ACME", "qty": 10, "limit": 12500}

HTTP/1.1 200 OK
Content-Type: application/json

{"order":41,"notional":125000}
```

A handler's own error is `422` with the error as the body; a refusal is
`400`, `401`, `403`, `409`, `429` or `503` with `{"refusal": {"kind": …,
"reason": …}}` (a digest mismatch also carries the `served` digest and an
unauthorized call the roles it `requires`); a handler that violated is
`500` and `{"refusal": {"kind": "server"}}`. `GET /.description` answers
the caller's whole description, from the same rows and the same sources the
next call is checked against. A request that is neither is refused
`malformed`; one that is cut off, or whose client goes away, ends its
connection alone, and the listener and the other connections go on.
`public.stop()` refuses what is queued, answers what is executing and
closes the listener after the replies.

### Streams: a topic bound to a hub

A stream is a topic bound to a **hub**, and the binding row carries what a
subscriber is promised: who may read it (`requires`), how many frames its
queue holds (`bound`) and what the queue sheds when it is full (`on_full`,
`drop_old` or `drop_new`; a subscriber that cannot keep up loses events,
never the subscription, so `refuse` is not a stream's policy).

```hale,fragment
main locus Desk {
    params {
        bearer: Tokens = Tokens { };
        hub_roles: Grants = Grants { operator: "dave" };
        hub: ws::Hub = ws::Hub { bind: "127.0.0.1:9000", principals: self.bearer, roles: self.hub_roles, as: "fills" };
    }
    bindings {
        Fills: self.hub requires: [operator], bound: 64, on_full: drop_old;
    }
}
```

The hub is a param: `ws::Hub` speaks WebSocket and `udp::Hub` datagrams.
Its default is the hub (the compiler numbers it and gives it its rows
there), so a construction that supplies the param, `Desk { hub: … }`, is
refused.
Its `bind:` address is bound when the program boots (one it cannot bind
stops the program with a diagnostic), `principals:` and `roles:` are the
two sources, and `as:` names the exposure, once among the program's. The
compiler adds the listener to the main locus and a binding for each
topic, so anything in the program that publishes `Fills` publishes to the
hub's subscribers; an event has to fit the bus frame (65,536 bytes
encoded, header included). `hale check --api` prints the hub as an
exposure of its stream rows, `hub@<stream digest>/fills`, and the
description a caller fetches from the hub's address
(`GET /.description`, with its credential) lists only the streams that
caller may subscribe to.

A client presents its credential at the upgrade (`Authorization: Bearer
<token>`, or `?access_token=<token>`), is named once by the bearer
source, and subscribes with a frame; the hub answers `subscribed` or a
`refusal`, then sends `event`s:

```text
{"type": "subscribe", "topic": "Fills"}
{"type": "subscribed", "topic": "Fills"}
{"type": "event", "topic": "Fills", "seq": 1, "payload": {"order": 1, "qty": 1, "price": 10}}
{"type": "refusal", "topic": "Fills", "refusal": {"kind": "unauthorized", "reason": "Fills requires operator", "requires": ["operator"]}}
```

A caller who may not read a stream is refused and nothing is queued for
it. Every event offered to a subscription takes the next `seq`,
delivered or shed, so a gap in `seq` is exactly the frames its queue shed
(after `drop_new` the gap shows at the next frame that arrives). The
subscription does not outlive its authority: when the credential expires,
or a role source announces a revision (it publishes a `std::api::Revision`
on `"__api.roles.revision"`, or a program tells the hub with
`self.hub.announce(n)`) after which the row's `requires` no longer holds,
the hub sends one `{"type": "unauthorized", "topic": "Fills", "reason":
"expired"}` (or `"revoked"`) and drops what that subscription had queued;
nothing published after the revision reaches a subscriber who lost the
grant. `hub.stop()` sends `{"type": "closed", "reason": "shutting_down"}`
after what was queued, and a connection that ends without it is a
transport failure. The program can ask the hub `subscribers()`,
`connections()` and `events()`.

A hub can also be the transport a surface is served over,
`api::serve(Public, self.hub, as: "desk", …)`: a call is a frame `{"type":
"call", "id": "c1", "call": "Orders::place", "payload": {…}}` on the same
connection, authenticated by the same credential, and the reply is a
`{"type": "reply", "id": "c1", "ok": true, "value": …}` frame between the
events. `udp::Hub` carries streams only: a subscriber is an address and an
`id` it chooses (every datagram carries it, and the credential rides the
subscribe), and a datagram lost on the way is a gap in `seq` like a shed
one. The contract is `spec/api.md` § Streams.

The `bindings { api: … }` entry below is the structural path to a socket
and still works as it did; `api::serve` is the new one.

## The structural path

This chapter explains each piece of the `api:` binding with an example
of its own. To see the pieces combined into one program, built step by
step from an empty file to a gated API, read
[Build an API](./build-an-api.md). Everything the program already
declares on its bus *is* this API, and one line at the deployment tier
hands it out.

## One entry, no other change

Take a billing service as it is: a locus that subscribes a topic,
publishes another, and exposes a field.

```hale,fragment
type Verdict { review_id: Int; verdict: String; }
type VerdictResult { ok: Bool; note: String; }
type PriceMoved { sym: String; price: Float; }
type Ledger { balance: Int; entries: Int; }

topic Verdicts { payload: Verdict; subject: "app.verdict"; }
topic Prices { payload: PriceMoved; subject: "app.price"; }

locus Billing {
    contract { expose ledger: Ledger; }
    params { ledger: Ledger = Ledger { balance: 100, entries: 0 }; }
    bus {
        subscribe Verdicts as on_verdict;
        publish Prices;
    }
    fn on_verdict(v: Verdict) -> VerdictResult {
        self.ledger.entries = self.ledger.entries + 1;
        return VerdictResult { ok: true, note: "ratified " + to_string(v.review_id) };
    }
    run() {
        while !self.draining {
            std::time::sleep(1s);
            Prices <- PriceMoved { sym: "ABC", price: 1.5 };
        }
    }
}

main locus App {
    params { billing: Billing = Billing { }; }
    placement { billing: cooperative(pool = work) where async_io; }
    bindings {
        api: unix("/run/app.sock", bound: 64, on_full: refuse);
    }
    run() { while !self.draining { std::time::sleep(100ms); } }   // serve until SIGTERM
}

fn main() {
    App { };
}
```

The `api:` entry is the whole change. (`App`'s `run()` is what keeps
the program up: `billing` runs on an `async_io` pool, which does not
hold the process open on its own once `main`'s `run()` ends.) It binds every topic a locus
of this seed subscribes as a **command** (`Verdicts`), every topic
such a locus publishes as a **stream** (`Prices`), and every `expose`
of the main locus or of its default children as a **read**
(`billing.ledger`). The handler's return type became the reply:
`on_verdict` returns a `VerdictResult`, so a caller gets one back.
Nothing else in the source knows the socket exists, and a program
without the entry pays nothing for it. What a library you import
does on its own bus is not your API: only the loci of your own seed
are served, so a head that imports a large core never hands out the
core's internal topics as commands (an imported *topic* your locus
subscribes is served under its qualified name, `lib::Orders`).

To serve a library's handler locus as it is, hold it as a param of
your main locus and name it after the transport:

```hale,fragment
import "../../dna/api" as api;    // wherever the libraries sit beside your seed
import "../../dna/core" as dna;

main locus Head {
    params { commands: api::Commands = api::Commands { }; core: dna::Dna = dna::Dna { }; }
    bindings {
        api: unix("/run/head.sock", bound: 64, on_full: refuse), serve: [commands];
    }
}
```

`commands` is on the surface — its handlers are commands and its
gates hold — and `core` is not: holding a locus never serves it,
naming it does. `hale check --dump-api` lists what you named under
`"serve"`.

For a program you are only trying out, skip even that line:

```sh
hale run --api /run/app.sock app.hl
```

puts the same entry on the main locus with the dev defaults. It
needs a `main locus` of your own to put it on; a bare `fn main`
program is refused with the rule, and so is one whose only `main
locus` comes from an import, since a library's bindings never bind
in your program. The path may be a param the program computed
(`api: unix(self.socket, …)` with `App { socket: … }` in `main`), so a
service can listen at one socket per record under `XDG_RUNTIME_DIR`;
`LOTUS_API` overrides whatever the entry says. A socket a live
process already holds is never stolen, and a binding that cannot
listen leaves the rest of the program serving, saying why.

## Talking to it

The socket speaks one JSON object per line. A request is a `call`,
a `read` or a `watch`, with an `id` you choose so you can match the
answer:

```text
{"id": 1, "call": "Verdicts", "payload": {"review_id": 7, "verdict": "ratify"}}
{"id": 2, "read": "billing.ledger"}
{"id": 3, "watch": "Prices"}
```

and each answer carries your `id` plus the binding's own
`request_id`:

```text
{"request_id": 1, "id": 1, "ok": true, "value": {"ok": true, "note": "ratified 7"}}
{"request_id": 2, "id": 2, "ok": true, "value": {"balance": 100, "entries": 1}, "as_of": "sha256:8e5d…"}
{"request_id": 3, "id": 3, "ok": true, "attached": "Prices"}
{"stream": "Prices", "value": {"sym": "ABC", "price": 1.5}}
```

(Each answer also carries the `caller` the binding established; see
[Who is calling](#who-is-calling). Answers come in the order the
program produces them, so match them by `id`.)

A command whose handler has no return type is answered `{"ok":
true, "accepted": true}` the moment it is dispatched: that is what
"accepted" means for this binding, and it is the same promise
`Verdicts <- v` makes in-process. From a shell, `socat` is enough
to try it:

```sh
printf '%s\n' '{"id":1,"call":"Verdicts","payload":{"review_id":7,"verdict":"ratify"}}' \
    | socat - UNIX-CONNECT:/run/app.sock
```

### Over HTTP

A caller that is not on your machine's socket — a browser behind your
web server, another service — reaches the same commands over the
binding's HTTP transport. Name it after the socket, with the locus that
says who a bearer token is (any locus satisfying `std::api::BearerSource`:
`principal(token)` answers the caller, `refused()` the reason a token
naming nobody gets):

```hale,fragment
locus Tokens {
    fn principal(token: String) -> std::api::Principal {
        if token == "t-alice" { return std::api::Principal { mode: "bearer", name: "alice" }; }
        return std::api::Principal { mode: "bearer", name: "" };
    }
    fn refused() -> String { return "no such token"; }
}

main locus App {
    bindings {
        api: unix("/run/app.sock", bound: 64, on_full: refuse),
            http("127.0.0.1", 8793, principals: Tokens { });
    }
}

fn main() { App { }; }
```

Each request is one POST whose body is one line of the same wire,
under `Authorization: Bearer <token>`:

```sh
curl -X POST -H 'Authorization: Bearer t-alice' \
    --data '{"call":"Verdicts","payload":{"review_id":7,"verdict":"ratify"}}' http://127.0.0.1:8793/
```

The answer is the same line, and its HTTP status is the refusal's kind
(200 answered, 401 `unauthenticated`, 403 `unauthorized`, 404
`unknown`, 503 `over_bound`, 400 otherwise). A token your source names
nobody is `unauthenticated`, and a watch stays on the socket. You write
no route and no forwarder: the binding authenticates the token, gates
the call and answers it, exactly as for a socket peer.

## The clients

You never write a client for a Hale program, because the binding
describes itself. `{"describe": true}` on the socket, or `hale
describe app.hl` on the source, returns the same document: every
command with its payload schema and reply type, every read, every
stream, and two notes that say what a gate and a read are and are
not. Four verbs read only that document:

```sh
hale describe /run/app.sock                # the description; --openapi, --mcp for the derived forms
hale call /run/app.sock Verdicts '{"review_id": 7, "verdict": "ratify"}'
hale call /run/app.sock billing.ledger     # a read, with its as_of
hale watch /run/app.sock Prices            # frames, one JSON line each
hale admin /run/app.sock                   # a page on 127.0.0.1:7473 over the description
claude mcp add app -- hale mcp --app /run/app.sock   # every command a tool, every read a resource
```

`hale call` prints the answer and exits 0; a refusal goes to stderr
with its kind and the whole receipt, and exits 1, so a script can
branch on it. `--receipt` prints the whole receipt on stdout
instead of the value alone: the request id, the echoed id, and what
later pieces add to it (the caller, the role that authorized it). `hale
describe app.hl --openapi` writes an OpenAPI 3.1 document with a
path per command, read and stream and every schema under
`components`; `--mcp` writes the tool and resource shapes an MCP
host lists. Both are derived from the description and pinned by a
conformance fixture in the compiler's tests, so a generated document
never drifts from what the binding serves. The description carries
no socket path or deployment detail: it says what the program is,
and where one copy listens is the deployment's business.

## When it says no

A refusal is an answer, never a failure of the program:

```text
{"request_id": 4, "id": 4, "ok": false, "refusal": {"kind": "malformed", "reason": "wrong_type: review_id"}}
```

The kinds are `malformed` (not a JSON object, no verb, or a payload
that does not decode; the reason names the field), `unknown` (no
such topic or read), `not_a_command` (you called a stream),
`not_a_stream` (you watched a command), `over_bound`,
`unauthenticated` (the kernel would not say who you are; nothing is
served to such a peer), and `unauthorized` (you asked for the full
description without `owner`). A gated item you may not use answers
`unknown`, like a name that does not exist: see below. A payload
is decoded before dispatch, strictly: a string where an `Int` is
declared is `wrong_type`, a missing field without a default is
`missing_field`, and the handler only ever sees a value of its
declared type.

## The two knobs

An outside caller is unbounded by nature, so the entry carries a
bound and a policy and the checker insists on both. `bound: 64`
means at most sixty-four commands and reads waiting on a handler
at once; the sixty-fifth caller gets `over_bound` and the program
never sees it. `refuse` is the only policy for requests, because a
caller waiting for an answer cannot be shed silently. Watchers have
their own pair, per connection: `watch_bound: 256, on_watch_full:
drop_old` keeps the newest frames for a slow reader and reports how
many it dropped on the next frame it does get (`{"stream": "Prices",
"dropped": 3}`). Leave both out and a watcher gets `bound` frames
with `drop_old`.

## Reads are snapshots

A read never reaches across threads into a locus's field. It is
answered on the locus's own pool, as a copy taken there, and the
answer says when: `as_of` is a digest of what was answered, so two
reads that agree on it saw the same state and a later command can
say "only if it is still this". A live view is what a stream is
for; the two verbs are different on purpose.

## Who is calling

The binding knows, and a handler can ask. Every answer carries the
principal the binding established for the connection: on the Unix
socket that is the peer's credentials, as the kernel vouches for
them.

```text
{"request_id": 7, "id": 1, "ok": true, "value": {...},
 "caller": {"mode": "unix", "name": "uid:1000", "uid": 1000, "gid": 1000, "pid": 4242}}
```

A handler that wants the caller declares it, and nothing on the
`subscribe` line changes:

```hale
type Refund { order_id: Int; amount: Int; }
type RefundResult { ok: Bool; by: String; }
topic Refunds { payload: Refund; }

locus Billing {
    bus { subscribe Refunds as on_refund; }
    fn on_refund(r: Refund, ctx: std::api::Context) -> RefundResult {
        // ctx.caller is who; ctx.via says "api" through the socket,
        // "http" through the HTTP transport, and "local" for a publish
        // inside the program.
        return RefundResult { ok: true, by: ctx.caller.name };
    }
}
```

The second parameter is `std::api::Context`: the caller, the
request id, `via` (`api` through the socket, `http` through the
binding's HTTP transport — see above — or `local`), and the role
that authorized the message (empty when the operation is not gated). A message that did not
come through the binding hands the handler the local principal, so a
handler never asks whether it was reached from outside; it reads
`via`. `local` says where a message did not come from, never that
it is trusted: a topic bound to another transport in `bindings { }`
cannot take a context handler at all. Both `Context` and `Principal` are ordinary structs: build
one in a test, forward one in a payload. A bearer token for HTTP
callers is the third mode and arrives with the HTTP transport.

## Who may call

The requirement that an operation needs a role is part of the
program, true wherever it runs; who holds the role here is a
deployment fact. So the requirement is written once, on the
operation, and the mapping lives beside the socket path.

```hale,fragment
type Ledger { balance: Int; entries: Int; }
type Refund { order_id: Int; amount: Int; }
type RefundResult { ok: Bool; by: String; }
type Move { amount: Int; }
topic Refunds { payload: Refund; }
topic Moved   { payload: Move; }

role refund_support;
role auditor;
role owner includes refund_support;      // whoever is owner may do what support may

locus Billing {
    params { ledger: Ledger = Ledger { balance: 100, entries: 0 }; }
    contract {
        @gated(role: auditor) expose ledger: Ledger;     // a gated read
    }
    bus {
        subscribe Refunds as on_refund;
        @gated(role: refund_support) publish Moved;      // a gated stream
    }
    @gated(role: refund_support)
    fn on_refund(r: Refund, ctx: std::api::Context) -> RefundResult {
        // ctx.role is the role that authorized this call: "refund_support",
        // or "owner" for an owner, so the handler can write its own audit row.
        Moved <- Move { amount: r.amount };
        return RefundResult { ok: true, by: ctx.caller.name };
    }
}
```

A `role` is declared vocabulary, like `group`: a name nothing
declares is an error, and so is `@gated` on anything but a
subscribed handler, an `expose` or a `publish`, because nothing else
is reached from the binding. `owner` is built in. What `@gated`
means is exactly one thing: a call, a read or a watch **arriving
through the socket** is refused unless the caller holds the role.
It is a gate at the boundary, not a proof about the program's
insides; a handler that calls `refund` from some other path is not
stopped by it, and the description says so in its `notes.gates` so
no client presents a gate as more than it is.

Who holds a role is written in `hale.toml`, per environment:

```toml
[environments.prod.roles]
refund_support = ["group:support-leads"]
auditor        = ["user:audit", "uid:1007"]
owner          = ["user:alice"]
```

`hale build --env prod` (or `hale run --env prod`) bakes that table
into the binding, held by the stdlib's `std::api::StaticRoles` source,
and `hale check --env prod` checks the binding with the same table, so
the check judges the program the build lowers. A member takes one of
six spellings:

| member | matches |
|---|---|
| `uid:<n>` | a socket peer with that uid |
| `gid:<n>` | a socket peer whose primary group, or one of the supplementary groups the kernel reports for the connection, is `<n>` |
| `user:<name>` | a socket peer with that account, resolved once at start per the account database |
| `group:<name>` | a socket peer in that group, resolved the same way |
| `bearer:<name>` | a caller on the [HTTP transport](#over-http) whose bearer source answered exactly `<name>` |
| `*` | any caller the binding authenticates, on either transport |

The two transports' spellings never cross. A `bearer:` member is
never a socket peer, and the four account spellings never match a
bearer caller, even one whose name reads the same: a Unix account and
a token's subject are different identities. A bearer name is the
source's, so it is not looked up in the account database, and it is
written as the source answers it: an OIDC subject such as
`bearer:oidc:auth0|123` included (printable ASCII without blanks or the
table's own `,`, `;` and `=`, at most 255). `LOTUS_API_ROLES="refund_support=uid:1000,bearer:desk;owner=user:alice"`
overrides the table at run time, which is how a test drives it. A table
naming a role the program does not declare, or a member outside
those spellings, is refused at start with the reason, the same rule
`hale check --matrix` holds `hale.toml` to; with no table at all
every gate refuses, and the build tells you. The matrix also insists
that every declared role is mapped in every environment, `[]`
meaning explicitly nobody.

An app can hand the binding its own source instead: a locus satisfying
`std::api::RoleSource` (`fn holds(p: std::api::Principal, r: String) -> Bool`), named on the
entry as an expression the main locus evaluates, so it can be built
with the program's own state and kept as a handle:

```hale,fragment
locus RecordRoles {                      // a std::api::RoleSource
    params { root: String = "."; }
    fn holds(p: std::api::Principal, r: String) -> Bool {
        return r == "owner" && p.name == "uid:1000";   // ask the record at self.root
    }
}

main locus Head {
    params { root: String = "."; roles: RecordRoles = RecordRoles { }; }
    bindings { api: unix("/run/head.sock", bound: 64, on_full: refuse, roles: self.roles); }
    birth() { self.roles.root = self.root; }
}

fn main() { Head { }; }
```

That is how a program whose positions are roles answers from its own
record.

An item you may not use is not shown to you and, if you name it
anyway, is `unknown`, exactly as a name that does not exist would
be: existence is not disclosed to a principal that cannot act on it.
The one refusal that names a role is the full description's:

```text
{"request_id": 9, "id": 3, "ok": false,
 "refusal": {"kind": "unauthorized", "reason": "needs role owner", "role": "owner"},
 "caller": {"mode": "unix", "name": "uid:1000", ...}}
```

An answer names what authorized it: `"role": "owner"` on the receipt,
the same value in `ctx.role`. A stream follows the gate of the topic's
handlers unless its `publish` states its own. `on_unauthorized: drop`
on the entry turns a refusal into silence, for a socket that should
not even answer.

A peer the kernel cannot vouch for (`uid` -1), or a bearer token your
source names nobody, is refused everything, gated or not: the
binding's whole claim is that it knows who is calling.

The description follows the same rule. `{"describe": true}` returns
the caller's slice: the commands, reads and streams it may use, and
only the schemas those need. `hale mcp --app` therefore lists exactly
the tools a principal may call, and `hale admin` shows what it may
reach. The whole document is itself a read, gated on `owner`
(`{"describe": "full"}`, `hale describe --full`); an owner's admin
page shows the rest greyed out with the role each item needs.

## What is left out, and why

- A topic whose payload has a field with no JSON form yet
  (`Decimal`, `Time`, `Duration`, `Bytes`, an array, an enum, a
  locus) stays off the API, with a warning at the entry naming the
  field. Adding the entry never breaks a build.
- A topic two handlers both answer is an error at the entry: one
  reply per command.
- An item another seed declared is described qualified (`api::Claim`);
  a caller may write the bare tail (`Claim`) when exactly one item
  bears it, and gets `unknown` otherwise.
- A `Drain<T>` batch handler is not reached through the binding
  yet; bulk requests wait on batch delivery over the cooperative
  queue.
- A watch over HTTP (a stream to a browser) waits: the HTTP
  transport answers calls, reads and describes, and a watch is the
  socket's.
- Bearer groups wait: the table grants a bearer caller a role by its
  own name (`bearer:<name>`) or through `*`, not by a group its
  source reports (an OIDC `groups` claim). A program that needs that
  today names its own `RoleSource`.
- Transitive privilege inference (flagging `api -> OrderPlaced ->
  on_order -> refund` as an escalation) is not part of `@gated`,
  which is a boundary check and says so.
