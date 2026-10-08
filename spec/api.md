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
which also computes the digest and prints descriptions; R2a ships the
runtime (the interfaces of § The runtime boundaries, the identity
sources, receiver failure, `api::serve` checked and lowered) over an
in-process fixture transport, `std::api::test::Rpc`; `unix::Rpc` proves
the same runtime over a socket in R2b and `http::Rpc` follows in R3;
hubs and stream authorization land in R5; R4 retires the structural
path (§ What this replaces). Each section names the step that ships it.
Until R1, the
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
    fn place(o: PlaceOrder) -> OrderReceipt fallible(ClosureViolation) { … }
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
| error | `E` when the handler is `fallible(E)`, `ClosureViolation` among them, or none |
| pool | the handler's pool, as the receiver instance a serve site binds is placed (§ Serving) |
| requires | the role names a caller must hold, as written |

A handler is a member fn of a locus. It takes at most one value
parameter, which is the request, and optionally a trailing `ctx:
std::api::Context` (§ Serving, `Context`), which is not part of the
request. The request, response and error types cross the boundary by
shape: a quantity's field carries its unit (the unit dialect's
`q(<denomination>)` tag, `spec/units.md` § Layout and the wire), so a
client knows `notional` counts `cent`.

**A handler is an ordinary method under F.42.** A handler that returns
a value and may `violate` is declared `fallible(ClosureViolation)`, as
any such method is (`spec/semantics.md` § A value-returning method that
may violate is fallible); being a row's handler adds no exemption, and
adds one requirement, law 7: a handler that may violate declares it
whatever it returns, where F.42 only warns about a method returning
nothing. The
exemption F.42 makes is for lifecycle bodies and bus handlers, whose
caller is the runtime and writes no `or`; a handler's callers inside
the program write `or` at every call, as the bare-call law requires,
and that is what keeps an internal call from reading a value the
handler never computed. A row's error column is therefore one of two
kinds, and the kind decides what a caller over a transport sees when
the handler fails:

- **`ClosureViolation`**: the failure is structural, the outcome is the
  **server error** (§ Outcomes), never the handler error, and a
  description carries no error schema for the member: its `error` is
  the string `"ClosureViolation"`;
- **any other `E`**: the failure is the **handler error**, its body `E`
  by codec, and the description carries `E`'s schema.

A handler is one or the other, never both, since a fn has one error
type and a fn that may violate declares no other (F.42). A handler
that may violate and returns a value but is not `fallible`, or is
`fallible(E)` with another `E`, is refused by F.42 at the check before
any row is admitted.

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
    fn place(o: PlaceOrder) -> OrderReceipt fallible(ClosureViolation) { … }
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
   \`plcae\`; did you mean \`place\`?", and "rpc \`Ordrs::place\`: no
   locus \`Ordrs\` is declared; did you mean \`Orders\`?". A lifecycle
   method, a mode or `on_failure` is no handler: "rpc \`Orders::run\`:
   \`run\` is a lifecycle method of \`Orders\`, and a handler is a
   member fn" (a mode and a failure handler are named so); and `@rpc`
   goes on a locus fn: "\`@rpc\` on \`helper\`: a handler is a member fn
   of a locus, and \`helper\` is a free fn".
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
   \`Ledger::dump\`: its response \`Export\` has a field \`raw:
   Bytes\`, which the JSON codec does not carry" (a field of a nested
   struct by its path, `inner.raw`), and for a type that is no struct
   the codec carries, "rpc \`Ledger::dump\`: its request is
   \`Bytes\`, which the JSON codec does not carry". A
   `ClosureViolation` error carries no schema (law 6), so no form is
   asked of it. R1 holds every row to the JSON codec, the one codec a
   serve site has (R2a lowers the JSON codec and no other; a serve site's
`codec:` is read when a second codec exists). A row is never
   left out of a served surface with a warning: the row is the intent,
   and an intent the program cannot honour is an error.
6. **A row's error type decides its failure.** A row whose error type
   is `ClosureViolation` fails as the server error and its description
   lists no error schema; a row with any other error type fails as the
   handler error with that type's schema. The check states it where it
   reports the row: "rpc \`Orders::place\`: its error type is
   \`ClosureViolation\`, so a failure is the server error; a description
   carries no error schema for it". The law is a statement of the row,
   not a refusal: the quoted line is a note `hale check --api` prints
   beside such a member, and the fact it states is what the description
   (the member's `error` form), the digest (the error slot) and the
   transport (the server-error outcome) each read.

7. **A handler that may violate is `fallible(ClosureViolation)`,
   whatever it returns.** F.42 requires the declaration of a
   value-returning method and only warns about one returning nothing,
   whose caller inside the program continues; a remote caller cannot
   read `self.draining`, so at the boundary the row requires it of both:
   "rpc \`Orders::flush\` may violate (\`violate stale\`) and returns
   nothing: an rpc handler that may violate is \`fallible(ClosureViolation)\`,
   so its caller receives the server error instead of a result". The
   may-violate judgment is F.42's.
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
     fallible; for a `fallible(ClosureViolation)` handler it is
     `ClosureViolation`'s (`locus:s;closure:s;diff:i`), so declaring a
     violation, or removing one, moves the digest as any change of
     error type does;
   - the required roles, sorted as bytes, each once, joined by `,`
     (0x2C), or `-` when the row requires none.

Member names and role names are identifiers and `::`, so no field
holds a TAB or a LF. The digest is the 64-bit FNV-1a fold
(`hale_graph::identity::Fnv64`: offset basis `0xcbf29ce484222325`,
prime `0x100000001b3`, each byte xor-ed in and then multiplied, the
fold every FNV identity uses, `spec/registry.md` § `digests`) over the
whole text, written `fnv1a64:` followed by sixteen lowercase hex
digits: `fnv1a64:a8930d6e7998e986`.

A type's **shape hash** is the hash of its **contract shape**
(`spec/model.md` § The shape of a type): the 64-bit FNV-1a fold of a
struct's fields in declaration order as `<field>:<tag>` joined by `;`
(`order:i;notional:q(cent)`, the tags of `spec/units.md` § Layout and
the wire), a nested struct or enum tagged by its own contract shape
hash (`#<hash>`), an enum `=enum(<variants>)`, and any other type `=`
and its tag. The form is deep, so a field changed inside a nested type,
or a variant added to an enum error type, moves every digest built on
it. A flat struct's contract shape is its payload contract's shape, so
its shape hash is the one a topic carrying it has always had; the
payload contract renders a nested field as the name-free tag `struct`
and keeps doing so, since it is a wire identity of its own.
`ClosureViolation`'s contract shape is its record's fields,
`locus:s;closure:s;diff:i`, folded like a flat struct (its payload
contract stays `opaque:ClosureViolation`).

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
        let public = api::serve(Public, http::Rpc { bind: "127.0.0.1:8080", codec: json, principals: self.bearer, roles: self.public_roles }, as: "public", receivers: { Orders: self.orders }, bound: 64, on_full: refuse);
        let admin = api::serve(Admin, unix::Rpc { path: "/run/desk/admin.sock", roles: self.admin_roles }, as: "admin", bound: 16, on_full: refuse);
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
- `bound:` and `on_full:` state the exposure's queue: the requests
  accepted and not yet answered that it holds, and `refuse`, the one
  policy for a request (§ The request lifecycle); a serve site that
  omits them is refused, as a topic binding without its bound is;
- `receivers:` binds every locus type the surface's rows name to one
  instance. The bound instance is the destination: its pool is the
  dispatch pool and its lifetime bounds the exposure. When the serving
  locus holds exactly one instance of a type, its binding may be
  elided and is inferred (both of `admin`'s above); two instances and no
  binding is an error naming both. Two serve sites may bind one
  surface to different instances;
- the call returns the **handle**: what the serving locus holds, joins
  and stops (`stop()`, § The request lifecycle).

**The exposure is a child of the serving locus** (R2a). A serve site
makes its exposure when the serving locus is born, as a param the
locus owns and settles with its other params (so before its `run()`),
and the `api::serve(…)` expression in a body is that exposure's
handle: its lifetime is the serving locus's, `stop()` or the locus's
dissolution shuts it down (§ The runtime boundaries, shutdown), and a
transport that cannot realize its listener fails the serving locus's
birth (§ The `Rpc` interface). The transport instance is held by the
exposure: a literal in the call is a child of the exposure, a name
(`self.fixture`) is a borrow of the instance the serving locus holds.
The receivers are the serving locus's own instances, bound by the
exposure to the bus (§ Receiver failure and generations); the role
source and the bearer source are held by reference.

The laws of a serve site (R2a; R1 parsed a serve site only as far as a
description names it, its surface, its transport instance's kind,
listener, codec and sources, `as:`, `receivers:`, `bound:` and
`on_full:`, checked none of these laws and refused to build a program
that held one):

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
5. **A serve site states its queue.** `bound:` and `on_full: refuse`
   are required: "serve of \`Public\` as \`public\`: a serve site states
   \`bound:\`, the requests it holds accepted and not yet answered, and
   \`on_full: refuse\`, the one policy for a request".

A serve site is also refused when it cannot be served at all: a surface
no \`api\` block (or \`@rpc\` handler) declares, "serve of \`Pubic\`: no
surface \`Pubic\` is declared; did you mean \`Public\`?"; no \`as:\`, "serve
of \`Public\`: a serve site names its exposure with \`as:\`"; no transport
instance; a receiver that is a param but is not built by a literal, "serve
of \`Public\`: \`orders\` is not built by a literal in a param of \`Desk\`: the
serve numbers the instance in the literal that builds it" (§ Receiver
failure and generations); and a serve site in a free fn, "\`api::serve\` in
\`serve_it\`: a serve site belongs to a locus's body, since its exposure is a
param of the serving locus; serve from the locus that holds the
receivers". A build refuses a serve site over a transport the compiler
does not ship (R2b and R3 add \`unix::Rpc\` and \`http::Rpc\`): "\`api::serve\`
over \`http::Rpc\`: this compiler serves a surface over
\`std::api::test::Rpc\`, the in-process transport, or a transport the
program declares; the socket transports follow".

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
   bound of requests accepted and not yet answered (`full`);
8. the receiver the member is bound to is not known to be unavailable
   (`unavailable`, § Receiver failure and generations).

Only then is the request **accepted**: it takes a server request id and
one unit of the exposure's bound, and is handed to its receiver's pool
through the cross-pool path (§ The runtime boundaries, dispatch), and
the runtime awaits its outcome without holding the pool the handler
needs. A request accepted can still end `unavailable` (its receiver
became unavailable before it ran) or `shutting_down` (the exposure
stopped before it ran): both mean it did not execute. **Authorization is
evaluated before anything is enqueued:** a refused request never
reaches the handler's queue, and its handler's state cannot tell it
arrived.

**Grants belong to the role-source instance**, never to a role name.
`public_roles` and `partner_roles` above are two sources: `alice`
holding `trader` in one holds nothing in the other, and the same
application deployed twice with two sources shares no grant. The
fixture's `admin_roles` and `hub_roles` grant `operator` to two
principals, the Unix peer `uid:1000` under the `admin` exposure and the
bearer `dave` under the hub (§ Streams), and neither holds it under the
other's source. A handler shared by two surfaces meets each surface's
`requires` under each exposure's source.

**`Context`** stays (#1108): a handler that declares `ctx:
std::api::Context` receives the caller the serve site established
(`caller`, a `std::api::Principal`), the request id, the transport it
came through (`via`: `unix`, `http`, `ws`, or `local` for a call made
inside the program) and the authorizing role (`role`: the first role
of the row's `requires` as written, empty for a row that requires
none). It is for the checks no row can state; the row's `requires` has
already held when the handler runs.

A handler of a served surface may declare `ctx: std::api::ServedContext`
instead: the same fields, and the name of the exposure the call came
through (`exposure`) and the execution generation of the receiver it
was admitted under (`generation`; § Receiver failure and generations),
which a call made inside the program has no value for ("" and 0).
`Context` itself is not extended: the standard library is lowered into
every program, so a field added to a stdlib type moves the IR of
programs that serve nothing; `ServedContext` is part of the runtime a
program carries only when it serves (§ The runtime boundaries).

**Descriptions read the rows dispatch reads.** Which members a
caller's description lists is decided by the same rows and the same
role source the checks above consult, so discovery and dispatch agree
by construction (§ The description). Discovery is a convenience: the
server still authorizes every request.

## The `Rpc` interface

`Rpc` is a stdlib `interface` in the shape of `__StdBusAdapter`,
implemented by a locus with lifecycle (`std::api::Rpc`, R2a). A
transport owns:

- **listening**, realized at birth: a listener it cannot bind fails the
  declaring locus's birth, as F.37 makes a binding that cannot open a
  birth failure (`spec/semantics.md` § The publish contract);
- **framing** a request to (member, bytes, correlation), and a reply
  back;
- **correlation**: the connection over HTTP and the Unix socket, an
  envelope id over UDP, the protocol's own over gRPC and MCP;
- **the encoding of each outcome** in its protocol's terms, fixed per
  transport (§ Outcomes).

Its methods, all of them infallible and all of them called by the
runtime, never the reverse:

```hale,fragment
interface std::api::Rpc {
    // The exposure that holds this transport tells it which exposure it
    // is; the answer is the transport's name for `Context.via`
    // ("unix", "http", "ws", "test").
    fn attach(exposure: Int) -> String;
    // Bytes in: one message of the transport's protocol, as the
    // transport read it from `correlation`'s connection, becomes a
    // `Request`. A message that is not a request is a `Request` of kind
    // `malformed` carrying the reason; framing never fails.
    fn frame(raw: Bytes, correlation: Int) -> std::api::Request;
    // Bytes out: the transport encodes `outcome` in its protocol's terms
    // (§ Outcomes) and writes it to `correlation`'s connection. A write
    // that fails is local to that connection.
    fn reply(correlation: Int, request_id: Int, caller: std::api::Principal, outcome: std::api::Outcome);
    // The runtime has finished with `correlation`'s connection: its
    // request completed after the connection was lost (§ The request
    // lifecycle), so the transport releases whatever it keeps for it.
    fn close_connection(correlation: Int);
    // Shutdown (§ The runtime boundaries): the exposure accepts nothing
    // more; the transport releases its listener.
    fn stop_listening();
}
```

The transport reaches the runtime over the bus, from its own receive
path: it publishes the raw message with its correlation
(`std::api::RpcIngress` on the wire subject `__api.rpc.ingress`, keyed
by the exposure it was attached to), and a lost connection
(`std::api::RpcLost` on `__api.rpc.lost`). A transport never decodes a
payload, never reads a role and never holds policy; the peer's
credentials travel in the `Request` its `frame` builds.

Its methods are infallible: a failure at the boundary is structural (a
dead listener is a birth failure, a broken connection the transport
failure outcome), never a value error, since a fallible interface call
does not lower (F.42, GH #1426). Dispatch, decoding by shape, the
`Context`, authorization and the digest check are the runtime's, and
the same for every transport. A transport may not own what is not its:
shapes and error types are the model's, the codec is the binding's
(F.36), roles and the bearer are the exposure's sources.

The stdlib implements `std::api::test::Rpc` (R2a: the in-process
fixture transport, below), `unix::Rpc` (R2b: the GH #1106 binding
re-homed), `http::Rpc` (R3), `ws::Hub` and `udp::Hub` (R5, § Streams),
`grpc::Rpc` and `mcp::Rpc` (R6); a program implements one the stdlib
lacks.

**The fixture transport** (`std::api::test::Rpc`) is a conforming
transport with no socket: a test hands it framed requests
(`call(correlation, member, payload, credential)`, `describe(…)`,
`lose(correlation)`) and reads the outcomes it was asked to deliver
(`outcome(correlation)`), each as the Unix JSON reply line of § Outcomes
(`request_id`, the client's `id`, the answer, `caller`). It goes through
the same admission, dispatch and completion as every transport, and it
is how the runtime is proven before a socket exists.

## The runtime boundaries

The `Rpc` responsibility list is a set of concrete types and methods,
published before any transport-specific dispatch so that transports,
hubs, DNA and Face build against one contract. Every transport goes
through this one path.

| boundary | the contract |
|---|---|
| **framed request** | what a transport hands the runtime: `std::api::Request`, the member's identity, the payload bytes, the client's digest if it sent one, and the transport's correlation; distinct from the server's request identity, which the runtime assigns when it receives the request |
| **exposure** | `std::api::Exposure`: the surface, the bound receiver instances, the codec, the bearer source, the role source, the queue bound and the serve handle, built by `api::serve` |
| **admission** | the ordered checks of § Serving, run before enqueue against the exposure's actual sources; the offline `--holds` flag of `hale check --api` is a description input, never a runtime authority |
| **dispatch** | enqueue on the bound instance's pool and await its typed outcome without blocking the scheduler work the handler, its owner's `on_failure` or the wait itself needs |
| **completion** | one owner of pending request state, reply storage and terminal completion, disconnected clients included |
| **transport** | listener lifecycle, framing, correlation and the wire encoding of the five outcomes; an ordinary connection failure (EOF, malformed input, a failed reply write) is local to that connection and never dissolves the shared listener; failure to bind at birth stays structural |
| **shutdown** | admission closes, queued work is refused `shutting_down`, executing work completes and replies if the connection lives, the listener is released, repeated `stop()` is safe, and the serve handle's dissolution or its owner's teardown drives the same shutdown, so cleanup never depends on a caller's explicit `stop()` |

**A lost connection.** A reply for a request whose connection was lost
before the request completed is dropped: completion (`finish`) sees the
lost mark, delivers nothing, and tells the transport the connection is
finished with (`close_connection`, once). A reply already written before
the loss is not unwritten: the record is gone, the later loss finds
nothing, and `close_connection` is not called. Either way the work ran
once and the unit of the bound is released once. The fixture transport's
`lose(correlation)` is synchronous with the mark: it returns once the
exposure has handled the loss, so a test that gates its handler and opens
the gate after `lose` holds the first order, and one that lets the handler
complete first holds the second, whatever the runner's speed.

**The runtime is part of a program only when it serves.** Every stdlib
declaration is lowered into every program, so the runtime of this
section (`api_rpc.hl`: the types below, the `Exposure`, the fixture
transport, the five topics) is not part of the bundled stdlib. The
compiler appends it to a program that serves a surface (`api::serve`) or
spells one of its names (`std::api::Request`, `std::api::test::Rpc`,
`std::api::ServedContext`, …), before the check; a program that does
neither lowers as it did before R2a, byte for byte.

```hale,fragment
// The framed request: what Rpc.frame returns.
type std::api::Request {
    kind: Int;                  // 0 a call, 1 a describe, 2 malformed
    member: String;             // "Orders::cancel"; empty for a describe
    bytes: Bytes;               // the payload, by the exposure's codec
    digest: String;             // the digest the client built against, or ""
    correlation: Int;           // the transport's: the connection, the envelope id
    credential: String;         // a bearer token the transport read, or ""
    peer: std::api::Principal;  // the peer the transport vouches for (a Unix
                                // peer's kernel credentials), or nobody
    reason: String;             // why a malformed request is malformed
}

// The outcome: what the runtime hands Rpc.reply. A transport failure is
// the absence of a reply, so it has no variant.
type std::api::Outcome = enum {
    Result(String),             // the response by the codec
    HandlerError(String),       // the handler's E by the codec
    Refusal(String, String, String),  // kind, reason, the extra: `served` on
                                // digest_mismatch, `requires` (comma-joined)
                                // on unauthorized, else ""
    ServerError,                // the handler violated; says nothing of it
};

// What a caller is handed to hold; stop() is idempotent.
interface std::api::Handle { fn stop(); }
```

**Receiving.** A request is received when the transport has published
it. The runtime takes the server's **request id** for it then, an
integer unique and increasing for the exposure's lifetime, and keeps
the transport's correlation beside it: the request id is the server's
identity for the request (the Unix reply's `request_id`, the `Context`'s
`request_id`), the correlation is the transport's way back to the
caller, and neither stands for the other.

**Admission** is the checks of § Serving, in that order, run on the
exposure's pool, one request at a time, against the sources the
exposure holds: the bearer source named by the transport instance's
`principals:` field and the role source named by its `roles:` field
(§ Identity sources). A refusal is an `Outcome::Refusal` handed to the
transport with the request id and the caller as far as it was
established.

**Dispatch** puts an accepted call on the pool of the receiver the
member is bound to, by the bus's cross-pool path, as a delivery to that
one instance, and never by a direct call (a method call across pools is
a check error, `spec/types.md` § Single-threaded-method invariant). The
call carries the request id, the member, the payload, the caller's
`Context` and the **generation** it was admitted under (§ Receiver
failure and generations), and is invoked on the receiver's pool through
the fallible ABI: a `fallible(E)` handler's error slot is the **handler
error**, a `ClosureViolation` is the **server error**, after the
owner's `on_failure` has run (§ Structural failure in a handler).
Nothing waits: the exposure holds the call as pending state, the
receiver's pool publishes the outcome back to the exposure when the
handler ends, and the exposure completes the request when that arrives.
A reply wait therefore never occupies the pool the handler, its owner's
`on_failure` or the completion needs, whether the receiver shares the
exposure's pool or not. An exposure puts **at most one call per
receiver** on that receiver's pool at a time and holds the rest in its
own queue, in the order accepted: a call is *queued* until the receiver
is free and *executing* from the moment it is on the receiver's pool, so
that what `stop()` and a receiver's failure refuse is exactly what has
not been handed over, and what has been handed over runs.

**Completion** has one owner, the exposure's pending table: a record
per accepted request keyed by the request id, holding the request (its
payload, the member), the `Context`, the generation, the transport's
correlation, whether the connection was lost, and where the request is
(queued, executing). The reply storage is the exposure's too: the
encoded outcome is the owned `String` the receiver's pool published, so
it outlives the handler's per-call scratch, and the request and the
`Context` live in the pending record from acceptance until completion,
whatever the transport does with its connection in between. A request
reaches one terminal outcome (an outcome delivered, an outcome dropped
because the connection was lost, or a refusal), and the record is
removed and the unit of the bound released exactly once, in that one
place (the table holds accepted requests and nothing else: a removed
record's place is taken by the next request, so it is never longer than
the bound, and the queue runs in the order of the request ids, not of the
places); a second event for the same request id finds no record and does
nothing.

**Shutdown.** `stop()` (idempotent) closes admission (a request received
after it is refused `shutting_down`), refuses every queued request
`shutting_down`, waits for the executing requests to complete and
delivers their outcomes to the connections still open, then has the
transport release its listener. The handle's dissolution and the serving
locus's teardown run the same shutdown, so a program that never calls
`stop()` still refuses its queue and lets its executing calls finish
before the listener goes. The wait is a yield to the scheduler (a
sleep, which drains the pool's queue), not a block, and it is bounded:
`stop()` waits ten seconds for a handler, a teardown one second, since a
teardown may be the process's, and a process shuts the pools of its
receivers down under the calls they are running. A call still executing
when the wait ends is abandoned: its reply is dropped, the transport is
told the connection is finished with (`close_connection`), its unit of
the bound is released once, and its caller observes the transport
failure, never a refusal (it may have run).

**Describing.** The runtime answers a describe request from the
exposure's live sources: the members the caller's `Context` may call
(`requires` held under this exposure's role source), by the rows the
admission checks read, so a description and an admission of the same
caller agree. The full document of § The description is what
`hale check --api` prints from the rows; the exposure's live answer
carries the identity, the caller and the members, and the schemas stay
the document's.

## Identity sources

The interfaces for credential validity and grant revision are R2's;
their use for live subscriptions (invalidation, delivery) stays R5's.

- A **bearer source** answers a credential with the principal and its
  validity: `fn expiry(token: String) -> Int` of
  `std::api::ExpiringBearerSource`, the instant the credential expires
  as nanoseconds since the Unix epoch (`std::time::nanos` of a `Time`),
  0 for no expiry, read against the runtime's clock
  (`std::time::current()`); the exposure refuses `unauthenticated`,
  reason `expired`, a credential past its expiry. A source written
  before the interface existed satisfies only `BearerSource` and is
  asked nothing about expiry: its credentials do not expire. (The
  answer is an `Int`, not a `Time`, because a locus method that returns
  a `Time` does not lower yet; the unit is the one `Time` is.)
- A **role source** answers a principal with its grants and the
  source's current revision as one pair, `std::api::Grants { roles:
  String, revision: Int }` (`roles` the comma-joined roles the principal
  holds directly), from `fn grants(p: Principal) -> Grants` of
  `std::api::RevisedRoleSource`, so a check never reads grants from one
  revision and the number from another. A revision changes when a grant
  is added or removed, and the source publishes the new revision on the
  wire subject `__api.roles.revision` (a `std::api::Revision { source:
  String, revision: Int }`; the standard library declares no `topic`, so
  a publisher writes `publish "__api.roles.revision" of type
  std::api::Revision;`) for a subscriber to learn of it; R5 reads it to
  invalidate live subscriptions. `holds(p, r)` of
  `std::api::RoleSource` stays the direct question: a source that
  implements `grants` as well is asked it once per request, one that does
  not is asked `holds` once per role the check needs, with the revision
  unstated (0).
- The runtime queries both on the exposure's own pool under Hale's
  cross-pool rules: a source is a locus the serving locus holds, placed
  with the exposure (or `sync = serialized`); a transport never holds
  policy.
- The interfaces are provider-independent: a local bootstrap table, a
  human identity federation and internal agent credentials are all
  sources.
- RPC authorization stays an admission-time check: expiry and revision
  add no re-authorization of queued calls and no cancellation of
  accepted work.

The stdlib's static table answers the pair as `std::api::RevisedStaticRoles`
(an `inner: std::api::StaticRoles` and a `revision`, 0 for a table fixed
for the process): its `grants` is the roles of the table the principal
holds, `holds` is `inner`'s. `StaticRoles` and `NoBearer` themselves are not
edited, for the reason `Context` is not (§ Serving): a stdlib locus is
lowered into every program. A bearer source that states no expiry is any
that has no `expiry`, `NoBearer` among them.

The optional interfaces change no existing program: `Principal` and the
two source interfaces keep the layout and the methods they had, so a
source written for the structural path is a source here, and the
exposure asks the extension only of a source whose declaration states
it (the compiler wires `expiry` and `grants` from the source's type).

## Outcomes

A caller sees one of five outcomes, and every transport encodes each,
by a mapping fixed in v1 and part of this contract: no transport
chooses another, no row carries a status, and the digest has nothing
to hash for one.

| outcome | meaning | HTTP | Unix JSON | gRPC | MCP |
|---|---|---|---|---|---|
| result | the handler returned | 200, body = the response by codec | `{"ok": true, "value": …}` | OK | result |
| handler error | the handler failed with its declared `E` | 422, body = `E` by codec | `{"ok": false, "error": E}` | FAILED_PRECONDITION, `E` in details | error object with `E` |
| refusal | the request was not accepted, or was accepted and did not run (the kinds below) | 400 / 409 / 401 / 403 / 429 / 503, body = `{"refusal": …}` | `{"ok": false, "refusal": {"kind": …, "reason": …}}` | INVALID_ARGUMENT / FAILED_PRECONDITION / UNAUTHENTICATED / PERMISSION_DENIED / RESOURCE_EXHAUSTED / UNAVAILABLE | error object |
| server error | the handler failed with `ClosureViolation`, its row's error type (§ Structural failure in a handler) | 500, body = `{"refusal": {"kind": "server"}}` | `{"ok": false, "refusal": {"kind": "server"}}` | INTERNAL | error object |
| transport failure | the connection or the protocol broke | the transport's own | EOF | the transport's own | the transport's own |

The refusal kinds, each with its status:

| kind | when (§ Serving, the checks) | HTTP | gRPC |
|---|---|---|---|
| `malformed` | not a request; no such member (`unknown_member`); a payload that does not decode (`missing_field`, `wrong_type`) | 400 | INVALID_ARGUMENT |
| `digest_mismatch` | the request's digest is not the served one; the refusal carries `"served"` | 409 | FAILED_PRECONDITION |
| `unauthenticated` | the sources name nobody | 401 | UNAUTHENTICATED |
| `unauthorized` | the `Context` does not hold the row's `requires`; the refusal carries `"requires"` | 403 | PERMISSION_DENIED |
| `full` | the exposure holds its bound of requests accepted and not yet answered | 429 | RESOURCE_EXHAUSTED |
| `shutting_down` | the exposure is stopping (§ The request lifecycle); the request was queued and did not run | 503 | UNAVAILABLE |
| `unavailable` | the receiver the member is bound to is draining, dissolved or restarting (§ Receiver failure and generations); the request did not run | 503 | UNAVAILABLE |

A refusal object is `{"kind": K, "reason": "<text>"}`, plus `"served"`
on `digest_mismatch` and `"requires"` on `unauthorized`. The server
error's object is `{"kind": "server"}` and says nothing of the
violation, which is the program's to report (§ Structural failure in a
handler).

**The Unix JSON transport** (`unix::Rpc`, R2b; the R2a fixture transport
encodes its outcomes as these lines) is a Unix domain stream
socket carrying one JSON object per line, as the GH #1106 binding's
wire was. A request:

```text
{"call": "Orders::cancel", "payload": {"order": 41}, "id": "c-1", "digest": "fnv1a64:40381db6685c9f75"}
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
or **accepted**; an accepted request is **queued** in the exposure
under its `bound` and `on_full` (as a topic binding's are), and handed to
its receiver's pool when the receiver is free (§ The runtime
boundaries, dispatch); then **executing**; then it has an **outcome**,
which is **delivered**, or **lost** when the connection is gone by then.
`refuse` is the one `on_full` policy for requests: a caller waiting for
an answer cannot be shed silently.

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

**What a request guarantees.** These five hold for every transport, and
`tests/hale/api/` holds the runtime to each:

1. **Waiting permits supervision to progress.** A reply wait, and
   `stop()`, cannot block the pool needed to run the handler or its
   owner's `on_failure`: the wait is the exposure's pending state, not a
   thread parked on the receiver's pool, and `stop()` waits by yielding
   to the scheduler (a sleep drains the pool's queue). This holds with
   the receiver on the exposure's pool and with it on another.
2. **Requests and replies outlive their use.** The request and its
   `Context` live in the pending record until completion, and survive the
   enqueue; the reply is an owned value that survives the handler's
   scratch cleanup; a disconnect frees nothing an executing request still
   uses.
3. **Completion and capacity are accounted for once.** Each accepted
   request reaches exactly one terminal outcome and releases its unit of
   the bound exactly once; a disconnected, still-executing request stays
   accounted for until it completes.
4. **No implicit rollback or retry.** A lost response cancels nothing
   and authorizes no resubmission; an explicit shutdown refuses the
   queued work and the executing work finishes.
5. **F.42 stands.** A violating handler is `fallible(ClosureViolation)`;
   its owner's `on_failure` runs first; when supervision lets execution
   return, the runtime reads the fallible result and answers the server
   error without exposing the record; when supervision exits the process,
   the caller observes a transport failure, never a swallowed structural
   failure; any other error type is the handler error.

A queued call known not to have executed (`unavailable`,
`shutting_down`) is distinguished from a call whose execution or
delivery is uncertain (a connection lost after acceptance), which is
never reported as a refusal.

## Receiver failure and generations

A bound receiver can become unavailable while its owner and the serve
handle live: draining after a violation, dissolved, or restarting.

- Unavailability is **per receiver**: the members bound to it are
  refused `unavailable` (503 / `UNAVAILABLE`, the Unix kind
  `unavailable`); the exposure's other members serve on. Requests queued
  for that receiver settle `unavailable` too, a refusal the caller may
  read as "not executed".
- Each bound instance has an **execution generation**, advanced by a
  restart in place or a replacement in the owner's field. A call
  accepted under one generation never executes under the next: still
  queued, it is refused `unavailable`; executing, it completes under the
  generation that ran it. The binding follows the owner's field, so a
  replacement is served without re-establishing the exposure.
- A **lost connection after acceptance is uncertain**: the work
  executes once and its reply is dropped; the client is never told it
  was refused. Only `unavailable` and `shutting_down` on queued work
  mean "did not execute".
- Listener failure, receiver failure, disconnect and repeated `stop()`
  compose without duplicate completion or premature destruction:
  completion has one owner (§ The runtime boundaries), and each accepted
  request reaches one terminal outcome and releases its capacity
  exactly once.
- F.42 stands (§ Structural failure in a handler): a violating
  handler's owner runs `on_failure` first; when supervision lets
  execution return, the runtime reads the fallible result and answers
  the server error without exposing the record; when supervision exits
  the process, the caller observes a transport failure.
- No implicit rollback or retry anywhere; durable idempotency, receipts
  and application retry are DNA's and the application's.

**How the runtime knows.** A receiver instance is bound to its exposure
by the serve site: the instance gains, at the serve site's compile, a
subscription of its own to the exposure's calls, a number the exposure
addresses it by, and an incarnation stamp it sets at every `birth()`
(its first, a restart's, a replacement's). It announces the stamp to
the exposure at each birth, and its `dissolve()` announces that it is
gone. The exposure keeps, per bound receiver, the stamp of the
incarnation it knows and a generation that grows by one at every
announcement; a call carries the generation it was admitted under and
the stamp it was handed over to, and the receiver executes a call only
if the stamp is its own incarnation's, so a call handed to an
incarnation that ended is never run by the next one, even at an address
the next one reuses. A receiver that is draining has no subscription to
hand a call to: the exposure learns it at the handover (the bus reports
a delivery no subscriber took) and settles that call, and every call
queued behind it, `unavailable`. A receiver whose stamp the exposure
has not yet learned (it was born before the exposure, or has not
answered) is asked, and its calls wait queued until it answers or the
bus reports no subscriber to ask. A receiver found unavailable is asked
again when the next request for it arrives, since it may be back (a
`restart(c)` lowers a failure's drain request and runs no `birth()`, so
nothing is announced); one that answers is a new generation, which
calls accepted before it do not enter. An announcement of an incarnation
the exposure already knows changes nothing, and the departure of an
incarnation a successor has already replaced is news to nobody.

One cell the exposure cannot take back: a call already handed to a
receiver (executing, § The runtime boundaries, dispatch) when another
exposure's call makes it fail. Two exposures bound to one instance each
hand it a call; the runtime has no primitive that withdraws a queued
cell, so the second runs if the failing receiver's pool still delivers
it, and is answered as it ends (R2b may narrow the window).

The instance bound is addressed by the number the serve site gives it,
which the compiler writes into every literal that builds the field's
instance in the serving locus: the param's own, and the right-hand side
of each `self.orders = Orders { … }` assignment. A receiver replaced
that way keeps the binding, because the number rides in the literal;
an instance built anywhere else carries no number and is bound to no
exposure.

## Structural failure in a handler

A handler that may violate is `fallible(ClosureViolation)` (§ Surfaces
and their rows, F.42), and the runtime calls it as any caller calls a
fallible fn (R2a): through the fallible ABI (GH #1426), so a violation
arrives in the error slot as the `ClosureViolation` the owner's
`on_failure` received, and no value is produced. The owner's
`on_failure` runs first, as for any violation (`spec/semantics.md` §
Inline closure violation, step 5); then the runtime answers the caller
with the server error, whose object says nothing of the violation,
which is the program's to report. The runtime's call is the transport
caller's `or`: it reads the error slot and never a reply the handler
did not produce. Nothing beyond F.42 and law 7 is asked of the handler.

The owner's `on_failure` decides what happens next, and the caller
sees it: when it lets execution return (it absorbs the failure, or
restarts or quarantines the receiver), the request is answered with the
server error; when supervision exits the process instead (`bubble` to
the root, a `restart` budget spent), the caller observes a transport
failure, never a swallowed structural failure. After a violation the
receiver is draining or restarting: its other members' queued calls
settle `unavailable` (§ Receiver failure and generations) while the
exposure's other receivers serve on.

## Streams

```hale,fragment
params {
    hub_roles: Grants = Grants { operator: "dave" };
    hub: ws::Hub = ws::Hub { bind: "127.0.0.1:9000", principals: self.bearer, roles: self.hub_roles, as: "fills" };
}
bindings {
    Fills: self.hub requires: [operator], bound: 64, on_full: drop_old;
}
```

Streams are topic bindings, and the exposure's fields live on the
binding row: `requires`, `bound`, `on_full`. A **hub** is a transport
instance that implements the stream adapter (`__StdBusAdapter`) and may
implement `Rpc` as well, so one listener carries rpcs and streams over
one connection authenticated once, at connect (R5). A hub's **stream
rows** are the topic bindings to it, each with its topic, payload,
direction (`out` for a topic the program publishes, `in` for one it
subscribes), codec, `bound`, `on_full`, whether it replays, and
`requires`.

- **Admission.** A subscription is authorized against the row's
  `requires` before it is admitted, from the `Context` the hub's
  sources established; a caller who may not read a stream is refused
  and buffers nothing.
- **Delivery.** A publish on a hub-bound topic is accepted (the publish
  contract, `spec/semantics.md` § The publish contract) when it is
  handed to every admitted subscriber's queue, each holding at most
  `bound` frames and shedding under `on_full` (`drop_old` sheds the
  oldest undelivered frame, `drop_new` the frame being published).
  Every event offered to a subscription takes the next `seq` of that
  subscription, delivered or shed, so the frames shed are the gap
  between two `seq`s the subscriber receives.
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
  sends one `unauthorized` frame naming the subscription's topic and
  why (`expired`, `revoked`), removes it, and drops whatever that
  subscription had buffered and not yet delivered. No event is
  delivered under an authorization older than the role source's
  current revision or past the credential's expiry; the check happens
  at the revision and at every delivery, whichever comes first. An rpc
  needs no such rule: each call is authorized on arrival.

There is no second stream declaration: the description derives every
stream a caller may use from the binding rows (§ The description).

**The hub exposure.** A hub that serves no surface is an exposure too,
of its stream rows alone. Its name is the hub's `as:` field, unique
among the program's exposure names as a serve site's `as:` is, and its
identity is

```text
hub@<stream digest>/<name>       hub@fnv1a64:26970854397ab154/fills
```

The **stream digest** is the hub's contract as a surface digest is a
surface's, and is folded the same way (§ The contract digest: the
64-bit FNV-1a fold, written `fnv1a64:` and sixteen lowercase hex
digits) over a UTF-8 text of lines, each ended by one LF:

1. the header line `hale-api-hub 1`;
2. one line per stream row, the rows ordered by topic name, compared
   as bytes. A row's line is eight fields separated by one TAB:
   - the topic (`Fills`);
   - the payload's shape hash, sixteen lowercase hex digits (§ The
     contract digest, the shape hash);
   - the direction, `out` or `in`;
   - the codec (`json`);
   - `bound`, in decimal;
   - `on_full`, `drop_old` or `drop_new`;
   - replay, `1` when the binding replays and `0` when it does not;
   - the required roles, sorted as bytes, each once, joined by `,`, or
     `-` when the row requires none.

It moves when a stream is bound to the hub or unbound, its payload's
shape changes, or any of its direction, codec, `bound`, `on_full`,
replay or `requires` changes: unlike an rpc's, those are what a
subscriber's loss statement is made of. It never covers the listener's
address, the hub's name or sources, the subscribers, the build identity
or the incarnation. `tests/api-contract/digest.md` works `fills`'s by
hand.

**The `ws` frames.** A hub speaks JSON frames, one object per WebSocket
text message, each naming itself in `"type"`; this is the `ws` outcome
form a hub exposure's description states:

| frame | from | shape |
|---|---|---|
| subscribe | client | `{"type": "subscribe", "topic": T}` |
| subscribed | hub | `{"type": "subscribed", "topic": T}`: the subscription is admitted |
| refusal | hub | `{"type": "refusal", "topic": T, "refusal": {"kind": K, "reason": …}}`: the subscription is not admitted |
| event | hub | `{"type": "event", "topic": T, "seq": N, "payload": P}` |
| unauthorized | hub | `{"type": "unauthorized", "topic": T, "reason": R}`, `R` being `"expired"` or `"revoked"`, once; then the subscription is gone |
| closed | hub | `{"type": "closed", "reason": "shutting_down"}`, at the hub's `stop()`; then the connection closes |

A refusal's object is § Outcomes', its kind one of `unauthenticated`
(the sources named nobody at connect), `unauthorized` (the `Context`
does not hold the row's `requires`, and the object carries
`"requires"`), `malformed` (not a frame, or a topic the hub binds no
stream for, reason `unknown_topic`) and `shutting_down`. `P` is the
payload by the row's codec. `seq` starts at 1 for a subscription and
grows by one per event offered to it, so a gap is the frames shed (the
delivery bullet above). A connection that ends without a `closed` frame
is the transport failure. The frames are R0's contract, which a
consumer builds against; the runtime that sends them, the timing of
expiry and revocation, and the framing of rpcs over WebSocket (a
correlation field, the largest frame) are R5's.

## Codecs

A request, a response and an error cross under the exposure's codec
(F.36). The JSON codec is generated from the type, as the GH #1106
binding's was: `Int`, `Float`, `Bool`, `String` and nested structs of
the same, with a `json:"key"` tag renaming a key; a plain alias
(`type Count = Int;`) is what its chain ends at, and a builtin record
(`IndexError`) its fields as the description gives them; an identity, a range
and a quantity are their integer (`spec/units.md` § Layout and the
wire: read as an `Int` and converted, a quantity by its denomination,
`n * 1cent`; a range narrows, and a value outside it is `wrong_type`),
and the description names the unit a quantity counts. A point is not
carried (its origin is not a count): a row whose shape holds one is
refused by law 5. Decoding is strict: a value of the wrong JSON kind is
`wrong_type`
(a payload that is one scalar is that scalar's complete JSON token: `true`
or `false` for a `Bool`, one quoted and correctly escaped string for a
`String`, a number for a `Float`, a number with no fraction or exponent for
an `Int` and for the integer a unit scalar counts; anything else is
refused before the request is queued), a
missing field without a literal default is `missing_field`, and a
handler only ever sees a decoded value. A row whose shape has a field
the codec does not carry is refused by the admission law (law 5),
never left out.

## The description

A description is scoped to one **exposure**, the unit a caller can
reach: a serve site's surface over its transport under its role
source (or a hub's stream rows, below), identified as

```text
<surface>@<digest>/<exposure name>       Public@fnv1a64:a8930d6e7998e986/public
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
  error schemas, and `requires`; a member whose error type is
  `ClosureViolation` carries the string `"ClosureViolation"` in place
  of an error schema (§ Surfaces and their rows);
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

**A hub exposure's description** (§ Streams, the hub exposure) is the
same document, fetched from the hub's listener (`GET /.description` at
its address, as over HTTP, before any WebSocket upgrade) and filtered
by the caller's `Context` under the hub's role source, with these
differences: the identity is `hub@<stream digest>/<name>` and the digest the stream
digest; `surface` is `null`; `members` is empty; `streams` holds the
hub's stream rows whose `requires` the caller holds; the outcome
encoding is the `ws` form, the frames of § Streams and nothing of an
rpc's (no member, no status); and the schemas are those of the listed
streams' payloads only, so a caller who may subscribe to nothing
receives no payload schema. In the fixture, `dave` holds `operator`
under `hub_roles`, so the description of `fills` for `dave` lists
`Fills` with `Fill`'s schema; `bob` holds nothing there, and the one
for `bob` lists no stream and no schema.

The **inventory** is the program-wide document `hale check --api`
prints from the rows, without a running program (R1): every surface
with its digest and all its rows, every exposure with its listener, its
sources and its receivers, and every hub with its exposure identity,
its stream digest, its listener, its sources and its stream rows. It is
a deployment inventory, not an authorization statement for any caller.

`hale check --api --exposure NAME --caller PRINCIPAL [--holds ROLE,…]`
prints one exposure's description for one caller, from the same rows.
What a caller holds is its role source's to say when the program runs
(a role source is program code, `fn holds`, which the check does not
run), so the roles it holds under that exposure's source are an input,
and the description lists exactly what they admit; the caller's `roles`
are those of them the exposure's rows and streams require. PRINCIPAL is
the principal as the exposure establishes it, a name (its mode the
transport's: `unix` over the Unix socket, `bearer` otherwise) or the
JSON object (`{"mode": "unix", "name": "uid:1000", "uid": 1000, …}`).
A served description is the server's, at `GET /.description` or `{"describe":
true}` (R2), its caller and roles established at the request. From R1
the compiler's documents for `tests/api-contract/program.hl` are the
fixtures beside it, byte for byte.

What R1 reads of a serve site, and only that: its surface (its first
argument, a surface's name); its transport instance's kind (the
literal's namespace, `http`, `unix`), its address (`bind:` or `path:`),
its codec (`codec:`, `json` when it names none) and its bearer and role
sources (`principals:` and `roles:`, each a `self.<param>` named with
the param's type, `kernel` for the Unix transport's peer when it names
no bearer source); `as:`; the `receivers:` it binds, and, for every
other locus type the surface's rows name, the one instance the serving
locus holds of it, inferred; `bound:` and `on_full:`. A receiver's pool
is the placement table's. Of a hub: the param a stream row binds
(`self.hub`), its literal's kind, `bind:`, `codec:` (`json` when none),
sources and `as:`; of each stream row, its topic and wire subject, its
direction (`out` when the program publishes the topic), the payload,
`bound:`, `on_full:` and `requires:`, and no replay.

Both are versioned (`"description": 1`, `"inventory": 1`); their format
is `spec/api-description.schema.json`. A document is served as compact
JSON, its keys in the schema's order; the fixtures are the same values
pretty-printed, and a producer is held to them as values. The OpenAPI,
JSON Schema and MCP forms of GH #1107 are projections of a surface's
rows, one per surface, each carrying the digest: `hale check --api
--surface NAME --openapi` (a `POST /call/<member>` per row, `200` its
response, `422` its handler error, `500` a `ClosureViolation` row's
server error, the refusals by their statuses, the digest as the
`Hale-Surface-Digest` header), `--json-schema` (every type the rows name
under `$defs`, each member's types by reference) and `--mcp` (a tool per
row, its input the request's schema, self-contained; a request whose
schema is not an object is wrapped as an object with one required
property named after the handler's parameter, which the MCP transport
unwraps before decoding by shape, as MCP requires an object); a builtin
record a row names (`IndexError`, …) is a schema of its contract-shape
fields, under its name; every component a form adds that is not a user
type is namespaced `hale.` (`hale.Refusal`), a name no Hale identifier
can spell, so none collides with the program's types; the fixture
program's are pinned beside the R0 documents
(`<Surface>.<form>.json`). The structural path's forms (`hale describe
--openapi`, `--mcp`) are unchanged until R4.

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
  over `unix::Rpc` (`admin`) under a third, whose operator is the Unix
  peer `uid:1000`; a fallible rpc (`Orders::cancel`,
  `fallible(OrderError)`: the handler error); two rpcs that violate
  (`Orders::place`, `Ledger::rebalance`, each
  `fallible(ClosureViolation)`: the server error); one outward stream,
  `Fills`, through a `ws::Hub` under a fourth role source, `hub_roles`,
  whose operator is the bearer `dave`. It is written in this document's
  syntax and parses from R1 on;
- `<exposure>.<caller>.description.json`: each exposure's description
  for two callers, the hub's `fills` among them (`dave`, who may
  subscribe to `Fills`, and `bob`, who may not);
- `inventory.json`: the program-wide document;
- `wire/unix/*.json`, `wire/http/*.json`: one request and its reply per
  outcome (result, handler error, each refusal kind, server error);
- `digest.md`: `Public`'s digest worked byte for byte, `Admin`'s, and
  the hub `fills`'s stream digest;
- `<Surface>.openapi.json`, `<Surface>.json-schema.json`,
  `<Surface>.mcp.json` (R1): each surface's three projections, as the
  compiler prints them.

`crates/hale-cli/tests/api_contract_fixtures.rs` validates every
document against the schema and holds the fixtures to this contract: an
exposure identity is its surface, digest and name; a caller's
description lists exactly the members whose `requires` the caller
holds, and exactly the streams of the hubs at its listener whose
`requires` it holds, with the schemas of what it lists and no other; a
hub exposure's identity is `hub@<stream digest>/<name>`, and its `ws`
form admits no member and no status; the digests, the stream digest
among them, are the ones `digest.md` folds; a row whose error type is
`ClosureViolation` carries no error schema and is the only kind of
member a server error is recorded for; every wire record encodes its
outcome as § Outcomes says. From R1 it also runs the compiler over
`program.hl` and holds what it prints to the fixtures byte for byte:
the inventory, each description for its caller, the model's digests and
row shape hashes against `digest.md`, and each surface's projections.
The plan's § 3 assertions that need a
running program (refusals before a handler's counter moves, queued
shutdown, a lost response, revocation while connected) are the exit
criteria of R2, R3 and R5. R2a holds the rpc half of them over the
in-process transport (`tests/hale/api/`: the witness's own `Orders`,
`Ledger`, rows and role sources served by `std::api::test::Rpc`,
each refusal leaving the handler's counter where it was, the five
outcomes, the lifecycle guarantees of § The request lifecycle); R2b
runs the same assertions over a Unix socket, R3 over HTTP, R5 adds the
stream half.

## Open points

- **Additive compatibility**: a client built against a subset of a
  surface's members, after v1's equality.
- **A per-variant status mapping** declared on a handler's error type,
  which would join the rows, the description and the digest.
- **A receiver binding naming a `@form` collection's element** (a
  surface over many instances of one type, keyed by the request), or
  only a single instance.
- **A hub that also serves a surface** (rpcs and streams on one
  connection): its description lists both, and the frames that carry
  an rpc over WebSocket are R5's (below).
- **A dispatch window wider than one call per receiver**: the exposure
  hands a receiver one call at a time (§ The runtime boundaries,
  dispatch) so that what it refuses is exactly what has not been handed
  over; a wider window needs a runtime primitive to withdraw a queued
  cell (R2b).
- **The hub's use of expiry and revision** (R5): the interfaces are
  § Identity sources'; a hub's invalidation of a live subscription from
  them is R5's.
- **MCP resources over streams.**
- **Whether a locus may implement two interfaces at once** (the hub);
  if not, the hub is two loci sharing one listener (R5).
- **A surface-level `requires` default** that rows inherit.
- **The wire framing of rpcs over WebSocket and UDP** (a correlation id
  field, the largest frame; R5, R6).
