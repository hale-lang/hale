# The API surface

A service that is authoritative over something ends up wanting a
surface that tools can plug into: a command line, a dashboard, an
MCP host. You could write an HTTP server, a JSON codec per message
and a routing table for it. You do not have to.

A program says what it exposes in rows: a **surface** is a table of
operations, each naming a handler and the roles a caller must hold
(`spec/api.md`). The compiler checks the rows, folds each surface into
a contract digest and prints what a caller can learn, without running
anything. `api::serve` puts a surface on a transport (a Unix socket, an
HTTP listener, an MCP endpoint, a WebSocket hub), and a client needs no
code of its own: a served exposure describes itself, and `hale api call`,
`hale watch`, `hale admin` and `hale mcp --app` read only that
description. To see the pieces combined into one program, built step by
step from an empty file to an API with roles, read
[Build an API](./build-an-api.md).

## Surfaces

An `api` block names a surface and lists its rows. Each `rpc` line is
one row: a member fn of a locus, and the roles a caller needs.

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
    fn place(req: OrderRequest) -> OrderResponse fallible(ClosureViolation) { … }
    @rpc(requires: [trader])
    fn cancel(req: CancelRequest) -> CancelResponse fallible(OrderError) { … }
}
```

Both spellings feed one table, and nothing else about the locus is
read: a handler is an operation because a row says so, not because it
subscribes a topic or returns a value. A handler can sit in several
surfaces, each with its own `requires`, and two versions of one API
over one set of handlers are two surfaces.

Roles are one vocabulary for the whole program, whichever seed declares
them. A role a library declares is named through its import alias —
`requires: [lib::reader]`, `@rpc(requires: [lib::writer])`, a hub
stream's `requires:`, or `role boss includes lib::writer;` — and is the
library's `reader`: the description and the refusals say `reader`.

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

### Lists and pages

A list crosses as a field of a record, and its type says how long it
may be: a `bounded[T; N]` field is a JSON array of up to `N` elements,
a `[T; N]` field one of exactly `N`. "List my items" returns a page,
which is a record:

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

A `bounded` field may be left out of a request, and is then empty; a
fixed array is always sent whole. The description states the bound
(`"maxItems": 50`, and `"minItems"` for a fixed array), the `.proto`
makes the field `repeated`, and the generated clients type it
(`bounded[Item; 50]` in Hale, `Array<Item>` in TypeScript). A request
whose list is longer than the field holds is refused `malformed` with
`too_long` (a fixed array sent short, `too_short`): the handler never
sees a list cut to fit. An unbounded `[T]` is not carried, since an
unbounded collection belongs to a locus (`@form(vec)`), not to a value
that crosses a boundary; neither is a list of lists, nor a request or
response that is a bare list.

### Instants and durations

A `Time` crosses as its RFC 3339 text in UTC, to the nanosecond:
`"2026-10-12T03:04:05.123456789Z"` (trailing zeros of the fraction
dropped, `"2026-10-12T03:04:05Z"` when there is none). A request must
say `Z`: a time with no zone or with an offset is refused `malformed`
rather than guessed at, and so is one past `Time`'s range (it ends in
2262). A `Duration` crosses as its count of nanoseconds, as every
quantity crosses as its count: `1500000000` is `1.5s`. The description
marks a `Time` `"format": "date-time"` and a `Duration` with its unit,
`q(ns)`; the `.proto` carries both as `int64` nanoseconds.

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

An *exposure* puts one surface on one transport for the instances that
answer it. It is a param of the locus that holds it, and its literal is
the serve call:

```hale,fragment
params {
    orders: Orders = Orders { };
    fixture: std::api::test::Rpc = std::api::test::Rpc { };
    public: api::Exposure = api::serve(Public, self.fixture, as: "public", receivers: { Orders: self.orders }, bound: 64, on_full: refuse);
}
```

Like any param it is born with its holder, after the params declared
before it (the receivers it binds are declared first, so they are born
before it; both are the holder's, born, drained and dissolved in
declaration order), and it dissolves with the holder. The param is
also the handle: `self.public.wait()` parks until the exposure is stopped
or the process drains, `self.public.stop()` stops it. (Before GH #1565 a
serve site was an expression in a body, `let public = api::serve(…);`;
the compiler now refuses that form and shows the param to write.)

A library can ship a surface for an application to serve: the library
declares `api Svc { … }` and the loci its rows name, and the application
holds the receivers and names the surface by its path under the import
alias, `api::serve(lib::Svc, …)`. The exposure describes it as `Svc`,
its declared name.

The transport is a locus that implements `std::api::Rpc`: it listens,
frames a request, and writes each outcome in its protocol's terms. The
standard library ships five today. `std::api::test::Rpc` has no socket: a
test hands it requests and reads the answers, through the same
admission, dispatch and completion every transport uses. `unix::Rpc` is
a Unix socket, `http::Rpc` an HTTP listener, `mcp::Rpc` an MCP server and
`grpc::Rpc` a gRPC one over HTTP/2. The sources are fields of the
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
        admin: api::Exposure = api::serve(Admin, unix::Rpc { path: "/run/desk/admin.sock", roles: self.admin_roles }, as: "admin", bound: 16, on_full: refuse);
    }
    run() {
        self.admin.wait();
        self.admin.stop();
    }
}
```

`self.admin.wait()` (or `std::api::run_until_stopped(self.admin)`, the
same call) is the holder's wait: it returns when the exposure is stopped
or the process drains, and until then it parks, waking the moment a call
arrives for the main thread, so a call is answered in the time the work
takes. A `run()` that does periodic work of its own keeps its `while
!self.draining { … sleep(…); }` loop; it works, and a call waits for the
loop's next slice. A main locus that writes no `run()` at all serves until
its exposures stop or the process drains: the compiler gives it the
wait.

The socket is bound when the program boots, so a path it cannot bind (a
missing directory, a socket another program is serving) stops the
program with a diagnostic instead of running without it. (An empty `path`
is not a path it cannot bind: it binds nothing, and the exposure has no
socket. A program that finds its path is another process's to hold passes
none.) Its accept and
read loops park, so they run on an `async_io` pool: the compiler adds the
listener to the main locus as a param of its own and places it where the
exposure's row says (`admin: cooperative(pool = io) where async_io;` in
`placement { }`), or, with no row, on a pool every unplaced Unix exposure
shares (see *Where an exposure's listener runs* below). A client writes
one JSON object per line and reads one
per reply, each carrying the `id` it was sent with, the `request_id` the
exposure assigned, and who the exposure took the caller to be:

```text
{"call": "Orders::cancel", "payload": {"order": 41}, "id": "c-1", "digest": "fnv1a64:40381db6685c9f75"}
{"request_id": 1, "id": "c-1", "ok": true, "value": {"order": 41, "was_open": true}, "caller": {"mode": "unix", "name": "uid:1000", "uid": 1000, "gid": 1000, "pid": 4242}}
{"describe": true}
```

A `describe` answers with the exposure's whole description for this
caller: its identity, listener, the roles the caller holds, the members
it may call with their schemas, the outcome encoding and the notes,
from the same rows and the same sources the next request is checked
against, so what it lists is what it admits. A connection that
breaks (an EOF, a line that is not an object, a reply that cannot be
written) is closed alone: the listener and the other connections go on.
`admin.stop()` answers what is executing, refuses what is queued as
`shutting_down`, and closes the socket after the replies. A program that
ends without calling it releases its sockets too, but a caller with a
request in flight then sees the connection end instead of a reply.

The exposure is the handle: `self.public.stop()` stops accepting, refuses
what is queued, lets what is executing finish and releases the listener,
and the holder's own teardown does the same if you never call it.
`receivers:` names the instance of each locus the surface's rows
call; when the holder holds exactly one instance of a type, it
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

A receiver answers one call at a time. The transports read and write
their connections concurrently, but the exposure hands each receiver one
call and holds the rest in its queue, in the order they were accepted,
so fifty one-second calls to one receiver take fifty seconds wherever
it is placed; that is what lets `stop()` and a failed receiver refuse
exactly what never ran. Calls to different receivers run at once. To
answer a surface's work concurrently, bind its members to several
receivers — instances of different types, each placed where its work
belongs — or serve the surface through more than one exposure, each with
a receiver of its own.

### Where an exposure's listener runs

A socket transport's listener and the connections it starts run on an
`async_io` pool, where an accept or a read parks a coroutine instead of
a thread. Only the main locus places, so the listener is a param of the
main locus the compiler adds, and `placement { }` says where it runs:

- an exposure the main locus holds is placed by its own row, `public:
  cooperative(pool = io) where async_io;`. The row places the listener; the
  exposure stays with the main locus, beside the sources it calls. A row
  that is not a named pool `where async_io` is refused, saying so;
- an unplaced exposure's listener shares a pool with every unplaced
  exposure of its transport, as it always did.

A server can be a locus of its own, holding its receivers, its sources
and its exposure, and the main locus places it like any other child:

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

The server, its receivers and its exposure run on `io`, and so does the
listener: a server's listener runs where the main locus places the param
the server sits under, a level down or several, when that row hosts
`async_io`, and on the shared pool otherwise (a server on the main thread,
or pinned). Two servers on two pools are two params with two rows. The
main locus holds the listener, so it holds each server once (a server held
twice, or placed as replicas, would serve one exposure twice and is
refused). A listener whose address reads the server (`bind:
self.bind`) is born after the server and reads it off the server as it
was built, so a construction that overrides it (`App { server: Server {
bind: "127.0.0.1:9000" } }`) is the address the listener binds; one whose
address reads nothing of the server is born before it. An unplaced
server's `run()` runs on the main thread while `App` is being built
(birth order is load-bearing, see [Lifecycle](./lifecycle.md)), so a
server whose address reads it must finish being built before its
listener exists: its params, the children it holds (their `run()`s run
where they are born), its `birth()` and, unplaced, its `run()`, and the
params of `App` before it, as the program actually builds them (a field
a literal supplies replaces its default, which is then not run; a locus
built anywhere in a param's value counts). What a construction runs
without being written counts too:

| run without being written | counted |
|---|---|
| a locus literal's left-out params | their defaults, and the locus's `birth()`, `birth_check`, `run()` and subscription keys |
| a record literal's left-out fields (`Setup { }`) | their defaults, through any record or locus they build |
| a record-typed param with no value | the `Cfg { }` the compiler builds for it |
| a call's left-out arguments | their defaults, run where the call is |
| a const that is not a literal | its value, run where it is read (unless a local or a parameter of the same name shadows it there) |

Enum payloads, generic parameters, `@form` and capacity arguments, array
sizes and scalar ranges are constants and run nothing. Any of those
that may not return is refused:
one that waits on its exposure (itself, or through a fn it calls), sleeps,
reads, or loops in a `while`, wherever the loop sits. A step that
returns, like `run() { println("started"); }`, is fine. Place a server
whose `run()` waits, wait in `App`'s `run()`, or give the listener an
address that reads nothing of the server. Either way the address the listener was bound to is
told to the exposure, so a server's description names the port the
kernel gave it. `App` above writes no `run()`; it serves until SIGTERM.

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

The listener is bound when the program boots (an address it cannot bind
stops the program with a diagnostic) and, like the Unix socket's, runs on
an `async_io` pool: `public`'s row puts its listener and connections on
`io`, and `partner`, unplaced, takes the pool every unplaced HTTP exposure
shares. The
description it serves states the address it was bound to: a
`bind: "127.0.0.1:0"` lets the kernel pick a port, and a client that
reads `/.description` sees `127.0.0.1:<that port>`, not `:0`. Both are the
same listener `std::http::Server` is built from (see
[HTTP](../everyday/http.md)): it accepts, hands each connection over the
bus, and a connection nothing takes is closed. A call is
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

#### Calls from a web page: `cors`

A page served from another origin can call the exposure only if the
exposure answers CORS for that origin. `cors:` lists the origins,
comma-separated (`"*"` for any); without it nothing changes:

```hale,fragment
public: api::Exposure = api::serve(Public, http::Rpc { bind: "127.0.0.1:8080", codec: json, principals: self.bearer, roles: self.public_roles, cors: "https://app.example.com, http://localhost:5173" }, as: "public", receivers: { Orders: self.orders }, bound: 64, on_full: refuse);
```

The browser's preflight (`OPTIONS /call/<member>` or `OPTIONS
/.description`, sent with no credential) from a listed origin is answered
`204` with that origin in `Access-Control-Allow-Origin`, the methods
(`POST, GET, OPTIONS`), the headers a call sends (`Authorization,
Content-Type, Hale-Surface-Digest`) and a ten-minute `Max-Age`; it never
reaches a receiver. Every response to a listed origin, refusals included,
carries `Access-Control-Allow-Origin`, so the page can read why a call was
refused. Any other origin's preflight is `403` with no CORS header, and its
calls carry none, so the browser stops the page. CORS is the browser's
guard and not authentication: a call is still admitted by its bearer and
its roles, whatever origin it came from.

### Serving to an MCP host

`mcp::Rpc` takes a `bind:`, a `principals:` and a `roles:`, and serves the
surface as an MCP server over HTTP: a JSON-RPC message in `POST /mcp`,
its answer in the response. The tools are the rows the caller may call
(`tools/list` is the caller's description, filtered, each tool named as
its member with `::` written `__`; an identifier's own underscores are
escaped where they would run into that, as the spec says), and
`tools/call` is a call, through
the same admission as an HTTP body:

```hale,fragment
public: api::Exposure = api::serve(Public, mcp::Rpc { bind: "127.0.0.1:8090", principals: self.bearer, roles: self.public_roles }, as: "public", receivers: { Orders: self.orders }, bound: 64, on_full: refuse);
```

```text
POST /mcp HTTP/1.1
Authorization: Bearer t-alice

{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"Orders__place","arguments":{"symbol":"ACME","qty":10,"limit":12500}}}

{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"{\"order\":41,\"notional\":125000}"}],"structuredContent":{"order":41,"notional":125000},"isError":false}}
```

A handler's own error and every refusal come back as a JSON-RPC error
object (`code`, `message`, and the error or the refusal as `data`): `-32001`
for the handler's error, `-32602` for a payload that does not decode,
`-32005` for a call the role forbids, `-32603` for a handler that violated;
an unauthenticated caller also gets HTTP `401`. The listener, its limits and
its failure behaviour are `http::Rpc`'s, since it is the same listener.
Not served: resources over streams, MCP over stdio, and a client that
needs a session or an event stream. `hale mcp` is a server for the
toolchain on stdio and is not a client of this endpoint.

### Serving over gRPC

`grpc::Rpc` takes a `bind:`, a `principals:` and a `roles:` (and, as
`http::Rpc` does, `codec: json`, the row's JSON codec) and serves unary
calls over HTTP/2, cleartext, many calls to a connection, in protobuf or in
JSON as each caller asks:

```hale,fragment
public: api::Exposure = api::serve(Public, grpc::Rpc { bind: "127.0.0.1:8070", codec: json, principals: self.bearer, roles: self.public_roles }, as: "public", receivers: { Orders: self.orders }, bound: 64, on_full: refuse);
```

A call is `POST /<Surface>/<rpc>`, the rpc the member written as an MCP
tool is (`Orders__place` for `Orders::place`; `Orders.place` still names it),
with `content-type: application/grpc` or `application/grpc+proto` for
protobuf, or `application/grpc+json` for the row's JSON, one gRPC message
(a zero byte, four bytes of length, the message) in and one out, answered in
the codec it came in. The
credential is the `authorization: Bearer <token>` metadata, and the digest
you generated against, if you send one, is the `hale-surface-digest`
metadata. The status is gRPC's:

```text
:method POST   :path /Public/Orders__place   content-type application/grpc+json
authorization Bearer t-alice   hale-surface-digest fnv1a64:a8930d6e7998e986
message {"symbol": "ACME", "qty": 10, "limit": 12500}

:status 200   content-type application/grpc+json
message {"order":41,"notional":125000}
trailers grpc-status 0
```

The same call as protobuf is the message `OrderRequest` of the generated file
(`symbol = 1`, `qty = 2`, `limit = 3`) and answers an `OrderResponse`.

#### The `.proto`, and finding the surface

The `.proto` is not written by hand: it is generated from the rows, like the
other forms, and `hale api export` writes it beside them.

```text
$ hale api export --surface Public --out api/public desk.hl
wrote api/public/Public.description.json
wrote api/public/Public.openapi.json
wrote api/public/Public.json-schema.json
wrote api/public/Public.mcp.json
wrote api/public/Public.proto
wrote api/public/DIGEST
```

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

A message is a record the rows reach and its fields are numbered in the order
the struct declares them, so the numbers hold as long as the surface digest
does; every scalar is `optional` so an absent field is told from a zero one
(and a field with no default that is absent is the same `missing_field`
refusal the JSON codec gives). The header of the file states the five
outcomes in these terms: a handler's error is the detail of its status, an
`Any` of the error's message, and a refusal is the detail `HaleRefusal`. `hale
check --api --surface Public --proto` prints the file, and `hale api export
--check` refuses a committed copy that drifted. Two fields of a record whose
default JSON names are the same (`a_b` and `aB`, or `a` and `_a`, which
protoc compares without regard to case) make a file `protoc` refuses, so the
record is refused instead, naming both fields; a `json:` tag on one of them
renames it. A record with the surface's own name (`api Todo` over a `type
Todo`) is the message `TodoMessage`: proto3 gives a service and a message one
scope, and the service keeps the surface's name because it is the gRPC path.

A client is whatever generates stubs from a `.proto`; `hale api client`
makes Hale, TypeScript and JavaScript clients and none speaks gRPC (a
browser's `fetch` cannot, and no client carries an HTTP/2 stack). The server also
answers gRPC server reflection (`grpc.reflection.v1`), so a tool that does not
have the file finds the surface through the server: it lists three services
(the surface, `hale.api.Description` and the reflection service itself) and
answers the descriptor of the file that declares any of them, the file a
compiler makes of the `.proto`. A reflection call is one stream: each request
is answered as it arrives, and the call stays open until the tool ends its
side. Reflection is for a caller the bearer source
names, so pass the `authorization` metadata to the tool as to a call.

A handler's own error is `FAILED_PRECONDITION` with the error in
`grpc-status-details-bin` (a `google.rpc.Status` whose detail is the error's
message, `OrderError` for `Orders::cancel`, or the error's JSON for a caller
that asked in JSON); a refusal is `INVALID_ARGUMENT`, `FAILED_PRECONDITION` (a
digest mismatch), `UNAUTHENTICATED`, `PERMISSION_DENIED`,
`RESOURCE_EXHAUSTED` or `UNAVAILABLE`, its reason in `grpc-message` and the
refusal object in the details; a handler that violated is `INTERNAL`. Each
is a trailers-only response, the status in the headers. The description is
the call `hale.api.Description/Describe`, answered for the caller the bearer
names. A stream the client resets, or a connection that fails, is that
stream's or that connection's alone, and `public.stop()` answers what is
executing, refuses what is queued, says `GOAWAY` on every connection and
closes the listener. The HTTP/2 itself, its windows, pings and settings,
is the runtime's built-in nghttp2, a library with no threads of its own,
read from a socket parked on the same `async_io` pool the other transports
use. The listener is the same one the HTTP transport's is (see
[HTTP](../everyday/http.md)): it accepts, hands each connection over the
bus to a connection locus of its own, and a connection nothing takes is
closed; `std::io::tcp::stop("127.0.0.1:8070")` closes the listener by its
address while the connections it accepted go on being served.

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
`connections()` and `events()`. The WebSocket hub's listener is the one
the HTTP transport's is (see [HTTP](../everyday/http.md)), named by its
bound address: each connection is handed over the bus to a locus of its
own, a connection nothing takes is closed, and
`std::io::tcp::stop("127.0.0.1:9000")` closes the listener while the
connections it accepted keep their subscriptions until `hub.stop()`.

A hub can also be the transport a surface is served over,
`api::serve(Public, self.hub, as: "desk", …)`: a call is a frame `{"type":
"call", "id": "c1", "call": "Orders::place", "payload": {…}}` on the same
connection, authenticated by the same credential, and the reply is a
`{"type": "reply", "id": "c1", "ok": true, "value": …}` frame between the
events. `udp::Hub` carries streams only: a subscriber is an address and an
`id` it chooses (every datagram carries it, and the credential rides the
subscribe), and a datagram lost on the way is a gap in `seq` like a shed
one. The contract is `spec/api.md` § Streams.

## The clients

You never write a client for a Hale program by hand, because a served
exposure describes itself and a surface generates its clients. `{"describe": true}` on the socket, `GET /.description`
over HTTP and on a hub's address return the same kind of document: the
exposure's identity and digest, where it listens, who the caller is and
which roles it holds, the members it may call with their schemas, the
streams it may subscribe to, how an outcome is encoded, and two notes
that say what a role check is and is not. Five verbs read only that
document:

```sh
hale describe /run/desk/admin.sock                  # the description the exposure serves this caller
hale describe http://127.0.0.1:8080 --bearer t-alice
hale call /run/desk/admin.sock Orders::cancel '{"order": 41}'
hale call http://127.0.0.1:8080 Orders::place --symbol ACME --qty 10 --limit 12500 --bearer t-alice
hale watch ws://127.0.0.1:9000 Fills --token t-dave # frames, one JSON line each
hale admin /run/desk/admin.sock                     # a page on 127.0.0.1:7473 over the description
claude mcp add desk -- hale mcp --app http://127.0.0.1:8080 --token t-alice   # every member a tool
```

`hale describe` and `hale call` are the short forms of `hale api describe`
and `hale api call` (below), with two conveniences: a bare path is
`unix:<path>`, and a bare JSON payload after the member is `--json`. Their
endpoint is a socket path, or `http://host:port` (the caller is whoever the
`--bearer` token is, or `HALE_API_BEARER`). `hale watch` takes a
`ws://host:port` hub and `hale mcp --app` also `mcp://host:port`. A call
reads the description first, so a member the caller may not call is not
offered, and it names the **digest** it read, so a program that changed under
the client refuses the call with `digest_mismatch` and the digest it serves.
`hale watch` prints `subscribed` and then each
`event` until the hub closes the connection.

`hale check --api desk.hl` prints the same document from the rows, with no
program running, and the forms a client is generated from
(`--surface Public --openapi`, `--json-schema`, `--mcp`). The description
carries what the exposure was given (its address) and what it
established (the caller), and nothing of the deployment beyond that.

### Driving a program from a terminal

`hale api describe` and `hale api call` are the verbs for an operator and for
scripts. They drive a running program over a Unix socket or HTTP, read the
description it serves, and turn each outcome into an exit code. Against the
desk of this chapter:

```sh
$ hale api describe http://127.0.0.1:8080 --bearer t-alice
Public@fnv1a64:a8930d6e7998e986/public  [http 127.0.0.1:8080]
caller: bearer alice; roles: trader
  Orders::cancel(order: Int (OrderId)) -> CancelResponse, error OrderError  requires: trader
  Orders::place(symbol: String, qty: Int, limit: Int (Money, q(cent))) -> OrderResponse, error ClosureViolation  requires: -

$ hale api call http://127.0.0.1:8080 Orders::place --symbol ACME --qty 3 --limit 125 --bearer t-alice
{"order":41,"notional":375}
$ hale api call http://127.0.0.1:8080 Orders::cancel --order 999 --bearer t-alice
{"code":"unknown_order","reason":"no order 999"}
$ echo $?
1
$ echo '{"book": "desk"}' | hale api call unix:/run/desk/admin.sock Ledger::rebalance --json -
{"moved":1500}
```

Each member's payload is built from one flag per field, typed by the schema
the description gave: an integer or a quantity's count (`--qty 3`), a number,
`true` or `false`, a string (`--symbol ACME`), and JSON for a record or a list
(`--items '[1, 2]'`). A flag that does not fit its field, a missing required
field and a flag that names no field are refused before anything is sent, with
the field's schema in the message. `--json '{…}'` (or `--json -` for stdin)
sends a payload as given, unchecked: the server's answer is then the check.
The verb keeps five flags for itself, `--json`, `--bearer`, `--digest`, `--raw`
and `--request-id` (the call's id, `hale-<pid>` by default); a payload field
with one of those names goes in `--json`. A field named `id` is an ordinary
flag: `--id 5`.

A result prints its value on stdout and exits 0. Everything else prints what the
wire said on stderr: a handler error exits 1, a refusal 2 (`unauthorized`,
`digest_mismatch`, `malformed` and the rest, with their reason), a server error
3, a connection that broke or never answered 4, a call the verb would not send
5. `--raw` prints the whole outcome on stdout instead, and the exit code stays.
The endpoint is `unix:<path>` or `http://host:port`; `grpc://` and `mcp://`
are refused with that message. The caller over HTTP is `--bearer T` (or
`HALE_API_BEARER`); over a socket it is the peer. See *Driving a served
program* in `spec/api.md` for the full table.

### Generated specs and clients

The rows carry everything a client needs (the shapes, the five outcomes,
the digest, the roles of each member, the stream rows), so a surface's
specs and clients are generated from them:

```sh
hale api export --surface Public --out api/public desk.hl
hale api client --surface Public --lang hale --out client/desk.hl desk.hl
hale api client --surface Public --lang ts   --out client/desk.ts desk.hl
hale api client --surface Public --lang ts   --check client/desk.ts desk.hl   # exit 1 on drift
hale api client --surface Public --lang js   --out web/desk.js desk.hl
```

`export` writes a bundle: `Public.description.json` (every member with its
roles, the exposures that serve it, the hubs, the schemas),
`Public.openapi.json`, `Public.json-schema.json`, `Public.mcp.json`,
`Public.proto` (the messages and the service a `grpc::Rpc` exposure speaks;
see Serving over gRPC) and `DIGEST` (the digest and the compiler's version).
Every file is a function
of the rows alone, so two runs and two checkouts write the same bytes, and
an imported type is named by its path under the import alias (`lib::Item`),
never by a path of the machine. `--check DIR` writes nothing and exits 1,
naming the digest that moved and the files that differ, so a committed
bundle cannot go stale unnoticed.

A client is typed by the rows. The Hale one is a module with a fn per member
(`orders_place(endpoint, bearer, request)`), whose answer is an enum of the
outcomes (`Result`, `HandlerError` when the row declares an error, `Refusal`
with its kind, reason and the roles it names, `ServerError`, `Lost`), and a
subscription locus per stream (`start`, then `next` yields each `Event(seq,
payload)` until `Expired`, `Revoked` or `Closed`). The TypeScript one is a
single file that needs only `fetch` and `WebSocket`: an async function per
member answering a tagged union (`result`, `handler_error`, `refusal`,
`server_error`; a connection that never answered is thrown as a
`TransportError`, because that request may have run) and an async iterable
per stream. The JavaScript one is that same client as an ES module a page
loads as it is, with no compile step: the same functions by the same names,
typed in JSDoc, so an editor (or `tsc --checkJs`) still checks a caller
against the rows. Every client sends the surface's digest on every call, so a
program that changed under it refuses it `digest_mismatch`; each names the
digest in a constant, and `--check` refuses a committed copy made against
another one.
A client speaks `unix:PATH` and `http://` (the Hale client) and `http://` and
`ws://` (both); TLS is not spoken, and neither is gRPC: a gRPC caller takes the
`.proto` to its own language's stub generator, or finds the surface through
server reflection.

## When it says no

A refusal is an answer, never a failure of the program:

```text
{"request_id": 4, "id": 4, "ok": false, "refusal": {"kind": "malformed", "reason": "wrong_type: qty"}}
```

The kinds are `malformed` (not a request, no such member, or a payload
that does not decode; the reason names the field), `digest_mismatch`,
`unauthenticated` (the sources name nobody; nothing is served to such a
caller), `unauthorized` (the caller does not hold what the row
`requires`, which the refusal names), `full` (the exposure holds its
`bound`), `shutting_down` and `unavailable`. A payload is decoded before
dispatch, strictly: a string where an `Int` is declared is `wrong_type`, a
missing field without a default is `missing_field`, and the handler only
ever sees a value of its declared type.

## Who is calling

The exposure knows, and a handler can ask. Every reply over a socket
carries the principal the exposure established for the connection: on the
Unix socket that is the peer's credentials, as the kernel vouches for
them; over HTTP it is the name the bearer source gave the token.

```text
{"request_id": 7, "id": 1, "ok": true, "value": {...},
 "caller": {"mode": "unix", "name": "uid:1000", "uid": 1000, "gid": 1000, "pid": 4242}}
```

A handler that wants the caller declares a second parameter:

```hale
type RefundRequest { order_id: Int; amount: Int; }
type RefundResponse { ok: Bool; by: String; }

locus Billing {
    fn refund(req: RefundRequest, ctx: std::api::Context) -> RefundResponse {
        // ctx.caller is who; ctx.via is the door: "unix", "http", "ws" or
        // "mcp" through a transport, "local" for a call that never crossed one
        return RefundResponse { ok: true, by: ctx.caller.name };
    }
}

fn main() { }
```

The second parameter is `std::api::Context`: the caller, the request id,
`via`, and the first role the row requires (`role`, empty when it
requires none). A call that did not come through a transport hands the
handler the local principal, so a handler never asks whether it was
reached from outside; it reads `via`. `local` says where a call did not
come from, never that it is trusted. `Context` and `Principal` are
ordinary structs: build one in a test, forward one in a payload.

## Who may call

The requirement that an operation needs a role is part of the program,
true wherever it runs; who holds the role here is a deployment fact. So
the requirement is written once, on the row, and the mapping is a source
the serve site names.

```hale,fragment
role refund_support;
role auditor;
role owner includes refund_support;      // whoever is owner may do what support may

api Billing {
    rpc Billing::refund requires: [refund_support];
    rpc Billing::ledger requires: [auditor];
}
```

A `role` is declared vocabulary, like `group`: a name nothing declares is
an error at the row. What `requires` means is exactly one thing: a call
**arriving through the exposure** is refused unless the caller holds the
role, and the refusal names it. It is a gate at the boundary, not a proof
about the program's insides; a handler that calls `refund` from some other
path is not stopped by it, and the description says so in its notes, so no
client presents the check as more than it is. A program that wants the
inside proved states it as a claim: `group read_side = { role reader };`
names the handlers a `reader` may call, and `forbid reaches(read_side, …)`
is checked over the paths behind them
([claims](../claims.md#group--the-vocabulary)). The requirement is a
property of the row, never of the handler, so one handler shared by two
surfaces meets each surface's own.

Who holds a role is the **role source** of the transport instance
(`roles: self.staff`): any locus satisfying `std::api::RoleSource`
(`fn holds(p: std::api::Principal, r: String) -> Bool`, the direct
question only; the exposure walks `includes`), built with the program's
own state and kept as a param:

```hale,fragment
locus RecordRoles {                      // a std::api::RoleSource
    params { root: String = "."; }
    fn holds(p: std::api::Principal, r: String) -> Bool {
        return r == "owner" && p.name == "uid:1000";   // ask the record at self.root
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

That is how a program whose positions are roles answers from its own
record. The standard library's `std::api::StaticRoles` is a table
`role=member,member;…`, passed as its `table` param and overridden by
`LOTUS_API_ROLES` at run time, which is how a test drives it. A member
takes one of six spellings:

| member | matches |
|---|---|
| `uid:<n>` | a socket peer with that uid |
| `gid:<n>` | a socket peer whose primary group, or one of the supplementary groups the kernel reports for the connection, is `<n>` |
| `user:<name>` | a socket peer with that account, resolved once at start per the account database |
| `group:<name>` | a socket peer in that group, resolved the same way |
| `bearer:<name>` | a caller on HTTP, MCP or a hub whose bearer source answered exactly `<name>` |
| `*` | any caller the exposure authenticates, on any transport |

The spellings of the two kinds of caller never cross. A `bearer:` member
is never a socket peer, and the four account spellings never match a
bearer caller, even one whose name reads the same: a Unix account and a
token's subject are different identities. A table naming a role the
program does not declare (`known:` lists them), or a member outside those
spellings, is refused at start with the reason; with no table every
`requires` refuses.

A caller the sources name nobody for (a peer the kernel cannot vouch for,
a bearer token the source answers with an empty name) is refused
everything, whatever the roles: the exposure's whole claim is that it
knows who is calling.

The description follows the same rule. It lists the members the caller
may call and only the schemas those need, so `hale mcp --app` lists
exactly the tools a principal may call and `hale admin` shows what it may
reach. A member a caller may not call is not shown to it, and if it
names it anyway the server answers `unauthorized`, naming what the row
requires.

## What is left out, and why

- A request or a response whose type has a field with no JSON form (a
  `Decimal`, `Time`, `Duration`, `Bytes`, an array, an enum, a locus) is an
  error at the row, naming the field.
- A `Drain<T>` batch handler is not an operation.
- A request that is not an object has no tool over MCP: the wrapped form
  needs the handler's parameter name, which a description does not carry.
- Resources over streams, MCP over stdio and `grpc::Rpc` are not
  served ([`spec/api.md`](https://github.com/hale-lang/hale/blob/main/spec/api.md)
  § Open points).
- A hub that also serves a surface lists its streams in its live
  description; the surface's own members are in the inventory.
- Bearer groups wait: a role source grants a bearer caller a role by its
  own name (`bearer:<name>`) or through `*`, not by a group its source
  reports (an OIDC `groups` claim). A program that needs that today names
  its own `RoleSource`.
