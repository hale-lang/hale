# Build an API

In Hale, you expose a service's methods as an external API by declaring an `api` surface and binding it to a transport with `api::serve`. The compiler generates request/response codecs, contract digests, and self-describing endpoints automatically, allowing CLI tools (`hale describe`, `hale call`, `hale watch`, `hale admin`, and `hale mcp`) to interact with your service without custom client code.

This tutorial builds a complete inventory and order service (`shop.hl`) step by step:

1. **Domain Logic**: Define message types, a bus topic, and a `Shop` locus.
2. **Unix Socket Exposure**: Declare an `api` surface and serve it over `unix::Rpc`.
3. **Read Operations**: Add a state query method with default request fields.
4. **Caller Context**: Inspect the authenticated caller principal and transport via `std::api::Context`.
5. **Role-Based Access Control**: Gate operations with `role` declarations and `std::api::StaticRoles`.
6. **HTTP Exposure**: Serve the same surface over `http::Rpc` with bearer token authentication.
7. **WebSocket Streaming**: Stream bus topics to external subscribers via `ws::Hub`.
8. **Schema Export, MCP & Admin UI**: Generate OpenAPI/JSON Schema specs, expose MCP tools, and launch the local admin dashboard.

For a complete reference on all API surface features, transports, and client generators, see [The API Surface](./api.md). For wire record design patterns, see [Designing API Messages](./api-messages.md).

---

## 1. Define the Domain Locus

Start with a standalone service that processes orders, tracks inventory, and publishes shipment events to the internal bus:

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
        println(
            "order " + to_string(self.stock.orders) + ": " + to_string(req.qty) + " " + req.item,
        );
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

Two parts of `Shop` will form our external API:

- **`Shop::on_order`** takes an `OrderRequest` and returns an `OrderResponse`. When added to an `api` surface, this method becomes a callable RPC endpoint. Domain-level rejections (such as insufficient stock returning `order_id: 0`, or a declared `fallible(E)` error type) are returned as structured responses.
- **`Shipments`** is a published bus topic. When bound to a streaming hub in Step 7, external clients can subscribe to live `Shipment` events.

Note that `App` places `shop` on an `async_io` worker pool (`work`) and runs a sleep loop on the main thread until the process drains, because an `async_io` pool does not keep the process alive by itself.

Run `hale check` to verify the program:

```sh
hale check shop.hl
```

```text
…/shop.hl:10:11: warning: bus topic `Shipments` is published but has no subscriber — the cells go nowhere. Add a `subscribe` for it, bind it to a transport, or drop the publish.
        bus { publish Shipments; }
              ^^^^^^^^^^^^^^^^^^
ok: 1 file(s) typechecked
```

---

## 2. Declare and Serve a Surface over a Unix Socket

An **API surface** (`api <Name> { ... }`) is a table of `rpc` rows naming locus methods. An **exposure** (`api::Exposure`) binds a surface to a network or socket transport inside a locus's `params` block.

Add the `Counter` surface and serve it on `/tmp/shop.sock` using `unix::Rpc`:

```hale,fragment
api Counter {
    rpc Shop::on_order;
}

main locus App {
    params {
        shop: Shop = Shop { };
        sock: api::Exposure = api::serve(Counter, unix::Rpc { path: "/tmp/shop.sock" }, as: "counter", bound: 64, on_full: refuse);
    }
    placement { shop: cooperative(pool = work) where async_io; }
    run() {
        while !self.draining { std::time::sleep(100ms); }
        self.sock.stop();
    }
}
```

Key parameters of `api::serve`:

- **`unix::Rpc { path: "/tmp/shop.sock" }`**: Binds a Unix domain stream socket at startup. If the socket path cannot be bound, the process exits with a diagnostic.
- **`as: "counter"`**: Assigns an exposure name within the application.
- **`bound: 64, on_full: refuse`**: Caps in-flight (accepted but unanswered) requests at 64 and refuses additional requests when full.
- **`self.sock.stop()`**: Completes in-flight requests, refuses queued requests as `shutting_down`, and closes the listener socket cleanly.

### Inspecting the Static API Inventory

Before running the program, pass `--api` to `hale check` to inspect the compiled API inventory and contract digest:

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

The **contract digest** (`fnv1a64:3c6a301bd526600d`) hashes the surface's members, request/response/error type shapes, and role requirements. If a surface changes incompatibly, clients presenting an outdated digest are rejected with `digest_mismatch` before any handler runs.

### Calling the Service from the CLI

Start the program in the background and query its self-description using `hale describe` (short for `hale api describe`):

```sh
hale run shop.hl &
hale describe /tmp/shop.sock
```

```text
Counter@fnv1a64:3c6a301bd526600d/counter  [unix /tmp/shop.sock]
caller: unix uid:1000; roles: none
  Shop::on_order(item: String, qty: Int) -> OrderResponse  requires: -
```

Over a Unix socket, the exposure authenticates the caller using kernel peer credentials (`uid:1000`). Pass `--json` to `hale describe` to view the full JSON description document, including JSON Schemas and outcome encodings.

Use `hale call` (short for `hale api call`) to invoke `Shop::on_order`. You can pass fields as typed CLI flags or as a raw JSON object:

```sh
hale call /tmp/shop.sock Shop::on_order --item lamp --qty 2
# Equivalent JSON form:
# hale call /tmp/shop.sock Shop::on_order '{"item": "lamp", "qty": 2}'
```

```text
{"order_id":1,"on_hand":8}
```

The service logs `order 1: 2 lamp` to stdout and returns the encoded `OrderResponse`.

### Strict Decoding and Exit Codes

Payloads are strictly validated before reaching the handler. Sending a string for an `Int` field in raw JSON triggers a `malformed` refusal at the transport boundary:

```sh
hale call /tmp/shop.sock Shop::on_order '{"item": "lamp", "qty": "two"}'
```

```text
{"request_id":5,"id":"hale-502593","ok":false,"refusal":{"kind":"malformed","reason":"wrong_type: qty"},"caller":{"mode":"unix","name":"uid:1000","uid":1000,"gid":1000,"pid":502593}}
```

`hale call` prints non-success outcomes to stderr and returns a distinct exit code for scripting:

| Exit Code | Outcome | Description |
|---|---|---|
| `0` | `Result` | Call succeeded; response JSON printed to stdout. |
| `1` | `HandlerError` | Handler failed with a declared `fallible(E)` error. |
| `2` | `Refusal` | Exposure refused the call (`malformed`, `unauthorized`, `unauthenticated`, `digest_mismatch`, `full`, `unavailable`, `shutting_down`). |
| `3` | `ServerError` | Handler triggered a `ClosureViolation`. |
| `4` | `Lost` | Connection broke or timed out before receiving a reply. |
| `5` | Client rejection | Rejected locally by `hale call` before sending (e.g., flag type mismatch or member hidden by caller roles). |

---

## 3. Add a Read Operation

Read operations use the same `rpc` syntax as write operations. Add a `stock_now` method to `Shop` with a default field value on `StockRequest`, and register it in `Counter`:

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

Because every field in `StockRequest` has a default value, you can call `Shop::stock_now` with no flags (which sends `{}`):

```sh
hale call /tmp/shop.sock Shop::stock_now
```

```text
{"on_hand":8,"orders":1}
```

The response is a value snapshot copied on `Shop`'s execution pool at the moment the call executes.

---

## 4. Read the Caller Context

To inspect who made a request or which transport delivered it, add a trailing `ctx: std::api::Context` parameter to any RPC handler. `Context` is populated by the exposure and does not appear in the wire request schema:

```hale,fragment
type OrderResponse { order_id: Int; on_hand: Int; by: String; via: String; }

fn on_order(req: OrderRequest, ctx: std::api::Context) -> OrderResponse {
    // ctx.caller.name identifies the caller; ctx.via identifies the transport
    …
    return OrderResponse { order_id: self.stock.orders, on_hand: self.stock.on_hand, by: ctx.caller.name, via: ctx.via };
}
```

Call `Shop::on_order` over the Unix socket:

```sh
hale call /tmp/shop.sock Shop::on_order --item lamp --qty 2
```

```text
{"order_id":1,"on_hand":8,"by":"uid:1000","via":"unix"}
```

Pass `--raw` to see the complete wire envelope returned by the exposure:

```text
{"request_id":2,"id":"hale-502660","ok":true,"value":{"order_id":1,"on_hand":8,"by":"uid:1000","via":"unix"},"caller":{"mode":"unix","name":"uid:1000","uid":1000,"gid":1000,"pid":502660}}
```

`ctx.via` reports the transport (`"unix"`, `"http"`, `"ws"`, `"mcp"`, or `"local"` for direct in-process calls). Both `std::api::Context` and `std::api::Principal` are plain structs that you can construct directly in unit tests.

---

## 5. Gate Operations with Roles

Role requirements are declared in source on `api` rows, while role grants (which principals hold which roles) are supplied at deployment time via a `RoleSource`.

Declare `clerk` and `manager` roles (where `manager includes clerk` grants managers all `clerk` permissions), attach `requires:` clauses to the surface rows, and wire `std::api::StaticRoles` into `unix::Rpc`:

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
        // Reads role grants from `table` param or LOTUS_API_ROLES at runtime (`role=member,member;…`).
        staff: std::api::StaticRoles = std::api::StaticRoles { known: "clerk manager" };
        sock: api::Exposure = api::serve(Counter, unix::Rpc { path: "/tmp/shop.sock", roles: self.staff }, as: "counter", receivers: { Shop: self.shop }, bound: 64, on_full: refuse);
    }
    placement { shop: cooperative(pool = work) where async_io; }
    // …
}
```

`std::api::StaticRoles` parses a semicolon-separated grant table (`role=member,member;...`) from its `table` parameter or the `LOTUS_API_ROLES` environment variable. Supported member patterns are `uid:<n>`, `gid:<n>`, `user:<name>`, `group:<name>`, `bearer:<name>`, and `*` (any authenticated caller). Its `known` field lists valid roles so typos in `LOTUS_API_ROLES` fail at startup. You can also implement custom role resolution by writing any locus with `fn holds(p: std::api::Principal, r: String) -> Bool` (`std::api::RoleSource`).

### Exercising Role Enforcement

1. **No roles granted**: When started with an empty role table, `uid:1000` only sees `Shop::on_order` in the description:

```sh
hale describe /tmp/shop.sock      # lists Shop::on_order only
hale call /tmp/shop.sock Shop::stock_now
```

```text
hale api call: Shop::stock_now is not a member this caller sees (members: Shop::on_order); --json sends a payload as given, and the server decides
```

`hale call` exits with code `5` without sending the request because `Shop::stock_now` is absent from the caller's description. Passing `--json '{}'` bypasses the client-side check and sends the request to the server, which rejects it with exit code `2` (`"refusal":{"kind":"unauthorized","reason":"Shop::stock_now requires clerk","requires":["clerk"]}`).

2. **Granting `clerk`**: Restart with `clerk=uid:1000`. Now `Shop::stock_now` succeeds, while `Shop::on_restock` remains hidden:

```sh
LOTUS_API_ROLES='clerk=uid:1000;manager=' ./shop
```

```text
$ hale call /tmp/shop.sock Shop::stock_now
{"on_hand":10,"orders":0}
$ hale call /tmp/shop.sock Shop::on_restock --qty 5
hale api call: Shop::on_restock is not a member this caller sees (members: Shop::on_order, Shop::stock_now); --json sends a payload as given, and the server decides
```

3. **Granting `manager`**: Because `manager includes clerk`, granting `manager=uid:1000` unlocks both `stock_now` and `on_restock`, and passes the matched role in `ctx.role`:

```sh
LOTUS_API_ROLES='clerk=;manager=uid:1000' ./shop
hale call /tmp/shop.sock Shop::on_restock --qty 5 --raw
```

```text
{"request_id":2,"id":"hale-502313","ok":true,"value":{"on_hand":15,"by":"uid:1000","role":"manager"},"caller":{"mode":"unix","name":"uid:1000","uid":1000,"gid":1000,"pid":502313}}
```

> [!IMPORTANT]
> Role requirements on an `api` surface are enforced at the transport boundary when an external request arrives. They do not restrict internal method calls within your Hale program. To prove internal call-graph isolation for role handlers, use [role groups in `claims` blocks](../claims.md#role-groups).

---

## 6. Serve the Surface over HTTP

To expose the same `Counter` surface over HTTP, define a bearer token authenticator satisfying `std::api::BearerSource` (`principal(token)` and `refused()`) and add an `http::Rpc` exposure:

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
        sock: api::Exposure = api::serve(Counter, unix::Rpc { path: "/tmp/shop.sock", roles: self.staff }, as: "counter", receivers: { Shop: self.shop }, bound: 64, on_full: refuse);
        web: api::Exposure = api::serve(Counter, http::Rpc { bind: "127.0.0.1:8794", codec: json, principals: self.tokens, roles: self.staff }, as: "web", receivers: { Shop: self.shop }, bound: 64, on_full: refuse);
    }
    // …
}
```

`Counter` is now served on two exposures (`counter` on Unix socket and `web` on HTTP) sharing the same contract digest and `staff` role table. Over HTTP:

- Calls use `POST /call/<member>` with `Authorization: Bearer <token>` and `Hale-Surface-Digest: <digest>`.
- `GET /.description` returns the caller-filtered description document.
- You can pass the token via `--bearer <token>` or the `HALE_API_BEARER` environment variable.

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

---

## 7. Stream Bus Topics over a WebSocket Hub

To stream the `Shipments` bus topic to external subscribers, add a `ws::Hub` parameter to `App` and bind `Shipments` to it in a `bindings { }` block:

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

The binding configures:

- **`requires: [clerk]`**: Only principals holding `clerk` can subscribe to `Shipments`.
- **`bound: 64, on_full: drop_old`**: Buffers up to 64 frames per subscriber and drops the oldest queued frame if a slow subscriber falls behind (valid stream policies are `drop_old` and `drop_new`).

Use `hale watch` to subscribe to live `Shipments` events while orders are placed:

```sh
hale watch ws://127.0.0.1:8796 Shipments --token t-front-desk
```

```text
{"type":"subscribed","topic":"Shipments"}
{"type":"event","topic":"Shipments","seq":1,"payload":{"order_id":1,"item":"lamp","qty":2}}
{"type":"event","topic":"Shipments","seq":2,"payload":{"order_id":2,"item":"chair","qty":1}}
```

Every event offered to a subscription increments `seq`, whether delivered or shed. A gap in `seq` tells the client exactly how many frames its queue dropped.

---

## 8. Export Specs, Serve MCP Tools, and Run the Admin UI

Once a surface is declared, Hale's built-in tooling works directly from the surface definition or live exposure description:

### OpenAPI, JSON Schema, and MCP Manifests

Generate specification documents directly from source without running the binary:

```sh
hale check --api shop.hl --surface Counter --openapi
hale check --api shop.hl --surface Counter --json-schema
hale check --api shop.hl --surface Counter --mcp
```

### Bridging to an MCP Host (`hale mcp --app`)

`hale mcp --app` bridges a live Unix, HTTP, or MCP exposure to an MCP host over stdio, exposing each authorized RPC member as an MCP tool:

```sh
claude mcp add shop -- hale mcp --app http://127.0.0.1:8794 --token t-front-desk
```

When the MCP host queries `tools/list`, it receives only the tools that `t-front-desk` is authorized to call:

```text
{"id":1,"jsonrpc":"2.0","result":{"tools":[{"description":"rpc Shop::on_order of Counter. Requires no role under the exposure's role source; the server still authorizes every request.","inputSchema":{"properties":{"item":{"type":"string"},"qty":{"type":"integer"}},"required":["item","qty"],"type":"object"},"name":"Shop__on_order","x-hale-requires":[]},{"description":"rpc Shop::stock_now of Counter. Requires clerk under the exposure's role source; …","name":"Shop__stock_now","x-hale-requires":["clerk"]}]}}
```

### Interactive Admin Dashboard (`hale admin`)

Launch a local loopback web UI with generated request forms for every authorized RPC member and live event tails for every stream:

```sh
hale admin /tmp/shop.sock --port 7474
```

```text
hale admin: http://127.0.0.1:7474/?token=03ea1263780059750082ce32e8900ab1  (over /tmp/shop.sock)
```

---

## Complete Program

The full source for `shop.hl` (tested in the compiler suite at `crates/hale-codegen/tests/fixtures/examples/92-build-an-api/main.hl`):

```hale,fragment
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

api Counter {
    rpc Shop::on_order;
    rpc Shop::stock_now requires: [clerk];
    rpc Shop::on_restock requires: [manager];
}

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
        hub: ws::Hub = ws::Hub { bind: "127.0.0.1:8796", principals: self.tokens, roles: self.staff, as: "shipments" };
        sock: api::Exposure = api::serve(Counter, unix::Rpc { path: "/tmp/shop.sock", roles: self.staff }, as: "counter", receivers: { Shop: self.shop }, bound: 64, on_full: refuse);
        web: api::Exposure = api::serve(Counter, http::Rpc { bind: "127.0.0.1:8794", codec: json, principals: self.tokens, roles: self.staff }, as: "web", receivers: { Shop: self.shop }, bound: 64, on_full: refuse);
    }
    placement { shop: cooperative(pool = work) where async_io; }
    bindings {
        Shipments: self.hub requires: [clerk], bound: 64, on_full: drop_old;
    }
    run() {
        while !self.draining { std::time::sleep(100ms); }
        self.sock.stop();
        self.web.stop();
    }
}

fn main() {
    App { };
}
```

## Related Reference

- **API Surfaces, Transports & Clients**: [The API Surface](./api.md) and [`spec/api.md`](https://github.com/hale-lang/hale/blob/main/spec/api.md).
- **Wire Record Design & Evolution**: [Designing API Messages](./api-messages.md).
- **Role Declarations & Hierarchy**: [`spec/types.md` § Roles](https://github.com/hale-lang/hale/blob/main/spec/types.md#roles-gh-1109-1417).
- **`std::api` Types & Interfaces**: [`spec/stdlib.md`](https://github.com/hale-lang/hale/blob/main/spec/stdlib.md) (`Principal`, `Context`, `RoleSource`, `BearerSource`, `StaticRoles`).
