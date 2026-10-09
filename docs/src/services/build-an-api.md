# Build an API

This page builds one program from an empty file to a working API with
roles: a surface served on a socket that `hale api call` drives, a handler
that knows who called it, rows that name what only staff may do, an HTTP
transport, a stream through a hub, and the tools that read the
description on top. Each step adds one thing and then exercises it from
outside the program. The concepts behind each piece are in
[The API surface](./api.md); this page shows how they fit together.

The program is a small shop. It takes orders, ships what it has, and
keeps a count of its stock. Every output below is what the commands
printed, trimmed where marked `…`. The example's socket is
`/tmp/shop.sock`, and the peer on it is uid 1000. (`hale describe` and
`hale call` are the short forms of `hale api describe` and `hale api
call`: a bare path is `unix:<path>`, and a bare JSON payload after the
member is `--json`. Same output, same exit codes.)

## 1. The domain

Start with the shop itself: a payload type for each message, a topic for
what it publishes, and a locus with the one handler that takes an order.

```hale
type OrderRequest { item: String; qty: Int; }
type OrderResponse { order_id: Int; on_hand: Int; }
type Shipment { order_id: Int; item: String; qty: Int; }
type Stock { on_hand: Int; orders: Int; }

topic Shipments { payload: Shipment; subject: "shop.shipment"; }

locus Shop {
    params { stock: Stock = Stock { on_hand: 10, orders: 0 }; }
    bus { publish Shipments; }

    fn on_order(req: OrderRequest) -> OrderResponse {
        if req.qty <= 0 || req.qty > self.stock.on_hand {
            return OrderResponse { order_id: 0, on_hand: self.stock.on_hand };
        }
        self.stock.on_hand = self.stock.on_hand - req.qty;
        self.stock.orders = self.stock.orders + 1;
        println("order " + to_string(self.stock.orders) + ": " + to_string(req.qty) + " " + req.item);
        Shipments <- Shipment { order_id: self.stock.orders, item: req.item, qty: req.qty };
        return OrderResponse { order_id: self.stock.orders, on_hand: self.stock.on_hand };
    }
}

main locus App {
    params { shop: Shop = Shop { }; }
    placement { shop: cooperative(pool = work) where async_io; }
    run() {
        while !self.draining { std::time::sleep(100ms); }
    }
}

fn main() {
    App { };
}
```

Two shapes here become the API later:

- `on_order` **returns** an `OrderResponse`. Once the handler is a row of a
  surface, a caller sending an `OrderRequest` gets that value back as the
  response. An order the shop cannot fill still gets an answer,
  `order_id: 0`. A refusal that belongs to your domain is a value you
  return (or an error type you declare: [the chapter](./api.md#surfaces)).
- `Shipments` is **published**. Once it is bound to a hub, a caller can
  subscribe to it and see every shipment, so it is a stream.

`App` places the shop on a pool of its own (`work`, an `async_io` pool)
and keeps the process up with its `run()` loop until it is told to stop.
An `async_io` pool does not keep the process open by itself.

The checker accepts the program. It also says that nothing can reach the
published topic yet:

```sh
hale check shop.hl
```

```text
…/shop.hl:10:11: warning: bus topic `Shipments` is published but has no subscriber — the cells go nowhere. Add a `subscribe` for it, bind it to a transport, or drop the publish.
        bus { publish Shipments; }
              ^^^^^^^^^^^^^^^^^^
ok: 1 file(s) typechecked
```

The next step offers the handler.

## 2. A surface, served

A **surface** is a table of operations: one `rpc` row per handler. A
**serve site** puts it on a transport. Add both:

```hale,fragment
api Counter {
    rpc Shop::on_order;
}

main locus App {
    params { shop: Shop = Shop { }; }
    placement { shop: cooperative(pool = work) where async_io; }
    run() {
        let sock = api::serve(Counter, unix::Rpc { path: "/tmp/shop.sock" }, as: "counter", bound: 64, on_full: refuse);
        while !self.draining { std::time::sleep(100ms); }
        sock.stop();
    }
}
```

Nothing else in the source changes: the handler is an operation because
a row says so, not because of what it subscribes. `unix::Rpc` is a Unix
stream socket; `as:` names this exposure of the surface; `bound: 64` is
the most requests it will hold accepted and not yet answered, and
`on_full: refuse` the one policy for the next (a caller waiting for an
answer cannot be shed silently). The socket is bound when the program
boots, so a path it cannot bind stops the program with a diagnostic.
`sock.stop()` answers what is executing, refuses what is queued, and
closes the socket after the replies; a program that ends without it
releases the socket too.

Without running anything, `hale check --api` prints what the program
offers, from the rows:

```sh
hale check --api shop.hl
```

```text
{
  "inventory": 1,
  "app": "App",
  "surfaces": [
    {
      "name": "Counter",
      "digest": "fnv1a64:3c6a301bd526600d",
      "members": [ … ]
    }
  ],
  "exposures": [
    {
      "exposure": "Counter@fnv1a64:3c6a301bd526600d/counter",
      "listener": { "transport": "unix", "address": "/tmp/shop.sock" },
      …
    }
  ],
  …
}
```

The **digest** is the surface's contract: it moves when a member, a
shape or a role changes, and for nothing else. A client that names it
is refused, not run, if the program moved under it.

Run the program, and a client needs no code: it reads the exposure's
description and calls what the description lists.

```sh
hale run shop.hl &
hale describe /tmp/shop.sock
```

```text
Counter@fnv1a64:3c6a301bd526600d/counter  [unix /tmp/shop.sock]
caller: unix uid:1000; roles: none
  Shop::on_order(item: String, qty: Int) -> OrderResponse  requires: -
```

That is the description for the caller the socket established (the
peer's kernel credentials, `uid:1000`): the exposure's identity, where it
listens, who the caller is and what it holds, and a row per member it may
call with its payload fields and result. `hale describe --json` prints the
document as the program wrote it: the same, with the schemas, how an
outcome is encoded, and two notes that say what a role check is and is
not. `hale call` sends one member. Its payload is one flag per field,
typed by the schema the description gave:

```sh
hale call /tmp/shop.sock Shop::on_order --item lamp --qty 2
```

```text
{"order_id":1,"on_hand":8}
```

(`hale call /tmp/shop.sock Shop::on_order '{"item": "lamp", "qty": 2}'`
sends the same payload as JSON.) and the program's own output shows the handler ran:
`order 1: 2 lamp`. The payload is decoded before the handler sees it,
strictly: a string where an `Int` is declared is refused at the edge, and
the handler never runs.

```sh
hale call /tmp/shop.sock Shop::on_order '{"item": "lamp", "qty": "two"}'
```

```text
{"request_id":5,"id":"hale-502593","ok":false,"refusal":{"kind":"malformed","reason":"wrong_type: qty"},"caller":{"mode":"unix","name":"uid:1000","uid":1000,"gid":1000,"pid":502593}}
```

(The outcome is on stderr and the exit code is 2, a refusal; a handler
error is 1, a server error 3, a connection that broke 4, a call refused
before it was sent 5. A script can branch on them. The typed form,
`--qty two`, never reaches the server: it is refused at the client with
the field's schema.) An order the shop cannot fill is not a refusal: it
is the value `on_order` returned, `{"order_id":0,"on_hand":8}`.

## 3. A read

A read is an operation whose handler returns the value. There is no
second kind of row: add a handler and a row.

```hale,fragment
type StockRequest { item: String = ""; }

locus Shop {
    // …
    fn stock_now(req: StockRequest) -> Stock { return self.stock; }
}

api Counter {
    rpc Shop::on_order;
    rpc Shop::stock_now;
}
```

```sh
hale call /tmp/shop.sock Shop::stock_now
```

```text
{"on_hand":8,"orders":1}
```

(A call with no payload sends `{}`, which `StockRequest` accepts because
its one field has a default.) The answer is a copy taken on the shop's
own pool, so it is the state at that instant, and a later write does not
touch it. A live view is what a stream is for (step 7).

## 4. Who is calling

The exposure knows who is calling, and a handler can ask. Declare a
second parameter, `ctx: std::api::Context`; it is not part of the
request:

```hale,fragment
type OrderResponse { order_id: Int; on_hand: Int; by: String; via: String; }

fn on_order(req: OrderRequest, ctx: std::api::Context) -> OrderResponse {
    // ctx.caller.name is who; ctx.via is the door it came through
    …
    return OrderResponse { order_id: self.stock.orders, on_hand: self.stock.on_hand, by: ctx.caller.name, via: ctx.via };
}
```

Over the socket the caller is the peer's kernel credentials, as the
kernel vouches for them:

```sh
hale call /tmp/shop.sock Shop::on_order --item lamp --qty 2
```

```text
{"order_id":1,"on_hand":8,"by":"uid:1000","via":"unix"}
```

`--raw` prints the answer as the exposure wrote it, with the
`request_id` it assigned, the `id` the client sent and the `caller` it
established:

```text
{"request_id":2,"id":"hale-502660","ok":true,"value":{"order_id":1,"on_hand":8,"by":"uid:1000","via":"unix"},"caller":{"mode":"unix","name":"uid:1000","uid":1000,"gid":1000,"pid":502660}}
```

`via` says which door a call came through (`unix`, `http`, `ws`, `mcp`),
and a call that never crossed a transport gets the local caller, so a
handler never asks whether it was reached from outside: it reads `via`.
`Context` and `Principal` are ordinary structs, so a test builds one.

## 5. Roles

That the operation needs a role is part of the program, true wherever it
runs; who holds the role here is a deployment fact. So the requirement
is written once, on the row, and who holds it is a source the serve site
names.

```hale,fragment
role clerk;
role manager includes clerk;

api Counter {
    rpc Shop::on_order;
    rpc Shop::stock_now requires: [clerk];
    rpc Shop::on_restock requires: [manager];
}

main locus App {
    params {
        shop: Shop = Shop { };
        // The role table: LOTUS_API_ROLES at run time, `role=member,member;…`.
        staff: std::api::StaticRoles = std::api::StaticRoles { known: "clerk manager" };
    }
    placement { shop: cooperative(pool = work) where async_io; }
    run() {
        let sock = api::serve(Counter, unix::Rpc { path: "/tmp/shop.sock", roles: self.staff }, as: "counter", receivers: { Shop: self.shop }, bound: 64, on_full: refuse);
        // …
    }
}
```

A role is declared vocabulary: a name nothing declares is an error at the
row, and `manager includes clerk` means whoever is a manager may do what
a clerk may. `std::api::StaticRoles` is the standard library's source: a
table `role=member,member;…`, passed as its `table` param or set at run
time in `LOTUS_API_ROLES`. A member is `uid:<n>`, `gid:<n>`,
`user:<name>`, `group:<name>`, `bearer:<name>` or `*` (any caller the
exposure authenticates); `known` lists the roles the program declares so
a table naming another is refused at start. Any locus satisfying
`std::api::RoleSource` (`fn holds(p: std::api::Principal, r: String) ->
Bool`) works as well, and is how a program whose positions are roles
answers from its own record.

Started with no table, nobody holds anything, and the description
says so: the rows this caller may not call are not listed.

```sh
hale describe /tmp/shop.sock      # members: Shop::on_order only
hale call /tmp/shop.sock Shop::stock_now
```

```text
hale api call: Shop::stock_now is not a member this caller sees (members: Shop::on_order); --json sends a payload as given, and the server decides
```

(The exit code is 5: nothing was sent. `--json '{}'` sends the call
anyway, which is how to see the server's own answer: exit 2,
`"refusal":{"kind":"unauthorized","reason":"Shop::stock_now requires clerk","requires":["clerk"]}`.)

Make uid 1000 a clerk and the read appears; the restock still does not:

```sh
LOTUS_API_ROLES='clerk=uid:1000;manager=' ./shop
```

```text
$ hale call /tmp/shop.sock Shop::stock_now
{"on_hand":10,"orders":0}
$ hale call /tmp/shop.sock Shop::on_restock --qty 5
hale api call: Shop::on_restock is not a member this caller sees (members: Shop::on_order, Shop::stock_now); --json sends a payload as given, and the server decides
```

And as the manager, the restock is in the description and the handler
sees the row's role in `ctx.role`:

```sh
LOTUS_API_ROLES='clerk=;manager=uid:1000' ./shop
hale call /tmp/shop.sock Shop::on_restock --qty 5 --raw
```

```text
{"request_id":2,"id":"hale-502313","ok":true,"value":{"on_hand":15,"by":"uid:1000","role":"manager"},"caller":{"mode":"unix","name":"uid:1000","uid":1000,"gid":1000,"pid":502313}}
```

The client declines to send flags for what the description does not list, but the *server*
authorizes every request: a request for the restock from a caller who
does not hold `manager` is refused `unauthorized`, naming what the row
requires, before the handler is touched (the refusal is the exposure's,
which the client's own check merely spares you; `--json` skips that check). The check is a
boundary check at the serve site; it says nothing about the program's
own call paths, and the description says so in its `notes`.

A peer the kernel cannot vouch for, or a bearer token the source names
nobody for, is refused everything, whatever the roles: the exposure's
whole claim is that it knows who is calling.

## 6. HTTP

A caller that is not on the machine's socket reaches the same surface
over a second serve site. A bearer token is who a caller is on HTTP: a
locus satisfying `std::api::BearerSource` says (`principal(token)`
answers the caller, `refused()` the reason a token naming nobody gets):

```hale,fragment
locus Tokens {
    fn principal(token: String) -> std::api::Principal {
        if token == "t-front-desk" {
            return std::api::Principal { mode: "bearer", name: "front-desk" };
        }
        return std::api::Principal { mode: "bearer", name: "" };
    }
    fn refused() -> String { return "no such token"; }
}

main locus App {
    params {
        shop: Shop = Shop { };
        tokens: Tokens = Tokens { };
        staff: std::api::StaticRoles = std::api::StaticRoles { known: "clerk manager" };
    }
    // …
    run() {
        let sock = api::serve(Counter, unix::Rpc { path: "/tmp/shop.sock", roles: self.staff }, as: "counter", receivers: { Shop: self.shop }, bound: 64, on_full: refuse);
        let web = api::serve(Counter, http::Rpc { bind: "127.0.0.1:8794", codec: json, principals: self.tokens, roles: self.staff }, as: "web", receivers: { Shop: self.shop }, bound: 64, on_full: refuse);
        // …
    }
}
```

The one surface is now two exposures, `counter` and `web`: one digest, two
descriptions, and the same role source here (so `bearer:front-desk` can
be given a role beside `uid:1000`). Each request is a `POST /call/<member>`
whose body is the payload, under `Authorization: Bearer <token>`, with the
digest in `Hale-Surface-Digest`; `GET /.description` answers the caller's
document. The clients do both for you:

```sh
LOTUS_API_ROLES='clerk=bearer:front-desk;manager=' ./shop
hale describe http://127.0.0.1:8794 --bearer t-front-desk
hale call http://127.0.0.1:8794 Shop::stock_now --bearer t-front-desk
hale call http://127.0.0.1:8794 Shop::on_order --item lamp --qty 1 --bearer t-front-desk
hale call http://127.0.0.1:8794 Shop::on_order --item lamp --qty 1 --bearer t-nobody
```

```text
Counter@fnv1a64:01c0c908d101b3bf/web  [http 127.0.0.1:8794]
caller: bearer front-desk; roles: clerk
  Shop::on_order(item: String, qty: Int) -> OrderResponse  requires: -
  Shop::stock_now(item?: String) -> Stock  requires: clerk
{"on_hand":10,"orders":0}
{"order_id":1,"on_hand":9,"by":"front-desk","via":"http"}
hale api call: refused: {"refusal":{"kind":"unauthenticated","reason":"no such token"}}
```

(`--bearer` names the caller; `HALE_API_BEARER` sets it for a session.
The exit code for the last is 2. Over HTTP the status is the contract's:
200 for a result, 422 for the handler's error, and 400, 401, 403, 409,
429 or 503 for a refusal with its kind and reason in the body; `--raw`
prints the body.) The handler saw the caller as `front-desk`, `via:
"http"`, and the program printed `order 1: 1 lamp for front-desk via http`.

## 7. A stream

`Shipments` is published by the shop and bound to a **hub**. The binding
row carries what a subscriber is promised: who may read it (`requires`),
how many frames its queue holds (`bound`) and what the queue sheds when
it is full (`on_full`, `drop_old` or `drop_new`). A subscriber that
cannot keep up loses events, never the subscription.

```hale,fragment
main locus App {
    params {
        // …
        hub: ws::Hub = ws::Hub { bind: "127.0.0.1:8796", principals: self.tokens, roles: self.staff, as: "shipments" };
    }
    bindings {
        Shipments: self.hub requires: [clerk], bound: 64, on_full: drop_old;
    }
    // …
}
```

The hub is a WebSocket listener with the same two sources. Anything in the
program that publishes `Shipments` publishes to the hub's subscribers, and
`hale watch` subscribes:

```sh
hale watch ws://127.0.0.1:8796 Shipments --token t-front-desk
```

```text
{"type":"subscribed","topic":"Shipments"}
{"type":"event","topic":"Shipments","seq":1,"payload":{"order_id":1,"item":"lamp","qty":2}}
{"type":"event","topic":"Shipments","seq":2,"payload":{"order_id":2,"item":"chair","qty":1}}
```

while orders are placed through the surface. Every event offered to a
subscription takes the next `seq`, delivered or shed, so a gap in `seq`
is exactly the frames its queue shed. A caller who holds no `clerk` has no stream in the hub's description,
and `hale watch` says so. (`hale describe` drives a `unix:` or `http://`
exposure; a hub's stream rows are read by `hale watch`.)

## 8. Tools

You still have not written a client. The tools below read only the
description an exposure serves, so each caller sees the slice its roles
allow.

**OpenAPI.** The forms a client is generated from are projections of a
surface's rows, one per surface, each carrying the digest, and they come
from the program without running it:

```sh
hale check --api shop.hl --surface Counter --openapi
hale check --api shop.hl --surface Counter --json-schema
hale check --api shop.hl --surface Counter --mcp
```

The OpenAPI document has a `POST /call/<member>` per row, the responses
by status, the digest as the `Hale-Surface-Digest` header and each
member's `requires` beside it.

**An MCP host.** `hale mcp --app` serves an exposure over MCP on stdio.
Each member the caller may call becomes a tool whose input schema is the
request's schema:

```sh
claude mcp add shop -- hale mcp --app http://127.0.0.1:8794 --token t-front-desk
```

The host's `tools/list` gets the members this caller may use:

```text
{"id":1,"jsonrpc":"2.0","result":{"tools":[{"description":"rpc Shop::on_order of Counter. Requires no role under the exposure's role source; the server still authorizes every request.","inputSchema":{"properties":{"item":{"type":"string"},"qty":{"type":"integer"}},"required":["item","qty"],"type":"object"},"name":"Shop__on_order","x-hale-requires":[]},{"description":"rpc Shop::stock_now of Counter. Requires clerk under the exposure's role source; …","name":"Shop__stock_now","x-hale-requires":["clerk"]}]}}
```

(A program can also serve MCP itself, with `mcp::Rpc`; `hale mcp --app
mcp://host:port` then forwards that listener's own tools.)

**A dashboard.** `hale admin` serves a page on the loopback with a form
for each member and a live tail for each stream. It prints the URL to
open, which carries a token:

```sh
hale admin /tmp/shop.sock --port 7474
```

```text
hale admin: http://127.0.0.1:7474/?token=03ea1263780059750082ce32e8900ab1  (over /tmp/shop.sock)
```

## The whole program

```hale,fragment
// Build an API (docs/src/services/build-an-api.md): one program grown a
// step at a time. A shop takes orders and answers each with a reply,
// streams its shipments, answers a stock query, knows who is calling,
// gates what only staff may do, and answers over HTTP as well as on its
// socket.

type OrderRequest { item: String; qty: Int; }
type OrderResponse { order_id: Int; on_hand: Int; by: String; via: String; }
type Shipment { order_id: Int; item: String; qty: Int; }
type Stock { on_hand: Int; orders: Int; }
type StockRequest { item: String = ""; }
type RestockRequest { qty: Int; }
type RestockResponse { on_hand: Int; by: String; role: String; }

topic Shipments { payload: Shipment; subject: "shop.shipment"; }

role clerk;
role manager includes clerk;

locus Shop {
    params { stock: Stock = Stock { on_hand: 10, orders: 0 }; }
    bus { publish Shipments; }

    fn on_order(req: OrderRequest, ctx: std::api::Context) -> OrderResponse {
        if req.qty <= 0 || req.qty > self.stock.on_hand {
            return OrderResponse { order_id: 0, on_hand: self.stock.on_hand, by: ctx.caller.name, via: ctx.via };
        }
        self.stock.on_hand = self.stock.on_hand - req.qty;
        self.stock.orders = self.stock.orders + 1;
        println("order " + to_string(self.stock.orders) + ": " + to_string(req.qty) + " " + req.item + " for " + ctx.caller.name + " via " + ctx.via);
        Shipments <- Shipment { order_id: self.stock.orders, item: req.item, qty: req.qty };
        return OrderResponse { order_id: self.stock.orders, on_hand: self.stock.on_hand, by: ctx.caller.name, via: ctx.via };
    }

    fn on_restock(req: RestockRequest, ctx: std::api::Context) -> RestockResponse {
        self.stock.on_hand = self.stock.on_hand + req.qty;
        return RestockResponse { on_hand: self.stock.on_hand, by: ctx.caller.name, role: ctx.role };
    }

    fn stock_now(req: StockRequest) -> Stock { return self.stock; }
}

// The shop's operations: who may call each is the row's `requires`.
api Counter {
    rpc Shop::on_order;
    rpc Shop::stock_now requires: [clerk];
    rpc Shop::on_restock requires: [manager];
}

// Who a bearer token on the HTTP transport is (a std::api::BearerSource).
locus Tokens {
    fn principal(token: String) -> std::api::Principal {
        if token == "t-front-desk" {
            return std::api::Principal { mode: "bearer", name: "front-desk" };
        }
        return std::api::Principal { mode: "bearer", name: "" };
    }
    fn refused() -> String { return "no such token"; }
}

main locus App {
    params {
        shop: Shop = Shop { };
        tokens: Tokens = Tokens { };
        // The role table: LOTUS_API_ROLES at run time, `role=member,member;…`.
        staff: std::api::StaticRoles = std::api::StaticRoles { known: "clerk manager" };
        hub: ws::Hub = ws::Hub { bind: "127.0.0.1:8796", principals: self.tokens, roles: self.staff, as: "shipments" };
    }
    placement { shop: cooperative(pool = work) where async_io; }
    bindings {
        Shipments: self.hub requires: [clerk], bound: 64, on_full: drop_old;
    }
    run() {
        let sock = api::serve(Counter, unix::Rpc { path: "/tmp/shop.sock", roles: self.staff }, as: "counter", receivers: { Shop: self.shop }, bound: 64, on_full: refuse);
        let web = api::serve(Counter, http::Rpc { bind: "127.0.0.1:8794", codec: json, principals: self.tokens, roles: self.staff }, as: "web", receivers: { Shop: self.shop }, bound: 64, on_full: refuse);
        while !self.draining { std::time::sleep(100ms); }
        sock.stop();
        web.stop();
    }
}

fn main() {
    App { };
}
```

The compiler's example corpus builds this program on every change, as
`crates/hale-codegen/tests/fixtures/examples/92-build-an-api/main.hl`.

## Where each piece is specified

- The surface, the serve site, the transports, the outcomes, streams and
  the description: [`spec/api.md`](https://github.com/hale-lang/hale/blob/main/spec/api.md)
  is the contract; [The API surface](./api.md) explains each piece on
  its own.
- Roles and `includes`.
  [`spec/types.md` § Roles](https://github.com/hale-lang/hale/blob/main/spec/types.md#roles-gh-1109-1417)
- `std::api`: `Principal`, `Context`, `RoleSource`, `BearerSource` and
  `StaticRoles`, in [`spec/stdlib.md`](https://github.com/hale-lang/hale/blob/main/spec/stdlib.md),
  plus the source, `crates/hale-stdlib/hl/api.hl`.
- The clients (`hale api describe` and `call`, with `hale describe` and
  `hale call` their short forms; `watch`, `admin`, `mcp --app`):
  [`spec/api.md` § The clients](https://github.com/hale-lang/hale/blob/main/spec/api.md#the-clients).
