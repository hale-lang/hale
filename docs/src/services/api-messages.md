# Designing an API's messages

[Build an API](./build-an-api.md) grows a program from an empty file to
a served surface with roles. This page is about the part a caller
actually touches: the records a surface carries. It builds one small
status service whose messages use each shape the codec offers (a
default, an optional field, a map, an enum and a typed error with
error classes), serves
it over HTTP, adds a member without breaking anyone, and streams the
same records as Server-Sent Events. Every output below is what the
commands printed.

## The messages

A status service answers "how are my dependencies?" and lets an
operator restart one:

```hale,fragment
role operator;

type Status = enum { Up, Degraded(String), Down };
type Check { name: String; status: Status; }

type HealthQuery {
    verbose: Bool = false;
    since: Time = `1970-01-01T00:00:00Z`;
    has_since: Bool = false;
}

type Health {
    checks: bounded[Check; 16] indexed_by name;
    note: String = "";
    has_note: Bool = false;
}

type Restart { name: String; }
type Problem = enum { @not_found NotFound(String), @unavailable Busy };

api Ops requires: [operator] {
    rpc Probe::health requires: [];
    rpc Probe::restart;
}
```

Each field's shape is a decision about what a caller can say:

- **`verbose: Bool = false` is a default.** A caller that leaves it out
  has said `false`, and nothing needs to tell "not sent" from "sent
  `false`". Most optional inputs are this.
- **`since` with `has_since` is a presence pair.** Here "not sent" means
  something different from any time: give me everything. The caller may
  leave `since` out or send `null`, and the handler reads `has_since`.
  Both fields have defaults, so a literal that has nothing to say names
  neither. The flag never crosses the wire.
- **`checks` is a map.** `indexed_by name` makes the list an object keyed
  by each check's `name` on the wire, which is how a caller wants to
  look one up. In the program it is still a bounded list, in order.
- **`Status` is an enum.** It crosses as `"Up"` or `{"Degraded":
  "cold"}`, and the description lists the variants, so a client knows
  every state there is. A `String` status would let any text through.
- **`Problem` is the error.** `restart` fails with `NotFound(name)` or
  `Busy`, and a caller branches on which one, instead of parsing a
  message. Each variant also declares its **error class**, which tells
  every transport what kind of failure it is: `@not_found` is a `404`
  over HTTP and `NOT_FOUND` over gRPC, `@unavailable` a `503` and
  `UNAVAILABLE`. The others are `@conflict` (409, `ABORTED`), `@invalid`
  (422, `INVALID_ARGUMENT`) and `@forbidden` (403, `PERMISSION_DENIED`).
  A variant with no class is the handler error's plain `422` and
  `FAILED_PRECONDITION`.
- **`requires: [operator]` on the surface** is every row's default.
  `health` overrides it with `requires: []`, so anyone the transport
  authenticates may ask. It is the same table, with the same digest, as
  writing the roles on each row.

The handlers are ordinary methods over those types. They never see
JSON:

```hale,fragment
/// The health of every dependency (a stand-in for real probes).
fn snapshot(n: Int) -> Health {
    let mut h = Health { };
    push(h.checks, Check { name: "db", status: Status::Up }) or discard;
    if n % 2 == 0 {
        push(h.checks, Check { name: "cache", status: Status::Degraded("cold") }) or discard;
    } else {
        push(h.checks, Check { name: "cache", status: Status::Up }) or discard;
    }
    return h;
}

locus Probe {
    params { restarts: Int = 0; }
    fn health(q: HealthQuery) -> Health {
        let mut h = snapshot(self.restarts);
        if q.has_since {
            h.note = "changes since " + std::time::iso8601(q.since);
            h.has_note = true;
        }
        return h;
    }
    fn restart(r: Restart) -> Health fallible(Problem) {
        if r.name != "db" && r.name != "cache" { fail Problem::NotFound(r.name); }
        self.restarts = self.restarts + 1;
        return snapshot(self.restarts);
    }
}
```

## What a caller is told

`hale check --api` prints, without running anything, what a client is
generated from. The JSON Schema export shows each decision above as the
caller will see it (trimmed):

```sh
hale check --api status.hl --surface Ops --json-schema
```

```text
"checks": {
  "type": "object",
  "additionalProperties": { "type": "object", "properties": { "status": { "oneOf": [ … ] } }, … },
  "maxProperties": 16,
  "x-hale-key": "name"
},
"note": { "anyOf": [ { "type": "string" }, { "type": "null" } ] }
…
"error": {
  "oneOf": [
    { "type": "string", "enum": [ "Busy" ] },
    { "type": "object", "properties": { "NotFound": { "type": "string" } }, "required": [ "NotFound" ], "additionalProperties": false }
  ],
  "x-hale-type": "Problem",
  "x-hale-error-classes": { "NotFound": "not_found", "Busy": "unavailable" }
}
```

The map is an object with at most 16 entries keyed by `name`; `note`
may be a string or `null` and is not required; the error is one of the
enum's variants, with the class each declares.

## Serving it

The surface goes on an HTTP listener, with a bearer token naming the
caller and a role table giving that caller `operator`:

```hale,fragment
locus Tokens {
    fn principal(token: String) -> std::api::Principal {
        if token == "t-ops" { return std::api::Principal { mode: "bearer", name: "ops" }; }
        return std::api::Principal { mode: "bearer", name: "" };
    }
    fn refused() -> String { return "no such token"; }
}

main locus App {
    params {
        probe: Probe = Probe { };
        tokens: Tokens = Tokens { };
        staff: std::api::StaticRoles = std::api::StaticRoles { known: "operator", table: "operator=bearer:ops" };
        web: api::Exposure = api::serve(Ops, http::Rpc { bind: "127.0.0.1:8794", codec: json, principals: self.tokens, roles: self.staff }, as: "web", receivers: { Probe: self.probe }, bound: 64, on_full: refuse);
    }
    run() {
        while !self.draining { std::time::sleep(100ms); }
        self.web.stop();
    }
}

fn main() {
    App { };
}
```

`hale describe` reads the served description and prints the shape of
each request, the optional field marked as one:

```text
$ hale describe http://127.0.0.1:8794 --bearer t-ops
Ops@fnv1a64:95d08b7e7dd9f4de/web  [http 127.0.0.1:8794]
caller: bearer ops; roles: operator
  Probe::health(since?: String (Time) or null, verbose?: Bool) -> Health  requires: -
  Probe::restart(name: String) -> Health, error Problem  requires: operator
```

A call is a `POST /call/<member>` with the request as its body. Each of
these went to `Probe::health` with `Authorization: Bearer t-ops`:

```text
{}                                        200 {"checks":{"db":{"status":"Up"},"cache":{"status":{"Degraded":"cold"}}}}
{"since": "2026-10-18T09:00:00Z"}         200 {"checks":{…},"note":"changes since 2026-10-18T09:00:00Z"}
{"since": null}                           200 {"checks":{"db":{"status":"Up"},"cache":{"status":{"Degraded":"cold"}}}}
{"since": "2026-10-18T09:00:00+01:00"}    400 {"refusal":{"kind":"malformed","reason":"wrong_type: since"}}
```

Leaving `since` out and sending `null` are the same absence, so there is
no `note`. A time with an offset is refused at the edge rather than
guessed at: the handler only ever sees a value that decoded. The error
answers with its variant as the body and its class as the status:

```text
$ curl -si -X POST localhost:8794/call/Probe::restart -H 'Authorization: Bearer t-ops' -d '{"name": "queue"}'
HTTP/1.1 404 Not Found
Content-Type: application/json
Cache-Control: no-store
Hale-Error-Class: not_found
Content-Length: 20
Connection: close

{"NotFound":"queue"}
```

A refusal can share a status with a class (403, 409 and 503 are both),
so the class is also named in `Hale-Error-Class`, which no refusal
carries. That is how a client tells "the handler said no" from "the
exposure refused the call", and the generated clients and `hale call`
read it for you:

```text
$ hale call http://127.0.0.1:8794 Probe::restart --name queue --bearer t-ops
handler error (not_found): {"NotFound":"queue"}
```

(It exits 1, a handler error. Over a Unix socket the reply line carries
`"class": "not_found"` beside the error; over gRPC the call ends with
`NOT_FOUND` and the error in its details.)

## Growing without breaking anyone

Callers will be built against this surface: generated clients, scripts,
an MCP host. Each member has a digest of its own, folded from that
member's row alone, and the clients send it on every call. Add a member
and only the surface's digest moves:

```text
before:  Ops                fnv1a64:95d08b7e7dd9f4de
         Probe::health      fnv1a64:38d1eff02ba6e98c
         Probe::restart     fnv1a64:36f121c52d2c64b9

after adding rpc Probe::uptime requires: [];
         Ops                fnv1a64:1ccf03bbfaa228b8
         Probe::health      fnv1a64:38d1eff02ba6e98c
         Probe::restart     fnv1a64:36f121c52d2c64b9
         Probe::uptime      fnv1a64:f233238323e2aec6
```

A client of `Probe::health` built before `uptime` existed keeps being
admitted. Change `Health` itself (a field renamed or retyped), or the
class a `Problem` variant declares, and every client naming the old
digest is refused `digest_mismatch` instead of being handed a contract
it was not built for. So grow a surface by adding
members, and make an incompatible change a new member (`health_v2`)
served beside the old one until its callers have moved.

## Streaming the same records

A dashboard wants health as it changes, not on request. A plain
`std::http::Server` handler can stream it as Server-Sent Events, and
`Health::api_encode` writes each event in exactly the shape the
surface's callers already parse:

```hale,fragment
/// Streams three snapshots as Server-Sent Events, one a second.
locus Watch {
    fn handle(req: std::http::Request) -> std::http::Response {
        let w = std::http::SseWriter { conn_fd: req.conn_fd };
        let mut n = 0;
        while n < 3 {
            let ev = std::http::SseEvent { event: "health", id: to_string(n), data: Health::api_encode(snapshot(n)) };
            w.event(ev) or { break; };
            std::time::sleep(1s);
            n = n + 1;
        }
        return w.end();
    }
}
```

In `App`, the server is a param placed on an `async_io` pool, so each
stream parks on its own coroutine and a slow subscriber holds nobody
else up. It also caps what a request may cost, and answers an oversized
one in the API's own error shape:

```hale,fragment
events: std::http::Server = std::http::Server {
    port: 8795,
    handler: Watch { },
    max_request_bytes: 4096,
    too_large: std::http::Response { status: 413, content_type: "application/json", body: "{\"error\":\"request too large\"}" },
};
// …
placement { events: cooperative(pool = events) where async_io; }
```

```text
$ curl -sN localhost:8795/
event: health
id: 0
data: {"checks":{"db":{"status":"Up"},"cache":{"status":{"Degraded":"cold"}}}}

event: health
id: 1
data: {"checks":{"db":{"status":"Up"},"cache":{"status":"Up"}}}

event: health
id: 2
data: {"checks":{"db":{"status":"Up"},"cache":{"status":{"Degraded":"cold"}}}}
```

A 5,000-byte request to the same port is answered before its body is
read:

```text
HTTP/1.1 413 Content Too Large
Content-Type: application/json
Content-Length: 29
Connection: close

{"error":"request too large"}
```

`w.event` fails only when the subscriber has gone away, which is the
cue to stop and return `w.end()`. Encoding with `Health::api_encode`
rather than building the JSON with `+` keeps the stream and the surface
on one codec: when `Health` gains a field, both carry it.

## Where each piece is specified

- The codec's shapes (optional fields, enums, maps, sets) and its
  refusals: `spec/api.md` § Codecs. The digests: § The contract digest.
  Error classes and each transport's status for them: § Error classes.
- Streamed responses and the Server's request cap: `spec/stdlib.md`,
  `std::http`. The chapter: [HTTP](../everyday/http.md#streaming-a-response).
- The style guide's rules for choosing between these shapes:
  `spec/styleguide.md` 2.11, and C10 for the HTTP edge.
