# `std::http`

HTTP/1.1 request parsing and response writing for Aperio
programs. Phase 3 (m83 → m86) ships two record types and two
functions that compose with `std::io::tcp::Stream` to build a
real HTTP server.

The shape is intentionally small: parse a `Request` from the
bytes a `Stream.recv` produced, build a `Response` record, hand
it to `write_response`. There is no router, no middleware, no
keep-alive, no header map — just the wire-format primitives.
Higher-level surface lands in Phase 3 v1.0.

## Types

### `std::http::Request`

```aperio
type Request {
    method:  String;   // "GET", "POST", etc.
    path:    String;   // request-target, e.g. "/index.html?q=1"
    version: String;   // "HTTP/1.1"
    body:    String;   // everything after `\r\n\r\n`, or ""
}
```

A parsed inbound request. Headers are not surfaced in v0 —
adding them needs either a generic `Map<String, String>` type
or a fixed-size array of `(String, String)` tuples. Most
content servers don't need request headers; the `docs-server`
example certainly doesn't.

A malformed input yields a Request with empty fields. Callers
that care should check `req.method == ""` and respond with 400.

### `std::http::Response`

```aperio
type Response {
    status:       Int;     // 200, 404, 500, ...
    content_type: String;  // "text/html; charset=utf-8", etc.
    body:         String;  // entity body
}
```

An outbound response. The wire format emits a fixed header set
(`Content-Type`, `Content-Length`, `Connection: close`); custom
headers are not currently expressible.

## Functions

### `std::http::parse_request`

#### Synopsis

```aperio
fn parse_request(raw: String) -> Request
```

Parses one HTTP request from a raw String (typically the result
of `Stream.recv`). Returns a Request record.

#### Semantics

- Splits the request line on the first two spaces:
  `METHOD SP PATH SP VERSION`. Anything after the second space
  becomes `version`.
- Body is everything past the first `\r\n\r\n` separator. If no
  separator is present, body is `""`.
- Headers between the request line and the body are skipped —
  they appear neither on the resulting Request nor (for
  responses) on the wire.
- Malformed inputs (no CRLF, no space) yield a Request with
  empty fields rather than erroring. Check `req.method == ""`
  to detect.

#### Assumptions

- The entire request line + headers fit in the recv buffer.
  The m83 Listener passes 8 KB by default, which covers
  browser GETs comfortably.
- Streaming reassembly for oversized requests is a Phase 3 v1.0
  follow-up.

#### Examples

```aperio
fn handle(s: std::io::tcp::Stream) {
    let raw = s.recv(8192);
    let req = std::http::parse_request(raw);

    if req.method == "" {
        // Malformed — respond 400.
        return;
    }

    println("method=", req.method, " path=", req.path);
}
```

### `std::http::write_response`

#### Synopsis

```aperio
fn write_response(stream: std::io::tcp::Stream, response: Response)
```

Builds the HTTP/1.1 wire format from a Response and ships it
through the Stream.

#### Wire format

```
HTTP/1.1 <status> <phrase>\r\n
Content-Type: <content_type>\r\n
Content-Length: <byte length of body>\r\n
Connection: close\r\n
\r\n
<body>
```

- `<phrase>` is the canonical reason phrase (`OK`,
  `Not Found`, etc.) for the common codes (200, 301, 400, 404,
  500). Unknown codes get the generic phrase `Status`.
- `Content-Length` is the byte length of `body`.
- `Connection: close` is hardcoded — the connection terminates
  after the response. Keep-alive is a Phase 3 v1.0 follow-up.

#### Semantics

- Headers and body are sent as two separate `Stream.send`
  calls. TCP is a stream; peers see the concatenation, which
  is what HTTP expects.
- The Stream is not closed by `write_response`; the closure of
  the connection happens at scope-exit when the Stream binding
  goes out of scope (the m82 dissolve mechanism). For the
  multi-accept Listener pattern, that's the end of the
  `on_connection` callback.

#### Examples

```aperio
fn handle(s: std::io::tcp::Stream) {
    let raw = s.recv(8192);
    let req = std::http::parse_request(raw);

    let resp = std::http::Response {
        status: 200,
        content_type: "text/html; charset=utf-8",
        body: "<h1>hello</h1>"
    };
    std::http::write_response(s, resp);
}

fn main() {
    std::io::tcp::Listener {
        host: "127.0.0.1",
        port: 8080,
        max_accepts: -1,           // run until killed
        on_connection: handle
    };
}
```

See `examples/http-hello/main.ap` for a complete server.

## Limitations (Phase 3 v0)

- **No request-header surface.** Headers between the request
  line and body are dropped. Phase 3 v1.0 adds either a
  generic Map type or a fixed-size header array.
- **No custom response headers.** The four-header set above
  is fixed. Same Phase 3 v1.0 follow-up.
- **No `Connection: keep-alive`.** Each request closes its
  connection. Persistent connections need a request-handling
  loop inside the `on_connection` callback.
- **No body chunking / streaming.** Single recv assumed for
  request reading; single send-pair for response writing.
  Bodies > 8 KB unsupported on the read side.
- **No HTTPS / TLS.** No TLS substrate ships.
- **No router / dispatch.** Path matching is hand-rolled with
  `==` / `starts_with` / `std::str::index_of`. See the
  `docs-server` example for the pattern.

## See Also

- [Roadmap](./roadmap.md) — Phase 3 v1.0 plan.
- [`std::io::tcp`](./io/tcp.md) — Listener + Stream surface
  these compose on.
- [`std::str`](./str.md) — `index_of` and friends, used for
  hand-rolled path dispatch.
- `examples/http-hello/main.ap` (in the language repo) — Phase
  3 capstone, smallest complete server.
- `examples/docs-server/main.ap` (in the language repo) —
  Phase 5 capstone, real path-dispatch + markdown rendering.
