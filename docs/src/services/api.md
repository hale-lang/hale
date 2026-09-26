# Drive it from outside

A service that is authoritative over something ends up wanting a
surface that tools can plug into: a command line, a dashboard, an
MCP host. You could write an HTTP server, a JSON codec per message
and a routing table for it. You do not have to. Everything the
program already declares on its bus *is* its API, and one line at
the deployment tier hands it out.

## One entry, no other change

Take a billing service as it is: a locus that subscribes a topic,
publishes another, and exposes a field.

```hale
type Verdict { review_id: Int; verdict: String; }
type VerdictResult { ok: Bool; note: String; }
type PriceMoved { sym: String; price: Float; }
type Ledger { balance: Int; entries: Int; }

topic Verdicts { payload: Verdict; subject: "app.verdict"; }
topic Prices { payload: PriceMoved; subject: "app.price"; }

locus Billing {
    contract { expose ledger: Ledger; }
    params { ledger: Ledger = Ledger { balance: 100, entries: 0 }; }
    bus {
        subscribe Verdicts as on_verdict;
        publish Prices;
    }
    fn on_verdict(v: Verdict) -> VerdictResult {
        self.ledger.entries = self.ledger.entries + 1;
        return VerdictResult { ok: true, note: "ratified " + to_string(v.review_id) };
    }
    run() {
        while !self.draining {
            std::time::sleep(1s);
            Prices <- PriceMoved { sym: "ABC", price: 1.5 };
        }
    }
}

main locus App {
    params { billing: Billing = Billing { }; }
    placement { billing: cooperative(pool = work) where async_io; }
    bindings {
        api: unix("/run/app.sock", bound: 64, on_full: refuse);
    }
}

fn main() {
    App { };
}
```

The `api:` entry is the whole change. It binds every topic a locus
of this seed subscribes as a **command** (`Verdicts`), every topic
such a locus publishes as a **stream** (`Prices`), and every `expose`
of the main locus or of its default children as a **read**
(`billing.ledger`). The handler's return type became the reply:
`on_verdict` returns a `VerdictResult`, so a caller gets one back.
Nothing else in the source knows the socket exists, and a program
without the entry pays nothing for it. What a library you import
does on its own bus is not your API: only the loci of your own seed
are served, so a head that imports a large core never hands out the
core's internal topics as commands (an imported *topic* your locus
subscribes is served under its qualified name, `lib::Orders`).

For a program you are only trying out, skip even that line:

```sh
hale run --api /run/app.sock app.hl
```

puts the same entry on the main locus with the dev defaults. It
needs a `main locus` to put it on; a bare `fn main` program is
refused with the rule. The path may be a param the program computed
(`api: unix(self.socket, …)` with `App { socket: … }` in `main`), so a
service can listen at one socket per record under `XDG_RUNTIME_DIR`;
`LOTUS_API` overrides whatever the entry says. A socket a live
process already holds is never stolen, and a binding that cannot
listen leaves the rest of the program serving, saying why.

## Talking to it

The socket speaks one JSON object per line. A request is a `call`,
a `read` or a `watch`, with an `id` you choose so you can match the
answer:

```text
{"id": 1, "call": "Verdicts", "payload": {"review_id": 7, "verdict": "ratify"}}
{"id": 2, "read": "billing.ledger"}
{"id": 3, "watch": "Prices"}
```

and each answer carries your `id` plus the binding's own
`request_id`:

```text
{"request_id": 1, "id": 1, "ok": true, "value": {"ok": true, "note": "ratified 7"}}
{"request_id": 2, "id": 2, "ok": true, "value": {"balance": 100, "entries": 1}, "as_of": "sha256:8e5d…"}
{"request_id": 3, "id": 3, "ok": true, "attached": "Prices"}
{"stream": "Prices", "value": {"sym": "ABC", "price": 1.5}}
```

A command whose handler has no return type is answered `{"ok":
true, "accepted": true}` the moment it is dispatched: that is what
"accepted" means for this binding, and it is the same promise
`Verdicts <- v` makes in-process. From a shell, `socat` is enough
to try it:

```sh
printf '%s\n' '{"id":1,"call":"Verdicts","payload":{"review_id":7,"verdict":"ratify"}}' \
    | socat - UNIX-CONNECT:/run/app.sock
```

## The clients

You never write a client for a Hale program, because the binding
describes itself. `{"describe": true}` on the socket, or `hale
describe app.hl` on the source, returns the same document: every
command with its payload schema and reply type, every read, every
stream, and two notes that say what a gate and a read are and are
not. Four verbs read only that document:

```sh
hale describe /run/app.sock                # the description; --openapi, --mcp for the derived forms
hale call /run/app.sock Verdicts '{"review_id": 7, "verdict": "ratify"}'
hale call /run/app.sock billing.ledger     # a read, with its as_of
hale watch /run/app.sock Prices            # frames, one JSON line each
hale admin /run/app.sock                   # a page on 127.0.0.1:7473 over the description
claude mcp add app -- hale mcp --app /run/app.sock   # every command a tool, every read a resource
```

`hale call` prints the answer and exits 0; a refusal goes to stderr
with its kind and the whole receipt, and exits 1, so a script can
branch on it. `--receipt` prints the whole receipt on stdout
instead of the value alone: the request id, the echoed id, and what
later pieces add to it (the caller, the role that authorized it). `hale
describe app.hl --openapi` writes an OpenAPI 3.1 document with a
path per command, read and stream and every schema under
`components`; `--mcp` writes the tool and resource shapes an MCP
host lists. Both are derived from the description and pinned by a
conformance fixture in the compiler's tests, so a generated document
never drifts from what the binding serves. The description carries
no socket path or deployment detail: it says what the program is,
and where one copy listens is the deployment's business.

## When it says no

A refusal is an answer, never a failure of the program:

```text
{"request_id": 4, "id": 4, "ok": false, "refusal": {"kind": "malformed", "reason": "wrong_type: review_id"}}
```

The kinds are `malformed` (not a JSON object, no verb, or a payload
that does not decode; the reason names the field), `unknown` (no
such topic or read), `not_a_command` (you called a stream),
`not_a_stream` (you watched a command), `over_bound`,
`unauthenticated` (the kernel would not say who you are; nothing is
served to such a peer), and `unauthorized` (you asked for the full
description without `owner`). A gated item you may not use answers
`unknown`, like a name that does not exist: see below. A payload
is decoded before dispatch, strictly: a string where an `Int` is
declared is `wrong_type`, a missing field without a default is
`missing_field`, and the handler only ever sees a value of its
declared type.

## The two knobs

An outside caller is unbounded by nature, so the entry carries a
bound and a policy and the checker insists on both. `bound: 64`
means at most sixty-four commands and reads waiting on a handler
at once; the sixty-fifth caller gets `over_bound` and the program
never sees it. `refuse` is the only policy for requests, because a
caller waiting for an answer cannot be shed silently. Watchers have
their own pair, per connection: `watch_bound: 256, on_watch_full:
drop_old` keeps the newest frames for a slow reader and reports how
many it dropped on the next frame it does get (`{"stream": "Prices",
"dropped": 3}`). Leave both out and a watcher gets `bound` frames
with `drop_old`.

## Reads are snapshots

A read never reaches across threads into a locus's field. It is
answered on the locus's own pool, as a copy taken there, and the
answer says when: `as_of` is a digest of what was answered, so two
reads that agree on it saw the same state and a later command can
say "only if it is still this". A live view is what a stream is
for; the two verbs are different on purpose.

## Who is calling

The binding knows, and a handler can ask. Every answer carries the
principal the binding established for the connection: on the Unix
socket that is the peer's credentials, as the kernel vouches for
them.

```text
{"request_id": 7, "id": 1, "ok": true, "value": {...},
 "caller": {"mode": "unix", "name": "uid:1000", "uid": 1000, "gid": 1000, "pid": 4242}}
```

A handler that wants the caller declares it, and nothing on the
`subscribe` line changes:

```hale
type RefundResult { ok: Bool; by: String; }

locus Billing {
    bus { subscribe Refunds as on_refund; }
    fn on_refund(r: Refund, ctx: std::api::Context) -> RefundResult {
        // ctx.caller is who; ctx.via says "api" through the binding
        // and "local" for a publish inside the program.
        return RefundResult { ok: true, by: ctx.caller.name };
    }
}
```

The second parameter is `std::api::Context`: the caller, the
request id, `via` (the binding's name, `local`, or the mark a
forwarding transport of the program's own set — see below), and the role
that authorized the message (empty when the operation is not gated). A message that did not
come through the binding hands the handler the local principal, so a
handler never asks whether it was reached from outside; it reads
`via`. `local` says where a message did not come from, never that
it is trusted: a topic bound to another transport in `bindings { }`
cannot take a context handler at all. Both `Context` and `Principal` are ordinary structs: build
one in a test, forward one in a payload. A bearer token for HTTP
callers is the third mode and arrives with the HTTP transport.

## Who may call

The requirement that an operation needs a role is part of the
program, true wherever it runs; who holds the role here is a
deployment fact. So the requirement is written once, on the
operation, and the mapping lives beside the socket path.

```hale
role refund_support;
role auditor;
role owner includes refund_support;      // whoever is owner may do what support may

locus Billing {
    contract {
        @gated(role: auditor) expose ledger: Ledger;     // a gated read
    }
    bus {
        subscribe Refunds as on_refund;
        @gated(role: refund_support) publish Moved;      // a gated stream
    }
    @gated(role: refund_support)
    fn on_refund(r: Refund, ctx: std::api::Context) -> RefundResult {
        // ctx.role is the role that authorized this call: "refund_support",
        // or "owner" for an owner, so the handler can write its own audit row.
        return RefundResult { ok: true, by: ctx.caller.name };
    }
}
```

A `role` is declared vocabulary, like `group`: a name nothing
declares is an error, and so is `@gated` on anything but a
subscribed handler, an `expose` or a `publish`, because nothing else
is reached from the binding. `owner` is built in. What `@gated`
means is exactly one thing: a call, a read or a watch **arriving
through the socket** is refused unless the caller holds the role.
It is a gate at the boundary, not a proof about the program's
insides; a handler that calls `refund` from some other path is not
stopped by it, and the description says so in its `notes.gates` so
no client presents a gate as more than it is.

Who holds a role is written in `hale.toml`, per environment:

```toml
[environments.prod.roles]
refund_support = ["group:support-leads"]
auditor        = ["user:audit", "uid:1007"]
owner          = ["user:riley"]
```

`hale build --env prod` (or `hale run --env prod`) bakes that table
into the binding; the members are matched against the peer's
credentials (`uid:`; `gid:` against the primary group and the
supplementary groups the kernel reports for the connection; `user:`
and `group:` resolved once at start per the account database; `*`
for any authenticated peer). `LOTUS_API_ROLES="refund_support=uid:1000;owner=user:riley"`
overrides it at run time, which is how a test drives it. A table
naming a role the program does not declare, or a member outside
those spellings, is refused at start with the reason, the same rule
`hale check --matrix` holds `hale.toml` to; with no table at all
every gate refuses, and the build tells you. The matrix also insists
that every declared role is mapped in every environment, `[]`
meaning explicitly nobody.

An app can hand the binding its own source instead: a locus with
`fn holds(p: std::api::Principal, r: String) -> Bool`, named on the
entry as an expression the main locus evaluates, so it can be built
with the program's own state and kept as a handle:

```hale
main locus Head {
    params { root: String = "."; roles: RecordRoles = RecordRoles { }; }
    bindings { api: unix("/run/head.sock", bound: 64, on_full: refuse, roles: self.roles); }
    birth() { self.roles.root = self.root; }
}
```

That is how a program whose positions are roles answers from its own
record.

An item you may not use is not shown to you and, if you name it
anyway, is `unknown`, exactly as a name that does not exist would
be: existence is not disclosed to a principal that cannot act on it.
The one refusal that names a role is the full description's:

```text
{"request_id": 9, "id": 3, "ok": false,
 "refusal": {"kind": "unauthorized", "reason": "needs role owner", "role": "owner"},
 "caller": {"mode": "unix", "name": "uid:1000", ...}}
```

An answer names what authorized it: `"role": "owner"` on the receipt,
the same value in `ctx.role`. A stream follows the gate of the topic's
handlers unless its `publish` states its own. `on_unauthorized: drop`
on the entry turns a refusal into silence, for a socket that should
not even answer.

A peer the kernel cannot vouch for (`uid` -1) is refused everything,
gated or not: the binding's whole claim is that it knows who is
calling.

The description follows the same rule. `{"describe": true}` returns
the caller's slice: the commands, reads and streams it may use, and
only the schemas those need. `hale mcp --app` therefore lists exactly
the tools a principal may call, and `hale admin` shows what it may
reach. The whole document is itself a read, gated on `owner`
(`{"describe": "full"}`, `hale describe --full`); an owner's admin
page shows the rest greyed out with the role each item needs.

## What is left out, and why

- A topic whose payload has a field with no JSON form yet
  (`Decimal`, `Time`, `Duration`, `Bytes`, an array, an enum, a
  locus) stays off the API, with a warning at the entry naming the
  field. Adding the entry never breaks a build.
- A topic two handlers both answer is an error at the entry: one
  reply per command.
- An item another seed declared is described qualified (`api::Claim`);
  a caller may write the bare tail (`Claim`) when exactly one item
  bears it, and gets `unknown` otherwise.
- A `Drain<T>` batch handler is not reached through the binding
  yet; bulk requests wait on batch delivery over the cooperative
  queue.
- A bearer token for HTTP callers waits for the HTTP transport
  (GH #1135); the Unix socket's peer credentials are the one identity
  today. Until then a program may forward: a request line carrying
  `"via": "<mark>"` is honoured only from a peer whose uid is the
  program's own (refused as `malformed` from anyone else), the mark
  rides on the receipt's `caller` and in `ctx.via`, and the principal
  is the forwarding process's — an HTTP handler that hands a browser's
  line to its own socket is gated and answered exactly like any peer.
- Transitive privilege inference (flagging `api -> OrderPlaced ->
  on_order -> refund` as an escalation) is a later, opt-in claim;
  `@gated` is a boundary check and says so.
