# Build an API

This page builds one program from an empty file to a working API with
roles: a socket that `hale call` and `hale watch` drive, a read, a
handler that knows who called it, gates on what only staff may do, an
HTTP transport, and the generated tools on top. Each step adds one
thing and then exercises it from outside the program. The concepts
behind each piece are in [The API binding](./api.md); this page shows
how they fit together.

The program is a small shop. It takes orders, ships what it has, and
keeps a count of its stock. Every output below is what the commands
printed, trimmed where marked `…`. The example's socket is
`/tmp/shop.sock`, and the peer on it is uid 1000.

This page builds the API on the structural path, the `api:` binding.
Surfaces (`api` blocks and `@rpc`, [The API binding](./api.md#surfaces))
are the rows that replace it: the compiler checks them and describes
them today, and when they are served the binding is retired (step R4
of GH #1417), and this page with it.

## 1. The domain

Start with the shop itself: a payload type for each message, a topic
for each kind of message, and a locus that handles one and publishes
the other.

```hale
type Order { item: String; qty: Int; }
type Placed { order_id: Int; on_hand: Int; }
type Shipment { order_id: Int; item: String; qty: Int; }
type Stock { on_hand: Int; orders: Int; }

topic Orders { payload: Order; subject: "shop.order"; }
topic Shipments { payload: Shipment; subject: "shop.shipment"; }

locus Shop {
    params { stock: Stock = Stock { on_hand: 10, orders: 0 }; }
    bus {
        subscribe Orders as on_order;
        publish Shipments;
    }

    fn on_order(o: Order) -> Placed {
        if o.qty <= 0 || o.qty > self.stock.on_hand {
            return Placed { order_id: 0, on_hand: self.stock.on_hand };
        }
        self.stock.on_hand = self.stock.on_hand - o.qty;
        self.stock.orders = self.stock.orders + 1;
        println("order " + to_string(self.stock.orders) + ": " + to_string(o.qty) + " " + o.item);
        Shipments <- Shipment { order_id: self.stock.orders, item: o.item, qty: o.qty };
        return Placed { order_id: self.stock.orders, on_hand: self.stock.on_hand };
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

- `on_order` **returns** a `Placed`. Once there is a binding, a caller
  sending an `Order` gets that value back as the reply, so `Orders` is
  a command with a reply. An order the shop cannot fill still gets an
  answer, `order_id: 0`. A refusal that belongs to your domain is a
  value you return.
- `Shipments` is **published**. Once there is a binding, a caller can
  attach to it and see every shipment, so it is a stream.

`App` places the shop on a pool of its own (`work`, an `async_io`
pool) and keeps the process up with its `run()` loop until it is told
to stop. An `async_io` pool does not keep the process open by itself.

The checker accepts the program. It also says that nothing can reach
it yet:

```sh
hale check shop.hl
```

```text
…/shop.hl:12:9: warning: bus topic `Orders` is subscribed but never published — its handler can't fire. Add a `publish` for it, bind it to a transport, or drop the subscription.
            subscribe Orders as on_order;
            ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
…/shop.hl:13:9: warning: bus topic `Shipments` is published but has no subscriber — the cells go nowhere. Add a `subscribe` for it, bind it to a transport, or drop the publish.
            publish Shipments;
            ^^^^^^^^^^^^^^^^^^
ok: 1 file(s) typechecked
```

The next step binds both topics.

## 2. The binding

Add one entry to the main locus:

```hale,fragment
main locus App {
    params { shop: Shop = Shop { }; }
    placement { shop: cooperative(pool = work) where async_io; }
    bindings {
        api: unix("/tmp/shop.sock", bound: 64, on_full: refuse);
    }
    // run() as before
}
```

Nothing else changes. The entry serves every topic your seed's loci
subscribe, as a command, and every topic they publish, as a stream.
The checker requires both knobs. `bound: 64` allows at most 64
requests to wait on a handler at once. `on_full: refuse` answers the
65th caller `over_bound` instead of queueing it. The two warnings
from step 1 are gone, because both topics are now bound.

Build it and start it:

```sh
hale build shop.hl
./shop
```

```text
built: shop
```

From a second terminal, ask the running program what it serves. You
did not write this document. The binding generates it from the
program:

```sh
hale describe /tmp/shop.sock
```

```text
{"hale_api":1,"app":"App","serve":[],"notes":{…},"commands":[{"name":"Orders","subject":"shop.order","payload":"Order","reply":"Placed","keyed_by":null,"role":null}],"reads":[],"streams":[{"name":"Shipments","subject":"shop.shipment","payload":"Shipment","role":null}],"schemas":{"Order":{"type":"object","properties":{"item":{"type":"string"},"qty":{"type":"integer"}},"required":["item","qty"]},…}}
```

In a third terminal, attach to the stream:

```sh
hale watch /tmp/shop.sock Shipments
```

Now place an order:

```sh
hale call /tmp/shop.sock Orders '{"item": "lamp", "qty": 2}'
```

```text
{
  "on_hand": 8,
  "order_id": 1
}
```

That is `on_order`'s return value. The watcher prints the shipment it
published:

```text
{"stream":"Shipments","value":{"item":"lamp","order_id":1,"qty":2}}
```

and the shop's own terminal shows the `println`:

```text
order 1: 2 lamp
```

The binding decodes a payload before any handler sees it. A field of
the wrong type is refused at the edge: `hale call` prints the receipt
on stderr and exits 1.

```sh
hale call /tmp/shop.sock Orders '{"item": "lamp", "qty": "two"}'
```

```text
hale call: refused: malformed: wrong_type: qty
{"request_id":6,"id":2,"ok":false,"refusal":{"kind":"malformed","reason":"wrong_type: qty"},"caller":{"mode":"unix","name":"uid:1000","uid":1000,"gid":1000,"pid":3093368}}
```

An order the shop cannot fill reaches the handler, and the handler's
answer comes back as an ordinary value with exit 0:

```sh
hale call /tmp/shop.sock Orders '{"item": "chair", "qty": 50}'
```

```text
{
  "on_hand": 8,
  "order_id": 0
}
```

## 3. A read

Callers want to see the stock without placing an order. Expose the
field in the shop's contract:

```hale,fragment
locus Shop {
    contract {
        expose stock: Stock;
    }
    // params, bus and on_order as before
}
```

An exposed member of a param-default child is a read, named
`<param>.<member>`, here `shop.stock`. `hale call` reads it as well:

```sh
hale call /tmp/shop.sock shop.stock
```

```text
{
  "as_of": "sha256:b3e031a99621c97a9e7c731070a1231dcee3463cdd3af0b6e01ef1265435e3d0",
  "value": {
    "on_hand": 10,
    "orders": 0
  }
}
```

The shop's own pool takes a copy of the field, so the answer is a
snapshot of that instant. `as_of` is a digest of the answered value.
Place an order and read twice:

```sh
hale call /tmp/shop.sock Orders '{"item": "lamp", "qty": 2}'
hale call /tmp/shop.sock shop.stock
hale call /tmp/shop.sock shop.stock
```

```text
{
  "on_hand": 8,
  "order_id": 1
}
{
  "as_of": "sha256:51e6be009957dab2267c3f5e3b7786a4016d1acdf414ce5e7315627f3aa42bec",
  "value": {
    "on_hand": 8,
    "orders": 1
  }
}
{
  "as_of": "sha256:51e6be009957dab2267c3f5e3b7786a4016d1acdf414ce5e7315627f3aa42bec",
  "value": {
    "on_hand": 8,
    "orders": 1
  }
}
```

The digest changed when the stock changed and stayed the same while it
did not. Two reads that agree on `as_of` saw the same state. For a live
view, use a stream: `hale watch` follows changes as they happen.

## 4. Who is calling

To know who called, a handler takes a second parameter,
`ctx: std::api::Context`. The `subscribe` line does not change. Have
the reply say who placed the order and how it arrived:

```hale,fragment
type Placed { order_id: Int; on_hand: Int; by: String; via: String; }

locus Shop {
    // contract, params and bus as before

    fn on_order(o: Order, ctx: std::api::Context) -> Placed {
        if o.qty <= 0 || o.qty > self.stock.on_hand {
            return Placed { order_id: 0, on_hand: self.stock.on_hand, by: ctx.caller.name, via: ctx.via };
        }
        self.stock.on_hand = self.stock.on_hand - o.qty;
        self.stock.orders = self.stock.orders + 1;
        println("order " + to_string(self.stock.orders) + ": " + to_string(o.qty) + " " + o.item + " for " + ctx.caller.name + " via " + ctx.via);
        Shipments <- Shipment { order_id: self.stock.orders, item: o.item, qty: o.qty };
        return Placed { order_id: self.stock.orders, on_hand: self.stock.on_hand, by: ctx.caller.name, via: ctx.via };
    }
}
```

To compare an outside caller with a local one, have the program place
one order itself when it starts:

```hale,fragment
main locus App {
    params { shop: Shop = Shop { }; }
    placement { shop: cooperative(pool = work) where async_io; }
    bus { publish Orders; }
    bindings {
        api: unix("/tmp/shop.sock", bound: 64, on_full: refuse);
    }
    run() {
        Orders <- Order { item: "window display", qty: 1 };
        while !self.draining { std::time::sleep(100ms); }
    }
}
```

`App` now publishes `Orders`, so `Orders` is also a stream:
`hale watch /tmp/shop.sock Orders` shows every order as it is placed.
Rebuild, restart, and call:

```sh
hale call /tmp/shop.sock Orders '{"item": "lamp", "qty": 2}'
hale call --receipt /tmp/shop.sock Orders '{"item": "lamp", "qty": 1}'
```

```text
{
  "by": "uid:1000",
  "on_hand": 7,
  "order_id": 2,
  "via": "api"
}
{"request_id":4,"id":2,"ok":true,"value":{"order_id":3,"on_hand":6,"by":"uid:1000","via":"api"},"caller":{"mode":"unix","name":"uid:1000","uid":1000,"gid":1000,"pid":3093565}}
```

The shop's terminal shows both kinds of caller:

```text
order 1: 1 window display for local via local
order 2: 2 lamp for uid:1000 via api
order 3: 1 lamp for uid:1000 via api
```

For a peer on the socket, `ctx.caller` is the principal the kernel
vouches for: `mode: "unix"` and `name: "uid:<n>"`, with the uid, gid
and pid. `ctx.via` is `"api"`. For the program's own publish,
`ctx.caller` is the local principal (`name: "local"`, credentials
-1), `via` is `"local"`, and the request id is 0. `ctx.role` is empty
in both cases, because nothing here is gated yet. `--receipt` prints
the whole answer line, which carries the same `caller`. A handler
never asks whether it was reached from outside; it reads `via`. A
return value from a local publish is ignored, so the opening order's
`Placed` goes nowhere.

## 5. Roles

Anyone on the socket may place an order. Restocking is for managers,
and the stock count and the shipment feed are for staff. Declare the
roles, then gate the three operations:

```hale,fragment
type Restock { qty: Int; }
type Restocked { on_hand: Int; by: String; role: String; }

topic Restocks { payload: Restock; subject: "shop.restock"; }

role clerk;
role manager includes clerk;

locus Shop {
    contract {
        @gated(role: clerk) expose stock: Stock;
    }
    params { stock: Stock = Stock { on_hand: 10, orders: 0 }; }
    bus {
        subscribe Orders as on_order;
        subscribe Restocks as on_restock;
        @gated(role: clerk) publish Shipments;
    }

    // on_order as before

    @gated(role: manager)
    fn on_restock(r: Restock, ctx: std::api::Context) -> Restocked {
        self.stock.on_hand = self.stock.on_hand + r.qty;
        return Restocked { on_hand: self.stock.on_hand, by: ctx.caller.name, role: ctx.role };
    }
}
```

A `role` is declared vocabulary, so a misspelled name is an error.
`manager includes clerk` means a manager may do whatever a clerk may.
`@gated(role: R)` is allowed in three places: a subscribed handler
(checked on every call), an `expose` (checked on every read), and a
`publish` (checked once, when a watcher attaches). It gates what
arrives through the binding, and nothing else. A call to `on_restock`
from inside the program is not checked.

The program says which operation needs which role. Which accounts hold
each role depends on the deployment, so that mapping goes in
`hale.toml`, per environment:

```toml
[claims]
no_base = true

[environments.dev]
source_only = true
entrypoints = ["."]

[environments.dev.roles]
clerk   = ["*"]
manager = ["uid:1000"]
owner   = []
```

A member names a socket peer by its account (`uid:<n>`, `gid:<n>`,
`user:<name>`, `group:<name>`), a bearer caller by the name its
source gives it (`bearer:<name>`, step 6), or is `*`: any caller the
binding authenticates, on either transport. `owner` is built in, and
`[]` says that nobody holds it here. `hale check --matrix` holds the
table to the program: every role is declared, and every declared role
is mapped:

```sh
hale check --matrix .
```

```text
=== ./. @ dev ===
ok: 1 file(s) typechecked

ok: 1 (entrypoint, environment) pair(s) checked
```

`hale build --env dev` bakes the table into the binary. Built that
way, uid 1000 is a manager:

```sh
hale build --env dev shop.hl
./shop
hale call --receipt /tmp/shop.sock Restocks '{"qty": 5}'
```

```text
{"request_id":2,"id":2,"ok":true,"value":{"on_hand":14,"by":"uid:1000","role":"manager"},"caller":{"mode":"unix","name":"uid:1000","uid":1000,"gid":1000,"pid":3096860},"role":"manager"}
```

The receipt's `"role"` is the role that authorized the call, and the
handler received the same value in `ctx.role`.

Without `--env` there is no table, and the build says so:

```text
note: 3 gated operation(s) and no role table: pass `--env <name>` to bake `[environments.<name>.roles]` from hale.toml, or set LOTUS_API_ROLES at run time; until then every gate refuses
built: shop
```

To try other tables without editing `hale.toml`, set `LOTUS_API_ROLES`
at run time. It overrides the table, in the form
`role=member,member;role=member`. Make uid 1000 a clerk but not a
manager:

```sh
LOTUS_API_ROLES='clerk=uid:1000;manager=' ./shop
```

Restocking is now outside this caller's slice:

```sh
hale call /tmp/shop.sock Restocks '{"qty": 5}'
```

```text
hale call: `Restocks` is neither a command nor a read this caller may use (the binding describes the slice your roles show)
  commands: Orders
  reads: shop.stock
  streams (use `hale watch`): Orders, Shipments
```

That is a refused call. The binding serves each caller only the part
of the description that caller may use, so `hale call` does not find
`Restocks` and does not send the request. A client that sends it anyway
gets a refusal of kind `unknown`, the same answer as for a name that
does not exist (step 6 shows one). A caller is not told that an
operation it may not use exists. The clerk can read the stock, and the
receipt names the role that authorized the read:

```sh
hale call --receipt /tmp/shop.sock shop.stock
```

```text
{"request_id":4,"id":2,"ok":true,"value":{"on_hand":9,"orders":1},"as_of":"sha256:bec792543f45001d67a7bb112af81762706f3d776405c27d140259ccbdffe3ca","caller":{"mode":"unix","name":"uid:1000","uid":1000,"gid":1000,"pid":3093613},"role":"clerk"}
```

The one refusal that names a role is the full description, a read
gated on `owner`:

```sh
hale describe --full /tmp/shop.sock
```

```text
hale describe: refused: unauthorized: needs role owner
```

Make uid 1000 a manager and nothing else
(`LOTUS_API_ROLES='clerk=;manager=uid:1000'`). The clerk-gated read
now answers too, authorized by `manager` through `includes`:

```sh
hale call --receipt /tmp/shop.sock Restocks '{"qty": 5}'
hale call --receipt /tmp/shop.sock shop.stock
```

```text
{"request_id":3,"id":2,"ok":true,"value":{"on_hand":14,"by":"uid:1000","role":"manager"},"caller":{"mode":"unix","name":"uid:1000","uid":1000,"gid":1000,"pid":3093627},"role":"manager"}
{"request_id":5,"id":2,"ok":true,"value":{"on_hand":14,"orders":1},"as_of":"sha256:34b10f1467c4a84d855d27c56ea331fdbddf10a16599a46e4b947d26ab3af06b","caller":{"mode":"unix","name":"uid:1000","uid":1000,"gid":1000,"pid":3093628},"role":"manager"}
```

and the manager's `hale watch /tmp/shop.sock Shipments` attaches and
receives frames. With no table at all, every gate refuses:

```sh
./shop      # built without --env, no LOTUS_API_ROLES
hale call /tmp/shop.sock shop.stock
```

```text
hale call: `shop.stock` is neither a command nor a read this caller may use (the binding describes the slice your roles show)
  commands: Orders
  reads: 
  streams (use `hale watch`): Orders
```

## 6. HTTP

A caller that is not on your machine reaches the same API over the
binding's HTTP transport. Add an `http(…)` clause after the socket. It
needs a locus that says who a bearer token is. That locus satisfies
`std::api::BearerSource`: `principal(token)` returns the caller, and
`refused()` returns the reason given when a token names nobody.

```hale,fragment
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
    // params, placement and bus as before
    bindings {
        api: unix("/tmp/shop.sock", bound: 64, on_full: refuse),
        http("127.0.0.1", 8794, principals: Tokens { });
    }
    // run() as before
}
```

Rebuild with the table from step 5 (`hale build --env dev shop.hl`)
and restart. Each HTTP request is one `POST` whose body is one line of
the socket's wire format (`{"call": …}`, `{"read": …}` or
`{"describe": …}`), under `Authorization: Bearer <token>`:

```sh
curl -s -w '\n%{http_code}\n' -X POST -H 'Authorization: Bearer t-front-desk' \
    --data '{"call":"Orders","payload":{"item":"lamp","qty":2}}' http://127.0.0.1:8794/
```

```text
{"request_id":1,"ok":true,"value":{"order_id":2,"on_hand":7,"by":"front-desk","via":"http"},"caller":{"mode":"bearer","name":"front-desk","uid":-1,"gid":-1,"pid":-1,"via":"http"}}
200
```

The handler is unchanged. Its context names the bearer principal, and
`via` is `"http"`. The response body is the reply line, and the
status code is the refusal kind:

| status | when |
|---|---|
| 200 | answered |
| 400 | `malformed` (and any other refusal not listed here) |
| 401 | `unauthenticated`: no bearer, or a token the source names nobody |
| 403 | `unauthorized`: the full description without `owner` |
| 404 | `unknown`: no such item, or one outside the caller's slice |
| 405 | anything but a `POST` |
| 503 | `over_bound` |
| 504 | the program did not answer within 30 s |

A token `Tokens` does not know gets a 401:

```text
{"request_id":0,"ok":false,"refusal":{"kind":"unauthenticated","reason":"the bearer token is refused: no such token"},"caller":{"mode":"bearer","name":"","uid":-1,"gid":-1,"pid":-1,"via":"http"}}
401
```

The gates are the same gates, and the table from step 5 grants roles
to bearer callers too. `clerk = ["*"]` covers every caller the binding
authenticates, so front-desk, named by `Tokens`, holds `clerk` and may
read the stock. The receipt names the role:

```sh
curl -s -w '\n%{http_code}\n' -X POST -H 'Authorization: Bearer t-front-desk' \
    --data '{"read":"shop.stock"}' http://127.0.0.1:8794/
```

```text
{"request_id":2,"ok":true,"value":{"on_hand":7,"orders":2},"as_of":"sha256:c0975c1f95dde12247d8944610c4a1d0573e9aed0aac69c059c25014c0de269c","caller":{"mode":"bearer","name":"front-desk","uid":-1,"gid":-1,"pid":-1,"via":"http"},"role":"clerk"}
200
```

Restocking needs `manager`, which only uid 1000 holds, so it is
outside front-desk's slice:

```sh
curl -s -w '\n%{http_code}\n' -X POST -H 'Authorization: Bearer t-front-desk' \
    --data '{"call":"Restocks","payload":{"qty":5}}' http://127.0.0.1:8794/
```

```text
{"request_id":3,"ok":false,"refusal":{"kind":"unknown","reason":"Restocks"},"caller":{"mode":"bearer","name":"front-desk","uid":-1,"gid":-1,"pid":-1,"via":"http"}}
404
```

To name one bearer caller in the table, write `bearer:` and the name
your source answers for the token. Make front-desk a manager beside
uid 1000:

```toml
[environments.dev.roles]
clerk   = ["*"]
manager = ["uid:1000", "bearer:front-desk"]
owner   = []
```

`hale check --matrix .` accepts it, and a misspelled prefix is still
refused, with the spellings listed:

```text
./hale.toml: environment `dev` role `manager`: `barer:front-desk` is not a role member: write `uid:<n>`, `gid:<n>`, `user:<name>`, `group:<name>`, `bearer:<name>` or `*` (any authenticated caller)
```

Rebuild with `--env dev`, restart, and restock over HTTP:

```sh
curl -s -w '\n%{http_code}\n' -X POST -H 'Authorization: Bearer t-front-desk' \
    --data '{"call":"Restocks","payload":{"qty":5}}' http://127.0.0.1:8794/
```

```text
{"request_id":1,"ok":true,"value":{"on_hand":14,"by":"front-desk","role":"manager"},"caller":{"mode":"bearer","name":"front-desk","uid":-1,"gid":-1,"pid":-1,"via":"http"},"role":"manager"}
200
```

A `bearer:` member is never a socket peer, and `uid:`, `gid:`,
`user:` and `group:` never match a bearer caller, even one whose name
is the same string: a Unix account and a token's subject are
different identities. A bearer `name` is the source's, so the table
does not look it up in the host's account database. A caller the
binding cannot authenticate, such as a token your source names
nobody, holds no role whatever the table says.

The table is the deployment's. When membership depends on the
program's own state instead, name your own source on the entry. A
source is any locus satisfying `std::api::RoleSource`, the program
keeps a handle to it, and it replaces the table for every caller,
socket peers included:

```hale,fragment
locus DeskRoles {
    params { on_shift: String = "front-desk"; }
    fn holds(p: std::api::Principal, r: String) -> Bool {
        return r == "clerk" && p.mode == "bearer" && p.name == self.on_shift;
    }
}

main locus App {
    params { shop: Shop = Shop { }; roles: DeskRoles = DeskRoles { }; }
    // placement and bus as before
    bindings {
        api: unix("/tmp/shop.sock", bound: 64, on_full: refuse, roles: self.roles),
        http("127.0.0.1", 8794, principals: Tokens { });
    }
}
```

The program changes `self.roles.on_shift` as shifts change, and the
gate follows. A watch is only on the socket, because the HTTP
transport answers calls, reads and describes.

## 7. Tools

You still have not written a client. The tools below read only the
description the binding serves, so each caller sees the slice its roles
allow. Run the shop with uid 1000 as a manager.

**OpenAPI.** `hale describe --openapi` writes an OpenAPI 3.1 document
from the source. It has a path per command, read and stream, and each
gate becomes a security requirement:

```sh
hale describe shop.hl --openapi
```

```text
{
  …
  "paths": {
    …
    "/call/Restocks": {
      "post": {
        "operationId": "call.Restocks",
        …
        "security": [
          {
            "role": [
              "manager"
            ]
          }
        ],
        "summary": "command Restocks: publish a Restock on subject shop.restock"
      }
    },
    "/read/shop.stock": {
      "get": {
        "operationId": "read.shop.stock",
        …
        "summary": "read shop.stock: a snapshot of a Stock, with its as_of digest"
      }
    },
    …
```

`--mcp` writes the MCP tool and resource shapes the same way.

**An MCP host.** `hale mcp --app` serves the running program over MCP
on stdio. Each command becomes a tool whose input schema is the
payload's schema, and each read becomes a resource:

```sh
claude mcp add shop -- hale mcp --app /tmp/shop.sock
```

The host's `tools/list` gets the commands this caller may use:

```text
{"id":2,"jsonrpc":"2.0","result":{"tools":[{"description":"Command Orders: sends a Order to the program and returns the Placed its handler answers with.","inputSchema":{"properties":{"item":{"type":"string"},"qty":{"type":"integer"}},"required":["item","qty"],"type":"object"},"name":"Orders"},{"description":"Command Restocks: sends a Restock to the program and returns the Restocked its handler answers with. Needs the role manager; a role gate is a boundary check at the binding, never a proof over the program's internal call paths.","inputSchema":{"properties":{"qty":{"type":"integer"}},"required":["qty"],"type":"object"},"name":"Restocks"}]}}
```

`resources/list` gets `hale://read/shop.stock`. Calling the `Orders`
tool places an order: `{"by":"uid:1000","on_hand":7,"order_id":2,"via":"api"}`.

**A dashboard.** `hale admin` serves a page on the loopback with a
form for each command, a button for each read, and a live tail for each
stream. It prints the URL to open, which carries a token:

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
// streams its shipments, exposes its stock as a read, knows who is
// calling, gates what only staff may do, and answers over HTTP as well
// as on its socket.

type Order { item: String; qty: Int; }
type Placed { order_id: Int; on_hand: Int; by: String; via: String; }
type Shipment { order_id: Int; item: String; qty: Int; }
type Stock { on_hand: Int; orders: Int; }
type Restock { qty: Int; }
type Restocked { on_hand: Int; by: String; role: String; }

topic Orders { payload: Order; subject: "shop.order"; }
topic Restocks { payload: Restock; subject: "shop.restock"; }
topic Shipments { payload: Shipment; subject: "shop.shipment"; }

role clerk;
role manager includes clerk;

locus Shop {
    contract {
        @gated(role: clerk) expose stock: Stock;
    }
    params { stock: Stock = Stock { on_hand: 10, orders: 0 }; }
    bus {
        subscribe Orders as on_order;
        subscribe Restocks as on_restock;
        @gated(role: clerk) publish Shipments;
    }

    fn on_order(o: Order, ctx: std::api::Context) -> Placed {
        if o.qty <= 0 || o.qty > self.stock.on_hand {
            return Placed { order_id: 0, on_hand: self.stock.on_hand, by: ctx.caller.name, via: ctx.via };
        }
        self.stock.on_hand = self.stock.on_hand - o.qty;
        self.stock.orders = self.stock.orders + 1;
        println("order " + to_string(self.stock.orders) + ": " + to_string(o.qty) + " " + o.item + " for " + ctx.caller.name + " via " + ctx.via);
        Shipments <- Shipment { order_id: self.stock.orders, item: o.item, qty: o.qty };
        return Placed { order_id: self.stock.orders, on_hand: self.stock.on_hand, by: ctx.caller.name, via: ctx.via };
    }

    @gated(role: manager)
    fn on_restock(r: Restock, ctx: std::api::Context) -> Restocked {
        self.stock.on_hand = self.stock.on_hand + r.qty;
        return Restocked { on_hand: self.stock.on_hand, by: ctx.caller.name, role: ctx.role };
    }
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
    params { shop: Shop = Shop { }; }
    placement { shop: cooperative(pool = work) where async_io; }
    bus { publish Orders; }
    bindings {
        api: unix("/tmp/shop.sock", bound: 64, on_full: refuse),
        http("127.0.0.1", 8794, principals: Tokens { });
    }
    run() {
        Orders <- Order { item: "window display", qty: 1 };
        while !self.draining { std::time::sleep(100ms); }
    }
}

fn main() {
    App { };
}
```

The compiler's example corpus builds this program on every change, as
`crates/hale-codegen/tests/fixtures/examples/92-build-an-api/main.hl`.

## Where each piece is specified

- The binding: the entry, the two knobs, the wire, refusals, replies,
  reads and the HTTP transport.
  [`spec/semantics.md` § The api binding](https://github.com/hale-lang/hale/blob/main/spec/semantics.md#the-api-binding-gh-1106)
- Roles, `includes` and `@gated`.
  [`spec/types.md` § Roles](https://github.com/hale-lang/hale/blob/main/spec/types.md#roles-gh-1109-1417)
- `std::api`: `Principal`, `Context`, `RoleSource`, `BearerSource` and
  `StaticRoles`, in [`spec/stdlib.md`](https://github.com/hale-lang/hale/blob/main/spec/stdlib.md),
  plus the source, `crates/hale-stdlib/hl/api.hl`.
- The description that `hale describe`, `hale call`, `hale mcp --app`
  and `hale admin` read.
  [`spec/model.md` § The description](https://github.com/hale-lang/hale/blob/main/spec/model.md#the-description-the-models-first-wire-form-gh-1107)
- Each piece explained on its own, with what is left out and why:
  [The API binding](./api.md).
