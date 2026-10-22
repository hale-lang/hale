# The API Surface

Hale programs declare their external RPC and streaming interfaces as **API surfaces** (`api` blocks and `@rpc` annotations). From a surface declaration, the compiler generates strict request/response codecs, computes cryptographic contract digests, serves self-describing endpoints over multiple transports (`unix::Rpc`, `http::Rpc`, `http::Rest`, `mcp::Rpc`, `grpc::Rpc`, `ws::Hub`), and produces typed clients and OpenAPI/Protobuf specifications.

```hale
role trader;

unit cent;
type Money = quantity Int in cent;
type OrderId = distinct Int;

type OrderRequest { symbol: String; qty: Int; limit: Money; }
type OrderResponse { order: OrderId; notional: Money; }
type CancelRequest { order: OrderId; }
type CancelResponse { order: OrderId; was_open: Bool; }
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

    fn place(req: OrderRequest) -> OrderResponse fallible(ClosureViolation) {
        if req.qty > 10000 { violate position_limit; }
        let id = OrderId(self.next);
        self.next = self.next + 1;
        self.open = self.open + 1;
        return OrderResponse { order: id, notional: req.limit * req.qty };
    }

    fn cancel(req: CancelRequest, ctx: std::api::Context) -> CancelResponse fallible(OrderError) {
        let n = Int(req.order);
        if n < 41 || n >= self.next {
            fail OrderError { code: "unknown_order", reason: "no order " + to_string(n) };
        }
        return CancelResponse { order: req.order, was_open: true };
    }
}

fn main() { }
```

For a step-by-step tutorial, see [Build an API](./build-an-api.md). For guidance on designing wire types and evolving schemas safely, see [Designing API Messages](./api-messages.md).

---

## Ecosystem Overview

| Area | Construct / Tool | Purpose |
|---|---|---|
| **Declaring Surfaces** | `api <Name> { rpc Locus::fn; }`<br>`@rpc` / `@rpc(requires: [...])` | Declares a table of callable operations, their required roles, and optional HTTP REST routes. |
| **Serving Exposures** | `api::serve(Surface, Transport, ...)` | Binds a surface to a listener (`unix::Rpc`, `http::Rpc`, `http::Rest`, `mcp::Rpc`, `grpc::Rpc`, `ws::Hub`, or `std::api::test::Rpc`). |
| **Streaming Hubs** | `ws::Hub`, `udp::Hub`<br>`bindings { Topic: self.hub ... }` | Streams internal bus topics to external WebSocket or UDP subscribers with role checks and bounded queues. |
| **Auth & Context** | `role`, `std::api::Context`,<br>`BearerSource`, `RoleSource`, `StaticRoles` | Authenticates callers (Unix kernel credentials or bearer tokens), checks role grants, and passes caller metadata to handlers. |
| **CLI Inspection & Driving** | `hale check --api`, `hale describe`,<br>`hale call`, `hale watch`, `hale admin`, `hale mcp` | Inspects static inventories, queries live descriptions, calls endpoints, tails streams, serves a web admin UI, or bridges to MCP. |
| **Spec & Client Codegen** | `hale api export`<br>`hale api client --lang hale\|ts\|js` | Exports OpenAPI, JSON Schema, MCP, and `.proto` specs, or generates zero-dependency Hale, TypeScript, and JavaScript clients. |
| **Consuming External APIs** | `hale api import`, `api::client`,<br>`VaultPrincipals`, `hale api drift` | Imports external OpenAPI (JSON/YAML) or MCP specs into typed `api::Client` handles tracked by the `client(Surface)` effect class. |

---

## Surfaces

### Declaring `api` Blocks and `@rpc` Methods

An `api` block names a surface and lists its operations as `rpc` rows. Each row references a locus method and optionally specifies role requirements (`requires: [...]`) or HTTP routes (`at "METHOD /path"`).

- **Handler Signature**: Each handler takes one request parameter and may accept a trailing `ctx: std::api::Context` (or `ctx: std::api::ServedContext`), which is supplied by the transport and omitted from the wire request schema.
- **Return Value & Errors**: The return type is the wire response. If the handler is `fallible(E)`:
  - When `E` is a user record or enum (like `OrderError` above), failures are returned to the caller as a structured `HandlerError` matching `E`'s schema.
  - When the handler may `violate` an inline closure, it must declare `fallible(ClosureViolation)`. If a violation occurs, the owner's `on_failure` policy runs first and the caller receives an opaque `ServerError` containing no internal violation state.

Instead of writing a separate `api` block, you can annotate locus methods with `@rpc` to add them to the seed's **default surface** (named after the seed):

```hale,fragment
locus Orders {
    @rpc
    fn place(req: OrderRequest) -> OrderResponse fallible(ClosureViolation) { … }
    @rpc(requires: [trader])
    fn cancel(req: CancelRequest) -> CancelResponse fallible(OrderError) { … }
}
```

Both forms compile to the same internal surface representation. A single handler method can appear in multiple `api` surfaces (for example, to expose different role requirements or versioned surfaces over the same implementation).

### Surface-Level Default Roles & Cross-Seed Roles

When most rows in a surface share the same role requirement, declare `requires:` on the `api` header. Rows without an explicit `requires:` inherit the surface default; a row with `requires: []` overrides the default to allow any authenticated caller:

```hale,fragment
api Admin requires: [operator] {
    rpc Ledger::rebalance;               // requires operator
    rpc Orders::cancel;                  // requires operator
    rpc Orders::status requires: [];     // requires no role (any authenticated caller)
}
```

This produces the exact same surface table and contract digest as spelling `requires: [operator]` on each row.

Roles form a single program-wide vocabulary across seeds. A role declared in an imported library is referenced via its import alias (`requires: [lib::reader]`, `@rpc(requires: [lib::writer])`, or `role boss includes lib::writer;`) and appears in descriptions and refusals under its base name (`reader`, `writer`).

### Compile-Time Surface Validation

`hale check` validates every `api` row at compile time and rejects:

- A row naming a nonexistent locus or method (with a did-you-mean suggestion).
- A handler taking zero or more than one request parameter, or a `Drain<T>` batch handler.
- A method listed more than once in the same surface.
- An undeclared role name in `requires:` (with a did-you-mean suggestion).
- A request, response, or error type containing a field the wire codec cannot serialize (such as `Bytes`, an unbounded `[T]` slice, or a locus).
- A void-returning handler that may `violate` a closure without declaring `fallible(ClosureViolation)`:

```text
rpc `Orders::plcae`: `Orders` declares no fn `plcae`; did you mean `place`?
rpc `Orders::cancel` requires `tradr`, which no `role` declares; did you mean `trader`?
rpc `Ledger::dump`: its response `Export` has a field `raw: Bytes`, which the JSON codec does not carry
rpc `Orders::flush` may violate (`violate stale`) and returns nothing: an rpc handler that may violate is `fallible(ClosureViolation)`, so its caller receives the server error instead of a result
```

### Contract Digests (Surface and Member Digests)

The compiler hashes every surface into a 64-bit FNV-1a **contract digest** (`fnv1a64:<hex>`):

- **Surface Digest**: Computed over all rows in member order, folding each member's name, the recursive shape hashes of its request, response, and error types (`spec/model.md` § The shape of a type), its error classes, its HTTP route (if any), and its required roles. It is unaffected by the surface's name, where it is served, or build flags.
- **Member Digest**: Computed over a single `rpc` row in isolation. Generated clients send the target member's digest on every call, and the server admits any request matching either the surface's current digest or that member's digest.

Because admission checks member digests, adding a new `rpc` row to a surface changes the surface digest without invalidating clients compiled against existing members. Modifying an existing member's request/response types, error classes, route, or roles changes its member digest, causing outdated clients to be rejected with `digest_mismatch`.

### Inspecting the Static API Inventory (`hale check --api`)

Pass `--api` to `hale check` to emit the program's complete API inventory as JSON (all surfaces, digests, exposures, and JSON Schemas) without running the program:

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

You can also simulate the filtered description a specific caller will receive on a named exposure, or export specification formats for a surface:

```sh
# Preview exposure description filtered by caller roles
hale check --api desk.hl --exposure public --caller bob
hale check --api desk.hl --exposure public --caller alice --holds trader

# Export surface specifications to stdout
hale check --api desk.hl --surface Public --openapi
hale check --api desk.hl --surface Public --json-schema
hale check --api desk.hl --surface Public --mcp
hale check --api desk.hl --surface Public --proto
```

---

## Request/Response Codec & Wire Shapes

Every type reachable from an `rpc` row is encoded and decoded strictly. Unknown fields in incoming JSON objects are ignored, while wrong field types, out-of-bounds lists, and missing required fields are rejected with `malformed` before the handler is invoked.

| Hale Type / Pattern | JSON Wire Format | Protobuf (`.proto`) Format | Refusal Conditions (`malformed`) |
|---|---|---|---|
| `Int`, `Float`, `Bool`, `String` | Standard JSON primitives | `optional int64`, `double`, `bool`, `string` | `wrong_type`, `missing_field` (if no default) |
| `distinct T` / `quantity Int in u` | Underlying scalar value (`Int` count for quantities) | Underlying scalar (`// Hale: Type, q(u)`) | `wrong_type` |
| `Time` | UTC RFC 3339 string ending in `Z` (`"2026-10-12T03:04:05.123456789Z"`) | `optional int64` (nanoseconds) | Non-UTC offset, missing `Z`, or past year 2262 (`wrong_type`) |
| `Duration` | Integer nanoseconds (`1500000000` = `1.5s`) | `optional int64` (nanoseconds) | `wrong_type` |
| `bounded[T; N]` | JSON array of `0..=N` elements (optional on input; defaults to `[]`) | `repeated T` | Length > `N` (`too_long`) |
| `[T; N]` | Fixed JSON array of exactly `N` elements | `repeated T` | Length > `N` (`too_long`), length < `N` (`too_short`) |
| `bounded[T; N] indexed_by k` | JSON object keyed by `T.k` (`{"db": {"status": "up"}}`) | `map<string, T>` | Length > `N` (`too_long`), repeated key (`duplicate_key`) |
| `bounded[T; N] indexed_by k -> v` | JSON object mapping `T.k` to `T.v` (`{"db": "ok"}`) | `map<string, V>` | Length > `N` (`too_long`), repeated key (`duplicate_key`) |
| `std::json::Value` | Arbitrary well-formed JSON (<= 64 levels deep, <= 64 KiB) | `optional string` | Invalid JSON, depth > 64, or size > 64 KiB |
| Presence pair (`x` + `has_x: Bool`) | Optional field; omitted or `null` sets `has_x = false` | Both fields declared as-is | `wrong_type` if non-null value fails to decode |
| Clear triple (`x` + `has_x` + `clear_x`) | Omitted -> neither; `null` -> `clear_x = true`; value -> `has_x = true` | All three fields declared as-is | `wrong_type` if non-null value fails to decode |
| `enum { A, B(T), C(T, U) }` | `"A"`, `{"B": val}`, `{"C": [v1, v2]}` | `enum` (unit-only) or `message` with `oneof` | Unknown variant, wrong payload shape/arity (`wrong_type`), `missing_field` |
| Type set (`type U = A \| B;`) | Tagged object by type name (`{"B": {...}}`) | `message` with `oneof` | Unknown variant or wrong inner shape (`wrong_type`) |

### Lists and Pagination

Lists must be fields of a record and have a static bound (`bounded[T; N]` or fixed `[T; N]`). Unbounded `[T]` slices, nested lists, and top-level bare list requests/responses are rejected at compile time.

```hale,fragment
type Item { id: Int; title: String; }
type Page { items: bounded[Item; 50]; more: Bool; }

locus Shelf {
    params { items: bounded[Item; 500]; }
    fn list(q: ListQuery) -> Page {
        let mut page = Page { more: false };
        for item in self.items {
            push(page.items, item) or { page.more = true; break; };
        }
        return page;
    }
}
```

### Keyed Maps (`indexed_by`)

A `bounded[T; N]` list of structs containing a `String` key field can be serialized on the wire as a JSON object using `indexed_by <key_field>`:

```hale,fragment
type Check { name: String; status: String; }
type Health { checks: bounded[Check; 16] indexed_by name; }
```

On the wire, `checks` is encoded as `{"db": {"status": "up"}, "cache": {"status": "down"}}`—each element's `name` field becomes its property key and is omitted from the inner object, then restored in order when decoded.

When the element struct has **only** two fields (the `String` key and a scalar, `Time`, or unit-enum value), use `indexed_by <key> -> <value>` to serialize the values directly without a wrapper object:

```hale,fragment
type Check { name: String; status: String; }
type Health { checks: bounded[Check; 16] indexed_by name -> status; }
// Wire format: {"db": "ok", "nats": "unavailable"}
```

In both Hale and generated TypeScript clients, an `indexed_by` field remains a list of `Check` records (`Check[]` in TypeScript), with the client converting to and from the wire map automatically.

### Free-Form JSON (`std::json::Value`)

Use `std::json::Value` for opaque JSON subtrees that your program passes through or inspects dynamically:

```hale,fragment
type Seat { name: String; config: std::json::Value; }
```

`std::json::Value` preserves the exact bytes sent by the caller (including key ordering, whitespace, and trailing decimal zeros like `2.50`). It must be valid JSON at most 64 levels deep and 64 KiB long. Inside Hale, read `.text` using `std::json` cursor functions (`std::json::find_field_raw(seat.config.text, "theme")`), and construct new values using `std::json::Value::parse(text) or ...` or `std::json::Value::null()` (direct struct literals and mutating `.text` are rejected by the compiler).

### Optional Fields (`has_<field>` and `clear_<field>`)

Because Hale does not have an `Option<T>` wrapper type, optional wire fields use companion boolean flags:

1. **Default Value (`limit: Int = 50;`)**: Use when omitting a field has the same meaning as passing a default value.
2. **Presence Pair (`text: String = ""; has_text: Bool = false;`)**: Use when you must distinguish an omitted/null field from a default value (or distinguish an empty list `[]` from an omitted list). When `text` is omitted or `null`, `has_text` is `false`. On output, `text` is only written when `has_text` is `true`, and `has_text` itself never appears on the wire.
3. **PATCH Clear Triple (`note: String = ""; has_note: Bool = false; clear_note: Bool = false;`)**: Use for PATCH endpoints that distinguish "leave unchanged" (omitted -> both `false`), "clear to null" (`"note": null` -> `clear_note = true`, encoded as `null` on output), and "set to value" (`"note": "hi"` -> `has_note = true`). Marked `"x-hale-null": "clear"` in the schema and typed `note?: string | null` in TypeScript.

### Enums and Type Sets

Enums serialize as a string for unit variants (`"Open"`), a single-key object for single-value variants (`{"Renamed": "Q3 plan"}`), and a single-key object holding an array for multi-value variants (`{"Moved": [3, 7]}`):

```hale,fragment
type Status = enum { Open, Closed };
type Change = enum { Renamed(String), Moved(Int, Int), Archived };
```

Type sets (`type Inbound = NewOrder | Cancel;`) serialize as enums keyed by each member's type name (`{"Cancel": {"id": 1}}`).

### Error Classes

When a handler's error type in `fallible(E)` is an enum, its variants can carry an error class annotation to select the transport status code:

```hale,fragment
type OrderError = enum { @not_found Missing, @conflict Taken(String), Closed };
```

| Error Class Annotation | HTTP Status Code | gRPC Status Code | `Hale-Error-Class` Header |
|---|---|---|---|
| *(unannotated)* | `422 Unprocessable Content` | `FAILED_PRECONDITION` | *(omitted)* |
| `@not_found` | `404 Not Found` | `NOT_FOUND` | `not_found` |
| `@conflict` | `409 Conflict` | `ABORTED` | `conflict` |
| `@invalid` | `422 Unprocessable Content` | `INVALID_ARGUMENT` | `invalid` |
| `@forbidden` | `403 Forbidden` | `PERMISSION_DENIED` | `forbidden` |
| `@unavailable` | `503 Service Unavailable` | `UNAVAILABLE` | `unavailable` |

Because HTTP status codes `403`, `409`, and `503` can also be returned by transport-level refusals, classified handler errors include the `Hale-Error-Class` HTTP header (and `"class"` field on Unix sockets). Error classes are recorded in the schema (`x-hale-error-classes`), passed to custom `std::api::Rpc` transports via `std::api::Outcome::HandlerError(body, class)`, and folded into the contract digest.

### Direct Codec Invocation (`T::api_decode` and `T::api_encode`)

You can call the compiler-generated wire codec directly on any API-eligible record or enum `T`, even outside `api::serve`:

```hale,fragment
let order = PlaceOrder::api_decode(body) or {
    return std::http::Response { status: 400, body: err.kind + ": " + err.field };
};
let reply = PlaceReply::api_encode(desk.place(order));
```

`T::api_decode` applies the exact same validation rules as a served exposure (`wrong_type`, `missing_field`, `too_long`, `too_short`, `duplicate_key`), reporting the offending field name in `err.field` (or `"payload"` when decoding a top-level enum).

---

## Serving Exposures (`api::serve`)

An **exposure** (`api::Exposure`) binds an `api` surface to a transport listener inside a locus's `params` block:

```hale,fragment
params {
    orders: Orders = Orders { };
    fixture: std::api::test::Rpc = std::api::test::Rpc { };
    public: api::Exposure = api::serve(Public, self.fixture, as: "public", receivers: { Orders: self.orders }, bound: 64, on_full: refuse);
}
```

### Lifecycle, Receivers, and Execution Order

- **Declaration & Birth Order**: `api::serve` must be the default initializer of a locus `params` field (calling `api::serve` inside a method body is a compile error). Declare receiver loci *before* the `api::Exposure` param so they are born before the exposure starts accepting calls.
- **Receiver Binding (`receivers:`)**: Maps each locus type referenced in the surface to a locus instance held by the enclosing locus. When the holder has exactly one instance of a receiver type, `receivers:` can be omitted; if two instances of the same type exist, omitting `receivers:` is a compile error.
- **Imported Surfaces**: To serve a surface declared in an imported library seed, pass `lib::SurfaceName` to `api::serve(lib::Svc, ...)`. The live description reports its base name (`Svc`).
- **Lifecycle Controls**:
  - `self.public.wait()` (or `std::api::run_until_stopped(self.public)`) parks until the exposure stops or the process begins draining. If `main locus` omits `run()`, the compiler generates this wait automatically.
  - `self.public.stop()` stops accepting new requests, refuses queued requests as `shutting_down`, waits for in-flight handlers to finish, and closes the listener.
- **Admission Order**: Every incoming request is checked in a fixed sequence before touching receiver state:
  1. Caller authentication (`unauthenticated`)
  2. Surface or member digest match (`digest_mismatch`)
  3. Member existence (`malformed`)
  4. Role authorization (`unauthorized`)
  5. Payload decoding (`malformed`)
  6. Exposure queue capacity (`full` when `bound` is reached under `on_full: refuse`)
- **Per-Receiver Serialization**: Transports read and write connections concurrently, but an exposure dispatches at most one call at a time to each receiver instance in FIFO order. Calls targeting different receiver instances run concurrently. If a receiver is draining, restarting, or replaced, calls to that receiver return `unavailable` while other receivers on the exposure continue serving.

### Listener Placement and Child Server Loci

Socket transport listeners (`unix::Rpc`, `http::Rpc`, `http::Rest`, `mcp::Rpc`, `grpc::Rpc`) run their accept and connection loops on an `async_io` coroutine pool:

- **Direct Placement on `main locus`**: Add a placement row naming the exposure (`public: cooperative(pool = io) where async_io;`). The listener and its connections run on `io`, while the exposure state remains with `main locus`. Unplaced exposures share a default per-transport `async_io` pool.
- **Encapsulated Child Server Loci**: You can group receivers, auth sources, and an `api::Exposure` inside a child server locus and place that child from `main locus`:

```hale,fragment
locus AdminServer {
    params {
        cfg: LiveConfig = LiveConfig { };
        tokens: Tokens = Tokens { };
        api: api::Exposure = api::serve(AdminApi, http::Rpc { bind: "127.0.0.1:8080", codec: json, principals: self.tokens }, as: "web", receivers: { LiveConfig: self.cfg }, bound: 64, on_full: refuse);
    }
}

main locus App {
    params { admin: AdminServer = AdminServer { }; }
    placement { admin: cooperative(pool = io) where async_io; }
}
```

When `admin` is placed on an `async_io` pool, the server locus, its receivers, and its transport listener all run on `io`. A server locus holding an exposure may only be instantiated once (instantiating it twice or with `replicas` is rejected).

> [!NOTE]
> **Dynamic Bind Addresses & Initialization Checks**: If a child server's transport address reads a field of the server locus (e.g., `bind: self.bind`), the compiler births the listener *after* the server finishes initializing so constructor overrides (`App { server: Server { bind: "127.0.0.1:9000" } }`) take effect. Because an unplaced child's `birth()` and `run()` execute on the main thread during `App` construction, the compiler statically verifies that every step prior to listener birth—including omitted param/field defaults, `birth()`, `birth_check`, subscription key expressions, and any unplaced `run()`—returns without blocking, looping (`while`), recursing, yielding, or waiting on the exposure.

---

## RPC Transports

### 1. Unix Domain Sockets (`unix::Rpc`)

`unix::Rpc` serves newline-delimited JSON over a local Unix stream socket and authenticates callers using kernel peer credentials (`uid`, `gid`, `pid`).

```hale,fragment
main locus Desk {
    params {
        admin_roles: Grants = Grants { operator: "uid:1000" };
        orders: Orders = Orders { };
        ledger: Ledger = Ledger { };
        admin: api::Exposure = api::serve(Admin, unix::Rpc { path: "/run/desk/admin.sock", roles: self.admin_roles }, as: "admin", bound: 16, on_full: refuse);
    }
    run() {
        self.admin.wait();
        self.admin.stop();
    }
}
```

- **Parameters**: `path: String` (passing `""` leaves the exposure unbound without a socket) and `roles: RoleSource`.
- **Wire Protocol**: Clients send one JSON object per line and read one JSON object per reply:

```text
{"call": "Orders::cancel", "payload": {"order": 41}, "id": "c-1", "digest": "fnv1a64:40381db6685c9f75"}
{"request_id": 1, "id": "c-1", "ok": true, "value": {"order": 41, "was_open": true}, "caller": {"mode": "unix", "name": "uid:1000", "uid": 1000, "gid": 1000, "pid": 4242}}
{"describe": true}
```

Sending `{"describe": true}` returns the caller-filtered JSON description document.

### 2. HTTP RPC (`http::Rpc`)

`http::Rpc` serves JSON RPC calls over HTTP/1.1 using bearer token authentication:

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
        public: api::Exposure = api::serve(Public, http::Rpc { bind: "127.0.0.1:8080", codec: json, principals: self.bearer, roles: self.public_roles }, as: "public", receivers: { Orders: self.orders }, bound: 64, on_full: refuse);
        partner: api::Exposure = api::serve(Public, http::Rpc { bind: "127.0.0.1:8081", codec: json, principals: self.bearer, roles: self.partner_roles }, as: "partner", receivers: { Orders: self.partner_orders }, bound: 64, on_full: refuse);
    }
    placement { public: cooperative(pool = io) where async_io; }
    run() {
        self.public.wait();
        self.public.stop();
        self.partner.stop();
    }
}
```

- **Parameters**: `bind: "host:port"` (`"127.0.0.1:0"` binds an ephemeral kernel port and reports the assigned port in `/.description`), `codec: json`, `principals: BearerSource`, `roles: RoleSource`, and optional `cors: "origin1, origin2"` (or `"*"`).
- **Endpoints & Headers**:
  - `POST /call/<member>` with `Authorization: Bearer <token>`, `Content-Type: application/json`, and optional `Hale-Surface-Digest: <digest>`.
  - `GET /.description` returns the caller's filtered description document.

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

- **HTTP Status Mapping**:
  - `200 OK`: Handler returned a result.
  - `422` (or `@not_found` `404`, `@conflict` `409`, `@forbidden` `403`, `@unavailable` `503` with `Hale-Error-Class`): Handler returned a declared `fallible(E)` error.
  - `400` (`malformed`), `401` (`unauthenticated`), `403` (`unauthorized`), `409` (`digest_mismatch`), `429` (`full`), `503` (`unavailable` / `shutting_down`): Exposure refused the call; body is `{"refusal": {"kind": "...", "reason": "..."}}`.
  - `500 Internal Server Error`: Handler triggered a `ClosureViolation`; body is `{"refusal": {"kind": "server"}}`.
- **Browser CORS (`cors:`)**: Setting `cors: "https://app.example.com, http://localhost:5173"` (or `"*"`) answers browser `OPTIONS` preflight requests from listed origins with `204 No Content` (`Access-Control-Allow-Origin`, `Access-Control-Allow-Methods: POST, GET, OPTIONS`, `Access-Control-Allow-Headers: Authorization, Content-Type, Hale-Surface-Digest`, `Access-Control-Max-Age: 600`) and attaches `Access-Control-Allow-Origin` to all responses (including refusals) for listed origins. Unlisted origins receive `403` on preflight and no CORS headers.

### 3. HTTP REST Routing (`http::Rest` and `fallback`)

When external callers or load balancers expect custom HTTP methods and URL paths (`GET /healthz`, `GET /v1/items/{id}`, `DELETE /v1/items/{id}`), annotate surface rows with `at "<METHOD> <path>"` and optional `status: <code>`, then serve the surface using `http::Rest`:

```hale,fragment
type Health { status: String; }
type ItemKey { id: Int; }

api Gateway {
    rpc Probe::live at "GET /healthz";
    rpc Items::get at "GET /v1/items/{id}";
    rpc Items::remove at "DELETE /v1/items/{id}" status: 204;
    rpc Items::create at "POST /v1/items" status: 201;
}

locus Svc {
    params {
        bearer: Tokens = Tokens { };
        grants: Grants = Grants { };
        web: api::Exposure = api::serve(Gateway, http::Rest { bind: "0.0.0.0:8080", codec: json, principals: self.bearer, roles: self.grants, anonymous: "public" }, as: "web", receivers: { Probe: self.probe, Items: self.items }, bound: 64, on_full: refuse);
    }
}
```

- **Request Assembly**: `http::Rest` populates the handler's request struct from three sources simultaneously:
  1. Path segment parameters (`{id}` populates field `id`, parsed as the field's declared type).
  2. URL query parameters (`?limit=3` populates scalar field `limit`).
  3. JSON request body fields.
- **Routing & HTTP Semantics**:
  - `status:` sets the HTTP success status (`200` by default; `204` omits the response body).
  - Unmatched paths return `404`; matched paths called with an unsupported method return `405 Method Not Allowed` with an `Allow` header; `HEAD` executes `GET` and omits the body; `OPTIONS` and `cors:` are handled per route.
  - Rows without `at "..."` remain reachable at `POST /call/<member>`.
  - Static segments take precedence over `{param}` segments (`GET /items/open` alongside `GET /items/{id}` is valid), whereas ambiguous overlapping patterns (`GET /p/{a}/x` vs `GET /p/y/{b}`) are rejected at compile time.
  - Routes are folded into both the surface and member contract digests.
- **Cross-Origin Writes**: a row refuses a body that is not `Content-Type: application/json` with `415` (what a cross-site form posts, `text/plain` or form-encoded, with no preflight), and a write with no body (a `DELETE`) from an `Origin` that `cors:` does not list with `403`. Your own clients send the header and no `Origin`; a page of yours on another origin is listed in `cors:`. The fallback is not guarded: it guards itself.
- **Body Cap (`max_request_bytes:`)**: a body is at most 1 MiB unless the transport says more; `max_request_bytes: 12582912` raises it, up to 1 GiB, as `std::http::Server`'s does. A request declaring more is `413` before its body is read. The cap is the listener's, so the fallback has it too.
- **Unauthenticated Probes (`anonymous:`)**: Setting `anonymous: "public"` assigns principal `"public"` when a request omits the `Authorization` header (requests with an invalid token still receive `401`). Protected rows (`requires: [operator]`) still refuse `"public"` unless the role source grants `"public"` that role.

#### Custom Non-RPC Endpoints (`fallback:`)

To serve raw HTTP endpoints (such as Server-Sent Events, raw streaming proxies, or custom WebSocket upgrades) on the **same port and origin** as `http::Rest`, pass a locus implementing `std::http::Handler` (`fn handle(req: std::http::Request) -> std::http::Response`) as `fallback:`:

```hale,fragment
locus DataPlane {
    fn handle(req: std::http::Request) -> std::http::Response {
        if req.path == "/v1/events" {
            let w = std::http::SseWriter { conn_fd: req.conn_fd };
            w.event(std::http::SseEvent { event: "ready", data: "{}" }) or discard;
            return w.end();
        }
        return std::http::Response { status: 404, body: "no such path" };
    }
}

main locus Svc {
    params {
        bearer: Tokens = Tokens { };
        data_plane: DataPlane = DataPlane { };
        web: api::Exposure = api::serve(Gateway, http::Rest { bind: "0.0.0.0:8080", codec: json, principals: self.bearer, fallback: self.data_plane }, as: "web", receivers: { Probe: self.probe, Items: self.items }, bound: 64, on_full: refuse);
    }
    placement {
        web: cooperative(pool = edge) where async_io;
        data_plane: cooperative(pool = edge) where async_io;
    }
}
```

Rules for `fallback`:

1. **Unmatched paths only**: Any path claimed by a surface route stays owned by `http::Rest` for all HTTP methods (e.g., `POST /healthz` on a `GET /healthz` route returns `405`, never reaching `fallback`).
2. **Independent authentication**: `fallback` bypasses the exposure's `principals`, `anonymous`, `roles`, and `cors` checks; it receives the raw `std::http::Request` and performs its own header inspection.
3. **Same pool placement**: `fallback` executes directly on the listener's connection coroutines and must be placed on the exact same `async_io` pool as the exposure (`edge` above).
4. **Field reference only**: When used inside a child server locus, `fallback:` must reference a param field (`self.front`) or `self`, never a method call (`self.make()`).

### 4. Model Context Protocol Server (`mcp::Rpc`)

`mcp::Rpc` exposes a surface directly as an MCP server over HTTP (`POST /mcp` speaking JSON-RPC 2.0):

```hale,fragment
public: api::Exposure = api::serve(Public, mcp::Rpc { bind: "127.0.0.1:8090", principals: self.bearer, roles: self.public_roles }, as: "public", receivers: { Orders: self.orders }, bound: 64, on_full: refuse);
```

- `tools/list` returns the caller's authorized rows as MCP tools, replacing `::` with `__` (`Orders__place`, escaping identifier underscores where necessary).
- `tools/call` validates arguments and invokes the handler:

```text
POST /mcp HTTP/1.1
Authorization: Bearer t-alice

{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"Orders__place","arguments":{"symbol":"ACME","qty":10,"limit":12500}}}

{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"{\"order\":41,\"notional\":125000}"}],"structuredContent":{"order":41,"notional":125000},"isError":false}}
```

- **JSON-RPC Error Codes**: `-32001` (`HandlerError`), `-32602` (`malformed` payload), `-32005` (`unauthorized` role), `-32603` (`ServerError` / closure violation). Unauthenticated requests return HTTP `401`.

### 5. gRPC over HTTP/2 (`grpc::Rpc`)

`grpc::Rpc` serves unary RPCs over cleartext HTTP/2 (powered by built-in `nghttp2` on the `async_io` pool), accepting both Protobuf (`application/grpc` or `application/grpc+proto`) and JSON (`application/grpc+json`) framing on the same listener:

```hale,fragment
public: api::Exposure = api::serve(Public, grpc::Rpc { bind: "127.0.0.1:8070", codec: json, principals: self.bearer, roles: self.public_roles }, as: "public", receivers: { Orders: self.orders }, bound: 64, on_full: refuse);
```

- **Path & Metadata**: Calls use `:path /<Surface>/<Locus>__<method>` (or `/<Surface>/<Locus>.<method>`), `authorization: Bearer <token>`, and optional `hale-surface-digest: <digest>`.

```text
:method POST   :path /Public/Orders__place   content-type application/grpc+json
authorization Bearer t-alice   hale-surface-digest fnv1a64:a8930d6e7998e986
message {"symbol": "ACME", "qty": 10, "limit": 12500}

:status 200   content-type application/grpc+json
message {"order":41,"notional":125000}
trailers grpc-status 0
```

- **Generated `.proto` & Server Reflection**:
  - `hale api export` (or `hale check --api --surface Public --proto`) generates a proto3 definition with deterministic field numbers matching declaration order and `optional` scalars so missing fields are distinguished from zero values:

```protobuf
service Public {
  // rpc Orders::place
  // requires no role
  // a violation is the server error (ClosureViolation): status 13, no handler error
  rpc Orders__place(OrderRequest) returns (OrderResponse);
}

message OrderRequest {
  optional string symbol = 1;
  optional int64 qty = 2;
  optional int64 limit = 3; // Hale: Money, q(cent)
}
```

  - If two fields in a Hale struct collide under proto3's case-insensitive camelCase rules (`a_b` and `aB`, or `a` and `_a`), the compiler rejects the struct unless one field is renamed with a `` `json:"..."` `` tag. If a record shares the surface's name (`api Todo` with `type Todo`), its proto message is named `TodoMessage`.
  - Every `grpc::Rpc` listener serves **gRPC Server Reflection** (`grpc.reflection.v1`) and `hale.api.Description/Describe` (authenticated by the `authorization` metadata), allowing tools like `grpcurl` to discover schemas without local `.proto` files.
- **gRPC Status & Error Details**:
  - `HandlerError`: Returns `FAILED_PRECONDITION` (or the `@not_found` / `@conflict` / `@invalid` / `@forbidden` / `@unavailable` gRPC status) with a `google.rpc.Status` payload in `grpc-status-details-bin` packing the error message (or JSON) in `Any`.
  - `Refusal`: Returns `INVALID_ARGUMENT`, `FAILED_PRECONDITION` (digest mismatch), `UNAUTHENTICATED`, `PERMISSION_DENIED`, `RESOURCE_EXHAUSTED`, or `UNAVAILABLE` with `HaleRefusal` in `grpc-status-details-bin`.
  - `ServerError`: Returns `INTERNAL`.
  - Calling `public.stop()` drains in-flight calls, sends `GOAWAY` on all HTTP/2 connections, and closes the listener. Calling `std::io::tcp::stop("127.0.0.1:8070")` closes the listener while allowing active connections to continue.

---

## Streaming Hubs (`ws::Hub` and `udp::Hub`)

To stream internal bus topics to external subscribers, declare a `ws::Hub` (WebSocket) or `udp::Hub` (UDP datagrams) in `params` and bind topics to it in a `bindings { }` block:

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

### Hub Protocol and Queue Semantics

- **Configuration**: A hub param must keep its default constructor literal (overriding `Desk { hub: ... }` is rejected). Published events must fit within the 65,536-byte bus frame limit (including header).
- **Queue Shedding (`on_full`)**: Stream bindings require `on_full: drop_old` or `on_full: drop_new` (`refuse` is not valid for streams because slow subscribers shed events rather than blocking publishers). Every event offered to a subscription increments `seq`, so any gap in received `seq` numbers indicates the exact number of shed frames.
- **WebSocket Frame Protocol**: Clients authenticate during the HTTP upgrade (`Authorization: Bearer <token>` or `?access_token=<token>`) and send a subscribe frame:

```text
{"type": "subscribe", "topic": "Fills"}
{"type": "subscribed", "topic": "Fills"}
{"type": "event", "topic": "Fills", "seq": 1, "payload": {"order": 1, "qty": 1, "price": 10}}
{"type": "refusal", "topic": "Fills", "refusal": {"kind": "unauthorized", "reason": "Fills requires operator", "requires": ["operator"]}}
```

- **Live Credential Expiry & Role Revocation**: If a subscriber's credential expires, or if the role source announces a policy revision (by publishing `std::api::Revision` to `"__api.roles.revision"` or calling `self.hub.announce(n)`) under which the subscriber no longer holds the required role, the hub immediately sends `{"type": "unauthorized", "topic": "Fills", "reason": "expired"}` (or `"revoked"`) and drops all queued frames for that subscription.
- **Inspection & Shutdown**: Call `self.hub.subscribers()`, `self.hub.connections()`, and `self.hub.events()` to inspect hub metrics. Calling `self.hub.stop()` flushes queued events and sends `{"type": "closed", "reason": "shutting_down"}`.
- **Multiplexing RPC over `ws::Hub`**: You can also pass a `ws::Hub` as the transport in `api::serve(Public, self.hub, as: "desk", ...)`. Clients can then send `{"type": "call", "id": "c1", "call": "Orders::place", "payload": {...}}` frames and receive `{"type": "reply", "id": "c1", "ok": true, "value": ...}` frames interleaved with stream events on the same WebSocket connection.
- **UDP Streams (`udp::Hub`)**: Carries streams only. Subscribers send a UDP datagram with a client-chosen `id` and bearer token; lost datagrams appear as gaps in `seq`.

---

## Authentication, Authorization & Refusals

### Refusal Kinds

A transport refusal rejects a request before the handler runs:

```text
{"request_id": 4, "id": 4, "ok": false, "refusal": {"kind": "malformed", "reason": "wrong_type: qty"}}
```

| Refusal Kind | HTTP Status | gRPC Status | Cause |
|---|---|---|---|
| `malformed` | `400` | `INVALID_ARGUMENT` | Invalid JSON/frame, unknown member, missing required field (`missing_field`), wrong field type (`wrong_type`), list bound violation (`too_long` / `too_short`), or duplicate map key (`duplicate_key`). |
| `unauthenticated` | `401` | `UNAUTHENTICATED` | Missing or unrecognized credential (bearer source returned empty name, or kernel peer credentials unavailable). |
| `unauthorized` | `403` | `PERMISSION_DENIED` | Authenticated caller does not hold a role listed in the row's `requires:` (includes `"requires": [...]`). |
| `digest_mismatch` | `409` | `FAILED_PRECONDITION` | Client's ` digest` matches neither the surface digest nor the member's digest (includes `"served": "<digest>"`). |
| `full` | `429` | `RESOURCE_EXHAUSTED` | Exposure has reached its `bound` of in-flight requests under `on_full: refuse`. |
| `unavailable` | `503` | `UNAVAILABLE` | Target receiver is draining, restarting, or replaced. |
| `shutting_down` | `503` | `UNAVAILABLE` | Exposure is stopping (`stop()` called); queued request did not run. |

### Caller Context (`std::api::Context`)

Any RPC handler can inspect the caller and transport by declaring a second parameter of type `std::api::Context` (or `std::api::ServedContext` to also read the exposure name and receiver generation):

```hale
type RefundRequest { order_id: Int; amount: Int; }
type RefundResponse { ok: Bool; by: String; }

locus Billing {
    fn refund(req: RefundRequest, ctx: std::api::Context) -> RefundResponse {
        // ctx.caller is the authenticated Principal;
        // ctx.via is "unix", "http", "ws", "mcp", or "local" for direct in-process calls.
        return RefundResponse { ok: true, by: ctx.caller.name };
    }
}

fn main() { }
```

`std::api::Context` provides:

- `ctx.caller`: `std::api::Principal` (`mode`, `name`, `uid`, `gid`, `pid`).
- `ctx.request_id`: Monotonic `Int` assigned by the exposure.
- `ctx.via`: Transport identifier (`"unix"`, `"http"`, `"ws"`, `"mcp"`, or `"local"` when invoked directly inside Hale).
- `ctx.role`: The first role from the row's `requires:` held by the caller (`""` if the row requires no roles).

### Role Sources (`std::api::RoleSource` and `StaticRoles`)

Role requirements (`requires: [refund_support]`) are declared on surface rows, while role hierarchy (`role owner includes refund_support;`) is declared at top level:

```hale,fragment
role refund_support;
role auditor;
role owner includes refund_support;

api Billing {
    rpc Billing::refund requires: [refund_support];
    rpc Billing::ledger requires: [auditor];
}
```

Who holds a role at runtime is determined by the `roles:` locus passed to the transport, which must satisfy `std::api::RoleSource` (`fn holds(p: std::api::Principal, r: String) -> Bool`; the exposure walks `includes` automatically):

```hale,fragment
locus RecordRoles {                      // satisfies std::api::RoleSource
    params { root: String = "."; }
    fn holds(p: std::api::Principal, r: String) -> Bool {
        return r == "owner" && p.name == "uid:1000";
    }
}

main locus Head {
    params {
        roles: RecordRoles = RecordRoles { };
        local: api::Exposure = api::serve(Billing, unix::Rpc { path: "/run/head.sock", roles: self.roles }, as: "head", bound: 64, on_full: refuse);
    }
    run() {
        self.local.wait();
        self.local.stop();
    }
}

fn main() { Head { }; }
```

For static or environment-driven deployments, use `std::api::StaticRoles`, which reads a semicolon-separated grant table (`role=member,member;...`) from its `table` parameter or the `LOTUS_API_ROLES` environment variable:

| `StaticRoles` Member Pattern | Matches |
|---|---|
| `uid:<n>` | Unix socket peer with user ID `<n>`. |
| `gid:<n>` | Unix socket peer with primary or supplementary group ID `<n>`. |
| `user:<name>` | Unix socket peer with account `<name>` (resolved once at startup). |
| `group:<name>` | Unix socket peer in group `<name>` (resolved once at startup). |
| `bearer:<name>` | HTTP, MCP, or Hub caller whose `BearerSource` returned principal `<name>`. |
| `*` | Any authenticated caller on any transport. |

Unix account patterns (`uid:`, `gid:`, `user:`, `group:`) never match bearer callers, and `bearer:` patterns never match Unix socket peers.

---

## CLI Driving & Client Code Generation

### Driving Live Exposures from the Terminal

Five built-in CLI commands interact with any live exposure by reading its self-description (`{"describe": true}` on Unix sockets, `GET /.description` on HTTP and WebSocket hubs):

```sh
# 1. Inspect caller-visible description
hale describe /run/desk/admin.sock
hale describe http://127.0.0.1:8080 --bearer t-alice

# 2. Invoke RPC members (flags per field, or raw JSON)
hale call /run/desk/admin.sock Orders::cancel '{"order": 41}'
hale call http://127.0.0.1:8080 Orders::place --symbol ACME --qty 10 --limit 12500 --bearer t-alice
echo '{"book": "desk"}' | hale api call unix:/run/desk/admin.sock Ledger::rebalance --json -

# 3. Call HTTP REST routes directly
hale api call http://127.0.0.1:8080 'GET /v1/items/42'

# 4. Subscribe to WebSocket hub streams
hale watch ws://127.0.0.1:9000 Fills --token t-dave

# 5. Launch loopback web dashboard or MCP stdio bridge
hale admin /run/desk/admin.sock
claude mcp add desk -- hale mcp --app http://127.0.0.1:8080 --token t-alice
```

- **Typed Flags vs. `--json`**: `hale call` validates `--<field>` flags against the exposure's JSON Schema before sending (exiting with code `5` on client-side schema mismatch). Passing `--json '{...}'` (or `--json -` for stdin) sends raw JSON directly to the server.
- **Reserved Flags**: `hale call` reserves `--json`, `--bearer`, `--digest`, `--raw`, and `--request-id` (defaults to `hale-<pid>`). If a request payload has a field matching one of those five names, pass it via `--json` (a field named `id` can be passed normally as `--id 5`).
- **Exit Codes**: `0` (`Result` on stdout), `1` (`HandlerError` on stderr), `2` (`Refusal` on stderr), `3` (`ServerError` on stderr), `4` (`Lost` / connection failure), `5` (client-side refusal before sending). Passing `--raw` prints the full wire envelope to stdout while preserving the exit code.

### Exporting Specification Bundles (`hale api export`)

`hale api export` writes all specification artifacts for a surface to a directory deterministically:

```sh
hale api export --surface Public --out api/public desk.hl
hale api export --surface Public --check api/public desk.hl   # exits 1 on drift in CI
```

Generated files in `api/public/`:

- `Public.description.json`: Full surface description, schemas, exposures, and hubs.
- `Public.openapi.json`: OpenAPI 3.0 specification.
- `Public.json-schema.json`: Standalone JSON Schema bundle.
- `Public.mcp.json`: MCP tool definitions.
- `Public.proto`: Protobuf v3 service and message definitions for `grpc::Rpc`.
- `DIGEST`: The surface contract digest and compiler version.

### Generating Typed Clients (`hale api client`)

Generate standalone, zero-dependency client modules for Hale, TypeScript, or JavaScript (ES module with JSDoc types):

```sh
hale api client --surface Public --lang hale --out client/desk.hl desk.hl
hale api client --surface Public --lang ts   --out client/desk.ts desk.hl
hale api client --surface Public --lang ts   --check client/desk.ts desk.hl   # exits 1 on drift
hale api client --surface Public --lang js   --out web/desk.js desk.hl
```

| Target | Supported Transports | RPC Return Shape | Stream Shape |
|---|---|---|---|
| **Hale (`--lang hale`)** | `unix:PATH`, `http://`, `ws://` | `<Member>Outcome` enum: `Result(T)`, `HandlerError(E)`, `Refusal(R)`, `ServerError`, `Lost(String)` | Subscription locus (`start()`, `next()` -> `Event(seq, payload)` \| `Expired` \| `Revoked` \| `Closed`) |
| **TypeScript (`--lang ts`)**<br>**JavaScript (`--lang js`)** | `http://`, `ws://` | Tagged union: `result`, `handler_error`, `refusal`, `server_error` (throws `TransportError` if connection drops mid-call) | Async iterable (`for await (const ev of ...)`) |

Every generated client embeds the per-member digest on each call and exposes the surface digest as a constant.

---

## Consuming External APIs

### Importing OpenAPI and MCP Specifications (`hale api import`)

Use `hale api import` to generate a typed Hale `api` declaration from an external OpenAPI 3.x document (JSON or YAML, local file or URL) or a live MCP server:

```sh
hale api import https://shelf.example/openapi.json --as Shelf --out shelf.hl
hale api import ./vendor/shelf.json --as Shelf
hale api import ./vendor/shelf.yaml --as Shelf
hale api import http://127.0.0.1:8931/mcp --as Tools
```

This generates a source file declaring the external surface and its wire types:

```hale
// Generated by `hale api import` from openapi ../shelf.json (OpenAPI 3.0.3: Shelf 1.0.0).

api Shelf from openapi "../shelf.json" {
    rpc create_item(ShelfNewItem) -> ShelfItem wire: "POST /items";
    rpc get_item(ShelfGetItemRequest) -> ShelfItem wire: "GET /items/{itemId}";
    // response: std::json::Value, an array with no `maxItems`, and a record holds a bounded list only
    rpc list_items() -> std::json::Value wire: "GET /items";
}

type ShelfGetItemRequest {
    item_id: Int `json:"itemId"`;
}

type ShelfItem {
    id: Int;
    name: String;
    has_state: Bool = false;
    state: ShelfItemState;
}

type ShelfItemState = enum { open, held, gone };

type ShelfNewItem {
    name: String;
}
```

- **Mapping Rules**: Operations map to snake_case `rpc` rows with `wire: "METHOD /path?query_param"`. Non-required properties generate `has_<field>` presence pairs; non-identifier JSON keys use `` `json:"..."` `` tags; schemas that cannot map to fixed Hale structs (`oneOf`, unbounded arrays without `maxItems`, free-form objects, or schemaless MCP outputs) fall back to `std::json::Value` with an explanatory comment.
- **Supported YAML Subset**: The built-in YAML reader follows the YAML 1.2 Core Schema (so `on` and `yes` are strings, not booleans) and supports block/flow mappings and sequences, plain/quoted scalars, `|` and `>` block scalars, comments, and a single `---` document marker. Anchors/aliases (`&`, `*`), merge keys (`<<`), tags (`!`), complex keys (`?`), duplicate keys, and multi-document streams are rejected with a line-numbered diagnostic.

### Calling External Surfaces (`api::client` and `VaultPrincipals`)

Bind an imported `api` surface (or a Hale `api` surface served by another process) to an `api::Client` param using `http::Client` or `mcp::Client`:

```hale,fragment
locus Desk {
    params {
        creds: std::api::VaultPrincipals = std::api::VaultPrincipals { slots: "editor=shelf-editor, *=shelf-reader" };
        shelf: api::Client = api::client(Shelf, http::Client { base: "https://shelf.example", principals: self.creds });
    }
    fn label(id: Int) -> String {
        return match self.shelf.get_item(ShelfGetItemRequest { item_id: id }) {
            ShelfGetItemOutcome::Result(item) -> item.name,
            ShelfGetItemOutcome::HandlerError(f) -> "failed: " + f.class,
            ShelfGetItemOutcome::Refusal(r) -> "refused: " + r.kind,
            ShelfGetItemOutcome::ServerError -> "server error",
            ShelfGetItemOutcome::Lost(why) -> "lost: " + why,
        };
    }
}
```

- **Outcome Matching**: Each row becomes a method returning `<Surface><Row>Outcome`. HTTP `404` maps to `HandlerError` with `class: "not_found"`, `401`/`403`/`429` map to `Refusal`, and `5xx` maps to `ServerError`. When calling a Hale-authored surface over `api::client`, `HandlerError` carries the server's typed error enum directly.
- **Secret Vault Credentials (`std::api::VaultPrincipals`)**: Maps surface roles to secret vault slots (`"editor=shelf-editor, *=shelf-reader"`), fetching the bearer token from the vault on each outbound request so raw tokens never appear in source or locus state.
- **Effect Class (`client(Surface)`)**: Outbound calls carry the `client(<Surface>)` effect class, allowing architectural claims to restrict external API calls:

```hale,fragment
claims {
    desks_stay_local: forbid reaches(desks, effects(client(Shelf)));
    // Use `effects(client(*))` to forbid calls to any consumed API surface.
}
```

- **Exporting Role-Scoped Agent Tools**: Run `hale api export --surface Shelf --tools-for editor` to export MCP tool definitions (`Shelf.create_item`, `Shelf.get_item`, ...) filtered to the operations `editor` is permitted to invoke.

### Detecting Upstream API Drift (`hale api drift`)

Run `hale api drift` in CI or a watcher to compare the upstream OpenAPI/MCP specification against your committed `api` rows by structural shape:

```sh
$ hale api drift .
drift Shelf added archive_item
drift Shelf removed create_item
drift Shelf changed get_item response
$ echo $?
1
```

- Exits `0` if all rows match (`Shelf: in step with ../shelf.json (4 rows)`), `1` if any row was `added`, `removed`, or `changed` (`request`, `response`, or `wire`), or `2` if the upstream document cannot be read.
- Pass `hale api drift --document` to output the upstream specification as canonical, key-sorted JSON regardless of whether the source was YAML or JSON.

---

## Current Limitations

- **Unsupported Wire Field Types**: `Decimal`, `Bytes`, unbounded slices, and locus types cannot appear in wire records.
- **Batch Handlers**: `Drain<T>` batch handlers cannot be exposed as `rpc` rows.
- **Non-Object MCP Requests**: An `rpc` row whose request type is a top-level enum rather than a struct has no MCP tool projection.
- **Hub Live Descriptions**: When a `ws::Hub` also serves an `api` surface via `api::serve`, its live `GET /.description` lists its stream bindings; the RPC surface members remain in the static `hale check --api` inventory.
- **Bearer Group Claims**: `StaticRoles` matches bearer principals by individual name (`bearer:<name>`) or `*`. To authorize bearer tokens by OIDC group claims, implement a custom `std::api::RoleSource`.
