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

GH #1417. This document is the contract. The `api` block and `@rpc`
parse to rows (R1), which also compute the digest and print descriptions;
the runtime (the interfaces of § The runtime boundaries, the identity
sources, receiver failure, `api::serve` checked and lowered) runs over an
in-process fixture transport, `std::api::test::Rpc` (R2a), `unix::Rpc` (R2b)
and `http::Rpc` (R3); hubs and stream authorization are `ws::Hub` and
`udp::Hub` (R5: § Streams); `mcp::Rpc` is the MCP transport (R6); and the
structural path is retired (R4: § What this replaced), the clients reading
descriptions (§ The clients). Each section names the step that ships it.
Since GH #1565 an exposure is a param of the locus that holds it, its
literal the serve call, placed by `placement { }` (§ Serving); the serve
expression in a body, the earlier form, is refused. A program calls
another system's surface through a client, a param built by
`api::client`, typed by the surface's rows (§ Calling a surface).
The fixtures under `tests/api-contract/` are what a consumer builds
against: the consumer program of § The witness, its descriptions per
exposure and caller, the program-wide inventory, one recorded request and
reply per outcome per transport, and the digest worked byte for byte. The
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

`api NAME [requires: [ROLE, …]] { … }` at top level declares a
**surface**; each `rpc Locus::fn [requires: [ROLE, …]];` line in it is
one **surface row**. A `requires` on the surface is its rows' default: a
row that writes none holds the surface's roles, and a row that writes its
own (`requires: []` for none) holds those instead. The default is resolved
into the rows where the block is read, so a surface written with it and
the same surface written row by row are one table and one digest, and the
admission law judges each row's roles (law 4 names the row). A surface
the program consumes rather than serves names each row's shape instead of
a handler (§ Imported surfaces). A row's columns:

| column | what it is |
|---|---|
| surface | the block's name |
| member | the handler as a caller names it: `Locus::fn`, the locus in its author spelling (`lib::Orders::cancel` for a locus another seed declares); a signature row's name (§ Imported surfaces) |
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
   \`Bytes\`, which the JSON codec does not carry". A list is a field's
   (§ Codecs): a request or response that is itself one is refused with
   the record to declare, "rpc \`Shelf::bare\`: its response is \`[Int;
   3]\`, a list, which crosses as a field of a record: declare one
   (\`type Page { items: [Int; 3]; }\`)". A
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
8. **A surface is served or consumed, never both.** Its rows all name
   handlers, or all name shapes (the signature rows of § Imported
   surfaces): "\`Mixed\` holds \`shape\`, a row that names a shape, and
   \`Desk::post\`, a row that names a handler: a surface is served (its
   rows name handlers) or consumed (its rows name shapes); declare two
   surfaces". A block that says where it was imported from (`from
   openapi "…"`) holds signature rows only: "rpc \`Desk::post\`:
   \`Notes\` is an imported surface, and its rows name the operations'
   shapes, not handlers: \`rpc name(Request) -> Response;\`".

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
hash (`#<hash>`), an enum `=enum(<variants>)` (a variant's declared
error class after it, `Missing@not_found`, § Error classes), and any other type `=`
and its tag. The form is deep, so a field changed inside a nested type,
or a variant added to an enum error type, moves every digest built on
it. A flat struct's contract shape is its payload contract's shape, so
its shape hash is the one a topic carrying it has always had; the
payload contract renders a nested field as the name-free tag `struct`
and keeps doing so, since it is a wire identity of its own.
`ClosureViolation`'s contract shape is its record's fields,
`locus:s;closure:s;diff:i`, folded like a flat struct (its payload
contract stays `opaque:ClosureViolation`).

**A member's digest** is the same fold over that member's line alone,
under its own header: the text `hale-api-member 1`, LF, and the row's
line as above (`tests/api-contract/digest.md` folds one by hand,
`fnv1a64:03c08c25c690f01f` for `Public`'s `Orders::place`). It moves when
that member's shapes, error type or required roles change, and for
nothing else: a member added to the surface, or removed from it, or
another member changed, leaves it where it is. A description lists each
member's digest beside its name (`"digest"`, § The description).

**Compatibility is additive (2026-10-18).** A description carries its
surface's name and digest, and a generated client carries the surface
digest it was generated against (`SURFACE_DIGEST`) and, on each call,
names its member's digest (`Hale-Surface-Digest` over HTTP and gRPC,
`"digest"` in a Unix line). A request may carry a digest; one that is
neither the served surface's nor its member's served digest is refused as
`digest_mismatch` before its payload is decoded or anything is queued,
and the refusal names the digest the exposure serves (`"served"`), so
the client can fetch the description again. So a client built against a
surface keeps calling it after the surface gains members, or loses ones
the client does not call, and is refused only for a member whose own
contract moved; a request naming the old surface's digest, which no
longer describes what is served, is refused as before. A request that
carries no digest is not checked. The admission runs in the exposure,
before the request reaches the receiver's dispatch. (Until 2026-10-18
compatibility was equality: only the surface's digest was admitted.)

## Serving

```hale,fragment
main locus Desk {
    params {
        bearer: Tokens = Tokens { };            // a std::api::BearerSource
        public_roles: Grants = Grants { trader: "alice" };       // a std::api::RoleSource
        admin_roles: Grants = Grants { operator: "uid:1000" };
        orders: Orders = Orders { };
        ledger: Ledger = Ledger { };
        public: api::Exposure = api::serve(Public, http::Rpc { bind: "127.0.0.1:8080", codec: json, principals: self.bearer, roles: self.public_roles }, as: "public", receivers: { Orders: self.orders }, bound: 64, on_full: refuse);
        admin: api::Exposure = api::serve(Admin, unix::Rpc { path: "/run/desk/admin.sock", roles: self.admin_roles }, as: "admin", bound: 16, on_full: refuse);
    }
    placement { public: cooperative(pool = io) where async_io; }
    run() {
        self.public.wait();
        self.public.stop();
        self.admin.stop();
    }
}
```

An **exposure** is a param (GH #1565): `NAME: api::Exposure =
api::serve(SURFACE, TRANSPORT, as: NAME, receivers: { TYPE: INSTANCE, … },
bound: N, on_full: refuse);` in the params of the locus that holds it, its
**holder**. The `api::serve(…)` call is the param's literal, the **serve
site**; it pairs one surface with one transport instance:

- the **surface** is named as declared (`Public`); a surface an imported
  seed declares is named by its path under the import alias
  (`lib::Svc`), and it is that seed's surface, described by its declared
  name (`Svc`): a library ships a surface and its receivers, and the
  application serves it with receivers it holds (`echoer: lib::Echoer`).
  The alias reaches the blocks its seed declares, whatever their rows
  name: a block of the library whose rows all name a locus the library
  imports is the library's, and an application's own block is never
  `lib::`'s, though its rows name the library's loci. A path that names
  no surface of the aliased seed is refused in the author's spelling
  ("serve of `lib::Own`: no surface `lib::Own` is declared");
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
  dispatch pool and its lifetime bounds the exposure. When the holder
  holds exactly one instance of a type, its binding may be elided and is
  inferred (both of `admin`'s above); two instances and no binding is an
  error naming both. Two serve sites may bind one surface to different
  instances;
- the param is the **handle**: what the holder holds, waits on and stops
  (`self.public.wait()`, `self.public.stop()`, the `std::api::Handle`
  methods; `std::api::run_until_stopped(self.public)` is the wait).

**The exposure is a child of its holder.** It is born with the holder's
other params, in declaration order (so before the holder's `run()`), and
dissolves with them: its lifetime is the holder's, `stop()` or the
holder's dissolution shuts it down (§ The runtime boundaries, shutdown),
and a transport that cannot realize its listener fails the program's
birth (§ The `Rpc` interface). The transport instance is held by the
exposure: a literal in the call is a child of the exposure, a name
(`self.fixture`) is a borrow of the instance the holder holds. The
receivers are the holder's own params, bound by the exposure to the bus
(§ Receiver failure and generations); the role source and the bearer
source are held by reference. The exposure runs on its holder's pool,
beside the sources it calls.

**Any locus may hold an exposure.** A server is a locus like any other:

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

**Where the listener runs.** A socket transport's listener (`unix::Rpc`,
`http::Rpc`, `mcp::Rpc`, `grpc::Rpc`) accepts and reads on a coroutine
that parks, so it runs on a named pool `where async_io`. The compiler adds
it to the main locus as a param of its own (`__rpc_u_<n>`, `__rpc_h_<n>`,
`__rpc_g_<n>`) and places it, with the connections it starts. It is
born as early in the main locus's birth as its address allows: for an
exposure the main locus holds, just before the exposure; for one a server
below it holds, just before the param of the main locus the server sits
under when the address reads nothing of the server, and just after it
when it does (`bind: self.bind`), so the server is built first. A child
born on the main thread runs its `run()` where it is born, before the next
param of its owner is born (`spec/semantics.md` § Birth order is
load-bearing), so when the listener comes after the server,
everything the main locus builds up to and including that param must
finish first. It is judged over the actual construction: each literal
that builds the main locus, wherever the program writes it (a free fn, a
method or hook of another locus, a param default, a fn param's default, a
record type's field default, …; `fn main() { App { server: Server { n: 0
} }; }`), each site judged on its own (a literal in a default once per
literal or call that leaves the default, in that one's scope), else, when none is written, its
defaults; a field a literal supplies replaces the default it would
otherwise run, at every depth. In it: each param's value (a call it makes,
resolved as the checker resolves it, an imported fn through the import
renames; a call that cannot be resolved is taken to possibly not return),
and everything a walked expression evaluates without writing it:

| evaluated implicitly | walked |
|---|---|
| a locus literal `C { … }` anywhere in a walked expression (an array or tuple element, a block, an `if`/`match` arm, a call's argument, a literal's field, a default) | the params it leaves to their defaults, recursively; the `where key == …` key of each of `C`'s subscriptions; `C`'s `birth()` and `birth_check` (awaited even when the param is placed on a pool: the main locus waits for a placed child's readiness); its `run()` (a child is built inline, its `run()` run where it is born), except the server's own param's when it is placed off the main thread |
| a record literal `T { … }` | the fields it leaves to their defaults, recursively (`type Setup { n: Int = { Spinner { }; 0 }; }` built by `Setup { }` builds a `Spinner`; `Setup { n: 0 }` does not) |
| a param with no value, typed as a record whose every field has a default | the `T { }` the compiler builds for it, its defaults walked |
| a call that leaves out arguments | each omitted param's default, evaluated at the call site, of the fn or method the checker resolves the call to (as lowering expands them) |
| a const that is no literal, read | its value, evaluated where it is read; a read as the checker resolves the name where it is evaluated, so a local or a parameter of the same name in scope shadows the const and a binding of a block that has ended does not. A default reads only its own fields and constants, and a caller binding that would shadow one where the default is left out is refused (`spec/types.md` § Locus params), each evaluation on its own |
| a hook's params | their defaults |

Nothing else runs code at construction: an enum variant's payload has no
default, a generic parameter is a type, `@form(…)` arguments, capacity
clauses, an array type's size and a scalar's `range:` are constants the
compiler reads, `placement`, `topology`, `bindings` and `claims` emit no
user code, a closure assertion is judged at closure, and a perspective's
`params` are not lowered. A step that
returns is fine. A step that may not return is
refused, naming it and why ("…its listener (\`App\`'s) is born after
\`self.server\`, once everything \`App\` builds up to it has finished, and
\`Server\`'s \`birth()\` may not return (it may block: …)"), with three ways
out: place the server on a pool when only its `run()` waits, move the
waiting into the main locus's `run()`, or write an address that reads
nothing of the server. "May not return" is the blocking analysis's: the
effect rows' `BLOCK` class, interprocedural (a wait on the exposure or
`run_until_stopped`, a sleep, a read, reached through any fn or method the
step calls, a stdlib retry with a backoff among them), an edge the
analysis cannot name, and, since the classes cannot judge a loop that
spins without blocking nor a publish that waits for room, a `while` loop
or an `or wait` anywhere in the step's statements or expressions (a
`let`'s initializer, a block's tail, an `if` or `match` expression, a
call's argument, a literal's field) or in a fn it reaches. Beside those:
a recursion the step reaches (a fn of the program that reaches itself,
directly or through others, methods and resolved fn values), refused even
when it has a base case, since the check cannot prove a recursion ends; a
param default that reaches its own call again; a fn used as a value,
judged as a call of it; a publish to a topic a binding carries over a
unix socket, an adapter or a shared-memory ring `on_overflow: block`; a
recovery (`restart`, …), which rebuilds a child the check does not
follow; every handler a publish in the step can run on the main thread,
judged as a call: the subscriptions of loci placed on main (or that
placement cannot place, an imported seed's among them) whose pattern the runtime matches the subject
against (the topic's subject, or a trailing `**` over it; a `where key
== …` filter taken as matching), and, for a subject built at run time,
every such subscription of the published payload's type (a handler on
the main thread runs inside the window, delivered in place or drained
from main's queue between births; a subscriber on another pool runs on
its own thread, and main does not wait for it); a `yield`, which
runs whatever main's queue holds and is refused outright; and
constructions nested past 32 levels. A `for` loop always ends
(its range, array, buffer, form or batch is finite when it starts).
`run() { println("started"); }` is admitted. The listener is inserted into whichever of
the main locus's `params` blocks declares that param. Then it is placed:

- for an exposure the main locus holds, by the exposure's own row
  (`public: cooperative(pool = io) where async_io;` above). The row places
  the listener; the exposure stays with the main locus. A row that cannot
  host a listener is refused: "placement entry \`public\`: an exposure's
  row places its listener, whose accept and reads park a coroutine, so it
  runs on a named pool \`where async_io\`; write \`public: cooperative(pool
  = io) where async_io\`", and a row on an exposure with no listener of its
  own (the fixture transport, a hub) is refused as having nothing to place;
- for an exposure a locus below the main locus holds, where the main
  locus places the param its holder sits under: the listener runs on that
  param's pool when its row is `cooperative(pool = P) where async_io`
  (`admin` above: the listener, the server and its receivers on `io`),
  and as an unplaced exposure's does otherwise. A holder nested deeper
  (a child of a child) is placed as any locus is, by its nearest placed
  ancestor, and its listener follows that ancestor's row;
- unplaced, on the transport's shared `async_io` pool: `__api_unix`,
  `__api_http` (which `mcp::Rpc` shares) or `__api_grpc`, which every
  exposure of the transport shares.

Since the main locus holds the listener, it holds the server once: an
exposure whose holder the main locus does not hold through its params is
refused ("exposure \`web\` is held by \`AdminServer\`, which no main locus
holds through its params: …"), and so is one whose holder it holds twice,
or as replicas ("exposure \`web\` is served twice: \`AdminServer\`, which
holds it, is held by \`App\` as \`self.a\` and \`self.b\`; …"). A
server's listener address (`bind:`, `path:`) is written in the server and
read off the server as it was built: each `self.<param>` it reads is the
main locus's `self.<path to the server>.<param>`, so the value a
construction gives the param (a default, the literal of the param that
holds the server, or an override at a construction of the main locus,
`App { server: Server { bind: … } }`, at any depth) is the address the
listener binds. A method of the server, or the server itself, cannot be
read that way and is refused, naming it. The exposure starts from the
`bind:` as written and is told the address the listener was bound to
(`host:<bound port>`) when the listener first asks its transport whether
the exposure is there, before anything is accepted, so its description
names the port the kernel gave it. (Before, the address was followed up
the defaults of the literals that build the server, which a
construction's override bypassed, opening the listener on the default
while the server held the override.)

**A main locus with no `run()`** that holds a listener (its own exposure's,
or one below it) serves until each of those exposures is stopped or the
process drains, as `run() { self.public.wait(); }` would: the compiler
gives it that run.

**The retired form.** Before GH #1565 a serve site was an expression in a
locus body (`let web = api::serve(…);` in `run()`) whose exposure was
already a param the compiler added, and only the main locus could serve
over a socket. The expression is refused, with the param to write: "\`api::serve\`
in a body of \`Desk\`: an exposure is a param of the locus that holds it,
built with it and placed by \`placement { }\`; declare it in \`params { }\`,
\`web: api::Exposure = api::serve(Public, …);\`, and use \`self.web\` where
the body used the handle (\`self.web.wait()\`, \`self.web.stop()\`)". This
is a breaking change: a program that served from a body is rewritten to
the param, one line per site, keeping its binds, roles, receivers and
bounds.

The laws of a serve site (R2a; R1 parsed a serve site only as far as a
description names it, its surface, its transport instance's kind,
listener, codec and sources, `as:`, `receivers:`, `bound:` and
`on_full:`, checked none of these laws and refused to build a program
that held one):

1. **An exposure is named once.** "exposure \`public\` is served twice:
   \`as:\` names one exposure; name this one apart"; and its holder is
   held once (above).
2. **Every receiver type is bound.** "serve of \`Public\` as
   \`partner\`: \`Orders\` is held twice, as \`self.orders\` and
   \`self.partner_orders\`, and the serve binds neither: name the one
   that answers (\`receivers: { Orders: self.orders }\`)"; a type the
   holder does not hold at all is the same error with no instance to
   name.
3. **A receiver lives as long as its exposure.** A receiver is a param of
   the holder, as the exposure is, so the holder's lifetime is both's; it
   is declared before the exposure param, so it is born before the
   exposure binds it (the holder's fields are born, drained and dissolved
   in declaration order, and the exposure's own shutdown refuses what it
   still holds): "serve of \`Public\` as \`public\`: \`orders\` is declared
   after \`public\`, the exposure that binds it; a receiver is born before
   the exposure that binds it, so declare \`orders\` first"; a receiver that is no param of the holder is refused,
   "serve of \`Both\`: \`self.ledger\` is no param of \`Desk\`; hold the
   receiver as a param".
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
instance; a param typed as something else, "exposure param \`public\`:
\`api::serve(…)\` builds an \`api::Exposure\`; declare \`public:
api::Exposure\`"; a receiver that is a param but is not built by a
literal, "serve of \`Public\`: \`orders\` is not built by a literal in a
param of \`Desk\`: the serve numbers the instance in the literal that
builds it" (§ Receiver failure and generations); a socket transport built
elsewhere than in a literal the compiler can read its listener from
("serve of \`Public\` as \`public\` over \`unix::Rpc\`: the compiler adds the
transport's listener from the literal that builds it and reads its address
there, and this transport is built elsewhere; …"); and a serve site in a
free fn, "\`api::serve\` in \`serve_it\`: an exposure is a param of the
locus that holds it and its receivers; declare it in that locus's params,
\`p: api::Exposure = api::serve(Public, …);\`". A build refuses a serve site
over a transport the compiler does not ship (one the stdlib lacks and the
program does not declare): "\`api::serve\` over \`quic::Rpc\`: this compiler
serves a surface over \`std::api::test::Rpc\`, the in-process transport,
\`unix::Rpc\`, \`http::Rpc\`, \`mcp::Rpc\`, \`grpc::Rpc\`, \`ws::Hub\`, \`udp::Hub\`,
or a transport the program declares; the other transports follow". A hub
is the main locus's (§ Streams), so a serve over a \`ws::Hub\` or a
\`udp::Hub\` from another locus is refused: "serve of \`Public\` over
\`ws::Hub\` in \`Server\`: a hub is the main locus's, its listener and its
stream bindings placed there; …".

Only the entry program's main locus is constructed, and the exposures
in its tree are the ones served. A serve site in an imported main locus
that the program also writes the name of (`lib::Head { }` as a part of its
own main locus, or as its `fn main`) would build and check and serve
nothing, so it is refused: "`api::serve` in `lib::Head`, a main locus
the program imports: only the entry program's main locus serves; serve
from the entry's main locus or hold the exposure there". A library whose
main locus is only imported for its types, as an application's tests
import the application, never runs its serve sites and is not refused.

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
re-homed), `http::Rpc` (R3, § The HTTP transport), `ws::Hub` and `udp::Hub` (R5, § Streams),
`mcp::Rpc` (R6, § The MCP transport) and `grpc::Rpc` (R7, § gRPC);
a program implements one the stdlib lacks.

**The fixture transport** (`std::api::test::Rpc`) is a conforming
transport with no socket: a test hands it framed requests
(`call(correlation, member, payload, credential)`, `describe(…)`,
`lose(correlation)`) and reads the outcomes it was asked to deliver
(`outcome(correlation)`), each as the Unix JSON reply line of § Outcomes
(`request_id`, the client's `id`, the answer, `caller`). It goes through
the same admission, dispatch and completion as every transport, and it
is how the runtime is proven before a socket exists.

### The Unix transport

`unix::Rpc` (`std::api::unix::Rpc`, R2b) serves an exposure over a Unix
domain stream socket, one JSON object per line (§ Outcomes states the
wire). It is written in the serve call, or held as a param and named:

```hale,fragment
admin: api::Exposure = api::serve(Admin, unix::Rpc { path: "/run/desk/admin.sock", roles: self.admin_roles }, as: "admin", bound: 16, on_full: refuse);
```

- **The principal is the peer's kernel credentials**, read once per
  connection: `mode: "unix"`, `name: "uid:<n>"`, `uid`, `gid`, `pid` and
  the supplementary groups. The transport therefore has no bearer
  source (`principals:` is not one of its fields); `roles:` is the
  role source, as for every transport. A connection whose peer the kernel
  will not name has uid -1 and is refused `unauthenticated` ("the kernel
  did not name the peer") before anything else is looked at.
- **The listener is bound at birth.** A path it cannot bind (a missing
  directory, a path a live process holds) fails the program's boot: the
  diagnostic on stderr and exit code 2, the root failure shape of a binding
  that cannot open; a stale socket file is replaced. An empty `path`
  binds nothing and is not a failure: the exposure has no socket, and the
  program boots (a program that computes the path at run time hands the
  transport none when the path is another process's to hold). The path is held
  from birth, but nothing is accepted until the exposure is attached to
  the bus (a client that connects earlier waits in the socket's backlog), so
  a request read is never published to an exposure that is not there yet.
  The socket file is removed when the exposure stops or is torn down, if
  the path still names the file the listener bound (a replacement may
  have taken a stopped listener's path; its file is left alone).
- **The listener runs on an `async_io` pool.** A socket's accept and read
  park a coroutine, so they need an `async_io` pool, and only the main
  locus places: the serve expansion adds the listener to the main locus as
  a param, placed by the exposure's row, or the row of the param its
  server sits under, or on the pool `__api_unix`, which every unplaced Unix
  exposure of the program shares (§ Serving). The listener is one of the TCP
  connection substrate's (spec/stdlib.md § The TCP connection substrate),
  named `unix:<path>`: it relays each accepted connection as a
  `std::io::tcp::Accepted` to the consumer on its route, whose handler
  starts the connection's locus as a flow child on the same pool. The
  transport itself, and the exposure, stay on the holder's pool; the two
  meet on the bus
  only (the subject `__api.unix.out`, keyed by exposure and connection).
- **Framing is a line.** A request is one JSON object on one line of at
  most 60000 bytes:
  `{"call": "Orders::cancel", "payload": {…}, "id": …, "digest": "…"}`
  or `{"describe": true}`. The reply is one line (§ Outcomes). `id` is
  any JSON value, echoed verbatim (`null` when the request has none);
  `request_id` is the exposure's. Replies are written in the order the
  program produces them, so a client correlates by `id`. A client may have
  many requests in flight on a connection, and several connections.
- **A connection fails alone.** A line that is not a JSON object is
  answered `malformed` with `request_id` 0 and the connection is closed. An
  object that is neither a call nor a describe, or a call that carries no
  `payload`, is refused `malformed` and the connection stays open. An EOF,
  a client that goes away after sending, and a reply write that fails
  (a reader that does not read for 5 seconds) end that connection: the
  requests it still has with the exposure are lost (§ The request
  lifecycle), and the listener and every other connection go on.
- **`describe`** answers `{"ok": true, "value": <description>}`: the whole
  document of § The description for the caller the exposure established
  (the peer's kernel credentials, `mode: "unix"`), the document HTTP
  answers: the exposure's identity, its listener (`unix` and the `path:`
  text), the roles that caller holds, the members it may call under the
  exposure's role source with their schemas, the outcome encoding of this
  section and the notes, built from the same rows and the same sources
  admission reads, so the two agree for the same caller. The in-process
  fixture transport answers the same document, its listener `test`.
- **`stop()`** closes the listener and every connection after the replies
  already sent: the queued requests are refused `shutting_down` and the
  executing ones are answered first, as § The request lifecycle states.
  Once the exposure is draining (a signal, or its owner's teardown, which
  drains the tree first: `runtime.md` § "Lifecycle obligations", line 15)
  it hands no queued request to a receiver; its shutdown refuses them.
  A program that ends without `stop()` is torn down by its owner and
  releases its sockets, but the pools of the listener and its connections
  are stopped before the exposure's shutdown can write to them, so a
  caller with a request in flight sees the connection end, which is the
  transport failure outcome, never a refusal: `stop()` is the orderly path.

### The HTTP transport

`http::Rpc` (`std::api::http::Rpc`, R3) serves an exposure over HTTP/1.1,
one request to a connection (`Connection: close`: no keep-alive, no
chunked bodies). It is written at a serve site, or held as a param and
named, with the fields `bind` (`host:port`), `codec` (`json`, the one codec
v1 has), `principals` (the bearer source, required), `roles` (the role
source) and `cors` (the browser origins it answers CORS for, below;
optional):

```hale,fragment
public: api::Exposure = api::serve(Public, http::Rpc { bind: "127.0.0.1:8080", codec: json, principals: self.bearer, roles: self.public_roles }, as: "public", receivers: { Orders: self.orders }, bound: 64, on_full: refuse);
```

- **The principal is the bearer.** The request's `Authorization: Bearer
  <token>` goes to the bearer source, which names the caller
  (`mode: "bearer"`, its `name`, the other fields the source sets); a
  token the source names nobody for, a missing header or a header that is
  not a bearer is refused `unauthenticated` before anything else is looked
  at, with the source's own reason (`refused()`, as the contract records:
  "no such token") or, when it states none, "the sources name nobody"; a
  credential past its `expiry` is refused with the reason `expired`. The
  role source is then asked what the caller holds (§ Identity sources).
- **The listener is bound at birth.** An address it cannot bind (a port a
  live program holds, a string that is not `host:port`) fails the
  program's boot: the diagnostic on stderr and exit code 2, as for the Unix
  socket. Nothing is accepted until the exposure is attached to the bus
  (a client that connects earlier waits in the listen backlog). The
  listener is placed as the Unix one is (§ Serving): by the exposure's row,
  or its server's, on an `async_io` pool, else on the `async_io` pool
  `__api_http`, which every unplaced HTTP exposure of the program shares;
  each connection is a locus of the listener's pool (the listener is a TCP
  connection substrate listener named `host:<bound port>`, as the Unix one
  is). The transport and the exposure stay on the holder's pool; the two
  meet on the bus
  (the subject `__api.http.out`, keyed by exposure and connection).
- **Framing is the request.** `POST /call/<member>` carries the payload as
  its body (the member's `::` may be sent as `%3A%3A`), the client's
  digest, when it sends one, in `Hale-Surface-Digest`, and the connection
  is the correlation. `GET /.description` asks for the caller's
  description. A request line, method or path that is neither, an empty
  body on a call, a head over 65536 bytes or a body over 1 MiB is
  `malformed` (400); a request read gets 5 seconds of silence and 10 in
  all.
- **The reply is the contract's.** Status and body are § Outcomes' HTTP
  column and nothing else: 200 with the response, 422 with `E`, 400, 409,
  401, 403, 429 and 503 with `{"refusal": {"kind": …, "reason": …}}` (a
  digest mismatch adds `"served"`, an unauthorized call `"requires"`), 500
  with `{"refusal": {"kind": "server"}}`. Every reply is
  `Content-Type: application/json`, `Cache-Control: no-store` and
  `Content-Length`d; it carries no request id (the exposure's ids are the
  Unix wire's).
- **`describe` is the whole document.** `GET /.description` answers 200
  with the document of § The description for the caller the bearer names:
  identity, the listener (`http` and the `bind:` text) and the codec, the
  caller and the roles of the rows it holds, the members it may call with
  their schemas, the HTTP outcome encoding, the schemas those members
  reach, and the notes. It is built from the rows and the sources admission
  reads, so what it lists is what the next call admits. A request that
  names nobody gets the same 401 as a call.
- **A connection fails alone.** A request cut off before it is whole, a
  head that never ends, a write that fails (a reader that does not read for
  5 seconds) and a client that goes away end that connection: nothing is
  written to it, and the listener and every other connection go on. A
  client that goes away after its request was published is a lost
  connection of the runtime (§ The request lifecycle): the work still
  runs, once, and holds its place against the bound until it completes. An
  EOF is the whole signal, so a client that half-closes its end after
  sending is taken to have gone.
- **CORS, when `cors` lists origins** (2026-10-12, downstream handoff).
  `cors: String` is a comma-separated list of origins, each matched whole
  against the request's `Origin` (`*` matches any); `""`, the default,
  answers no CORS and changes nothing above. With it set, an `OPTIONS` of
  a route the transport serves (`/call/<member>`, `/.description`) is a
  preflight the transport answers itself: it reaches the runtime as a
  request refused at framing (no credential is read, nothing is
  dispatched), and its reply is `204` with no body when the origin is
  listed, `403` with `{"refusal": {"kind": "unauthorized", "reason": …}}`
  when it is not or none was sent. A listed origin's preflight carries
  `Access-Control-Allow-Origin: <origin>`, `Access-Control-Allow-Methods:
  POST, GET, OPTIONS`, `Access-Control-Allow-Headers: Authorization,
  Content-Type, Hale-Surface-Digest` and `Access-Control-Max-Age: 600`;
  every other response to a listed origin, refusals included, carries
  `Access-Control-Allow-Origin: <origin>`, and no response to an origin
  not listed carries a CORS header. Every response of a transport with
  `cors` set carries `Vary: Origin`. An `OPTIONS` of any other path is
  `malformed`, as without `cors`. CORS is the browser's guard and decides
  no admission: a call is authenticated and authorized as it would be
  without it.
- **`stop()`** answers the executing calls, refuses the queued ones
  `shutting_down` (503), and closes the listener and every connection
  after the replies already sent. A request that arrives while `stop()`
  waits for an executing call is refused `shutting_down` as well; one that
  arrives after the listener is closed is refused by the network. As for
  the Unix socket, a program that ends without `stop()` releases its
  listener, and a caller with a request in flight sees the connection end.

### The MCP transport

`mcp::Rpc` (`std::api::mcp::Rpc`, R6) serves an exposure as a Model
Context Protocol server over the Streamable HTTP transport's plain-JSON
form: a JSON-RPC 2.0 message in the body of `POST /mcp`, its answer in the
response, one request to a connection. It is written at a serve site, or
held as a param and named, with the fields `bind` (`host:port`),
`principals` (the bearer source, required) and `roles` (the role source):

```hale,fragment
public: api::Exposure = api::serve(Public, mcp::Rpc { bind: "127.0.0.1:8090", principals: self.bearer, roles: self.public_roles }, as: "public", receivers: { Orders: self.orders }, bound: 64, on_full: refuse);
```

It is built on § The HTTP transport's listener and connections, whole: the
same limits, the same bearer, the same placement (`__api_http` unplaced), the
same bind-at-birth (the diagnostic reads `api: mcp::Rpc could not listen
on …`), the same refusal of a serve from a locus that is not the main
locus, and a connection that fails alone. What is its own is the framing
and the encoding.

- **Tools are rpcs.** `tools/list` answers the tools of § The description
  for the caller the bearer names, which is the caller's description
  filtered by the rows and sources admission reads: a tool per member the
  caller may call, named as the member with `::` written `__`
  (`Orders__place`; a tool name may not contain `:`; an identifier that
  holds `__` or starts or ends with `_` writes each of its `_` as `_-`, a
  `-` being no part of an identifier, so no two members share a tool
  name), described as `hale
  check --api --mcp` describes it, its `inputSchema` the request's schema
  made self-contained (a type it reaches under `$defs`), `x-hale-requires`
  the roles. A caller holding no role sees the members that require none.
  A member whose request is not an object (an MCP tool's input is one)
  has no tool; the wrapped form `--mcp` prints needs the handler's
  parameter name, which a description does not carry, so `hale mcp --app`
  over a description lists the same tools (§ The clients, Open points).
- **`tools/call` is a call.** `params.name` is the tool and
  `params.arguments` the payload, which goes through admission exactly as
  an HTTP body does; the digest, when the client sends one, is the header
  `Hale-Surface-Digest` or `params._meta["hale/digest"]`. The tool is
  found, not decoded: the member is the row whose tool name it is, and a
  name no row has is `malformed`, `unknown_member`.
- **The session methods are the transport's.** `initialize` answers the
  client's `protocolVersion` (the current one, `2025-06-18`, when it
  names none), the capability `tools` and `serverInfo` as the surface's
  name and digest; `ping` answers `{}`; a notification (a message with no
  `id`) is answered `202` with no body; any other method is `-32601`. All
  of them need a caller the bearer names, as every request does. There is
  no session id, no batch (`-32600`), no server-initiated message and no
  event stream: `GET` and `DELETE` on `/mcp` are `405`.
- **Outcomes** are § Outcomes' MCP column. A result is a JSON-RPC result
  `{"content": [{"type": "text", "text": <the response, as text>}],
  "structuredContent": <the response, when it is an object>, "isError":
  false}`. Every other outcome is a JSON-RPC **error object** `{"code": C,
  "message": K, "data": D}`: a handler error is `-32001`, `"handler_error"`
  and `D` = `E` by codec; a refusal is `K` = its kind and `D` = the
  refusal object of § Outcomes (`kind`, `reason`, and `served` or
  `requires`) with the code `malformed` `-32602` (a message that is not a
  JSON-RPC request is `-32700` parse error or `-32600` invalid request, an
  unknown method `-32601`, and an unknown tool or a payload that does not
  decode `-32602`), `digest_mismatch` `-32003`, `unauthenticated` `-32004`,
  `unauthorized` `-32005`, `full` `-32006`, `shutting_down` `-32007`,
  `unavailable` `-32008`; the server error is `-32603`, `"server_error"`,
  `D` = `{"kind": "server"}`. The answer carries the request's `id`
  verbatim (`null` when the message had none that could be read) and is
  HTTP `200`, except `unauthenticated`, which is `401` as MCP's
  authorization requires. A transport failure is the connection ending
  without a response.
- **The description** an MCP exposure serves has `listener.transport`
  `"mcp"` and the outcome encoding above under `outcomes` (`hale check
  --api --exposure NAME --caller NAME` prints it); it is not reachable
  over the wire, where `tools/list` is the discovery.

**Resources over streams** are not served, and the capability `resources`
is not advertised. A subscription (`resources/subscribe`, then
`notifications/resources/updated`) is a server-initiated message on a
connection that outlives a request: it needs the event stream of the
Streamable HTTP transport (or a WebSocket), a hub (R5) to carry the
stream, and the session identity MCP gives with `Mcp-Session-Id` so a
notification finds its client; none of the three exists in `mcp::Rpc`, which
holds no connection past its answer. The streams of a surface are in its
description (`streams`) for a client that knows another door.

**What `hale mcp` is.** `hale mcp` (`crates/hale-cli/src/mcp.rs`) is an MCP
*server* on stdio: a bridge from a host to the toolchain, or with `--app
ENDPOINT` to a served exposure (§ The clients): `tools/list` is the
description's members for the caller the endpoint names (over a socket, over
HTTP, over a hub's listener), `tools/call` a call naming the digest read, and
for an `mcp://host:port` endpoint (an `mcp::Rpc` listener) the JSON-RPC is
forwarded, since that listener's own `tools/list` is its discovery. A host
that only speaks stdio reaches an HTTP exposure this way. `mcp::Rpc` with `stdio` in place
of `bind` is not shipped: the runtime has
no primitive to read the process's own standard input as a stream (a
`std::io::tcp::Stream` over descriptor 0 is a `recv(2)` on a pipe), and
`hale mcp` already owns that framing for the toolchain's server.

### gRPC

`grpc::Rpc` (`std::api::grpc::Rpc`, R7) serves an exposure over gRPC, unary
calls only, on cleartext HTTP/2 (prior knowledge: no TLS, no ALPN, no
upgrade). It is written at a serve site, or held as a param and named, with
the fields `bind` (`host:port`), `codec` (`json`, the one value v1 has: the
row's JSON codec, which `application/grpc+json` speaks; protobuf is the
transport's own second codec, served beside it and chosen per request by the
content type, § The protobuf codec),
`principals` (the bearer source, required) and `roles` (the role source):

```hale,fragment
public: api::Exposure = api::serve(Public, grpc::Rpc { bind: "127.0.0.1:8070", codec: json, principals: self.bearer, roles: self.public_roles }, as: "public", receivers: { Orders: self.orders }, bound: 64, on_full: refuse);
```

**The protocol is a library, and the library is not ours.** HTTP/2 (frames,
HPACK, SETTINGS and PING, flow-control windows, GOAWAY) is nghttp2 (MIT),
vendored under `crates/hale-codegen/runtime/third_party/nghttp2` and built
into the runtime by the cc step that builds the rest of its C, with the same
flags, the sanitizers' included; a program that serves over gRPC is linked
with it statically, and a program that does not carries none of it. The
library is sans-I/O: bytes in, bytes out, a queue of events, all of it
computation on the thread the runtime placed (`runtime/lotus_h2.c` is the
glue and owns no thread, socket or blocking call), so there is no second
scheduler: the socket is a descriptor `std::io::tcp` reads and writes, and a
read parks its coroutine on the `async_io` pool as every read there does.

**Two layers.** `std::io::h2` (`io_h2.hl`) is the server: a listener on the
`async_io` pool its exposure's placement names, `__api_grpc` unplaced (which
every unplaced gRPC exposure of the program shares), and a connection locus per accepted socket, which feeds each read to
its session, writes what the session drains and raises the session's stream
events on the bus (`__api.h2.event`: a request's headers whole, bytes of its
body, its end, a reset, a GOAWAY, a stream closed, the connection ended).
`grpc::Rpc` is the transport on top: it reads those events, keeps a table of
the streams (at most 4096 held at a time; one past that is refused
`REFUSED_STREAM`), and asks the connection to answer a stream
(`__api.h2.cmd`). The correlation is the connection and the stream, so many
calls are in flight on a connection, and on many connections. The listener is
bound at birth, with `http::Rpc`'s diagnostic (`api: grpc::Rpc could not
listen on …`, exit code 2), nothing is accepted until the exposure is attached
to the bus, and a serve over `grpc::Rpc` from a locus that is not the main
locus is refused, written at the site or held as a param. The listener is one
of the TCP connection substrate's (spec/stdlib.md § The TCP connection
substrate), named `host:<bound port>` as the HTTP one is: it relays each
accepted connection as a `std::io::tcp::Accepted` to the consumer on its
route, whose handler starts the connection locus as a flow child on the same
pool, so a connection nothing takes is closed and `std::io::tcp::stop(name)`
closes the listener while the connections it accepted are served on.

- **Framing is a call.** `POST /<Surface>/<rpc>`, the rpc the generated
  `.proto` names (§ The protobuf codec): the member written as an MCP tool is,
  `Orders__place` for `Orders::place`. The older spellings name the same
  member: `Orders.place` (the `::` as `.`), `Orders::place` and
  `Orders%3A%3Aplace`. The first segment is the exposure's surface: another
  service is `malformed`. `content-type` is `application/grpc` or
  `application/grpc+proto` (protobuf) or `application/grpc+json` (the row's
  JSON), and the response is answered in the codec it was asked in, under the
  same content type. Any other suffix, or another media type, is refused
  `malformed` with the reason. A call is one message: a compression flag (0),
  four bytes of length and the message. A flag of 1, a `grpc-encoding` other
  than `identity`, a prefix that is cut off, no message or more than one, a
  message of more than 1048576 bytes, and an empty or non-text JSON message
  (one that begins `pb:`, which the exposure holds a protobuf call as, is not
  JSON text either) are `malformed` (`INVALID_ARGUMENT`), and the stream is
  answered; an empty message is a valid protobuf message, whose absent fields
  the codec then reports. The bearer is the
  `authorization: Bearer <token>` metadata, refused as `http::Rpc` refuses it,
  and the digest, when sent, is the `hale-surface-digest` metadata.
- **The reply is the contract's.** A result is a `HEADERS` frame (`:status 200`,
  `content-type`), the response as one message in `DATA`, and the trailers
  `grpc-status: 0`. Every other outcome is a trailers-only response (one
  `HEADERS` frame closing the stream, `:status 200`, `content-type`,
  `grpc-status`, `grpc-message`, `grpc-status-details-bin`), the status the
  table of § Outcomes gives: `FAILED_PRECONDITION` (9) for a handler error,
  or its class's code (§ Error classes), with the message `handler_error`; `INVALID_ARGUMENT` (3),
  `FAILED_PRECONDITION` (9, a digest mismatch), `UNAUTHENTICATED` (16),
  `PERMISSION_DENIED` (7), `RESOURCE_EXHAUSTED` (8) and `UNAVAILABLE` (14, for
  `shutting_down` and for `unavailable`) for a refusal, its reason the
  message; `INTERNAL` (13), message `server_error`, for a handler that
  violated. `grpc-message` is percent-encoded as gRPC states (a byte outside
  printable ASCII, and `%`, as `%XX`). `grpc-status-details-bin` is the
  base64 of a `google.rpc.Status` carrying the same code and message and one
  `Any` detail. Under `application/grpc+json`: for a handler error, type
  `type.hale.dev/hale.api.HandlerError` and the value `E` by codec (JSON
  bytes); for a refusal or the server error, type
  `type.hale.dev/hale.api.Refusal` and the refusal object of § Outcomes
  (`kind`, `reason`, and `served` or `requires`; the server error's
  `{"kind": "server"}`). Under protobuf the detail is a generated message:
  for a handler error, type `type.hale.dev/<the message of E>` and its value
  the encoded `E` (`OrderError` for `Orders::cancel`); for a refusal or the
  server error, type `type.hale.dev/HaleRefusal` and its value the encoded
  `HaleRefusal` (`kind`, `reason`, one `requires` a role and `served`, those
  the kind carries; the server error is `kind: "server"` alone). A transport
  failure is the stream reset or the connection ended without a status.
- **`describe` is a reserved method.** `POST /hale.api.Description/Describe`
  (service `hale.api.Description`, which no surface can be named) answers, as
  one message, the whole document of § The description for the caller the
  bearer names, with the listener `grpc` and the outcome encoding above; the
  request message is ignored. Under `application/grpc+json` the message is the
  document; under protobuf it is `hale.api.DescriptionDocument { string json
  = 1; }`, the document as that field. A caller nobody names gets the same
  `UNAUTHENTICATED` as a call.
- **The protobuf codec.** A program that serves over `grpc::Rpc` carries,
  beside the JSON codec of each record its rows name, a protobuf codec
  generated from the same declaration: proto3's wire format (varints,
  length-delimited fields, `double` as eight bytes little endian), the
  fields numbered `1, 2, …` in the order the struct declares them. Decoding
  is as strict as the JSON codec's, in its words: a field of the wrong wire
  type is `wrong_type` (named by the field), a field with no default that is
  absent is `missing_field`, a message that is not protobuf (a cut field, a
  varint of more than ten bytes) is `wrong_type: payload`, an unknown field is
  skipped by its wire type, and the last of a field sent twice wins. Every
  scalar is written whether or not it equals its default (the `.proto` marks it
  `optional`), so a zero is a value. The codec joins a program only with a
  `grpc::Rpc` exposure; a non-gRPC exposure's generated code is unchanged. The
  exposure holds a protobuf call as the text `pb:` + a message name + `:` +
  the base64 of the message (a String ends at its first NUL, a message does
  not); the adapters of a gRPC exposure branch on the prefix and run the same
  steps with the other codec.
- **The `.proto`** is generated from the rows (§ Generated specs and
  clients): `hale api export` writes `NAME.proto` and `hale check --api
  --surface NAME --proto` prints it. `syntax = "proto3"`, no package, one
  `service NAME` (the surface) with a unary rpc per member, named as above; a
  message per record the rows reach, named as the schema document names the
  type with the `.` of a qualified imported type as `_` (`lib_Item`), and `Message` after it when
  that is the surface's own name (a service and a message share proto3's
  scope, and the service keeps the surface's name because it is the gRPC
  path: `api Todo` over `type Todo` is `service Todo` and `message
  TodoMessage`, and its handler error's detail is
  `type.hale.dev/TodoMessage`); a field of record type is a
  field of that message, and every scalar field is `optional`
  (`Int`, an identity, a range and a quantity `int64`, `Float` `double`, `Bool`
  `bool`, `String` `string`; a comment names the Hale type). A row whose
  request, response or error is a scalar is carried in a one-field message
  `<rpc>Request`, `<rpc>Response` or `<rpc>Error` (`value = 1`), a row with no
  request or response in `HaleEmpty`, and a refusal in `HaleRefusal`. The file's
  header states the five outcomes and these rules. A shape the codec does not
  carry has no encoding and is refused naming the row and the field, never
  approximated, and so is a name two types would share, and so is a record two
  of whose fields have the same default proto3 JSON name (the field's name
  with its underscores removed and the letter after each capitalised: `a_b`
  and `aB` are both `aB`), compared without regard to case as protoc compares
  them (`a` and `_a` are `a` and `A`): `protoc` refuses such a file, so
  `check --api --surface NAME --proto` and `api export` refuse the record,
  naming both fields and the JSON name; a `json:` tag on one of the fields
  renames it. It is a rule of the protobuf form, not of the surface laws: the
  JSON codec has no such collision, and the other forms are still written. The
  field numbers are
  held by the surface digest: it folds each struct's fields in declaration
  order, so a reorder or an insertion moves the digest, and no number moves
  while the digest holds.
- **Server reflection.** `grpc.reflection.v1.ServerReflection/ServerReflectionInfo`
  is served, for a caller the bearer source names (as the description is, a
  surface names its members): it lists the services (the surface,
  `hale.api.Description` and the reflection service) and answers the
  `FileDescriptorProto` of the file that declares a symbol or has a name: the
  surface's `.proto` (`NAME.proto`), the description method's
  (`hale/api/description.proto`) and the service's own
  (`grpc/reflection/v1/reflection.proto`), each built from the same model as
  the text, so it is the file a compiler makes of the `.proto`. A symbol or a
  file no one has is an `error_response` (`NOT_FOUND`, 5), and so is a request
  for extensions, which the server has none of. The service is
  bidirectional, and a client such as `grpcurl` waits for the answer to a
  request before it ends its side of the stream, so each whole message is
  answered as it arrives, with a DATA frame (several in one stream are
  answered in order; a message split across DATA frames is answered once,
  when it is whole), and the call stays open until the client ends its side,
  when the server ends it with `grpc-status: 0`; a transport that stops ends
  an open reflection call the same way, before its GOAWAY. Every end goes
  through one path that knows whether the response has started: a call not
  yet answered ends as a trailers-only response (the status in the headers),
  one that has answered ends with the status (`grpc-status`, `grpc-message`)
  as the trailers of the open response. An error reaches the open response
  the same way: a message over the limit (8) ends the call at once, a message
  the client's half-close leaves unfinished is 13, and the bearer is asked
  again at every message, so a source that stops naming the caller ends the
  call with 16. The descriptor is the surface's, not
  the caller's: the roles are checked when a call is made, as the description
  filters by them and the `.proto` does not. `grpc.reflection.v1alpha` is not
  served.
- **A stream fails alone.** A client that resets a stream, or goes away, after
  its request was published is a lost connection of the runtime (§ The request
  lifecycle): the work still runs, once, and holds its place against the bound
  until it completes. A stream reset before its request was whole is
  forgotten. A frame the protocol forbids ends that connection (the library
  has said `GOAWAY` with the error, which is written first), the peer's own
  `GOAWAY` ends it once its streams are done, and a write that fails (a
  reader that does not read for 5 seconds) ends it; the listener and every
  other connection go on.
- **`stop()`** answers the executing calls, refuses the queued ones
  `shutting_down` (`UNAVAILABLE`), sends `GOAWAY` on every connection after
  the replies already queued, and closes the listener; a connection ends when
  its streams in flight are answered. A request that arrives while `stop()`
  waits for an executing call is refused `shutting_down` as well.

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

**The receiver's side is a seam** (GH #1530, 2026-10-18; spec/semantics.md
§ Seams). A call reaches its receiver's pool as a bus delivery keyed by the
exposure, whose stamp is checked first (a call from before the receiver's
generation is `unavailable`); what runs then is a seam the compiler writes
on the receiver, `c -> decode -> self -> encode`. The decoder is one hop: it
reads the body as JSON, or as protobuf when the exposure is gRPC and the body
says `pb:`, and makes the row's **mirror** member, a record of the request,
the context and the call's identity (its exposure and request id, and its
wire). The mirror is generated, one member per row and never per request
type, so two rows that take one request type stay two members; its names
are the compiler's own (`__RpcIn_…`, `__RpcOut_…`) and no description,
digest, export or client names them. The locus hop on `self` is the
dispatch: each member goes to its row's eater, a generated method that
unpacks it into the handler's arguments, the context last, and calls the
handler on the receiver's pool, in its arena. The encoder takes the set of
every row's response and every row's error, and the server error a
violating handler answers (`ClosureViolation`), and writes each as the
outcome it is, on the call's wire. Coverage is the surface's rows: every
member has its eater, and the receiver's other methods are methods. A
body neither codec reads ends the seam, and the call is `malformed`.

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
neither lowers as it did before R2a, byte for byte. The runtime carries
the TCP connection substrate (`io_tcp_serve.hl` and its topic,
spec/stdlib.md § The TCP connection substrate), since its socket
transports' listeners are built on it.

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
    HandlerError(String, std::api::ErrorClass),  // the handler's E by the
                                // codec, and the class its variant declares
    Refusal(String, String, String),  // kind, reason, the extra: `served` on
                                // digest_mismatch, `requires` (comma-joined)
                                // on unauthorized, else ""
    ServerError,                // the handler violated; says nothing of it
};

// A handler error's class (§ Error classes): `Unclassed` for a variant
// that declares none and for an error type that is a record.
type std::api::ErrorClass = enum { Unclassed, NotFound, Conflict, Invalid, Forbidden, Unavailable };

// What a caller is handed to hold; stop() is idempotent. wait() returns
// when stop() or the drain has ended the exposure (§ Parking).
interface std::api::Handle { fn stop(); fn wait(); }

// h.wait() as a free function, for a serving main locus's run().
fn std::api::run_until_stopped(h: std::api::Handle);
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

A receiver is therefore an actor: its calls run one at a time, in the
order accepted, whatever the transport and wherever it is placed. The
transports read and write connections concurrently and the exposure
admits concurrently, but fifty one-second calls to one receiver take
fifty seconds, on an `async_io` pool as anywhere. Calls to different
receivers run at once: each receiver is a slot of its own in the
exposure, and the handlers of receivers on one `async_io` pool interleave
where they park. A program that wants a surface's work answered
concurrently binds its members to several receivers (members of
different receiver types, each instance placed where its work belongs),
or serves the surface through several exposures, each with a receiver
instance of its own; a window wider than one call per receiver is § Not
yet.

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

**Parking.** `h.wait()`, and `std::api::run_until_stopped(h)` which is
the same call, return when `stop()` has run on the exposure or the
process or the holder is draining. Between those they wait in
bounded slices of 100 ms on `std::time::__idle_wait`: on the main thread
that is a wait on main's bus queue which a delivery from another thread
ends at once (the queue's wake descriptor, written by the enqueue, the
pattern an `async_io` pool's wake descriptor already is), and anywhere
else it is a `sleep`. A serving `main locus` whose `run()` ends in
`run_until_stopped` therefore answers a call in the time the work takes
and its cross-pool hops take, not in the next slice of a `sleep` loop. The
wait never leaves the thread's own scheduling: no thread is added and the
main thread is still the one that runs main's handlers. A program that
keeps its own `while !self.draining { sleep(…); }` loop is still correct
and answers at the latency of its slice, since main's bus queue is
drained only when its thread next looks at it; a loop that does periodic
work of its own keeps that loop and gains the park by waiting with
`h.wait()` only where it has nothing else to do.

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
caller agree. The exposure's live answer is the full document of § The
description for that caller, on every transport: the one the exposure
builds from its rows, filtered by the sources; `hale check --api
--exposure NAME --caller P --holds R` prints the same document from the
rows alone.

## Identity sources

The interfaces for credential validity and grant revision are R2's;
their use for live subscriptions (invalidation, delivery) is R5's (§ Streams).

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
  std::api::Revision;`) for a subscriber to learn of it; a hub's connections read it to
  invalidate live subscriptions (§ Streams). `holds(p, r)` of
  `std::api::RoleSource` stays the direct question: a source that
  implements `grants` as well is asked it once per request, one that does
  not is asked `holds` once per role the check needs, with the revision
  unstated (0).
- The runtime queries both on the exposure's own pool under Hale's
  cross-pool rules: a source is a locus the exposure's holder holds, placed
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
chooses another, and no row carries a status. A handler error's status
is the error class its variant declares, if it declares one (§ Error
classes); the class is part of the error type's shape, which the digest
holds.

| outcome | meaning | HTTP | Unix JSON | gRPC | MCP |
|---|---|---|---|---|---|
| result | the handler returned | 200, body = the response by codec | `{"ok": true, "value": …}` | OK | result |
| handler error | the handler failed with its declared `E` | 422, or its class's status and `Hale-Error-Class` (§ Error classes), body = `E` by codec | `{"ok": false, "error": E}`, and `"class"` for a classed one | FAILED_PRECONDITION, or its class's code, `E` in details | error object with `E` |
| refusal | the request was not accepted, or was accepted and did not run (the kinds below) | 400 / 409 / 401 / 403 / 429 / 503, body = `{"refusal": …}` | `{"ok": false, "refusal": {"kind": …, "reason": …}}` | INVALID_ARGUMENT / FAILED_PRECONDITION / UNAUTHENTICATED / PERMISSION_DENIED / RESOURCE_EXHAUSTED / UNAVAILABLE | error object |
| server error | the handler failed with `ClosureViolation`, its row's error type (§ Structural failure in a handler) | 500, body = `{"refusal": {"kind": "server"}}` | `{"ok": false, "refusal": {"kind": "server"}}` | INTERNAL | error object |
| transport failure | the connection or the protocol broke | the transport's own | EOF | the transport's own | the transport's own |

The refusal kinds, each with its status:

| kind | when (§ Serving, the checks) | HTTP | gRPC |
|---|---|---|---|
| `malformed` | not a request; no such member (`unknown_member`); a payload that does not decode (`missing_field`, `wrong_type`, a list's `too_long` or `too_short`) | 400 | INVALID_ARGUMENT |
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

### Error classes

A variant of an enum may declare an **error class** before its name
(2026-10-17):

```text
type OrderError = enum { @not_found Missing, @conflict Taken(String), Closed };
```

A class is transport-neutral: it says what kind of failure the variant
is, and each transport answers a handler error of that variant with its
own status for the class. A variant that declares none, and an error type
that is a record, keep the handler error's 422 and `FAILED_PRECONDITION`.
The checker refuses any other word (`` `@missing` on the variant `A` of
`E` is no error class ``).

| class | the handler says | HTTP | gRPC |
|---|---|---|---|
| `@not_found` | what the request names does not exist | 404 | NOT_FOUND (5) |
| `@conflict` | the request conflicts with the current state (it exists already, it changed, it is closed) | 409 | ABORTED (10) |
| `@invalid` | the request is well formed and the handler judged it invalid | 422 | INVALID_ARGUMENT (3) |
| `@forbidden` | the caller may call the member and may not do this | 403 | PERMISSION_DENIED (7) |
| `@unavailable` | what the handler needs is unavailable; a later call may succeed | 503 | UNAVAILABLE (14) |

The body is the handler error's whatever its class: `E` by codec. Three
of the statuses are a refusal's too (403, 409, 503), so over HTTP a classed
handler error names its class in the header `Hale-Error-Class:
not_found`, which no refusal carries, and a client tells the two apart by
it, never by the body (an enum's variant may be named `refusal`); under
CORS the response exposes it (`Access-Control-Expose-Headers`). An
unclassed handler error's 422 carries no such header, as before.
`@invalid` keeps HTTP's 422, which an unclassed error has too, because 400
is `malformed`'s, the transport's own judgment that a request does not
decode; gRPC tells the two apart (INVALID_ARGUMENT, FAILED_PRECONDITION).
`@conflict` is ABORTED and not ALREADY_EXISTS: the class covers every
conflict with the state (a duplicate, a version that moved, an order
already closed), and ALREADY_EXISTS asserts one cause the class does not
carry; the variant in the details names which. Over gRPC the message stays
`handler_error` and the details' `Any` stays `E`, under the class's code.
The Unix reply line names the class after the error, `{"ok": false,
"error": "Missing", "class": "not_found"}`, and a line for an unclassed
variant has no `class`. The MCP transport's error object has no member
for it: its `code` (-32001) and `data` (`E`) are unchanged, and the class
is the description's to say.

The class is contract: the description and the schema documents carry it
on the enum, `"x-hale-error-classes": {"Missing": "not_found", "Taken":
"conflict"}` (only on an enum that declares one), the OpenAPI form lists
each class's status as a response of the member (where a refusal has that
status too, its schema is either body, `anyOf` the refusal and `E`, since a body may match both and the header, not the body, tells them apart), and
the enum's contract shape renders a class after its variant
(`=enum(Missing@not_found|Taken(s)@conflict|Closed)`), so a class declared,
changed or removed moves every digest built on the type, and an enum that
declares none hashes as it always has. A served description (`GET
/.description`, `Describe`) carries the classes as the compiler's does.
The generated clients and `hale call` read a response carrying
`Hale-Error-Class` as the handler error, and `hale call` names the class
(`handler error (not_found): "Missing"`).

The runtime interface carries the class: `std::api::Outcome`'s
`HandlerError(String, std::api::ErrorClass)` (§ The runtime boundaries),
`Unclassed` for a variant that declares none. This is a breaking change
for a transport the program writes, whose `match` on an outcome spells
`HandlerError(b, c)` where it spelled `HandlerError(b)` (2026-10-17).

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

**The HTTP transport** (`http::Rpc`, R3; § The HTTP transport) takes one request per
connection: `POST /call/<member>` (`/call/Orders::place`) whose body is
the payload by codec, under `Authorization: Bearer <token>`, with the
digest, when the client sends one, in the header `Hale-Surface-Digest`.
`GET /.description` answers the caller's description with 200. The
response's status and body are the table's; a request that is neither
is `malformed`.

**The MCP transport** (`mcp::Rpc`, R6; § The MCP transport) takes the
same recorded calls as `tools/call` messages in `POST /mcp`, and answers
each outcome as a JSON-RPC message: a result for the response, an error
object (`code`, `message`, `data`) for the handler's error, a refusal and
the server error, with the codes of § The MCP transport; `tests/api-contract/
wire/http/*.json` supply the payloads, so the `data` of each error is
the recorded body.

**The gRPC transport** (`grpc::Rpc`, R7, R8b; § gRPC) takes the same recorded
calls as unary gRPC calls, `POST /Public/Orders.place` (or `/Public/Orders__place`,
the rpc of the `.proto`) with the recorded body as the one request message,
and answers each outcome as the gRPC column says: the recorded response body
as the response message under `grpc-status: 0`, or a trailers-only response
whose `grpc-status-details-bin` carries the recorded error or refusal body as
the value of its `Any` detail. Under `application/grpc+json` those bodies are
the recorded JSON; under `application/grpc` and `application/grpc+proto` each is
the same value as the message of the generated `.proto` (the recorded
`{"order": 41, "notional": 125000}` is an `OrderReceipt`: field 1, 41, field 2,
125000), the details' `Any` carrying `OrderError` or `HaleRefusal`.
`tests/api-contract/wire/http/*.json` supply the bodies, so nothing is
recorded twice, and `tests/api-contract/Public.proto` and `Admin.proto` are the
files the messages are held to.

Every recorded exchange of the HTTP and Unix transports is under
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
instance in the holder: the param's own, and the right-hand side
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
instance built by a literal in the main locus's `params`: `ws::Hub`
(WebSocket) or `udp::Hub` (datagrams, § Over datagrams), with `bind:`
the address it listens on, `principals:` and `roles:` its two sources and
`as:` its name. The literal is the param's default and the only hub:
the compiler numbers it and gives it its rows there, so a construction
that supplies the param is refused ("param `hub` is a hub that streams
are bound to; its default is the hub, and a construction cannot supply
another"). It is two kinds of locus sharing one listener, so that no
locus has to serve two interfaces: the hub itself holds the sources,
admits subscriptions and is the `Rpc` a surface may be served over
(`api::serve(S, self.hub, …)`: one listener carries rpcs and streams over
one connection, authenticated once, at connect, § Rpcs on a hub); and
each topic bound to it is an adapter binding, the stream adapter
(`__StdBusAdapter`) with the topic's JSON codec, that hands each event
published to the connections' queues. A hub's **stream rows** are the
topic bindings to it, each with its topic, payload, direction (`out`: a
stream carries what the program publishes, and a row for a topic nothing
publishes is refused), codec, `bound`, `on_full`, whether it replays, and
`requires`. A row may bind a topic another seed declares and require a
role another seed declares, both through the import alias
(`lib::Ticks: self.hub requires: [lib::reader], …`): the topic is the
library's topic and the role the library's role (`spec/types.md` §
Roles). A row states `bound:` and `on_full:`; `refuse` is not a
policy of a stream, since a subscriber that cannot keep up loses events,
never the subscription. A hub has at most sixteen stream rows.

The hub's address is bound in the program's birth, before any user code
runs, and a program that cannot bind it does not boot (a diagnostic and
exit status 2, as for a socket transport). A hub is a main-locus param
for the reason a socket transport's listener is: its accept and read loops
run on a pool of their own (`__api_ws`), which only the main locus places,
and the compiler adds the listener to the main locus, born after every
param the program declares (so the hub's `bind:` may read one). A `ws::Hub`'s
listener is one of the TCP connection substrate's (spec/stdlib.md § The TCP
connection substrate), named `host:<bound port>`: it relays each accepted
connection as a `std::io::tcp::Accepted` to the consumer on its route, whose
handler starts the connection's locus as a flow child on `__api_ws`, so a
connection nothing takes is closed and `std::io::tcp::stop(name)` closes the
listener while the connections it accepted are served on (the hub's `stop()`
still ends them, with `closed`). A `udp::Hub` has no connections, and its
endpoint is one socket of its own.
Each stream is a thread of its own for the publish fanout to write
through (an adapter binding's), so a hub with many streams costs that many
threads.

- **Admission.** A subscription is authorized against the row's
  `requires` before it is admitted, from the `Context` the hub's
  sources established; a caller who may not read a stream is refused
  and buffers nothing. A `subscribe` is answered `subscribed` or `refusal`
  (the frames below), and one for a subscription already admitted is
  answered `subscribed` again and changes nothing (its `seq` and its queue
  go on). The roles are read as a serve site's are: directly, or through a
  role that includes them, from the role source's grants-and-revision pair
  when it states one (`RevisedRoleSource`) and its `holds` otherwise.
- **Delivery.** A publish on a hub-bound topic is accepted (the publish
  contract, `spec/semantics.md` § The publish contract) when it is
  handed to every admitted subscriber's queue, each holding at most
  `bound` frames and shedding under `on_full` (`drop_old` sheds the
  oldest undelivered frame, `drop_new` the frame being published).
  Every event offered to a subscription takes the next `seq` of that
  subscription, delivered or shed, so the frames shed are the gap
  between two `seq`s the subscriber receives: after `drop_new` sheds the
  newest, the gap shows at the next frame that arrives. A subscription's
  queue is kept by its connection, which alone knows when its socket takes
  a frame: a frame is written as soon as the socket takes it, and a
  connection that writes slowly (a client that does not read) queues up to
  `bound` frames behind the write. Frames go out one at a time, in the
  order they were offered. An event has to fit the bus frame, 65,536 bytes
  on the wire (the encoded payload and 17 bytes of header): a larger one
  reaches the program's local subscribers and no remote one, as it does for
  any adapter, and takes no `seq`.
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

  How the hub knows. A role source announces a revision on the wire
  subject `__api.roles.revision` (§ Identity sources), which every
  connection subscribes to. On one, the connection delivers nothing to its
  subscriptions, and asks the hub to recheck its subscribers' roles against
  the source's grants now; what is published meanwhile is queued, and sheds
  under the row's policy, but is not written. The answer ends each
  subscription whose `requires` no longer holds (one `unauthorized` frame,
  reason `revoked`, and its queue dropped, so no frame offered after the
  revision reaches a subscriber who lost the grant) and releases the rest,
  whose queued frames then go out in order. A revision that changes
  nothing for a subscriber costs it a pause and no frame. A source that
  cannot announce its own revisions (a table fixed in the program) tells
  the hub, `self.hub.announce(revision)`, with the same effect; a source
  that never announces is never rechecked, and only the credential's
  expiry ends a subscription. The credential's expiry is the connection's
  to enforce, from the instant the bearer source stated at connect: no
  event is written at or past it, and the instant itself ends the
  subscriptions (one `unauthorized` frame, reason `expired`) with no event
  or request to wake the connection. A spent credential admits nothing new
  (`unauthenticated`, reason: the bearer token is refused: expired).

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
| closed | hub | `{"type": "closed", "reason": "shutting_down"}`, at the hub's `stop()`, or when the drain reaches a connection still open (a signal, or its program's teardown, which drains the tree before the hub's own stop comes round: `runtime.md` § "Lifecycle obligations", line 15), after what is already queued for it; then the connection closes |

A refusal's object is § Outcomes', its kind one of `unauthenticated`
(the sources named nobody at connect: no credential was presented, or the
bearer source refused it, with its reason), `unauthorized` (the `Context`
does not hold the row's `requires`: reason `<topic> requires <role>`, and
the object carries `"requires"`), `malformed` (not a frame, a frame of an
unknown type, a subscribe naming no topic, or a topic the hub binds no
stream for: reason `unknown_topic: <topic>`) and `shutting_down`. A
refusal for a frame that names no topic carries `"topic": null`. `P` is
the payload by the row's codec. `seq` starts at 1 for a subscription and
grows by one per event offered to it, so a gap is the frames shed (the
delivery bullet above). A connection that ends without a `closed` frame
is the transport failure, and so is a program that ends without calling
`stop()`: its connections end with no `closed` frame. The frames are R0's
contract, which a consumer builds against; R5 delivers them as follows.

- **The credential.** A client presents its credential at the WebSocket
  upgrade, `Authorization: Bearer <token>`, or `?access_token=<token>` on
  the request's path for a client that cannot set a header; the hub's
  bearer source names who it is, and its expiry, once. A connection that
  presents none, or one the source refuses, is not turned away: the
  upgrade completes, and each of its subscribes is refused
  `unauthenticated`.
- **The WebSocket.** The upgrade is RFC 6455's (`Sec-WebSocket-Key` answered
  by the derived accept key). Client frames are masked text; a ping is
  answered with a pong, which is written after the frames queued before the ping; a client's close frame is answered with one and the
  connection closes; a fragmented message, or one past 64 KiB, closes the
  connection. The hub's frames are single unmasked text frames.
- **`stop()`.** `hub.stop()` (and the stop of a serve handle for a surface
  served over the hub, and the hub's teardown) stops accepting, writes
  `closed` to each connection after what was queued for it, then the close
  frame, and releases the address. A subscribe after `stop()` is refused
  `shutting_down`. `stop()` is idempotent.
- **A connection that breaks** (an EOF, a failed write, a client that does
  not read for 5 s of one write) is forgotten alone: its subscriptions go
  with it, and the hub and its other connections go on.
- **The description** is served at the hub's listener: `GET
  /.description`, with the credential as above, answers the caller's
  document (§ The description) as compact JSON, `401` with a refusal
  object when the sources name nobody. It is the document `hale check
  --api --exposure NAME --caller PRINCIPAL --holds ROLE,…` prints for
  that caller, which the compiler wrote the pieces of: `tests/api-contract/
  fills.dave.description.json` and `fills.bob.description.json` are what
  the witness's hub answers `dave` and `bob`, as values.

**Rpcs on a hub.** A surface served over a hub (`api::serve(Public,
self.hub, as: "desk", …)`) is carried on the connection a subscriber
holds, with the same sources: the caller's `Context` is the one the
credential presented at the upgrade established, checked by the exposure
as for any transport. A call is a frame

```text
{"type": "call", "id": "c1", "call": "Orders::place", "payload": {…}, "digest": "…"}
```

(`id` and `digest` optional) and is answered by

```text
{"type": "reply", "request_id": 7, "id": "c1", "ok": true, "value": {…}, "caller": {"mode": "bearer", "name": "dave"}}
```

whose `ok`, `value`, `error` and `refusal` are § Outcomes' unix reply's, the
`id` the client's, echoed raw. A reply and the events of a subscription
share the connection, in the order the hub wrote them. A call for a
connection that ends before its reply is lost as any accepted request is
(§ Receiver failure and generations).

**Over datagrams.** `std::api::udp::Hub` is the `ws` frames minus the
connection. A subscriber is an (address, `id`) pair: every datagram of a
subscriber carries the `id` it chose in its subscribe (any JSON string or
number, echoed raw) as the correlation a connection otherwise is, so one
socket may hold several subscribers; the credential rides the subscribe,
`{"type": "subscribe", "topic": T, "id": ID, "token": TOKEN}`, since there
is no connection to authenticate once at connect, and the hub's sources
name the subscriber at its first datagram. It is answered by a
`subscribed` or `refusal` datagram and then sends `event` datagrams
(`{"type": "event", "id": ID, "topic": T, "seq": N, "payload": P}`), the
one `unauthorized` datagram at expiry or revocation, and `closed` to each
subscriber at `stop()`; a subscriber ends its subscription with
`{"type": "unsubscribe", "id": ID}`, which frees the id (its `seq` starts
again at 1 when the id is used again), and `{"type": "describe", "id": ID,
"token": TOKEN}` is answered by `{"type": "description", "id": ID,
"document": …}` (or `"document": null` and the refusal). A subscriber that
vanishes without unsubscribing is held until `stop()`. A datagram is sent
when the hub writes it and is not retried: the network may lose or
reorder it, so a gap in `seq` is a frame shed or lost, and the loss
statement of a `udp::Hub` stream says so; the rest of admission, delivery,
expiry and revocation is the `ws` form's. The description's outcome form
for a datagram hub is `"transport": "udp"`
(`spec/api-description.schema.json`). A request and its reply over
datagrams are not framed in this release: a serve of a surface over a
`udp::Hub` is refused, saying so. UDP sockets share a port, so a second
program for the same UDP address is not refused; an address no interface
holds fails the boot.

**What a program asks of a hub.** The hub is a param, and its methods are
the program's: `events()`, the events its topics have offered; `subscribers()`,
the subscriptions admitted and not ended, over every connection;
`connections()`; `announce(revision)`, above; and `stop()`.

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
refused by law 5.

**A Float** crosses JSON as the same double: the encoder writes the
shortest number that reads back as it (`0.30000000000000004`,
`99.999995`, `100`, `1e-300`), a field, a list's element and a whole
value alike, and the decoder reads it with `std::str::parse_float`, a
subnormal included. NaN and the infinities are no JSON number: the
encoder writes them as `nan`, `inf` and `-inf`, which the decoder (and
any other JSON reader) refuses, `wrong_type`, so such a value does not
cross JSON. Protobuf carries a Float as its eight bytes, every double
included.

**An instant and a duration.** The stdlib's `Time` (a point over
`Duration` with no origin, `spec/types.md`) is the one point the codec
carries, as its RFC 3339 text in UTC: what `std::time::iso8601` writes,
the fraction to the nanosecond with its trailing zeros dropped and none
when it is zero (`"2026-10-12T03:04:05.123456789Z"`,
`"2026-10-12T03:04:05Z"`). It is read back by `std::time::parse_time`,
the `Z` required: a text with no zone, one with an offset (`+01:00`,
which `parse_time` refuses rather than read as UTC), an instant outside
`Time`'s range (`1677-09-21T00:12:43.145224192Z` through
`2262-04-11T23:47:16.854775807Z`) and a number are `wrong_type`. Its
schema is `{"type": "string", "format": "date-time", "x-hale-type":
"Time"}`. A `Duration` is the quantity it is (`quantity Int in ns`):
its count of nanoseconds, as any quantity is its count in its
denomination (`spec/units.md` § Layout and the wire), `{"type":
"integer", "x-hale-type": "Duration", "x-hale-unit": "q(ns)"}`, never
a text such as `"1.5s"`. A plain alias of either is what it stands
for. A literal default of either (`` = `2026-01-01T00:00:00Z` ``, `= 5s`)
is kept when the field is left out, as a scalar's is, and so is a
literal default of a unit scalar.

**A list** is a field's: `bounded[T; N]` or `[T; N]` (a literal `N` of
at least one), its elements what a field of their type is (a record, a
scalar, a string, an identity, a range or a quantity by its count). It
is a JSON array, and the description says its bound: `{"type": "array",
"items": …, "maxItems": N}`, and `"minItems": N` for a fixed array. A
`bounded` field may be left out of a payload, and is empty, as its
declaration leaves it; a fixed array is always sent, all `N` of it (its
default is not read). A payload never makes a list shorter than it says:
more elements than the list holds is `too_long`, fewer than a fixed
array's length `too_short`, both `malformed`, and the handler does not
run. What is not carried, each refused by law 5 naming the field: an
unbounded `[T]` (an unbounded collection is locus-owned, `@form(vec)`,
and a type holds bounded data only, `spec/types.md` § bounded[T; N]), a
list of lists, and a list of a shape the codec does not carry (a
`Decimal<S>` among them: an api surface has no schema for one yet, as a
field or as an element, though the generated codecs read and write it as
JSON's tier 2 does, a number at its scale, and protobuf as its digits). A
request or response is never itself a list: a page of records is a
record, `type Page { items: bounded[Item; 50]; }`, which leaves room
for what a page says beside its items.

**An optional field** is a presence pair (2026-10-17): a field `x` beside
a field `has_x` of the same record whose type is `Bool` (an alias of it
included). Hale has no optional type; the flag is the presence. On the
JSON wire `x` is a key that may be left out, and `null` is its absence
as well: a payload carrying `x` with a value other than `null` sets
`has_x`, one that leaves it out or sends `null` leaves `has_x` false and
`x` at its default, or its type's zero where it declares none (a range
that leaves out `0` has its least value as its zero; a record's zero is
built of its fields' defaults and zeros, and is built only when the record
is absent; a `bounded` list's is empty). The last of a key sent twice wins:
a `null` after a value is an absence, the earlier value gone. `has_x` is no key: the decoder skips one a payload sends, as it
skips any unknown key, and the encoder never writes it. The encoder writes
`x` only when `has_x` is set. The value is as strict as any field's: `"x":
5` for a `String` is `wrong_type`. Any shape a field may have may be
optional, a list included (JSON tells a present empty list, `[]`, from an
absent one). A description gives `x` the schema `{"anyOf": [<its schema>,
{"type": "null"}]}`, lists it under no `required`, and has no property for
`has_x`. Protobuf carries the pair as declared: `x` its field, `has_x` a
`bool` field of its own, each numbered by its place in the record (the
`.proto` notes the pair in the two fields' comments), and a message
carrying neither reads as an absence, `x` at its default or zero. The
digest is the record's shape, which already holds both fields.

**A three-way field** is a presence triple (2026-10-19): a presence pair
and a third field `clear_x` of the same record whose type is `Bool`. It
is the field of a PATCH, where leaving `x` out keeps what the server
holds, `null` clears it, and a value sets it. On the JSON wire `null` is
then a value of its own: a payload that sends `"x": null` sets `clear_x`
and leaves `has_x` false and `x` at its default or zero; one that sends a
value sets `has_x`; one that leaves `x` out sets neither. The last of a
key sent twice wins, as ever. `clear_x` is no key either. The encoder
writes `x`'s value when `has_x` is set, `"x": null` when `clear_x` is set
instead, and nothing when neither is (a record that sets both is written
as set). A pair without the third field reads `null` as an absence, as
above, so declaring the triple is what makes `null` mean clear. A
description gives `x` the pair's schema with `"x-hale-null": "clear"`
beside it, and lists it under no `required`. Protobuf carries the three
fields as declared, `clear_x` a `bool` of its own. The generated clients
carry it: the Hale client declares the triple; in TypeScript the property
is optional and may be `null` (`x?: T | null`), and the client sends
`null` as `null` and leaves out an `undefined` one.

**An enum** crosses as its variant (2026-10-17), externally tagged: a
payload-free variant is its name as a JSON string (`"Up"`), a variant
of one value the object `{"<Variant>": <value>}`, and one of several
`{"<Variant>": [<value>, ...]}` with exactly as many. A value is what a
field of its type is (a scalar, an identity, a range or a quantity by its
count, a `Time`, a record, a payload-free enum); a list, and an enum with
payloads of its own, are not a variant's value, and law 5 refuses the
row naming the variant. An enum may be a field, a list's element when it
is payload-free (a `bounded` holds no enum with payloads, `spec/types.md`),
a row's whole request or response, and its error type. Decoding is
strict, and every refusal is `wrong_type` for the field the enum is read
for (`payload` for a row's whole value): an unknown variant, a payload
variant sent as a bare name, a payload-free one sent as an object, an
object of other than one key, a value of the wrong kind, or the wrong
number of values. An absent enum field is its default when it declares
one (`scope: Scope = Scope::key`), and otherwise `missing_field`, unless
it is optional (above): its zero is its first variant, with its values'
zeros. A description gives a payload-free enum
`{"type": "string", "enum": [<variants>], "x-hale-type": "<Enum>"}` and
one with payloads, inline, `{"oneOf": [<that, for its payload-free
variants>, {"type": "object", "properties": {"<Variant>": <value
schema>}, "required": ["<Variant>"], "additionalProperties": false},
...], "x-hale-type": "<Enum>"}`, a variant of several values an array
schema with `prefixItems` and its count as `minItems` and `maxItems`.
In protobuf a payload-free enum is a proto3 `enum` whose values are
`<Enum>_<Variant>` (proto3 scopes an enum's values beside it, so the
prefix keeps two enums apart), numbered from 0 in declaration order: a
field of it is a varint, a list of it a `repeated` field, and a number no
variant has is `wrong_type`. An enum with payloads is a message of one
`oneof value`, a member per variant numbered from 1 in declaration order:
a payload-free variant a `bool` (any value of it is the variant), a
variant of one value that value's field, one of several the message
`<Enum>_<Variant>` of fields `v0`, `v1`, ... A message carrying no
member is `missing_field`; of several, the last wins, and a member whose
value is a message (a record, several values) sent again before another is
merged with the one before it, as proto3 merges a `oneof`. The message of
several values has plain proto3 fields, which a client leaves out at their
zero, so a value it does not carry is its zero (`0`, `""`, the epoch, the
first variant), never `missing_field`. A row whose whole request or response is a payload-free enum
crosses gRPC in the one-field message a scalar row has, its `value` the
enum. The digest is the enum's shape, which holds its variants and their
values. A set (`spec/types.md` § Set types) crosses as the enum it is,
each member a variant of one value named for its type: `type Inbound =
NewOrder | Cancel;` is `{"Cancel": {"id": 1}}`, and its description the
`oneof` above.

**A map** is a list keyed by one of its records' fields (2026-10-17):
`checks: bounded[Check; 16] indexed_by name;`, the `@form(hashmap)` model
projected onto a value (spec/forms.md), no new type. The clause's rules
are the checker's (spec/types.md § bounded[T; N]): a `bounded` list of a
user-declared struct, keyed by a `String` field of it. On the JSON wire
it is an object keyed by each record's `name`, the record without its
key as the value (`{"db": {"status": "up"}, "cache": {"status":
"down"}}`): the key lifted out on encode, in the list's order, and
restored on decode, where a value's own key field, if a payload sends
one, is not read. A key the object repeats is refused `malformed`,
`duplicate_key`; more entries than the list holds is `too_long`; an
absent map is empty, as an absent `bounded` list is. A map may be
optional (a presence pair, above): an absent key and `null` are its
absence, `{}` a present empty map, and its schema is the map's or `null`;
in protobuf its flag is a field of its own, as any optional field's is. The description
gives it `{"type": "object", "additionalProperties": <the record's schema
without its key>, "maxProperties": N, "x-hale-key": "<key>"}`. In
protobuf it is a proto3 `map<string, T>`, each entry its key and the
whole record, the key field carried in it too: the value is the record's
own message, the one every other use of the type names, rather than a
second message per map. On decode the entry's key wins over the record's,
and a key repeated is `duplicate_key` here as well (proto3 lets a parser
keep the last; the codec is as strict as the JSON one). The digest
folds the key in (`checks:bounded[...]@name`), since the field crosses
as an object and not a list. A key field with a presence pair or triple
(`name` beside `has_name`, and `clear_name`) is supplied by the map: a
record read from it, through JSON or protobuf, has `has_name` set and
`clear_name` unset, whatever the record sent, and neither flag crosses
in the value.

**A map of values** (2026-10-19) names the value too: `checks:
bounded[Check; 16] indexed_by name -> status;`, where `Check` is `{ name:
String; status: String; }`. Each element crosses as its `status` alone,
keyed by its `name` (`{"db": "ok", "nats": "unavailable"}`), and is built
back of the member's key and value on decode. It is still the hashmap
model projected onto a value, the list's elements carrying their key, so
the program holds the same bounded list of records. The clause names
which field is the value rather than guessing it from the record's shape:
a record of one field besides its key keeps crossing as a map of records
(`{"db": {"status": "up"}}`), and spelling a map of values never moves a
wire that exists.

The element has no fields but the key and the value, so the decoder
builds every record whole, and the check refuses any other field at the
clause; the key's own presence flags are the key's (the decoder sets the
key present and not cleared, as for a map of records). The value is a scalar (a unit-dialect one by its count), a `Time`
or a payload-free enum, read through its aliases as the codec reads every
field; a record, a list (an alias of one included), or an enum with
payloads is refused at the clause, because a map of records already
carries one. What the check admits is the codec's own answer
(`json_gen::uncarried_wire_maps`), so no admitted map is left out of a
codec. A
member's value is read as a list element of its type is, and one of the
wrong kind is `wrong_type`. A key the object repeats is `duplicate_key`,
more entries than the list holds `too_long`, and an absent map is empty,
as for a map of records.

The description gives it `{"type": "object", "additionalProperties":
<the value's schema>, "maxProperties": N, "x-hale-key": "<key>",
"x-hale-value": "<value>"}`. In protobuf it is a proto3 `map<string, V>`
of the value's scalar or enum, each entry its key (1) and value (2) as
plain proto3 fields. So an entry that leaves its value out (a proto3
writer drops a zero) is the value's zero, and one that leaves its key out
is the empty key. The digest folds the value in beside the key
(`checks:bounded[...]@name->status`). The generated clients keep the
records and put each one's value alone on the wire.

**Free-form JSON** (2026-10-19) is a `std::json::Value` field: any JSON
value the program carries and never decodes into a type of its own (a
seat's configuration, a change's before and after). It is held as its
text, and the codecs carry that text byte for byte. The decoder takes the
member's value as sent, and the encoder writes it back, so key order,
spacing and a number's spelling survive. A value is well-formed JSON at
most 64 levels deep, or it is refused `wrong_type`, and at most 64 KiB,
or it is refused `too_long`. A list of values, a map of them, and an
optional one (a presence pair) are carried as for any field. A row's
whole request or response is never a `Value`: it is a record's field, and
law 5 refuses the row. The description gives it `{"x-hale-any": true}`,
an empty schema marked so a reader does not look inside, and the contract
digest tags it `json`. Protobuf carries the text as a `string`, commented
as JSON. A `google.protobuf.Value` would keep neither the bytes nor a
number's precision. The Hale client declares `std::json::Value`. The
TypeScript and JavaScript clients type it `unknown`, as the response's
`JSON.parse` gave it and as `JSON.stringify` writes it back, so a client
is not byte-faithful; the server is. A program builds a value with
`std::json::Value::parse(text) -> Value fallible(ParseError)`, held to the
same rules, or takes `std::json::Value::null()`. The check refuses a
literal of one (by either spelling), a store into one (`v.text = …`,
through a `let mut` copy or a record's field alike), and every other
constructor, so every `Value` is one `parse` admitted; a codec reads one
through `parse` too, and writes its text as it is. It reads one through its `.text` with
`std::json`'s cursors (`find_field_raw`, `object_first`, ...). Two values
are equal when their texts are; there is no semantic JSON equality.

**A program's own use of the codec** (2026-10-17). `T::api_decode(text) -> T
fallible(JsonError)` and `T::api_encode(value) -> String` call the api
codec of a record or an enum `T` from any program, served or not: the same
codec a row of `T` crosses, as strict, with the same refusals
(`wrong_type`, `missing_field`, `too_long`, ...), a program needing no
surface to read or write the wire a surface speaks. A program that
handwrites a REST router over a surface decodes a request body into a
row's request type and encodes the result with them. An enum decoded whole
names `payload` as the field it refuses, as a row's whole enum does. `T`
is a record or an enum the codec carries whole (law 5's classification of
a row's request); any other type is refused at the call (`` `Blob::api_decode`:
the api codec carries `Blob` only if it is a record or an enum all of which
the JSON codec carries ... ``). The codecs are generated once per program,
shared with its served rows.

Decoding is strict: a value of the wrong JSON kind is `wrong_type`, an
element of a list and an array that is not well-formed JSON included
(a payload that is one scalar is that scalar's complete JSON token: `true`
or `false` for a `Bool`, one quoted and correctly escaped string for a
`String`, a number for a `Float`, a number with no fraction or exponent for
an `Int` and for the integer a unit scalar counts; anything else is
refused before the request is queued), a
missing field without a default is `missing_field`, and a handler only
ever sees a decoded value. A field with a default may be left out, and
the description lists it under no `required`: whatever the declaration
writes for it (a literal, an enum's variant, a record, a value computed
from others) is what the absent field decodes to, through either codec,
as the record's own literal would build it. A default reads only its own
fields and constants, never the scope of the literal that leaves it
(`spec/types.md` § Locus params), so every default a record can declare
is one the decoder builds on its own, and the description and both
decoders agree on each field. A list is the one exception,
by its shape: a `bounded` list left out is empty and a fixed array is
always sent, a default of either unread (above). A row whose shape has a field
the codec does not carry is refused by the admission law (law 5),
never left out.

**Protobuf** (R8b) is the second codec of a `grpc::Rpc` exposure, generated
from the same declaration: the same set of shapes and the same strictness,
carried as proto3 (§ gRPC, The protobuf codec). A field is numbered by its
place among the struct's declared fields, from 1, and a `json:"key"` tag
names the proto field as it names the JSON key. A list is a `repeated`
field, its bound in the field's comment (`// Hale: at most 50`); the
encoder writes each element as an occurrence of its own, and the decoder
reads that and the packed form proto3 writes by default for a numeric
element, in any mix, holding the bound as the JSON codec does
(`too_long`, `too_short`). A `Time` is an `int64`, its nanoseconds since
the epoch (which is all of `Time`; the field's comment says so), not
`google.protobuf.Timestamp`, which would make the well-known type a
dependency of every file and of the descriptor reflection serves; a
`Duration` is an `int64`, its nanoseconds. An optional field is its two
fields, an enum a proto3 `enum` or a `oneof` message, and a map a
`map<string, T>` (above). The set of shapes is the JSON codec's, so
there is no shape without a proto3 encoding. The `.proto`
generator refuses all the same, by row, so that were the codec to carry one it
would not be approximated.

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
- the listener (transport and address) and the codec. The address is
  the one the listener was bound to, not its `bind:` as written: a TCP
  listener's `host:<bound port>`, so a `bind:` of port 0 (or a bind
  read from the environment) is described by the port the kernel gave
  it; a Unix listener's path; a UDP hub's `host:<bound port>`;
- the caller: the principal the exposure established, and the roles it
  holds among those the exposure's rows and streams require;
- the **members** the caller may call: name, the member's digest (§ The
  contract digest), request, response and error schemas, and `requires`; a member whose error type is
  `ClosureViolation` carries the string `"ClosureViolation"` in place
  of an error schema (§ Surfaces and their rows);
- the **streams** of the hubs at that listener the caller may subscribe
  to: topic, wire subject, direction, payload schema, codec, `bound`,
  `on_full`, the loss statement, whether the binding replays, and
  `requires`;
- the transport's **outcome encoding** (§ Outcomes);
- the JSON Schema of every type the listed members and streams name,
  and nothing else, a quantity's field carrying `x-hale-unit` (its
  `q(…)` tag), a `Time`'s `"format": "date-time"`, a list's `maxItems`,
  an optional field's `anyOf` with `null`, and no property for its
  presence flag, an enum inline with its variants, a map's
  `additionalProperties` and `x-hale-key` (§ Codecs);
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
literal's namespace, `http`, `unix`), its address (`bind:` or `path:`,
as written: the compiler binds nothing, so its document states a
`bind:` of port 0 as port 0 where the served one states the bound port),
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
(`<Surface>.<form>.json`).

## The clients

`hale describe`, `hale call`, `hale watch`, `hale admin` and `hale mcp
--app` are clients of a served exposure, and read only its description
(`crates/hale-cli/src/api_drive.rs` for the first two, `api_client.rs`
for `watch` and `admin`, `mcp.rs`). None knows a member's or a stream's
name in advance. `hale describe` and `hale call` are the short forms of
`hale api describe` and `hale api call` (§ Driving a served program): an
**endpoint** is `unix:PATH` (a bare path stands for it) or
`http://host:port` (an `http::Rpc` listener, the caller named by
`--bearer T` or `HALE_API_BEARER`); `grpc://`, `mcp://`, `ws://` and a
TLS scheme are refused naming the two forms. `hale watch` and `hale
admin` keep their own endpoint forms below (`ws://host:port` for a hub's
listener; `--token T` or `HALE_API_TOKEN` for the bearer) until they
join the api verbs. A `grpc::Rpc` listener is described by `hale check
--api` and called by any gRPC client (§ gRPC; Open points).

- **`hale describe ENDPOINT`** is `hale api describe ENDPOINT`: the
  exposure's description for the caller the endpoint names, with
  `--json` the bytes it served (`{"describe": true}` over the socket,
  `GET /.description` over HTTP), never re-serialized, else the readable
  rendering; exit 0, 2 when the exposure refuses, 4 when the endpoint
  does not answer, 5 usage. The compiler's projections of a surface (the
  inventory, one exposure per caller, OpenAPI, JSON Schema, MCP,
  `.proto`) are `hale check --api` and `hale api export`, from the rows
  and without running the program; an endpoint does not print them.
- **`hale call ENDPOINT MEMBER [JSON | --field VALUE …]`** is `hale api
  call`: a bare JSON payload stands for `--json`. The description is read
  first, and the call names the **digest** it read in the transport's own
  place (`digest` in the line, `Hale-Surface-Digest` over HTTP), so a
  program that changed under the client refuses with `digest_mismatch`
  and the served digest instead of running a different contract. A
  result's value is printed on stdout and exits 0; a handler error exits
  1; a refusal 2, its kind and reason on stderr (a role's refusal names
  what the row `requires`); a server error 3; a transport failure 4;
  usage 5. `--raw` prints the outcome as the exposure wrote it (the reply
  line over a socket, the body over HTTP). A hub's `ws://` listener is
  not a call endpoint: its `call` frame is § Rpcs on a hub, driven by the
  generated clients.
- **`hale watch WS-ENDPOINT TOPIC`** reads the hub's description, refuses a
  topic the caller may not subscribe to (it is not listed), subscribes
  and prints each frame as one JSON line (`subscribed`, then `event`s
  with their `seq`) until the hub closes the connection (`closed`, exit
  0); a refusal or an `unauthorized` frame ends it with exit 1. A stream
  is the hub's: a socket or an HTTP endpoint is refused, saying `ws://`.
- **`hale admin ENDPOINT`** serves a page on `127.0.0.1` (`--port`, 7473
  by default) listing the description's members and streams, a form per
  member and a live tail per stream; every action is one request to the
  endpoint. The page is served only to the holder of the launch token the
  process printed, a request whose `Host` or `Origin` is not the page's
  is refused, and a call carries the digest of the description read at
  that moment.
- **`hale mcp --app ENDPOINT`** is the MCP bridge of § The MCP transport.

`hale api describe` and `hale api call` are the scriptable form of the first two
verbs, with an exit code per outcome and a payload typed by the member's schema
(§ Driving a served program).

The clients speak the R0 wire and nothing else: there is no read verb, no
`as_of`, no owner-only full form, and no reply to a call that carries more
than the transport's outcome (§ Outcomes). `tests/api-contract/` is what
they are held to, and `crates/hale-cli/tests/api_clients.rs` runs them
against one program served over a socket, HTTP, MCP and a hub.

### Generated specs and clients

The rows carry everything a client needs: the shapes, the five outcomes
(§ Outcomes), the digest, the roles of each member and the stream rows. A
spec or a client of a surface is therefore **generated from the rows, never
written by hand**, and a committed one names its surface's digest and is
refused when it drifts.

**`hale api export --surface NAME [--out DIR | --check DIR] [target]`** writes
the surface's bundle:

| file | content |
|---|---|
| `NAME.description.json` | the surface-wide document: `"inventory": 1` (the description schema's inventory form) restricted to the surface: its digest and every member with its `requires`, the exposures that serve it, every hub of the program with its stream rows, and the schemas they name |
| `NAME.openapi.json`, `NAME.json-schema.json`, `NAME.mcp.json` | the forms of § The description, as `check --api --surface NAME --openapi`, `--json-schema` and `--mcp` print them |
| `NAME.proto` | the protobuf form (§ gRPC, The `.proto`), as `check --api --surface NAME --proto` prints it: the messages and the service the gRPC transport speaks |
| `DIGEST` | two lines: the surface's digest (`fnv1a64:` and sixteen hex digits) and `hale <version>`, the compiler that wrote it |

`--check DIR` writes nothing and exits 1 when the committed bundle is not what
the surface now generates, naming the digest that moved (the first line of
`DIGEST`) and each file that differs or is missing. The compiler's version in
`DIGEST` is the bundle's provenance and is not compared: a bundle another
release wrote that still reads the same is current.

**The determinism rule.** A generator is a pure function of the rows and the
schemas they name: it reads no path, no clock and no environment, so the same
surface yields the same bytes on any run and any checkout. Two consequences
bind the documents. **A schema's name** (in a schema's key, a `$ref` and an
`x-hale-type`) is the name the type is declared with, an imported type's
included (`Item`, declared in the library the program imports as `lib`).
Only when another type of the program is declared with the same name, or a
generated form declares it for itself, is the imported one qualified, by the
path its importer writes with `.` for `::` (`lib.Item`). The names the forms
declare for themselves are one list, which every generator names its
declarations from: the `.proto`'s `HaleRefusal` and `HaleEmpty`, each client
preamble's names such as `ApiRefusal` or `Refusal`, a builtin record such as
`ClosureViolation`; for each member of the program's surfaces, the `.proto`'s
rpc `Orders__place` and one-field wrappers `Orders__placeRequest`,
`…Response` and `…Error`, and a client's `OrdersPlaceOutcome` and member fn;
for each topic, a client's `FillsEvent`, `FillsSubscription` and
`subscribeFills`; and for each surface, the `.proto`'s service. What the
forms generate after a schema is reserved by the schema's final name, since
it is spelled from it: the spelling of a qualified name (`lib_Item`), the
TypeScript and JavaScript clients' codecs (`encode_lib_Item`,
`decode_lib_Item`; the Hale client's are the type's own `api_encode` and
`api_decode`) and an enum's
`.proto` variant names (`a_Shape_Pair` for the variant `Pair` of an imported
`a.Shape`); the naming reserves them to a fixpoint, so a third library's
record `a_Shape_Pair` is `c.a_Shape_Pair`. A form declares nothing else but a
schema's name, which a test holds every generator to. The program's own type keeps the bare name, and two imported
types whose importers write one path (two seeds, each importing a `lib` of
its own) are told apart by a count after it (`lib.Item.2`), in the order of
their mangled names. So an imported type is never named by the cross-seed
mangled name, which embeds the library's location, and no name holds `::`,
which OpenAPI refuses in a component's key. The digest does not move with a
name: it is over the contract's shapes. (Until 2026-10-17 an imported type
was named by its importer's path, `lib::Item`, downstream handoff: an
exported OpenAPI form did not lint.) And a tool, a path or a schema is named by what the
program declares, in an order the rows fix (members by name as bytes, schemas
by name).

**`hale api client --surface NAME --lang hale|ts|js [--out FILE | --check FILE]
[target]`** writes a client of the surface (to stdout without `--out`). A
client:

- is typed by the rows: a type for every struct a member or a stream names (a
  quantity, an identity or a range crosses as the integer it counts, a
  `Duration` among them; a `Time` is a `Time` in Hale and its RFC 3339 text,
  a `string`, in TypeScript, since a `Date` holds milliseconds; a list is
  the client language's: `bounded[T; N]` or `[T; N]` in Hale, an array in
  TypeScript, whose bound the server holds; an optional field is the
  presence pair in Hale and an optional property (`text?: string`), left
  out of the object when absent, in TypeScript, and a presence triple's
  `text?: string | null`, its `null` sent as `null`; an enum is the server's
  enum in Hale and in TypeScript the union of its variants as the wire
  writes them (`"Empty" | { Circle: number } | { Rect: [number, number] }`);
  a wire map is its records in either, `bounded[T; N] indexed_by <key>` in
  Hale and an array in TypeScript, keyed into the wire's object and back
  by the client; any shape the JSON codec does not carry is refused, never
  approximated), and
  one function per member taking the endpoint, the bearer and the request;
- sends the surface's digest on every call (`digest` in the line over a socket,
  `Hale-Surface-Digest` over HTTP) and names it in a constant, so a program
  that changed under it answers `digest_mismatch` with the digest served;
- returns the outcomes of § Outcomes: the result, the handler's error (only for a
  row that declares one, with its schema), the refusal with its kind, reason,
  the roles it requires and the digest served, the server error, and a
  connection that never answered, which is never reported as a refusal;
- subscribes to each stream row of the program's hubs over `ws://`, yielding
  typed events with their `seq` (a gap is the frames shed) and ending with the
  hub's `expired`, `revoked` or `closed` frame, a `refusal` when the subscription
  was not admitted, and a lost connection when the hub ended it with no closed
  frame.

The **Hale client** declares each record as the server does, so that the
compiler's api codec (§ Codecs, a program's own use of it) reads and writes it:
a field named for its JSON key (and tagged `json:"<key>"` where the key is no
identifier), an optional field and its `has_` flag, each enum the rows
reach with its variants, a map with its `indexed_by` clause, and a field a
payload may leave out given its type's zero as its default (an empty record
where one can be built, an enum's first variant with its values' zeros), so
a value is built naming only what it means (`T { code: "x" }`). It encodes a
request with `T::api_encode` and decodes a reply with `T::api_decode`, an
enum's as a record's, as strictly as the server reads a request: a reply that does
not decode is `Lost` with the codec's kind and field ("the result did not
decode: wrong_type: id"). It is one module (a function `<locus>_<fn>` per member whose
answer is the enum `<Member>Outcome` = `Result(T) | HandlerError(E) |
Refusal(ApiRefusal) | ServerError | Lost(why)`, and a locus `<Topic>Subscription`
per stream with `start`, `next`, `next_for`) speaking `unix:PATH`, `http://` and
`ws://`; it adds `api_describe` and `api_lists`, which read the description the
endpoint serves the caller. The **TypeScript client** is one `.ts` module with no
dependency beyond `fetch` and `WebSocket` (`http://` and `ws://`; a socket is not
spoken, `fetch` has none): an async function per member answering
`{kind: "result" | "handler_error" | "refusal" | "server_error"}` and throwing
`TransportError` for the fifth, and `subscribe<Topic>(opts)`, an async iterable
of events; the credential rides a hub upgrade as `?access_token=`. Its
`encode_<T>` and `decode_<T>` per record and enum convert between its types and
the wire; a map's key is lifted out of each record on the way out and put
back on the way in (`apiMapEncode`, `apiMapDecode`, which only a client of a
surface that names a map carries). The
**JavaScript client** (`--lang js`) is the TypeScript client as one ES module a
page or `node` loads with no compile step: the same exports by the same names,
the same wire and outcomes, its types JSDoc (`@typedef` per record and outcome,
`@param` and `@returns` per function, a `// @ts-check` header), so an editor or
`tsc --checkJs` holds a caller to the rows as the `.ts` module's types do. TLS
is not spoken by any of them. **None speaks gRPC, and that is by decision**: a gRPC call
needs HTTP/2 with trailers, which `fetch` does not give a script and which a
generated Hale client has no stack for (nghttp2 is linked into served programs,
not into a program that calls), so a gRPC mode would need a library beyond the
client's own. The client path of a `grpc::Rpc` exposure is its `.proto`
(`hale api export` writes it) and the stub generator of the caller's language
(`protoc`, `buf`, `grpc-tools`), or `grpcurl`, which finds the surface through
server reflection without the file; the stubs call `Orders__place` and send the
digest in the `hale-surface-digest` metadata and the bearer in `authorization`.

`--check FILE` writes nothing and exits 1 when the committed client is not what
the surface now generates, naming the digest it was made against when that is
not the surface's. A client never carries more than the contract: the recorded
requests of `tests/api-contract/wire/` are what the generated clients write, and
the recorded replies what they read (the TypeScript and JavaScript clients run
the same compiled scenarios to the same outcomes)
(`tests/hale/api/client_test.hl`, `crates/hale-cli/tests/fixtures/ts-client/`).

## Imported surfaces

A surface the program **consumes** is declared like one it serves, with
rows that name each operation's shape instead of a handler, and called
through a client (§ Calling a surface). `hale api
import` writes one from an OpenAPI 3 document or an MCP endpoint's tool
list, and `hale api drift` holds it to that source afterwards. The block is
an ordinary `api` declaration: the checker admits it, the description and
the generated forms list it like any surface, and nothing serves it.

The OpenAPI document is JSON or YAML: it is JSON when it opens with `{`,
and YAML otherwise, or when its name ends `.yaml` or `.yml` or a URL
answers `application/yaml` or `text/yaml`; `hale api drift` reads it the
same way. The YAML reader gives the value the JSON reader gives for the same
document, and reads the subset an OpenAPI document is written in: block
mappings and sequences by indentation, plain, single- and double-quoted
scalars (with escapes), `|` and `>` block scalars with `-` and `+`
chomping, flow collections, comments, and one document with its `---` and
`...` markers. Scalars resolve by the YAML 1.2 core schema (`~` and `null`
are null; `on` and `yes` are strings; a mapping key is the text it is
written as). It refuses, naming the line, anchors and aliases (`&`, `*`),
merge keys (`<<`), tags (`!`), complex keys (`?`), directives, duplicate
keys, tab indentation and more than one document.

```hale,fragment
api Shelf from openapi "../shelf.json" {
    rpc create_item(ShelfNewItem) -> ShelfItem wire: "POST /items";
    // response: std::json::Value, an array with no `maxItems`, and a record holds a bounded list only
    rpc list_items(ShelfListItemsRequest) -> std::json::Value wire: "GET /items?limit";
}

type ShelfNewItem {
    name: String;
    has_note: Bool = false;
    note: String = "";
}
```

**The block.** `api NAME from openapi "LOCATION"` or `from mcp
"LOCATION"` names where the surface was read from: an `http://` or
`https://` URL, or a file path relative to the file holding the block. A
**signature row**, `rpc name(Request) -> Response [wire: "…"] [requires:
[ROLE, …]];`, names its member (`name`, no locus), its request (the
parentheses may be empty) and its response (none without the arrow), and
`wire:` is the operation as its endpoint names it: an OpenAPI operation's
method and path (`"GET /items/{itemId}"`, its query parameters' names
after a `?`), an MCP tool's name
(`"get-weather"`). A signature row has no error column. Its types are
checked where they are written (an unknown name is refused there), and
law 5 holds them to the JSON codec, with one allowance: a signature row's
whole request or response may be a `std::json::Value`, the shape of a
source that declares none. A signature row's member digest is a handler
row's, over the same slots. Law 8 keeps the two kinds of row apart, and a
serve site of a consumed surface is refused: "serve of \`Shelf\`:
\`Shelf\` is a surface the program consumes, whose rows name shapes and
no handler, so nothing here answers them". A signature row is no
handler of a `role R` group (docs/src/claims.md).

**`hale api import SOURCE --as NAME [--out FILE] [--openapi | --mcp]`**
reads the source and writes the block, then the records and enums it
names, each prefixed with the surface's name (`ShelfItem`), into `FILE`
(`-` for stdout; by default the surface's name in snake case with `.hl`,
in the current directory). A file is read as an OpenAPI document; a URL
as one when it serves one by `GET`, and as an MCP endpoint otherwise
(`--openapi` and `--mcp` say which). The mapping:

- **An OpenAPI operation** (OpenAPI 3; Swagger 2.0 and a YAML document are
  refused naming why) is a row named by its `operationId` in snake case
  (`listItems` is `list_items`), else by its method and path
  (`get_items_item_id`); `wire:` is `"METHOD /path"`, followed by `?` and
  its query parameters' names joined by `&` when it has any (`"GET
  /items?limit"`), so a client tells the query from the body. Its request is the
  JSON body's schema when the operation has no path or query parameter, a
  record of its parameters when it has no body, and a record of both,
  the body as the field `body`, when it has both; a header or cookie
  parameter is not carried. Its response is the JSON schema of its
  lowest-numbered `2xx` response, none when that response has no JSON
  content.
- **An MCP tool** (every page of `tools/list`, after `initialize`, over
  the Streamable HTTP transport, a JSON or an event-stream answer alike;
  the notification and every page name the session and the protocol
  version the server chose, `MCP-Protocol-Version`, and a version the
  import does not speak, 2025-06-18, 2025-03-26 or 2024-11-05, is refused)
  is a row named by the tool's name in snake case, `wire:` its name; its
  request is a record of its `inputSchema` (none when the schema declares
  no property), its response a record of its `outputSchema`, or a
  `std::json::Value` when it declares none.
- **A schema**: an object with properties is a record, its fields named
  by the key in snake case with a `json:"key"` tag when the two differ (a
  key that would name a field `has_…` or `clear_…`, which reads as another
  field's presence flag, is `key_has_…`, its key in the tag); a
  property the object does not require, or that admits `null`, is a
  presence pair (§ Codecs); a string enum whose values are all names is an
  enum; `integer`, `number`, `boolean` and `string` are `Int`, `Float`,
  `Bool` and `String`; an array with a `maxItems` of at most 1024 is a
  `bounded` list; a `$ref` inside the document is the type it names,
  declared once. What the codec cannot carry is a `std::json::Value`,
  with a comment saying why: a union (`oneOf`, `anyOf`, an `allOf` of
  several), an object with no properties or a map, an array with no
  bound, a list of lists, a whole list as a request or response, a
  recursive shape, a reference outside the document, a schema with no
  type.

The file is a function of the source: two imports of one document write
the same bytes, so it is committed with the program and re-imported to
change it, never edited by hand.

**`hale api drift [file.hl | dir] [--surface NAME]`** reads each imported
surface's source again, maps it as the import does, and compares the two
row by row by **shape**: a row's request, response and `wire:`, every
record they name written out as its fields' JSON keys and types (an
optional field marked as such) and every enum as its variants, so a record
renamed in the program is no drift. Each difference is one line on
stdout, in the order of the rows' names, in a form a watcher parses:

```text
drift Shelf added add_item
drift Shelf removed create_item
drift Shelf changed get_items_item_id response
```

`changed` names the slot that moved, `request`, `response` or `wire`, a
line each. A surface in step prints `Shelf: in step with ../shelf.json (4
rows)`. Exit 0 in step, 1 on drift, 2 when a source cannot be read or the
usage is wrong; a block with no `from` is not checked. `hale check --api`
runs the same check over every file the program loads, the seeds it
imports included, each seed's blocks against its own records and their
sources relative to their own files, after its document: the lines go to
stderr, and drift exits 1.

## Calling a surface

A program calls another system through a **client**: a param of the locus
that holds it, recognized by its literal as an exposure is (§ Serving), and
typed by the rows of the surface it calls.

```hale,fragment
locus Desk {
    params {
        creds: std::api::VaultPrincipals = std::api::VaultPrincipals { slots: "editor=shelf-editor, *=shelf-reader" };
        shelf: api::Client = api::client(Shelf, http::Client { base: "http://127.0.0.1:9000", principals: self.creds });
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

**The site.** `NAME: api::Client = api::client(SURFACE, TRANSPORT);` in
the params of its holder. The **surface** is one the program consumes
(§ Imported surfaces) or a Hale surface whose rows the program declares
and another program serves; a surface an imported seed declares is named
by its path under the alias (`lib::Svc`), as at a serve site. The
**transport** is a literal of one of two loci of `std::api`:

- `http::Client { base, principals }`: `base` is `http://host:port` (a
  path prefix allowed; `https://` as `std::http` speaks it). A row of a
  Hale surface is `POST <base>/call/<member>` under the row's member
  digest in `Hale-Surface-Digest`, as the generated clients call it (§ The
  clients). A row of an imported OpenAPI surface is its operation, `wire:
  "METHOD /path/{param}?query&names"`: each field of the request the path
  names (by its JSON key) fills it; for `GET`, `HEAD` and `DELETE` every
  other field is the query; for another method the request is the body
  when the wire names no parameter (neither in the path nor after `?`),
  and otherwise the field `body` is the body and every other field the
  query, the layout `hale api import` gives such a request; a `null`
  field, and an optional field left out, is no query parameter.
- `mcp::Client { url, principals }`: `url` is the MCP endpoint
  (`http://host:port/mcp`), spoken over the Streamable HTTP transport as
  `hale api import` speaks it. The client opens the session at its first
  call (`initialize` with the protocol version `2025-06-18`, a version the
  endpoint answers outside 2025-06-18, 2025-03-26 and 2024-11-05 losing
  the call, then `notifications/initialized`) and names the session id and
  the version on every call after it; a JSON answer and an event-stream
  answer are both read. Each row is a `tools/call`: a Hale row's tool is
  its member's (`Orders__place`, § The MCP transport) with its member
  digest in `params._meta["hale/digest"]` and the header; an imported
  row's is its `wire:`. An endpoint that ends the session (`404`) loses
  that call, and the next opens another.

A surface imported from MCP is not called over `http::Client`, nor one
imported from OpenAPI over `mcp::Client`, nor, over `mcp::Client`, a Hale
surface with a row whose request is no JSON object (an enum): an MCP tool's
input is an object, and `mcp::Rpc` serves such a row as no tool. No call is retried, a row that
streams is not called, and the client holds no connection past a call.

**Principals.** `principals:` is who the client is to the system it calls,
as an exposure's `principals:` is who calls it: a locus implementing
`std::api::ClientPrincipals`, `fn bearer(role: String) -> String`, the
bearer presented as `Authorization: Bearer` for a row that requires
`role`. A row's bearer is the first of its `requires` the principals hold
one for, else the one for no role (`bearer("")`); "" presents none, and a
client that names no principals presents none. The standard library's
is `std::api::VaultPrincipals { slots }`: `ROLE=SLOT` pairs, comma
separated, each the vault slot (`std::secret`'s `vault:` name) holding the
bearer for that role, `*=SLOT` the slot for a row that requires none and
for a role with no slot of its own. A slot is read at each call, as a
vault credential is, and one that holds nothing presents no bearer.

**What the compiler writes** (`client_expand`, before the check): a
locus per surface the program calls, `__ApiClient_<Surface>`, which the
param is built as, holding the transport. It has a method per row:

| | an imported row | a Hale row |
|---|---|---|
| name | the row's (`create_item`) | its member in snake case (`Orders::place` is `orders_place`) |
| parameter | the request, `request: T`; none for `()` | the request; none for a handler that takes none |
| answer | `<Surface><Row>Outcome`, the row in camel case (`ShelfCreateItemOutcome`) | the same (`ShopOrdersPlaceOutcome`) |
| `HandlerError` | `HandlerError(std::api::Failure)` | `HandlerError(E)`, the handler's own error type; none for a handler that declares none or `ClosureViolation` |

The answer is § Outcomes as one enum: `Result(T)` (or `Result` for a row
with no response), the `HandlerError` above, `Refusal(std::api::Refusal)`
(`kind`, `reason`, `requires` joined by `,`, `served`), `ServerError`,
and `Lost(String)`, a call that got no outcome (the endpoint could not be
reached, the answer was no outcome, or a value did not decode, which names
the codec's kind and field). A whole request or response is a record or
an enum, carried by its api codec (§ Codecs), or, for an imported row, a
`std::json::Value`; a row that carries anything else is refused, as are
two rows that would be one method and a row whose method would be a
lifecycle name (`run`, `birth`, …).

**Outcomes read back as the served side maps them.** A Hale row's
answer is read by § Outcomes' table: over HTTP a `200` is the result, a
response naming `Hale-Error-Class` or a `422` the handler error, decoded as
`E` (so a caller matches the variants of the handler's own enum, `@not_found
Missing` answered `404` read back as `Missing`), a `{"refusal": …}` body
the refusal, `{"refusal": {"kind": "server"}}` the server error; over MCP
a result is the response (its `structuredContent`, else its text), `-32001`
the handler error (its `data`), `-32603` the server error, and any other
error the refusal its `data` and code name. An imported OpenAPI row's
`2xx` is the result; `400`, `401` and `429` are the refusals `malformed`,
`unauthenticated` and `full`; a status an error class answers with (`404`,
`409`, `422`, `403`, `503`, § Error classes) is the handler error, a
`std::api::Failure` whose `class` is that class's word (`not_found`), and
any other `4xx` the handler error with no class; any other `5xx` is the
server error. An imported MCP tool's result is the result, and a result
with `isError` the handler error, the tool's text as the failure's body.

**A call is an effect.** Each method carries the class `client(<Surface>)`
(`@effects(is: { client(Shelf) })`), an instantiation of an effect family
the compiler declares over the surfaces the program calls, so the claims
name the calls with nothing of their own: `forbid reaches(desks,
effects(client(Shelf)))` is violated by a path from a desk to a call of
`Shelf`, its witness ending at the method (`Desk::look ->
__ApiClient_Shelf::get_item`), and `effects(client(*))` names every
surface's calls. The family is the existing indexed one (`effect
NAME(domain)`, docs/src/claims.md), its domain the surface names, so a
claim, an `@effects(none: { client(Shelf) })` and a `@budget` read it as
any declared class.

**Placement.** A client is a param like any other. It runs on its holder's
pool unless a `placement` row names it, and its methods are called from
its own pool (F.31: a call from another pool is refused); a holder placed
`where async_io` parks each wait of a call there, as `std::http` does.

**The site's law.** A site the compiler cannot build is refused at the
call, naming why ("client of \`Shelff\`: no surface \`Shelff\` is
declared", "client of \`Shelf\` over \`mcp::Client\`: row \`get_item\` is the
HTTP operation \`GET /items/{itemId}\`; a surface imported from OpenAPI is
called over \`http::Client\`", "the transport is a literal of
\`http::Client { base, principals }\` or \`mcp::Client { url, principals }\`",
"\`api::client(…)\` builds an \`api::Client\`; declare \`NAME:
api::Client\`"). A client is a param's literal and nothing else: an
`api::client(…)` anywhere else (a `let`, an argument, a body) is refused
("\`api::client(…)\` builds a client, which is a param of the locus that
calls it: …").

**A leg's tools.** **`hale api export --surface NAME --tools-for ROLES
[target]`** prints the tools a caller acting as `ROLES` (comma separated,
`''` for none) is given of the surface: one per row those roles admit (a
row that requires none, or whose every required role one of them holds, a
role holding each role it includes), named `<Surface>.<row>`, the row as
the client names its method (`Shelf.create_item`, `Shop.orders_place`),
each with the MCP form's input schema, `x-hale-member` and
`x-hale-requires`, under `x-hale-surface`, `x-hale-digest` and
`x-hale-roles`. The namespace keeps two surfaces' rows apart in one tool
list; it writes no bundle, so it is given without `--out` and `--check`.

## Driving a served program

`hale api describe` and `hale api call` are the operator's verbs: they drive any
served program from the description it serves, with nothing generated and
nothing in the program (`crates/hale-cli/src/api_drive.rs`). They speak the
wire of § The `Rpc` interface and § Outcomes and nothing else, and they are
held to `tests/api-contract/program.hl` served over both transports
(`crates/hale-cli/tests/api_drive.rs`). They are the single-purpose, scriptable
form of the generic clients of § The clients: an exit code per outcome, a
payload typed by the member's schema, one request id of their own.
`hale describe` and `hale call` are their short forms (a bare path is `unix:<path>`, a bare payload after the member is `--json`): the same output, exit codes and flags.

```text
hale api describe <endpoint> [--json] [--bearer T]
hale api call <endpoint> <member> [--json '<payload>' | --json - | --<field> <value> …]
              [--bearer T] [--request-id ID] [--digest D] [--raw]
```

**Endpoints.** `unix:<path>` is a `unix::Rpc` socket, spoken as § The Unix
transport states (`{"describe": true}`, `{"call": …}` lines; the caller is the
peer's kernel credentials, so there is no bearer). `http://host:port` is an
`http::Rpc` listener (`GET /.description`, `POST /call/<member>` with the
digest in `Hale-Surface-Digest`); the caller is the bearer, `--bearer T`,
else the environment's `HALE_API_BEARER`, and without either the request names
nobody and is refused `unauthenticated`. A `grpc://` or `mcp://` endpoint, and
anything that is neither form, is refused before any connection with one
message that names the two forms the verbs drive today (`unix:<path>` and
`http://host:port`): those transports have clients of their own protocol.

**`describe`** prints the description the endpoint answers for the caller, and
nothing it does not (it is the per-caller document of § The description,
filtered by the roles that caller holds, as the transport answers it). `--json`
prints the document as the program wrote it, one line, byte for byte. The
default is a readable rendering: the exposure's identity line
(`<surface>@<digest>/<name>` and the listener), the caller and its roles, then
one row per member the caller may see:

```text
Public@fnv1a64:a8930d6e7998e986/public  [http 127.0.0.1:8080]
caller: bearer alice; roles: trader
  Orders::cancel(order: Int (OrderId)) -> Cancelled, error OrderError  requires: trader
  Orders::place(symbol: String, qty: Int, limit: Int (Money, q(cent))) -> OrderReceipt, error ClosureViolation  requires: -
```

A payload field is `name: Type`, with `?` after the name when the schema does
not require it; a type is the language's word (`Int`, `Float`, `Bool`,
`String`, a record by its name, a list `[T; at most N]` or, fixed, `[T; N]`),
and a field of a distinct or quantity type adds the type's name and a
quantity's unit in parentheses.

**`call`** sends one member. What it reads from the description, and when: the
description is fetched once per call, before the payload is built, and it is
used for two things only: the member's payload schema, which types the flags,
and the surface's `digest`, which the call names (in the line's `digest`, in
`Hale-Surface-Digest`) so a program that changed under the verb is refused
`digest_mismatch` and not run as another contract; `--digest D` names another.
A description that cannot be fetched is a transport failure (exit 4). A
description that is *refused* (HTTP 401) is the call's outcome when the payload
needs it (typed flags: exit 2, the refusal as the wire says it); a payload given
as `--json` goes on without it and the server's own answer to the call is the
outcome, since the server authorizes every request whatever the description
listed.

The payload is either the JSON given (`--json '<payload>'`; `--json -` reads
all of stdin) or built from typed flags, one per payload field, typed by the
field's schema:

| schema | flag value |
|---|---|
| `integer` (`Int`, a distinct integer, a quantity by its count) | an integer: `--qty 3`, `--limit 125` |
| `number` (`Float`) | a number: `--price 2.5` |
| `boolean` (`Bool`) | `true` or `false`; the bare flag `--rush` is `true` |
| `string` | the text: `--name alice` |
| a record, a list | JSON checked against the schema, nested records and lists included (a record: every required field, no unknown field, each field of its type; a list: every item of the item type, and no more items than its `maxItems` nor fewer than its `minItems`; an optional field may be absent): `--items '[1,2]'`, `--ship '{"city": "Oslo"}'` |

`--field=value` is `--field value`; a field's `_` may be written `-`. A field
not required and not given is not sent. The typed form is refused before
anything is sent (exit 5, the field and its schema in the message) when a
required field is missing, a flag names no field of the member's payload, a flag
is given twice or a value does not parse as its field's type (a record or list value is checked recursively; the message names the path, `ship.city` or `items[0]`); so is `--json`
together with typed flags, a `--json` that is not JSON and a typed-flag call of
a member the description does not list (the message lists the members it does).
The names `json`, `bearer`, `request-id`, `digest` and `raw` are the verb's
own: a payload field of one of those names (`request_id` included, since a
field's `_` may be written `-`) is given with `--json`. `id` is not among them:
`--id 5` sets a payload field `id`. A `--json` payload
is not checked against the schema: the server decides, and its refusal
(`wrong_type`, `missing_field`) is the answer. The typed form and `--json` of
the same payload produce the same wire line (the payload serialized from the
parsed value, so key order and whitespace of the input do not reach the wire).

The request id is the verb's: `hale-<pid>` unless `--request-id ID` gives one, sent as a
JSON string. Over the Unix socket the reply's `id` must be it: a reply that
answers another request is a transport failure, as is a connection that ends
with no reply. (HTTP carries no id; the connection is the correlation.)

**Outcomes and exit codes.** Each outcome of § Outcomes is one exit code. A
result prints its value as JSON on stdout; every other outcome prints the wire
outcome as the transport wrote it (the whole reply line over the socket, the
body over HTTP) on stderr. `--raw` prints the whole wire outcome on stdout
whatever its kind, and nothing on stderr; the exit code does not change.

| outcome | stdout / stderr | exit |
|---|---|---|
| result | the value on stdout | 0 |
| handler error | the wire outcome on stderr | 1 |
| refusal (any kind) | the wire outcome on stderr: the kind and the reason as the wire says them, `served` on `digest_mismatch`, `requires` on `unauthorized` | 2 |
| server error | `{"refusal": {"kind": "server"}}` on stderr | 3 |
| transport failure (nothing answered, the connection lost, a reply that is no outcome of the wire or another request's) | a message on stderr | 4 |
| usage (an unknown flag, a refused endpoint form, a payload refused before it was sent) | a message on stderr | 5 |

`describe` exits 0 when the endpoint answered a description, 2 when the
exposure refused to describe (the wire refusal on stderr), 4 when nothing
answered (a refused connection, no listener, a connection that closed) or what
did answer is not a Hale exposure (the message says which), and 5 for usage.

## What this replaced

Before R4 a program's API was a structural fact: one entry, `bindings {
api: unix(…) }` on the main locus, put on the surface every topic the
seed's loci (and the loci its `serve:` clause named) subscribed or
published and every `expose` member of main and its default children,
gated by `@gated` on handlers and members; the entry was desugared before
the check into envelope types, topics, synthesized subscriptions and two
loci that owned the socket, and an admission law read locus rows
(GH #1106). That path is retired in one cutover (R4), with no adapter, no
dual route and no flag: the parser refuses `bindings { api: … }` and
`@gated` with one diagnostic that names the replacement (an `api` block
or `@rpc` with `requires`, `api::serve(…)`, a topic binding to a hub), the
synthesis, the admission law over locus rows, the exposed reads, the
`--api <path>` build flag and `hale check --dump-api` are gone, and the
programs that declared the entry serve surfaces. A read is an `rpc` row
whose handler returns the value (it carries no `as_of` digest); a stream
is a topic bound to a hub; a call through a surface is a call of the
row's handler, not a publish on a topic, so the topic's other
subscribers never hear it; a caller lacking a role is refused
`unauthorized` and told what the row requires. What carries over is
restated above in these terms: the Unix wire's line framing,
correlation and principal (§ Outcomes), the codec (§ Codecs), the
publish contract's bearing on streams (§ Streams), `std::api::Context`
and the two source interfaces (§ Serving). `hale call`, `watch`, `admin`
and `mcp` read per-exposure descriptions (§ The clients).

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
runs the same assertions over a Unix socket, R3 over HTTP, R6 holds the
`Public` half over MCP (the recorded calls as `tools/call`, `tools/list`
against `Public.mcp.json`), R7 over gRPC (the recorded calls as unary calls, the bodies in messages and details, § gRPC), R5 adds the stream
half over WebSocket and datagrams (`crates/hale-codegen/tests/
api_hub_streams.rs`, `api_hub_desk.rs` and `api_hub_udp.rs`: the witness's
hub half, its `dave` and `bob`, a revoked grant, an expired credential,
shedding, `stop()`, and the served descriptions held to the fixtures as
values).

## Open points

- **Binding an imported surface** to a client (`api::client(Surface,
  mcp::Client | http::Client)`), which reads the signature rows' `wire:`;
  until then an imported surface is declared, checked and described, and
  called by nothing the compiler generates.
- **The import's reach**: a YAML document, Swagger 2.0, a header or cookie
  parameter, an operation's error responses, authentication beyond what a
  public document needs, and a `$ref` to another document are not read.

- **`grpc://` endpoints for `hale describe` and `hale call`**: the
  clients are Rust and have no HTTP/2 stack (the runtime's nghttp2 is
  linked into compiled programs, the test suite's client is hand-written
  frames); a client would need HTTP/2 framing, HPACK, flow control and the
  gRPC message framing, and the `Describe` method would be its discovery.
  Until then a gRPC exposure's description is the program's
  (`hale check --api`) and the stdlib's `hale.api.Description/Describe`,
  and its schema, for `grpcurl` or a generated stub, is the `.proto` and server
  reflection.
- **`grpc.reflection.v1alpha`**, the older name of the reflection service older
  tools ask for first: v1 is served, the messages are wire-identical, and only
  the package is another; a tool that speaks nothing newer is refused
  `malformed` (`no such service`) until it is added.
- **Reflection is for the caller the bearer names, and the descriptor is the
  surface's**: it is not filtered by the roles a caller holds, as a description
  is. A surface whose member names are themselves confidential to a role needs
  a filtered descriptor, which the `.proto` the digest names cannot be.
- **A receiver binding naming a `@form` collection's element** (a
  surface over many instances of one type, keyed by the request), or
  only a single instance.
- **A hub that also serves a surface** (rpcs and streams on one
  connection): the rpcs are carried (§ Rpcs on a hub), but the hub's
  live description lists its streams only; the full document of a surface
  and a hub together is the inventory's.
- **A dispatch window wider than one call per receiver**: the exposure
  hands a receiver one call at a time (§ The runtime boundaries,
  dispatch) so that what it refuses is exactly what has not been handed
  over; a wider window needs a runtime primitive to withdraw a queued
  cell (R2b).
- **MCP resources over streams**, and MCP over stdio: a subscription
  needs an event stream, a hub and a session identity, and stdio needs a
  runtime primitive to read the process's own standard input (§ The MCP
  transport).
- **The wrapped MCP input over a description**: an rpc whose request is
  not an object has no tool in `hale mcp --app` (as over `mcp::Rpc`),
  because the tool's wrapping property is the handler's parameter name,
  which neither a description nor a row carries; carrying it would add a
  field to every description, which the R0 documents a consumer built
  against do not have.
- **The roles manifest**: `[environments.<env>.roles]` and the `--matrix`
  coverage check have no consumer at run time since the compiler stopped
  baking the table (`StaticRoles` takes a `table` param or
  `LOTUS_API_ROLES`); `hale replay --env` is no consumer either, since
  the table no longer reaches the binary's identity; either the manifest is retired or it becomes the
  default of a `StaticRoles` table.
- **A description over `hale call` for a hub that also serves a
  surface**: the hub's live document lists streams only (above), so `hale
  call ws://…` does not check the member against it.
- **The wire framing of rpcs over UDP** (a correlation id field, the
  largest datagram; R6). Over WebSocket they are framed (§ Streams).
- **Leases for a datagram subscriber** that vanishes without
  unsubscribing, and a refusal for a second program binding a UDP address.
