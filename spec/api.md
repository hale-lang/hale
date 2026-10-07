# The API surface

A program exposes an operation by saying so in a row: a **surface** is
a named table of operations (**rpcs**), each row naming a handler and
the roles a caller must hold; a **serve site** puts one surface on one
transport instance, under one bearer source and one role source, bound
to the receiver instances that answer; a **stream** is a topic binding
to a **hub**, with the roles a subscriber must hold on its row. What a
caller can learn is a **description**, scoped to one exposure and
filtered by the caller's roles, read from the same rows dispatch reads.
Exposure is an intent about the boundary held in rows the model hashes,
never a property of a locus's structure.

GH #1417. This document is the contract, stated before any of it ships
(step R0 of the plan): the `api` block and `@rpc` parse to rows in R1,
which also computes the digest and prints descriptions; the runtime
serves `unix::Rpc` in R2 and `http::Rpc` in R3; hubs and stream
authorization land in R5; R4 retires the structural path (§ What this
replaces). Each section names the step that ships it. Until R1, the
fixtures under `tests/api-contract/` are what a consumer builds against:
the consumer program of § The witness, its descriptions per exposure
and caller, the program-wide inventory, one recorded request and reply
per outcome per transport, and the digest worked byte for byte. The
description format is `spec/api-description.schema.json` (JSON Schema,
draft 2020-12).

The examples use the declarations of `tests/api-contract/program.hl`:

```hale,fragment
role trader;
role operator;

unit cent;
type Money = quantity Int in cent;
type OrderId = distinct Int;

type PlaceOrder { symbol: String; qty: Int; limit: Money; }
type OrderReceipt { order: OrderId; notional: Money; }
type CancelOrder { order: OrderId; }
type Cancelled { order: OrderId; was_open: Bool; }
type OrderError { code: String; reason: String; }

locus Orders {
    fn place(o: PlaceOrder) -> OrderReceipt { … }
    fn cancel(c: CancelOrder, ctx: std::api::Context) -> Cancelled fallible(OrderError) { … }
}
```

## Surfaces and their rows

```hale,fragment
api Public {
    rpc Orders::place;
    rpc Orders::cancel requires: [trader];
}
api Admin {
    rpc Orders::cancel requires: [operator];
    rpc Ledger::rebalance requires: [operator];
}
```

`api NAME { … }` at top level declares a **surface**; each `rpc
Locus::fn [requires: [ROLE, …]];` line in it is one **surface row**.
A row's columns:

| column | what it is |
|---|---|
| surface | the block's name |
| member | the handler as a caller names it: `Locus::fn`, the locus in its author spelling (`lib::Orders::cancel` for a locus another seed declares) |
| request | the type of the handler's one value parameter, or none |
| response | the handler's return type, or none for `()` |
| error | `E` when the handler is `fallible(E)`, or none |
| pool | the handler's pool, as the receiver instance a serve site binds is placed (§ Serving) |
| requires | the role names a caller must hold, as written |

A handler is a member fn of a locus. It takes at most one value
parameter, which is the request, and optionally a trailing `ctx:
std::api::Context` (§ Serving, `Context`), which is not part of the
request. The request, response and error types cross the boundary by
shape: a quantity's field carries its unit (the unit dialect's
`q(<denomination>)` tag, `spec/units.md` § Layout and the wire), so a
client knows `notional` counts `cent`.

A handler may sit in several surfaces, and in each it is held to that
surface's row: `Orders::cancel` above requires `trader` through
`Public` and `operator` through `Admin`. Two versions of one API over
one set of handlers are two surfaces. A surface declared and never
served is a table nobody reads, which is how a library ships one for
an application to serve.

**`@rpc`.** On a locus fn, `@rpc` (optionally `@rpc(requires:
[ROLE, …])`) contributes the same row to the seed's **default
surface**, named after the seed (`spec/packages.md`):

```hale,fragment
locus Orders {
    @rpc
    fn place(o: PlaceOrder) -> OrderReceipt { … }
    @rpc(requires: [trader])
    fn cancel(c: CancelOrder, ctx: std::api::Context) -> Cancelled fallible(OrderError) { … }
}
```

The block and the sugar feed one row family, `surface`
(`spec/registry.md`), and nothing else about the locus is read: not its
subscriptions, not its `expose` members, not its position in the tree.
A reply type on a bus handler does not make it an rpc; a handler is an
rpc because a row says so. A bus handler that declares a reply type is
a handler whose reply the runtime may ask for (R2), nothing more.

**The admission law (R1).** A surface's rows are admitted at the check,
before anything is served:

1. **A row names a handler.** `Locus::fn` resolves to a member fn of a
   declared locus: "rpc \`Orders::plcae\`: \`Orders\` declares no fn
   \`plcae\`; did you mean \`place\`?". A lifecycle method, a mode or
   `on_failure` is no handler.
2. **A handler takes one request.** At most one value parameter beside
   a trailing `ctx: std::api::Context`: "rpc \`Orders::fill\`: a
   handler takes its request as one parameter, and \`fill\` takes two:
   declare a type for the request".
3. **A member is one row of its surface.** "rpc \`Orders::cancel\` is
   in \`Admin\` twice: a member is one row; keep one, with the roles it
   requires".
4. **A required role is declared.** "rpc \`Orders::cancel\` requires
   \`tradr\`, which no \`role\` declares; did you mean \`trader\`?".
5. **Every shape has a codec form.** The request, response and error
   types encode under the serve sites' codecs (§ Codecs): "rpc
   \`Ledger::export\`: its response \`Export\` has a field \`raw:
   Bytes\`, which the JSON codec does not carry". A row is never left
   out of a served surface with a warning: the row is the intent, and
   an intent the program cannot honour is an error.

## The contract digest

A surface's **digest** is its type-level contract: the hash of its rows
in canonical order, each row reduced to its member name, its request,
response and error shapes, and its `requires`. It moves when a member
is added or removed, a shape or an error type changes, or a required
role changes, and for nothing else. It never covers:

- the listener's address, the transport, the codec;
- the receiver instances a serve site binds, the pool they run on, the
  serve site itself or its exposure name;
- the surface's own name (two surfaces of the same rows have one
  digest; the exposure identity, § The description, carries the name);
- the build identity (`exec_digest`) or the runtime incarnation;
- the `includes` hierarchy of the roles, which says who holds a role,
  as a role source does, not what a member requires.

**The hash input**, byte for byte. A UTF-8 text of lines, each ended by
one LF (0x0A):

1. the header line `hale-api-surface 1`;
2. one line per row, the rows ordered by member name, compared as
   bytes. A row's line is five fields separated by one TAB (0x09):
   - the member name (`Orders::cancel`);
   - the request's shape hash, sixteen lowercase hex digits, or `-`
     when the handler takes no request;
   - the response's shape hash, or `-` when the handler returns `()`;
   - the error type's shape hash, or `-` when the handler is not
     fallible;
   - the required roles, sorted as bytes, each once, joined by `,`
     (0x2C), or `-` when the row requires none.

Member names and role names are identifiers and `::`, so no field
holds a TAB or a LF. The digest is the 64-bit FNV-1a fold
(`hale_graph::identity::Fnv64`: offset basis `0xcbf29ce484222325`,
prime `0x100000001b3`, each byte xor-ed in and then multiplied, the
fold every FNV identity uses, `spec/registry.md` § `digests`) over the
whole text, written `fnv1a64:` followed by sixteen lowercase hex
digits: `fnv1a64:fe65e6d3036ee1eb`.

A type's **shape hash** is its payload contract's hash
(`spec/model.md` § Sorts, the `payloads` table): the 64-bit FNV-1a fold
of its canonical structural shape, the struct's fields in declaration
order as `<field>:<tag>` joined by `;` (`order:i;notional:q(cent)`,
the tags of `spec/units.md` § Layout and the wire), and for a type that
is not a bare struct the fold of `opaque:<type>`.

**Open (R1): the contract shape.** The payload contract renders a
nested struct as the tag `struct` and every type that is not a bare
struct by its name, so a field changed inside a nested type, or a
variant added to an enum error type, moves no digest built on it,
against the rule above. R1 states the shape the digest folds for every
type a row names; the framing above does not change with it. Until
then the digests in `tests/api-contract/` are computed with the payload
contract as it stands (the fixture's types are flat structs, which it
renders whole) and are provisional; `tests/api-contract/digest.md`
works `Public`'s by hand.

**Compatibility is equality (v1).** A description carries its
surface's name and digest, and a generated client carries the digest
it was generated against. A request may carry that digest; a request
whose digest is not the served one is refused as `digest_mismatch`
before its payload is decoded or anything is queued, and the refusal
names the digest the exposure serves (`"served"`), so the client can
fetch the description again. A request that carries no digest is not
checked. Whether a client built against a subset of a surface's
members may call a surface that added others is an open point.

## Serving

```hale,fragment
main locus Desk {
    params {
        bearer: Tokens = Tokens { };            // a std::api::BearerSource
        public_roles: Grants = Grants { trader: "alice" };       // a std::api::RoleSource
        admin_roles: Grants = Grants { operator: "uid:1000" };
        orders: Orders = Orders { };
        ledger: Ledger = Ledger { };
    }
    run() {
        let public = api::serve(Public, http::Rpc { bind: "127.0.0.1:8080", codec: json, principals: self.bearer, roles: self.public_roles }, as: "public", receivers: { Orders: self.orders });
        let admin = api::serve(Admin, unix::Rpc { path: "/run/desk/admin.sock", roles: self.admin_roles }, as: "admin");
        while !self.draining { std::time::sleep(100ms); }
        public.stop();
        admin.stop();
    }
}
```

A **serve site**, `api::serve(SURFACE, TRANSPORT, as: NAME, receivers:
{ TYPE: INSTANCE, … })`, pairs one surface with one transport instance
and makes an **exposure**:

- the **transport instance** is a locus implementing `Rpc` (§ The `Rpc`
  interface), whose fields carry the listener (`bind:`, `path:`), the
  codec (`codec:`, F.36), the **bearer source** (`principals:`, a
  `std::api::BearerSource`) and the **role source** (`roles:`, a
  `std::api::RoleSource`). The Unix transport's principal is its peer's
  kernel credentials (`mode: "unix"`, `name: "uid:<n>"`), so it takes
  no bearer source;
- `as:` names the exposure, unique within the program;
- `receivers:` binds every locus type the surface's rows name to one
  instance. The bound instance is the destination: its pool is the
  dispatch pool and its lifetime bounds the exposure. When the serving
  locus holds exactly one instance of a type, its binding may be
  elided and is inferred (both of `admin`'s above); two instances and no
  binding is an error naming both. Two serve sites may bind one
  surface to different instances;
- the call returns the **handle**: what the serving locus holds, joins
  and stops (`stop()`, § The request lifecycle).

The laws of a serve site (R1):

1. **An exposure is named once.** "exposure \`public\` is served twice:
   \`as:\` names one exposure; name this one apart".
2. **Every receiver type is bound.** "serve of \`Public\` as
   \`partner\`: \`Orders\` is held twice, as \`self.orders\` and
   \`self.partner_orders\`, and the serve binds neither: name the one
   that answers (\`receivers: { Orders: self.orders }\`)"; a type the
   serving locus does not hold at all is the same error with no
   instance to name.
3. **A receiver outlives its exposure.** A receiver is a param of the
   serving locus or a child it owns for as long as the handle lives; a
   `let`-bound child that dissolves before the handle's `stop()` is
   refused: "serve of \`Admin\`: \`ledger\` is \`let\`-bound and
   dissolves at the end of this block, before \`admin.stop()\`; hold it
   as a param".
4. **A bound type is one the rows name.** "serve of \`Public\`:
   \`receivers:\` binds \`Ledger\`, which no row of \`Public\` names".

**What happens to a request.** The transport turns bytes into a
request: the member, the payload's bytes and a correlation (§ The `Rpc`
interface). The runtime then holds it to these checks in this order,
and the first that fails is the refusal the caller receives:

1. the request is a request: the transport could frame it and it names
   a member (`malformed`);
2. the caller's **`Context`** is established from the exposure's
   sources: the bearer source names a principal, or the peer's
   credentials do (`unauthenticated` when they name nobody);
3. a digest the request carries is the served one (`digest_mismatch`);
4. the surface has the member (`malformed`, reason `unknown_member`);
5. the `Context` holds every role of the row's `requires` under the
   exposure's role source, a role held directly or through a role that
   `includes` it (`unauthorized`);
6. the payload decodes by the request's shape under the codec
   (`malformed`, reason `missing_field` or `wrong_type` and the field);
7. the exposure is accepting (`shutting_down`) and holds fewer than its
   bound of requests accepted and not yet answered (`full`).

Only then is the call enqueued on the receiver's pool through the
cross-pool path, and the runtime awaits its outcome. **Authorization is
evaluated on the transport's thread before anything is enqueued:** a
refused request never reaches the handler's queue, and its handler's
state cannot tell it arrived.

**Grants belong to the role-source instance**, never to a role name.
`public_roles` and `partner_roles` above are two sources: `alice`
holding `trader` in one holds nothing in the other, and the same
application deployed twice with two sources shares no grant. A handler
shared by two surfaces meets each surface's `requires` under each
exposure's source.

**`Context`** stays (#1108): a handler that declares `ctx:
std::api::Context` receives the caller the serve site established
(`caller`, a `std::api::Principal`), the request id, the transport it
came through (`via`: `unix`, `http`, `ws`, or `local` for a call made
inside the program) and the authorizing role (`role`: the first role
of the row's `requires` as written, empty for a row that requires
none). It is for the checks no row can state; the row's `requires` has
already held when the handler runs.

**Descriptions read the rows dispatch reads.** Which members a
caller's description lists is decided by the same rows and the same
role source the checks above consult, so discovery and dispatch agree
by construction (§ The description). Discovery is a convenience: the
server still authorizes every request.

## The `Rpc` interface

`Rpc` is a stdlib `interface` in the shape of `__StdBusAdapter`,
implemented by a locus with lifecycle (R2). A transport owns:

- **listening**, realized at birth: a listener it cannot bind fails the
  declaring locus's birth, as F.37 makes a binding that cannot open a
  birth failure (`spec/semantics.md` § The publish contract);
- **framing** a request to (member, bytes, correlation), and a reply
  back;
- **correlation**: the connection over HTTP and the Unix socket, an
  envelope id over UDP, the protocol's own over gRPC and MCP;
- **the encoding of each outcome** in its protocol's terms, fixed per
  transport (§ Outcomes).

Its methods are infallible: a failure at the boundary is structural (a
dead listener is a birth failure, a broken connection the transport
failure outcome), never a value error, since a fallible interface call
does not lower (F.42, GH #1426). Dispatch, decoding by shape, the
`Context`, authorization and the digest check are the runtime's, and
the same for every transport. A transport may not own what is not its:
shapes and error types are the model's, the codec is the binding's
(F.36), roles and the bearer are the exposure's sources.

The stdlib implements `unix::Rpc` (R2: the GH #1106 binding re-homed),
`http::Rpc` (R3), `ws::Hub` and `udp::Hub` (R5, § Streams), `grpc::Rpc`
and `mcp::Rpc` (R6); a program implements one the stdlib lacks.

## Outcomes

A caller sees one of five outcomes, and every transport encodes each,
by a mapping fixed in v1 and part of this contract: no transport
chooses another, no row carries a status, and the digest has nothing
to hash for one.

| outcome | meaning | HTTP | Unix JSON | gRPC | MCP |
|---|---|---|---|---|---|
| result | the handler returned | 200, body = the response by codec | `{"ok": true, "value": …}` | OK | result |
| handler error | the handler failed with its declared `E` | 422, body = `E` by codec | `{"ok": false, "error": E}` | FAILED_PRECONDITION, `E` in details | error object with `E` |
| refusal | the request was not accepted (the kinds below) | 400 / 409 / 401 / 403 / 429 / 503, body = `{"refusal": …}` | `{"ok": false, "refusal": {"kind": …, "reason": …}}` | INVALID_ARGUMENT / FAILED_PRECONDITION / UNAUTHENTICATED / PERMISSION_DENIED / RESOURCE_EXHAUSTED / UNAVAILABLE | error object |
| server error | the handler violated (§ Structural failure in a handler) | 500, body = `{"refusal": {"kind": "server"}}` | `{"ok": false, "refusal": {"kind": "server"}}` | INTERNAL | error object |
| transport failure | the connection or the protocol broke | the transport's own | EOF | the transport's own | the transport's own |

The refusal kinds, each with its status:

| kind | when (§ Serving, the checks) | HTTP | gRPC |
|---|---|---|---|
| `malformed` | not a request; no such member (`unknown_member`); a payload that does not decode (`missing_field`, `wrong_type`) | 400 | INVALID_ARGUMENT |
| `digest_mismatch` | the request's digest is not the served one; the refusal carries `"served"` | 409 | FAILED_PRECONDITION |
| `unauthenticated` | the sources name nobody | 401 | UNAUTHENTICATED |
| `unauthorized` | the `Context` does not hold the row's `requires`; the refusal carries `"requires"` | 403 | PERMISSION_DENIED |
| `full` | the exposure holds its bound of requests accepted and not yet answered | 429 | RESOURCE_EXHAUSTED |
| `shutting_down` | the exposure is stopping (§ The request lifecycle) | 503 | UNAVAILABLE |

A refusal object is `{"kind": K, "reason": "<text>"}`, plus `"served"`
on `digest_mismatch` and `"requires"` on `unauthorized`. The server
error's object is `{"kind": "server"}` and says nothing of the
violation, which is the program's to report (§ Structural failure in a
handler).

**The Unix JSON transport** (`unix::Rpc`, R2) is a Unix domain stream
socket carrying one JSON object per line, as the GH #1106 binding's
wire was. A request:

```text
{"call": "Orders::cancel", "payload": {"order": 41}, "id": "c-1", "digest": "fnv1a64:98daa4b3e265ed98"}
{"describe": true}
```

`"id"` is the client's (any JSON value, echoed verbatim) and
`"digest"` is optional. Every answer carries `"request_id"`, an integer
the transport assigns, unique and increasing for the exposure's
lifetime (0 for a line that is not a request at all), the client's
`"id"`, and `"caller"`, the principal the exposure established:

```text
{"request_id": 1, "id": "c-1", "ok": true, "value": {"order": 41, "was_open": true}, "caller": {"mode": "unix", "name": "uid:1000", "uid": 1000, "gid": 1000, "pid": 4242}}
{"request_id": 2, "id": "c-2", "ok": false, "error": {"code": "unknown_order", "reason": "no order 999"}, "caller": {…}}
{"request_id": 3, "id": "c-3", "ok": false, "refusal": {"kind": "unauthorized", "reason": "…", "requires": ["operator"]}, "caller": {…}}
```

A describe answers `{"ok": true, "value": <description>}`. Answers
arrive in the order the program produces them, so a refusal may precede
the answer to an earlier request still with its handler; a client
correlates by `id`.

**The HTTP transport** (`http::Rpc`, R3) takes one request per
connection: `POST /call/<member>` (`/call/Orders::place`) whose body is
the payload by codec, under `Authorization: Bearer <token>`, with the
digest, when the client sends one, in the header `Hale-Surface-Digest`.
`GET /.description` answers the caller's description with 200. The
response's status and body are the table's; a request that is neither
is `malformed`.

Every recorded exchange of both transports is under
`tests/api-contract/wire/<transport>/<outcome>.json`.

## The request lifecycle

A request is **received**; then **refused** (the checks of § Serving)
or **accepted**; an accepted request is **queued** on the receiver's
pool, under the exposure's `bound` and `on_full` as a topic binding's
are; then **executing**; then it has an **outcome**, which is
**delivered**, or **lost** when the connection is gone by then. `refuse`
is the one `on_full` policy for requests: a caller waiting for an
answer cannot be shed silently.

- **No cancellation.** A handler runs to completion. A client's timeout,
  or a connection lost after acceptance, never implies the operation
  did not run, and ending the wait never implies rollback.
- **`stop()`** on a serve handle stops accepting (a request received
  after it is refused `shutting_down`), refuses every queued request
  with `shutting_down`, lets the executing calls finish and delivers
  their outcomes to the connections still open; it returns when they
  have.
- **Receipts.** Long work returns an ordinary typed receipt and exposes
  its status through another rpc. Idempotency, durable execution and
  recovery are the application's.

## Structural failure in a handler

The runtime invokes a handler to produce a reply through the fallible
ABI F.42 (GH #1426) gives `violate`: a violation lands in the error slot
as the `ClosureViolation`, the runtime maps it to the server error
outcome, and the owner's `on_failure` runs as it does for any violation
(`spec/semantics.md` § Inline closure violation). No rule on the
handler is needed, and no caller ever reads a reply the handler did not
produce (R2).

## Streams

```hale,fragment
params { hub: ws::Hub = ws::Hub { bind: "127.0.0.1:9000", principals: self.bearer, roles: self.admin_roles }; }
bindings {
    Fills: self.hub requires: [operator], bound: 64, on_full: drop_old;
}
```

Streams are topic bindings, and the exposure's fields live on the
binding row: `requires`, `bound`, `on_full`. A **hub** is a transport
instance that implements the stream adapter (`__StdBusAdapter`) and may
implement `Rpc` as well, so one listener carries rpcs and streams over
one connection authenticated once, at connect (R5).

- **Admission.** A subscription is authorized against the row's
  `requires` before it is admitted, from the `Context` the hub's
  sources established; a caller who may not read a stream is refused
  and buffers nothing.
- **Delivery.** A publish on a hub-bound topic is accepted (the publish
  contract, `spec/semantics.md` § The publish contract) when it is
  handed to every admitted subscriber's queue, each holding at most
  `bound` frames and shedding under `on_full` (`drop_old` sheds the
  oldest undelivered frame, `drop_new` the frame being published);
  frames shed are counted and the count is reported on the next frame
  the subscriber receives.
- **Replay.** A reconnect implies replay only if the binding provides
  it, and its row says whether it does; a hub binding provides none in
  v1, so a reconnecting subscriber receives what is published after it
  is admitted again.
- **Expiry and revocation.** Authorization of a live subscription is
  not once for all. A bearer source states the credential's expiry on
  the `Context` it produces, and a role source carries a revision that
  changes when a grant is added or revoked. The hub invalidates a
  subscription when its credential expires, or when, at a role-source
  revision, its row's `requires` no longer holds for the subscriber: it
  sends one `unauthorized` frame naming the subscription, removes it,
  and drops whatever that subscription had buffered and not yet
  delivered. No event is delivered under an authorization older than
  the role source's current revision or past the credential's expiry;
  the check happens at the revision and at every delivery, whichever
  comes first. An rpc needs no such rule: each call is authorized on
  arrival.

There is no second stream declaration: the description derives every
stream a caller may use from the binding rows (§ The description).

## Codecs

A request, a response and an error cross under the exposure's codec
(F.36). The JSON codec is generated from the type, as the GH #1106
binding's was: `Int`, `Float`, `Bool`, `String` and nested structs of
the same, with a `json:"key"` tag renaming a key; an identity, a range,
a quantity and a point are their integer (`spec/units.md` § Layout and
the wire), and the description names the unit a quantity counts.
Decoding is strict: a value of the wrong JSON kind is `wrong_type`, a
missing field without a literal default is `missing_field`, and a
handler only ever sees a decoded value. A row whose shape has a field
the codec does not carry is refused by the admission law (law 5),
never left out.

## The description

A description is scoped to one **exposure**, the unit a caller can
reach: a serve site's surface over its transport under its role
source, identified as

```text
<surface>@<digest>/<exposure name>       Public@fnv1a64:fe65e6d3036ee1eb/public
```

The document a caller fetches from a listener (`GET /.description`,
`{"describe": true}`) describes that exposure only, filtered by the
caller's `Context` under that exposure's role source:

- the identity: `"description": 1`, the exposure, its name, the surface
  and its digest;
- the listener (transport and address) and the codec;
- the caller: the principal the exposure established, and the roles it
  holds among those the exposure's rows and streams require;
- the **members** the caller may call: name, request, response and
  error schemas, and `requires`;
- the **streams** of the hubs at that listener the caller may subscribe
  to: topic, wire subject, direction, payload schema, codec, `bound`,
  `on_full`, the loss statement, whether the binding replays, and
  `requires`;
- the transport's **outcome encoding** (§ Outcomes);
- the JSON Schema of every type the listed members and streams name,
  and nothing else, a quantity's field carrying `x-hale-unit` (its
  `q(…)` tag);
- the notes, in the document's own text: authorization is a boundary
  check at the serve site, never a proof over the program's internal
  call paths; a request accepted runs to completion whatever becomes of
  the caller's wait; the server still authorizes every request.

The same surface served twice has one digest and two exposures, and a
caller's description under each may differ: in the fixture, `alice`
may cancel through `public` and not through `partner`, `carol` the
reverse.

The **inventory** is the program-wide document `hale check --api`
prints from the rows, without a running program (R1): every surface
with its digest and all its rows, every exposure with its listener, its
sources and its receivers, and every hub with its stream rows. It is a
deployment inventory, not an authorization statement for any caller.

Both are versioned (`"description": 1`, `"inventory": 1`); their format
is `spec/api-description.schema.json`. A document is served as compact
JSON, its keys in the schema's order; the fixtures are the same values
pretty-printed, and a producer is held to them as values. The OpenAPI,
JSON Schema and MCP forms of GH #1107 are projections of the
description; a generated client is one per surface and carries the
digest.

## What this replaces

Today a program's API is a structural fact: one entry, `bindings { api:
unix(…) }` on the main locus, puts on the surface every topic the
seed's loci (and the loci `serve:` names) subscribe or publish and
every `expose` member of main and its default children, gated by
`@gated` on handlers and members; the entry is desugared before the
check into envelope types, topics, synthesized subscriptions and two
loci that own the socket, and the admission law reads locus rows. That
path is `spec/semantics.md` § The api binding (GH #1106), and it stays
the shipped behavior until R4 retires it: the entry, the synthesis, the
exposed reads, `@gated` and `serve:` go, the thirteen sites and the DNA
applications that declare the entry move to surfaces, and `hale call`,
`watch`, `admin` and `mcp` read descriptions. What carries over is
restated above in these terms: the Unix wire's line framing,
correlation and principal (§ Outcomes), the codec (§ Codecs), the
publish contract's bearing on streams (§ Streams), `std::api::Context`
and the two source interfaces (§ Serving).

## The witness

`tests/api-contract/` is the consumer fixture of the plan's § 3, made
by hand:

- `program.hl`: two surfaces sharing `Orders::cancel` under different
  `requires`; `Public` over `http::Rpc` twice (`public`, `partner`),
  under two role sources and bound to two `Orders` instances; `Admin`
  over `unix::Rpc` (`admin`) under a third; a fallible rpc
  (`Orders::cancel`); two rpcs that violate (`Orders::place`,
  `Ledger::rebalance`); one outward stream, `Fills`, through a
  `ws::Hub`. It is written in this document's syntax and parses from R1
  on;
- `<exposure>.<caller>.description.json`: each exposure's description
  for two callers;
- `inventory.json`: the program-wide document;
- `wire/unix/*.json`, `wire/http/*.json`: one request and its reply per
  outcome (result, handler error, each refusal kind, server error);
- `digest.md`: `Public`'s digest worked byte for byte.

`crates/hale-cli/tests/api_contract_fixtures.rs` validates every
document against the schema and holds the fixtures to this contract:
an exposure identity is its surface, digest and name; a caller's
description lists exactly the members whose `requires` the caller
holds; the digests are the ones `digest.md` folds; every wire record
encodes its outcome as § Outcomes says. The plan's § 3 assertions that
need a running program (refusals before a handler's counter moves,
queued shutdown, a lost response, revocation while connected) are the
exit criteria of R2, R3 and R5.

## Open points

- **The contract shape** a digest folds for a nested type or an enum
  (§ The contract digest; R1).
- **Additive compatibility**: a client built against a subset of a
  surface's members, after v1's equality.
- **A per-variant status mapping** declared on a handler's error type,
  which would join the rows, the description and the digest.
- **A receiver binding naming a `@form` collection's element** (a
  surface over many instances of one type, keyed by the request), or
  only a single instance.
- **The serve site's `bound` and `on_full`**: § The request lifecycle
  holds an exposure to them; their spelling at the serve site is R2's.
- **A hub that serves no surface**: its streams are in the inventory,
  and a description lists a stream only for an exposure served at the
  hub's listener, whose outcome encoding over WebSocket is R5's (the
  table above has no WebSocket column).
- **`on_full: refuse` on a stream**: what it does to a subscriber
  whose queue is full (the plan's own example writes it; the watcher
  queues of today know only `drop_old` and `drop_new`; R5).
- **Expiry and revision as interfaces**: the field on `Context` that
  states a credential's expiry, and how a role source announces a
  revision (R5).
- **MCP resources over streams.**
- **Whether a locus may implement two interfaces at once** (the hub);
  if not, the hub is two loci sharing one listener (R5).
- **A surface-level `requires` default** that rows inherit.
- **The wire framing of rpcs over WebSocket and UDP** (a correlation id
  field, the largest frame; R5, R6).
