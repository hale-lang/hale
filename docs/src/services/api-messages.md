# Designing API Messages

An API surface's wire contract is determined directly by the Hale types used in its `rpc` rows. By choosing specific record field patterns, enum shapes, and error class annotations, you control how the compiler encodes JSON and Protobuf payloads, validates requests, generates OpenAPI/JSON Schema specifications, and maps domain errors to HTTP and gRPC status codes.

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

---

## Wire Modeling Patterns

| Pattern | Hale Declaration | Wire Representation & Behavior |
|---|---|---|
| **Field Default** | `verbose: Bool = false;` | Omitted from JSON Schema `required`. If omitted by the caller, the decoder substitutes the default value. Defaults may be literals, enums (`Scope::key`), or records, and may reference module constants/items. |
| **Presence Pair** | `since: Time = ...;`<br>`has_since: Bool = false;` | Distinguishes "omitted or `null`" (`has_since == false`) from a provided value (`has_since == true`). `has_since` is never sent on the wire; when encoding, the field is omitted if `has_<field>` is `false`. |
| **Keyed Map** | `checks: bounded[Check; 16] indexed_by name;` | Stored in Hale as an ordered `bounded[Check; 16]` list, but encoded on the wire as a JSON object keyed by each element's `name` field (`{"db": {"status": "Up"}}`). |
| **Enum** | `enum { Up, Degraded(String), Down }` | Unit variants encode as strings (`"Up"`). Payload variants encode as single-key objects (`{"Degraded": "cold"}`). |
| **Error Class** | `@not_found NotFound(String)` | Maps `fallible(E)` enum variants to transport status codes (`404` / gRPC `NOT_FOUND`) and sets the `Hale-Error-Class` response header. |
| **Surface Default Role** | `api Ops requires: [operator] { ... }` | Sets the default `requires` list for every row in the surface. Individual rows can override it (e.g., `requires: []` to allow any authenticated caller). Produces the exact same contract digest as spelling roles on each row. |

### Error Class Status Mappings

When a handler fails with a `fallible(E)` enum variant, the variant's decorator selects the transport status code:

| Decorator | HTTP Status | gRPC Status | `Hale-Error-Class` Header |
|---|---|---|---|
| *(none)* | `422 Unprocessable Content` | `FAILED_PRECONDITION` | *(omitted)* |
| `@not_found` | `404 Not Found` | `NOT_FOUND` | `not_found` |
| `@conflict` | `409 Conflict` | `ABORTED` | `conflict` |
| `@invalid` | `422 Unprocessable Content` | `INVALID_ARGUMENT` | `invalid` |
| `@forbidden` | `403 Forbidden` | `PERMISSION_DENIED` | `forbidden` |
| `@unavailable` | `503 Service Unavailable` | `UNAVAILABLE` | `unavailable` |

---

## Writing the Handlers

Handlers operate on decoded Hale structs and enums without manual JSON parsing:

```hale,fragment
/// Builds a health snapshot (stand-in for live dependency probes).
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

---

## Inspecting Generated Schemas

Run `hale check --api` with `--json-schema` to verify how your message types project to external schemas without starting the service:

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

- `checks` is described as an object with `maxProperties: 16` and `"x-hale-key": "name"`.
- `note` is optional and nullable (`string` or `null`).
- `Problem` lists its variants alongside `"x-hale-error-classes"`.

---

## Serving and Exercising the Wire Contract

Bind `Ops` to an `http::Rpc` exposure with bearer authentication and a static role table:

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

Inspect the served surface with `hale describe`:

```text
$ hale describe http://127.0.0.1:8794 --bearer t-ops
Ops@fnv1a64:95d08b7e7dd9f4de/web  [http 127.0.0.1:8794]
caller: bearer ops; roles: operator
  Probe::health(since?: String (Time) or null, verbose?: Bool) -> Health  requires: -
  Probe::restart(name: String) -> Health, error Problem  requires: operator
```

### Presence Pair & Timestamp Validation

Sending different payloads to `POST /call/Probe::health` (with `Authorization: Bearer t-ops`) demonstrates how presence pairs and strict timestamp parsing behave:

```text
{}                                        200 {"checks":{"db":{"status":"Up"},"cache":{"status":{"Degraded":"cold"}}}}
{"since": "2026-10-18T09:00:00Z"}         200 {"checks":{…},"note":"changes since 2026-10-18T09:00:00Z"}
{"since": null}                           200 {"checks":{"db":{"status":"Up"},"cache":{"status":{"Degraded":"cold"}}}}
{"since": "2026-10-18T09:00:00+01:00"}    400 {"refusal":{"kind":"malformed","reason":"wrong_type: since"}}
```

- Omitting `since` or sending `{"since": null}` leaves `has_since` set to `false`, so `h.has_note` stays `false` and `note` is omitted from the response.
- `Time` values on the wire must be UTC RFC 3339 strings ending in `Z`. Non-UTC offsets (`+01:00`) are rejected at the transport boundary with `400` (`wrong_type: since`).

### Distinguishing Classified Handler Errors from Transport Refusals

When `Probe::restart` fails with `Problem::NotFound("queue")`, the exposure responds with HTTP `404` and the `Hale-Error-Class: not_found` header:

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

Because some error classes share HTTP status codes with transport refusals (`403`, `409`, `503`), the `Hale-Error-Class` header unambiguously marks the response as a domain handler error rather than an exposure refusal. Both `hale call` and generated clients inspect this header automatically:

```text
$ hale call http://127.0.0.1:8794 Probe::restart --name queue --bearer t-ops
handler error (not_found): {"NotFound":"queue"}
```

`hale call` exits with code `1` (handler error). Over a Unix socket, the reply line includes `"class": "not_found"` alongside the error payload; over gRPC, the call returns `NOT_FOUND` with the encoded error in `grpc-status-details-bin`.

---

## Evolving a Surface Without Breaking Clients

Every `api` surface computes a **surface digest** over all of its rows, plus an individual **member digest** for each `rpc` row (`spec/api.md` § The contract digest). Generated clients send the member digest on every request, and the exposure admits any request matching either the current surface digest or that specific member's digest.

Adding a new `rpc` row changes the overall surface digest, but leaves existing member digests untouched:

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

Existing clients calling `Probe::health` or `Probe::restart` continue working without regeneration. Conversely, if you rename or retype a field in `Health` or change an error class on `Problem`, the member digest changes and outdated clients are rejected with `digest_mismatch`.

> [!TIP]
> **Safe API Evolution Rule**: Grow a surface by adding new `rpc` members. When making a breaking change to an existing operation's request, response, or error type, introduce a new member (e.g., `Probe::health_v2`) alongside the existing one until all callers have migrated.

---

## Streaming Wire Records as Server-Sent Events (SSE)

Every record and enum eligible for an API surface provides a compiler-generated `<Type>::api_encode` method. You can use `T::api_encode` inside a standard `std::http::Server` handler to stream Server-Sent Events using the exact same JSON wire format as your RPC surface:

```hale,fragment
/// Streams three health snapshots as Server-Sent Events, one per second.
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

Declare the HTTP server as a parameter on `App`, place it on an `async_io` pool so each SSE connection parks on its own coroutine, and configure `max_request_bytes` and `too_large` to cap incoming request sizes:

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

Connecting with `curl -sN` streams the `Health::api_encode` payloads:

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

If the client disconnects early, `w.event(ev)` fails, breaking out of the loop and closing the stream via `w.end()`. Any request exceeding `max_request_bytes` (e.g., a 5,000-byte request) is rejected before reading the body:

```text
HTTP/1.1 413 Content Too Large
Content-Type: application/json
Content-Length: 29
Connection: close

{"error":"request too large"}
```

## Related Reference

- **Codec Shapes, Digests & Error Classes**: [The API Surface](./api.md) and [`spec/api.md`](https://github.com/hale-lang/hale/blob/main/spec/api.md) (§ Codecs, § The contract digest, § Error classes).
- **HTTP Streaming & Request Caps**: [HTTP](../everyday/http.md#streaming-a-response) and [`spec/stdlib.md`](https://github.com/hale-lang/hale/blob/main/spec/stdlib.md) (`std::http`).
- **Style Guide Rules**: `spec/styleguide.md` (§ 2.11 for API message shapes, C10 for HTTP edge limits).
