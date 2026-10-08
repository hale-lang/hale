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
for nothing else: never for the listener address, the receiver
instances a serve site binds, the serve site itself, the build identity
(`exec_digest`) or the runtime incarnation. The digest is the
type-level contract a client is generated against; which instance
answers and under which role source is the exposure's (§ 2.8). The description carries
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
    let h1 = api::serve(Public, http::Rpc { bind: "0.0.0.0:8080", codec: json, principals: self.bearer, roles: self.public_roles },
                        as: "public", receivers: { Orders: self.orders }, bound: 64, on_full: refuse);
    let h2 = api::serve(Admin, unix::Rpc { path: "/run/app.sock", roles: self.admin_roles },
                        as: "admin", receivers: { Orders: self.orders, Ledger: self.ledger }, bound: 16, on_full: refuse);
    …
}
```

A serve site pairs one surface with one transport instance, names the
exposure (`as:`), supplies the bearer source and the role source, and
binds every receiver type the surface's rows name to one instance
(`receivers:`), and states the exposure's queue (`bound:`, the
requests accepted and not yet answered, and `on_full: refuse`, the
only policy for a request: a call is never dropped silently). A
`unix::Rpc` takes its principal from the kernel's peer credentials
(#1108) and names a bearer source only to accept tokens as well; an
`http::Rpc` always names one. The bound instance is the destination: its pool is
the dispatch pool and its lifetime bounds the exposure, so a receiver
must be a param of the serving locus or a child it owns for as long
as the handle lives (a `let`-bound child that dissolves before
`stop()` is refused). When the serving locus holds exactly one
instance of a receiver type the binding may be elided and is
inferred; two instances of the type and no binding is a check error
naming both, and two serve sites may bind the same surface to
different instances. The transport turns a
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
| handler error | the handler's declared `E` | 422, body = `E` by codec | `{"ok":false,"error":E}` | FAILED_PRECONDITION, `E` in details | error object with `E` |
| refusal | undecodable request, digest mismatch, unauthenticated, unauthorized, surface full, shutting down | 400 / 409 / 401 / 403 / 429 / 503 | `{"ok":false,"refusal":{kind,reason}}` | INVALID_ARGUMENT / FAILED_PRECONDITION / UNAUTHENTICATED / PERMISSION_DENIED / RESOURCE_EXHAUSTED / UNAVAILABLE | error object |
| server error | the handler violated (F.42's structural failure) | 500 | `{"ok":false,"refusal":{"kind":"server"}}` | INTERNAL | error object |
| transport failure | the connection or protocol broke | the transport's own | EOF | the transport's own | the transport's own |

The mappings are fixed in v1 and part of the contract (`spec/api.md`
states them; a transport may not choose others), so no row carries a
status and the digest has nothing to hash for it; a per-variant
mapping declared on `E` is an open point (§ 8).

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

An RPC handler is an ordinary method under F.42: if it returns a
value and may `violate`, it is declared `fallible(ClosureViolation)`,
and no exemption is added for it (the exemption is for bus handlers
and lifecycle bodies, whose caller is the runtime; an RPC handler's
internal callers write `or`, which is what keeps those calls safe).
The row's error type then decides the outcome: `ClosureViolation`
means the member's failure is the server error and the description
carries no error schema for it; any other `E` means the handler
error with `E`'s schema; a handler is one or the other, since a fn
has one error type. The runtime invokes the handler through the
fallible ABI, the violation lands in the error slot, the owner's
`on_failure` runs first as for any violation, and the undefined reply
of today (#1426's Deferred) is gone. The error slot of such a row in
the digest is `ClosureViolation`'s shape hash, so declaring or
removing a violation changes the digest, as an authority fact should.

### 2.7 Streams

Streams are topic bindings, and the exposure fields live on the
binding row:

```hale
params { hub: ws::Hub = ws::Hub { bind: "0.0.0.0:9000", principals: self.bearer, roles: self.public_roles }; }
bindings {
    Prices: self.hub requires: [trader], bound: 256, on_full: drop_old;
    Fills:  self.hub requires: [operator], bound: 64, on_full: drop_new;
}
```

A hub is a transport instance that implements the stream adapter
(`__StdBusAdapter`) and may implement `Rpc` as well, so one listener
carries rpcs and streams over one connection authenticated once at
connect. A stream row's `on_full` is `drop_old` or `drop_new`, the
two policies a watcher queue has; a stream is never `refuse`d, since
a subscriber that cannot keep up loses events, not the subscription.
A hub that serves no surface is still an exposure, of its stream rows
alone, identified `hub@<stream digest>/<name>`; the stream digest is
FNV-1a/64 over `hale-api-hub 1` and one line per stream row sorted by
topic (`topic`, payload shape hash, direction, codec, `bound`,
`on_full`, replay, `requires`), and a caller's description from the
hub's listener lists the streams the caller may subscribe to, with the
`ws` outcome form: the subscribe, subscribed, refusal, event (with a
per-subscription `seq` whose gaps are shed frames), unauthorized and
closed frames. R0 freezes the identity, the envelope and the
descriptions so a consumer can build against them; R5 delivers them. A subscription is authorized against the row's `requires`
before it is admitted, from the `Context` the hub's sources
established; a caller who may not read a stream buffers nothing. The
description derives every stream a caller may use from the binding
rows: payload shape, direction, codec, `bound` and `on_full`, and the
publish contract's delivery and loss statement (F.37). There is no
second stream declaration. A reconnect implies replay only if the
binding provides it, and the row says whether it does.

Authorization of a live subscription is not once-for-all. A bearer
source states the credential's expiry on the `Context` it produces,
and a role source carries a revision that changes when a grant is
added or revoked. The hub invalidates a subscription when its
credential expires or when, at a role-source revision, its row's
`requires` no longer holds for the subscriber: it sends one
`unauthorized` frame naming the subscription, removes it, and drops
whatever that subscription had buffered and not yet delivered. No
event is delivered under an authorization older than the role
source's current revision or past the credential's expiry; the
check happens at the revision event and at every delivery, whichever
comes first. An rpc needs no such rule, each call being authorized
on arrival.

### 2.8 The description

A description is scoped to one exposure, the unit a caller can
reach: a serve site's surface over its transport under its role
source, identified by `surface@digest/exposure-name` (the `as:` the
serve site declares, unique within the program). The document a
caller fetches from a listener describes that exposure only, filtered
by the caller's `Context` under that exposure's role source: the
members the caller may call (request and response schemas, error
schema, `requires`), the streams of the hubs at that listener the
caller may subscribe to (topic, direction, payload schema, bounds,
loss statement, `requires`), the outcome encoding of the transport,
and the identity. The same surface served twice has one digest and
two exposures, and a caller's description under each may differ. The
program-wide document `hale check --api` prints lists every exposure
with its listener and role source and is a deployment inventory, not
an authorization statement for any caller. Versioned
(`"description": 1`). The generators of #1107 (OpenAPI, JSON Schema, MCP tools)
are projections of this document; `hale check --api` prints it from
the rows without a running program (descriptions filtered by role are
the server's, at `GET /.description` or the unix `{"describe":true}`
request); generated clients are one per surface and carry the digest.

### 2.9 The runtime boundaries (R2)

The `Rpc` responsibility list of § 2.4 is a set of concrete types and
methods, published before any transport-specific dispatch so that
transports, hubs, DNA and Face build against one contract:

| boundary | the contract |
|---|---|
| **framed request** | what a transport hands the runtime: the member's identity, the payload bytes, the client's digest if it sent one, and the transport's correlation; distinct from the server's request identity, which the runtime assigns on acceptance |
| **exposure** | the surface, the bound receiver instances, the codec, the bearer source, the role source, the queue bound and the serve handle, built by `api::serve` |
| **admission** | the ordered checks of § 2.3, run before enqueue against the exposure's actual sources; R1's offline `--holds` flag is a description input, never a runtime authority |
| **dispatch** | enqueue on the bound instance's pool and await its typed outcome without blocking the scheduler work the handler, its owner's `on_failure` or the wait itself needs |
| **completion** | one owner of pending request state, reply storage and terminal completion, disconnected clients included |
| **transport** | listener lifecycle, framing, correlation and the wire encoding of the five outcomes; an ordinary connection failure (EOF, malformed input, a failed reply write) is local to that connection and never dissolves the shared listener; failure to bind at birth stays structural (§ 2.4) |
| **shutdown** | admission closes, queued work is refused `shutting_down`, executing work completes and replies if the connection lives, the listener is released, repeated `stop()` is safe, and the serve handle's dissolution or its owner's teardown drives the same shutdown, so cleanup never depends on a caller's explicit `stop()` |

A conforming in-process fixture transport ships with the interfaces
and exercises the same path as `unix::Rpc`.

### 2.10 Identity sources

The interfaces for credential validity and grant revision are R2's;
their use for live subscriptions (invalidation, delivery) stays R5's.

- A **bearer source** answers a credential with the principal and its
  validity: an expiry `Time` or none (no expiry), read against the
  runtime's clock; the exposure refuses `unauthenticated` past expiry.
- A **role source** answers a principal with its grants and the
  source's current revision as one pair, so a check never reads grants
  from one revision and the number from another; a revision changes
  when a grant is added or removed, and a subscriber of the source's
  revision topic learns of it (R5 reads it to invalidate live
  subscriptions).
- The runtime queries both on the exposure's own pool under Hale's
  cross-pool rules (a source is a locus the serving locus holds, placed
  with it or `sync = serialized`); a transport never holds policy.
- The interfaces are provider-independent: local bootstrap, a human
  identity federation and internal agent credentials are all sources.
- RPC authorization stays an admission-time check: expiry and
  revision add no re-authorization of queued calls and no cancellation
  of accepted work.

### 2.11 Receiver failure and execution generations

A bound receiver can become unavailable while its owner and the serve
handle live: draining after a violation, dissolved, or restarting.

- Unavailability is **per receiver**: the members bound to it are
  refused `unavailable` (503 / `UNAVAILABLE`); the exposure's other
  members serve on. Requests queued for that receiver settle
  `unavailable` too, a refusal the caller may read as "not executed".
- Each bound instance has an **execution generation**, advanced by a
  restart in place or a replacement in the owner's field. A call
  accepted under one generation never executes under the next: still
  queued, it is refused `unavailable`; executing, it completes under
  the generation that ran it. The binding follows the owner's field,
  so a replacement is served without re-establishing the exposure.
- A **lost connection after acceptance is uncertain**: the work
  executes once and its reply is dropped; the client is never told it
  was refused. Only `unavailable` and `shutting_down` on queued work
  mean "did not execute".
- Listener failure, receiver failure, disconnect and repeated `stop()`
  compose without duplicate completion or premature destruction:
  completion has one owner (§ 2.9), and each accepted request reaches
  one terminal outcome and releases its capacity exactly once.
- F.42 stands: a violating handler's owner runs `on_failure` first;
  when supervision lets execution return, the runtime reads the
  fallible result and answers the server error without exposing the
  record; when supervision exits the process, the caller observes a
  transport failure, never a swallowed structural failure.
- No implicit rollback or retry anywhere; durable idempotency,
  receipts and application retry are DNA's and the application's.

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
`Fills` and one with it gets them; a grant revoked while the
connection stays open ends that subscription with one `unauthorized`
frame and delivers nothing published after the revocation; `Public`
served a second time under a third role source at a second listener
has the same digest, a different exposure identity, and a description
that differs per caller between the two; and a second `Orders`
instance bound at the second serve site answers that exposure's calls
while the first answers the other's (two instances of one receiver
type, resolved by binding, with the unbound case a check error). One DNA read and one DNA durable
command are migrated to this path in R3, before the rest of DNA.

## 4. Steps

Each step is one pane and one PR, green on its own, with the spec
written before the code in the same PR. Each exit criterion is a test.

| step | delivers | exit criterion |
|---|---|---|
| **R0 contract** | `spec/api.md` (§ 2 as the one contract; the binding section of `semantics.md` reduced to a pointer plus what stays structural: publish contracts, codecs); the description format (§ 2.8) as a JSON Schema under `spec/`; hand-made conforming fixtures (`tests/api-contract/`: one description document for the § 3 program, one recorded request and reply per outcome, the digest algorithm worked by hand on one surface); `spec/registry.md`'s `surface` family named with its producers and consumers | `docs_snippets` green; a Rust test validates the fixture documents against the schema; the DNA and Face teams can build against the fixtures with no compiler change |
| **R1 rows** | the `api` block and `@rpc` parsed to one row family in `hale-model`; the digest; `hale check --api` printing the description from rows; the OpenAPI, JSON Schema and MCP generators re-homed onto rows; the admission law over rows (a row whose handler, shapes or error type do not exist is refused); registry family and laws | the § 3 program's rows, digest and description match R0's fixtures byte for byte; no `shape_hash` of a program without surfaces moves |
| **R2a interfaces** | § 2.9's boundaries as concrete stdlib types and the `Rpc` interface's methods, § 2.10's identity interfaces, § 2.11's semantics, all in `spec/api.md` first; `api::serve` checked by the serve-site laws and lowered to build the exposure; the runtime's admission, dispatch and completion over the bound instance's pool; a conforming in-process fixture transport; the reply handler through the fallible ABI (§ 2.6) | the § 3 fixture's rpc half passes over the in-process transport: refusals before the counter moves, the five outcomes, two instances of one receiver type reaching their own exposures, a violating call answered as the server error with the owner's `on_failure` first, same-pool and cross-pool placement without a blocked scheduler; R3 and R5 can build against the published interfaces |
| **R2b unix** | `unix::Rpc` on the R2a runtime (the #1106 socket and codec primitives reused, its binding synthesis not); the lifecycle evidence of § 4a over real sockets | § 4a's table passes over unix |
| **R3 http and the DNA proof** | `http::Rpc` with principals; the § 3 fixture's rpc half over unix and http (two exposures, two receiver instances, both role sources); one DNA read and one DNA durable command migrated to surfaces | the fixture's rpc half passes; the two DNA operations pass through the new path in the DNA suite |
| **R5 hubs** | `ws::Hub` (streams and rpc on one connection) and `udp::Hub`; stream rows' `requires`, expiry and revocation, description derivation; the § 3 fixture's `Fills` half, so the fixture passes whole | a subscriber without the role gets nothing and buffers nothing; revocation while connected ends the subscription; the description lists `Fills` only for `operator`; the § 3 fixture passes whole |
| **R4 cutover** | one coordinated replacement with the cleanup included (decision 21): every affected consumer on the new path (the 13 sites, the two DNA apps, DNA's verbs and reads, voice) and the superseded machinery deleted in the same cutover: `api_gen`'s synthesis, exposed reads, `@gated`, `serve:`, the api admission over locus rows, `bindings { api: … }`, the old description and client paths (`hale call`, `watch`, `admin`, `mcp` move to descriptions), `dna/api/contract/v1`, their fixtures, generators, commands and documentation; meaningful behaviour tests retargeted to the final path; forward migrations for any persisted state that changes; the F.40 leftovers in this area closed | `git grep 'api: unix'` and `git grep '@gated'` find nothing; no compatibility adapter, dual route or fallback exists; the DNA suite green; `spec/semantics.md` has no api binding section |
| **R6 transports** | `grpc::Rpc`, `mcp::Rpc` (tools are rpcs; resources over streams are an open point) | each passes the § 3 fixture's `Public` half over its protocol |

Dependencies: R0 opens as soon as #1426 is on `main`. R1 follows R0.
R2a follows R1 and is the first R2 handoff: R3 (HTTP) and R5 (hubs)
build against its published interfaces and fixture transport while
R2b (unix) finishes, and integrate on the verified R2b runtime. R4
follows R2b, R3 and R5: it is one cutover, not a separately supported
generation, and the packages before it establish none either. R6
follows R3 and keeps its own scope, as does the habitat product.

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
13. A serve site binds each receiver type to one instance
    (`receivers:`), inferred only when the serving locus holds exactly
    one; the instance's pool and lifetime are the exposure's.
14. A live subscription is re-authorized at the credential's expiry
    and at every role-source revision; an invalidated subscription
    gets one `unauthorized` frame and its undelivered buffer is
    dropped.
15. Discovery is per exposure (`surface@digest/exposure-name`),
    filtered by the caller under that exposure's role source; the
    program-wide description is an inventory.
16. Outcome mappings are fixed per transport in v1 and stated in the
    contract; no row or digest carries a status.
17. A serve site states its queue (`bound:`, `on_full: refuse`); a
    stream row's `on_full` is `drop_old` or `drop_new`; a hub that
    serves no surface is an exposure of its stream rows.
18. Credential expiry and the role-source revision are fields of the
    bearer and role source interfaces, shaped in R5 with the hub that
    reads them.
19. An RPC handler is an ordinary method under F.42, with no
    exemption and one added requirement: a handler that may violate
    declares `fallible(ClosureViolation)` whatever it returns, since a
    remote caller cannot read `self.draining` (law 7); a row whose
    error type is `ClosureViolation` fails as the server error and
    carries no error schema, any other error type as the handler
    error. `ClosureViolation`'s contract shape is its record's fields.
20. The hub exposure's identity, stream digest, frame envelope and
    caller-filtered description are part of the R0 contract; R5
    delivers them at run time.
21. The cutover is one coordinated replacement with the cleanup
    included: no compatibility adapters, dual serving, fallback routes
    or legacy clients; every affected consumer migrates and the old
    machinery, fixtures, generators, commands and docs are deleted in
    the same cutover; persisted state that changes gets a forward
    migration (the owner's direction, 2026-10-08).
22. R2 publishes § 2.9's concrete interfaces and a conforming
    in-process fixture transport first (R2a), then proves them over
    unix (R2b); every transport goes through the shared admission,
    dispatch and completion path.
23. The identity interfaces (§ 2.10: a bearer source's validity, a
    role source's grants-and-revision pair, the cross-pool rule) are
    R2's; live-subscription invalidation stays R5's; RPC authorization
    stays admission-time only.
24. Receiver unavailability is per receiver (`unavailable`, 503);
    queued calls to it settle `unavailable`; each bound instance has an
    execution generation advanced by a restart or replacement, and a
    call never moves to the next generation; the binding follows the
    owner's field.
25. A lost connection after acceptance is uncertain and is never
    reported as a refusal; only `unavailable` and `shutting_down` on
    queued work mean "did not execute"; no implicit rollback or retry.
26. Serve-handle dissolution or owner teardown drives shutdown; an
    ordinary connection failure is local to its connection.

## 8. Open points

- Additive compatibility of digests (a client built against a subset
  of members) after v1's equality.
- A per-variant status mapping declared on a handler's error type,
  which would join the rows, the description and the digest.
- Whether a receiver binding may name a `@form` collection's element
  (a surface over many instances of one type keyed by the request),
  or only a single instance.
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
