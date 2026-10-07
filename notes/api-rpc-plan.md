# The API surface rework (#1417) — plan

The direction was decided on 2026-10-06 (the issue) and sharpened on
2026-10-07 by a consumer's requirements (the issue's first comment)
and by F.42 (a value-returning fn that may violate is fallible). This
plan turns the direction into steps a pane can deliver, each a PR,
each green, each with its exit criterion. The consumer fixture of § 3
is the acceptance of the whole track.

## 1. Where the tree is

- One entry, `bindings { api: unix(…) }` on the main locus, binds the
  program's API: every topic a surfaced locus subscribes is a
  command, every topic it publishes a stream, every `expose` member
  of main or a param-default child a read; `@gated` on a handler and
  `role` declarations are the role rows; `serve: [p]` adds a held
  locus type; `http(…, principals: …)` adds a second transport. The
  entry is desugared before the check (`hale-syntax/src/api_gen.rs`)
  into envelope types, topics, a synthesized subscription per
  subscriber, a read subject per exposed member and two loci on an
  `async_io` pool that own the socket. The admission law
  (`check.rs` `check_api_binding`) reads those locus rows.
- 13 files declare the entry: 11 under `tests/hale`, the
  `92-build-an-api` example, one CLI fixture; two DNA apps
  (`dna/api/main.hl`, `dna/api/practice_review/main.hl`) use `unix`
  plus `http`, `roles:` and `serve:`. Seven DNA api tests and the
  `dna/api/contract/v1` artifacts (OpenAPI, JSON Schema, a validator)
  describe the structural path. No downstream handoff uses the entry.
- The readers: `hale-cli/src/api_client.rs` (`hale call`, `watch`,
  `admin`), `mcp.rs`, the OpenAPI, JSON Schema and MCP description
  generators of #1107; `std::api::Context` and the principal at the
  binding (#1108); `__StdApiRoleSource`, `__StdApiBearerSource`,
  `__StdApiStaticRoles`, `__StdApiNoBearer` in `api.hl`.
- The spec: `spec/semantics.md` § "The api binding", 1,389 lines.
- The stdlib's adapter interface is `__StdBusAdapter { fn send(subject:
  String, bytes: Bytes); }`, and the unix transports are loci with
  lifecycle whose birth realizes the transport (F.37: a binding the
  transport cannot honour fails the declaring locus's birth).
- Two facts from F.42 (#1426): a `fallible` perspective or interface
  call does not lower, so an interface's methods are infallible and a
  failure at the boundary is structural; and a bus handler that
  declares a reply type and violates still returns an undefined value
  to the runtime, which this track closes (§ 2.6).

## 2. The contract, as the compiler will hold it

Exposure is an intent about the boundary, held in rows the model
hashes, not in the structure of a locus. Five families, one spec
(`spec/api.md`, new; the binding section of `semantics.md` goes).

### 2.1 Surface rows

```hale
api Public {
    rpc Orders::place;
    rpc Orders::cancel requires: [trader];
}
api Admin {
    rpc Orders::cancel requires: [operator];
    rpc Ledger::rebalance requires: [operator];
}
```

A surface is a named table; each `rpc` line is one row: the surface,
the handler (`Locus::fn`), the request shape, the response shape (the
unit dialect's `q(<denomination>)` tags included, so a quantity
crosses with its unit), the error type when the handler is
`fallible(E)`, the handler's pool, and `requires`, the role names a
caller must hold. A handler may sit in several surfaces; two versions
over one set of handlers are two surfaces; a surface declared and
never served is a table nobody reads, which is how a library ships one
for an app to serve. `@rpc` on a handler, with an optional
`requires:`, is sugar that contributes the same row to the seed's
default surface, named after the seed; the block and the sugar feed
one family and nothing else about the locus is read. A reply type on
a bus handler no longer makes it an rpc: a handler is an rpc because a
row says so.

### 2.2 The contract digest

A surface's digest is the model's hash over its rows in canonical
order: member name, request and response shape hashes, error type
shape, `requires`. It changes when a member is added or removed, a
shape or error type changes, or an authority requirement changes, and
for nothing else: never for the listener address, the build identity
(`exec_digest`) or the runtime incarnation. The description carries
the surface name and digest; a request may carry the digest it was
generated against, and a mismatch is refused before anything is
decoded or queued, with the refusal naming the digest the server
serves so the client can re-fetch the description. v1 compatibility
is equality; additive compatibility (a client built against a subset)
is an open point (§ 8).

### 2.3 Serving, authorization, `Context`

```hale
params {
    bearer: std::api::Bearer = …;          // __StdApiBearerSource
    public_roles: std::api::Roles = …;     // __StdApiRoleSource
    admin_roles: std::api::Roles = …;
}
run() {
    let h1 = api::serve(Public, http::Rpc { bind: "0.0.0.0:8080", codec: json, principals: self.bearer, roles: self.public_roles });
    let h2 = api::serve(Admin, unix::Rpc { path: "/run/app.sock", principals: self.bearer, roles: self.admin_roles });
    …
}
```

A serve site pairs one surface with one transport instance and
supplies the bearer source and the role source. The transport turns a
request into (row, bytes, correlation); the runtime establishes the
caller's `Context` (principal and roles) from the serve site's
sources, checks the surface digest, checks the row's `requires`
against the `Context`, decodes the payload by shape with the codec,
enqueues the call on the handler's pool through the existing
cross-pool path, and awaits the reply. Authorization is evaluated on
the transport's thread before anything is enqueued: a refused request
never reaches the handler's queue. Grants belong to the role-source
instance the serve site names, never to a role name, so the same
application deployed twice with different sources shares nothing. A
handler shared by two surfaces meets each surface's `requires`. The
handler's `std::api::Context` parameter stays (#1108) for the checks
no row can state. Descriptions are filtered by the caller's roles and
read the same rows dispatch does, so discovery and dispatch agree by
construction. The handle a serve returns is what `main` holds, joins
and stops.

### 2.4 The `Rpc` interface

`Rpc` is a stdlib `interface` in the shape of `__StdBusAdapter`,
implemented by a locus with lifecycle: it owns listening (realized at
birth; a listener it cannot bind fails the declaring locus's birth,
as F.37 does for a broker), framing a request to (row id, bytes,
correlation) and a reply back, correlation (an envelope id over UDP,
the connection over HTTP and unix, the protocol's own over gRPC and
MCP), and the encoding of each outcome (§ 2.5) in its protocol's
terms. Its methods are infallible: the boundary's failures are
structural, not value errors. Dispatch, decoding by shape, `Context`,
authorization and the digest check are the runtime's and the same
for every transport. The stdlib implements `unix::Rpc` (the #1106
binding re-homed), `http::Rpc`, `ws::Hub` (§ 2.7), `udp::Hub`,
`grpc::Rpc` and `mcp::Rpc`; a user implements one the stdlib lacks. A
transport is refused if it tries to own what is not its: shapes and
error types (the model's), the codec (F.36), roles and bearer (the
surface's and the serve site's).

### 2.5 Outcomes and the request lifecycle

A caller sees five outcomes, and every transport encodes each:

| outcome | meaning | HTTP | unix JSON | gRPC | MCP |
|---|---|---|---|---|---|
| result | the handler returned | 200, body by codec | `{"ok":true,…}` | OK | result |
| handler error | the handler's declared `E` | 4xx class the surface maps, body = `E` by codec | `{"ok":false,"error":E}` | status the surface maps | error object with `E` |
| refusal | digest mismatch, unauthorized, surface full, shutting down | 409 / 401 or 403 / 429 / 503 | `{"ok":false,"refusal":{kind,reason}}` | FAILED_PRECONDITION / PERMISSION_DENIED / RESOURCE_EXHAUSTED / UNAVAILABLE | error object |
| server error | the handler violated (F.42's structural failure) | 500 | `{"ok":false,"refusal":{"kind":"server"}}` | INTERNAL | error object |
| transport failure | the connection or protocol broke | the transport's own | EOF | the transport's own | the transport's own |

A request's life: received → refused (digest, authorization, bound
`on_full`, shutting down) or accepted → queued on the handler's pool
(the serve site's `bound` and `on_full`, as a topic binding's) →
executing → an outcome → delivered, or lost when the connection is
gone by then. Handlers run to completion: there is no cancellation of
an executing call, and a client's timeout or a lost connection after
acceptance never implies the operation did not run; ending the wait
never implies rollback. `stop()` on a serve handle stops accepting,
refuses what is queued with `shutting_down`, lets executing calls
finish and replies to them if the connection is alive. Long work
returns a typed receipt and exposes its status through another rpc;
idempotency and recovery are the application's.

### 2.6 Structural failure in a reply handler

A handler invoked by the runtime to produce a reply is invoked through
the fallible ABI F.42 gave `violate`: a violation lands in the error
slot as the `ClosureViolation`, the runtime maps it to the server
error outcome, and the owner's `on_failure` runs as it does for any
violation. The undefined reply of today (#1426's Deferred) is gone
without any rule on the handler.

### 2.7 Streams

Streams are topic bindings, and the exposure fields live on the
binding row:

```hale
params { hub: ws::Hub = ws::Hub { bind: "0.0.0.0:9000", principals: self.bearer, roles: self.public_roles }; }
bindings {
    Prices: self.hub requires: [trader], bound: 256, on_full: drop_old;
    Fills:  self.hub requires: [operator], bound: 64, on_full: refuse;
}
```

A hub is a transport instance that implements the stream adapter
(`__StdBusAdapter`) and may implement `Rpc` as well, so one listener
carries rpcs and streams over one connection authenticated once at
connect. A subscription is authorized against the row's `requires`
before it is admitted, from the `Context` the hub's sources
established; a caller who may not read a stream buffers nothing. The
description derives every stream a caller may use from the binding
rows: payload shape, direction, codec, `bound` and `on_full`, and the
publish contract's delivery and loss statement (F.37). There is no
second stream declaration. A reconnect implies replay only if the
binding provides it, and the row says whether it does.

### 2.8 The description

One JSON document per deployment, versioned (`"description": 1`):
the served surfaces (name, digest, listener, codec, members with
request and response schemas, error schema, `requires`), the bound
streams (topic, hub listener, direction, payload schema, bounds,
loss statement, `requires`), and the outcome encoding of each
transport. The generators of #1107 (OpenAPI, JSON Schema, MCP tools)
are projections of this document; `hale check --api` prints it from
the rows without a running program (descriptions filtered by role are
the server's, at `GET /.description` or the unix `{"describe":true}`
request); generated clients are one per surface and carry the digest.

## 3. The witness: the consumer fixture

`tests/hale/api/consumer_fixture/` (built as one program, served in
one process, driven by `hale test`): two surfaces `Public` and
`Admin` sharing `Orders::cancel` under different `requires`; `Public`
over `http::Rpc` and `Admin` over `unix::Rpc` with two role sources;
one `fallible(OrderError)` rpc; one outward binding `Fills` through a
`ws::Hub`. The fixture asserts: the generated description and a
generated client agree (every member the client can call is in the
description for its roles, and nothing else); an unauthorized call on
each surface is refused before the handler's counter moves; a digest
mismatch is refused naming the served digest; the fallible rpc's
error arrives as the handler error outcome; a violating rpc arrives as
the server error and the owner's `on_failure` ran; `stop()` refuses
the queued call with `shutting_down` and finishes the executing one;
a lost connection after acceptance leaves the operation executed
(the next read shows it); a subscriber without the role gets no
`Fills` and one with it gets them. One DNA read and one DNA durable
command are migrated to this path in R3, before the rest of DNA.

## 4. Steps

Each step is one pane and one PR, green on its own, with the spec
written before the code in the same PR. Each exit criterion is a test.

| step | delivers | exit criterion |
|---|---|---|
| **R0 contract** | `spec/api.md` (§ 2 as the one contract; the binding section of `semantics.md` reduced to a pointer plus what stays structural: publish contracts, codecs); the description format (§ 2.8) as a JSON Schema under `spec/`; hand-made conforming fixtures (`tests/api-contract/`: one description document for the § 3 program, one recorded request and reply per outcome, the digest algorithm worked by hand on one surface); `spec/registry.md`'s `surface` family named with its producers and consumers | `docs_snippets` green; a Rust test validates the fixture documents against the schema; the DNA and Face teams can build against the fixtures with no compiler change |
| **R1 rows** | the `api` block and `@rpc` parsed to one row family in `hale-model`; the digest; `hale check --api` printing the description from rows; the OpenAPI, JSON Schema and MCP generators re-homed onto rows; the admission law over rows (a row whose handler, shapes or error type do not exist is refused); registry family and laws | the § 3 program's rows, digest and description match R0's fixtures byte for byte; no `shape_hash` of a program without surfaces moves |
| **R2 serve** | the `Rpc` interface and the runtime's dispatch (`Context` from the serve site's sources, digest check, `requires` before enqueue, decode by shape, cross-pool enqueue, awaited reply, the five outcomes, `stop()` semantics); `unix::Rpc` re-homed from #1106; the reply handler invoked through the fallible ABI (§ 2.6); the serve handle | the § 3 fixture's unix half passes: refusals before the counter moves, the five outcomes on the wire, queued shutdown, lost response |
| **R3 http and the DNA proof** | `http::Rpc` with principals; the full § 3 fixture; one DNA read and one DNA durable command migrated to surfaces | the § 3 fixture passes whole; the two DNA operations pass through the new path in the DNA suite |
| **R4 cutover** | the structural path retired: `api_gen`'s synthesis, exposed reads, `@gated`, `serve:`, the api admission over locus rows, `bindings { api: … }`; the 13 sites and the two DNA apps migrated; `hale call`, `watch`, `admin`, `mcp` over descriptions; `dna/api/contract/v1` replaced by the description; the seven DNA api tests over the new path; the F.40 leftovers in this area closed (`hale check --api` is a reader; the round-trip artifacts) | `git grep 'api: unix'` finds nothing; the DNA suite green; `spec/semantics.md` has no api binding section |
| **R5 hubs** | `ws::Hub` (streams and rpc on one connection) and `udp::Hub`; stream rows' `requires` and description derivation; the § 3 fixture's `Fills` half | a subscriber without the role gets nothing and buffers nothing; the description lists `Fills` only for `operator` |
| **R6 transports** | `grpc::Rpc`, `mcp::Rpc` (tools are rpcs; resources over streams are an open point) | each passes the § 3 fixture's `Public` half over its protocol |

R0 opens as soon as #1426 is on `main`. R1 and R2 may run as two
panes once R0 is merged, R2 against R0's fixtures. R3 waits for R2;
R4 for R3; R5 for R2; R6 for R3.

## 5. What is retired, re-homed, kept

- Retired (R4): exposure as a structural property (the api binding as
  a birth-realized child, exposed fields as read subjects, `@gated`,
  `serve:`), the admission law over locus rows, `hale check --api`'s
  F.40 wording, the `dna/api/contract/v1` artifacts.
- Re-homed: the unix JSON binding as `unix::Rpc` (R2); descriptions
  and generic clients as readers of rows (R1, R4); roles from
  `@gated` to `requires` on rows (R1); a reply type on a bus handler
  from "this is an rpc" to "a reply the runtime may ask for" (R2).
- Kept: `std::api::Context` and the principal at the serve site
  (#1108); `__StdApiRoleSource`, `__StdApiBearerSource` and the two
  stdlib sources; the codec clause (F.36); the publish contract
  (F.37); topic bindings as the only stream declaration.

## 6. Exit criteria for the track

The § 3 fixture passes over unix, http and ws; one DNA read and one
durable command run through surfaces; `spec/api.md` is the contract
and the binding section is gone; the registry has the `surface`
family with its laws; no program without surfaces changed a hash; the
descriptions a consumer built against in R0 are what R1 generates.

## 7. Decisions

1. `@rpc` is sugar beside the block; both feed one row family.
2. The authority requirement is a property of the row (`requires`),
   never of the handler; `@gated` goes.
3. Authorization is evaluated at the serve site before enqueue; a
   refused request never reaches the handler's queue.
4. Grants belong to the role-source instance named at the serve site.
5. The digest is over rows (members, shapes, error types, requires)
   and excludes listener, build identity and incarnation; v1
   compatibility is equality.
6. Five outcomes, encoded by every transport; a violation in a reply
   handler is the server error (F.42's error slot).
7. No cancellation of an executing call; `stop()` refuses the queue,
   finishes the executing.
8. Streams are binding rows with `requires`; descriptions derive from
   them; no second declaration.
9. A hub may serve both the stream adapter and `Rpc` on one listener.
10. `Rpc`'s methods are infallible; a listener that cannot bind fails
    the declaring locus's birth (F.37).
11. One-release cutover (R4), no temporary route table; the 13 sites
    and the two DNA apps migrate in the same PR.
12. The contract (R0) ships before any transport, with fixtures
    consumers build against.

## 8. Open points

- Additive compatibility of digests (a client built against a subset
  of members) after v1's equality.
- MCP resources over streams.
- Whether a locus may serve two interfaces at once (the hub); if not,
  the hub is two loci sharing one listener, decided in R5.
- Whether a surface can carry a `requires` default that rows inherit.
- The wire framing of rpcs over WebSocket and UDP (correlation id
  field, maximum frame), decided in R5 and R6.

## 9. Risks

- R4 is DNA-heavy and gated by the DNA suite; the two early
  migrations in R3 are what make it a mechanical step rather than a
  design one.
- The `Rpc` interface is the first stdlib interface with several
  methods; interface dispatch limits found in #1426 (fallible calls
  do not lower) shape it, and a new limit found in R2 is a stop rule.
- Descriptions filtered per caller must stay cheap: the filter is a
  set test over `requires`, computed from the same rows as dispatch.
