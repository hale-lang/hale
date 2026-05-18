# Build an HTTP server

`std::http::Server` ships an accept-recv-parse-dispatch-write
loop. You provide a **handler locus** that satisfies the
`std::http::Handler` interface; the Server calls
`handler.handle(req)` for every request and writes whatever
`Response` you return.

The handler is a normal locus, so cross-request state (a
counter, an in-memory store, a logger handle) lives in its
`params`.

## Hello, server

```aperio
locus Routes {
    params { hits: Int = 0; }

    fn handle(req: std::http::Request) -> std::http::Response {
        self.hits = self.hits + 1;

        if req.method == "GET" && req.path == "/health" {
            return std::http::Response { status: 200, body: "ok" };
        }
        if req.method == "GET" && req.path == "/" {
            return std::http::Response {
                status: 200,
                content_type: "text/html",
                body: f"<h1>hits: {to_string(self.hits)}</h1>"
            };
        }
        return std::http::Response { status: 404, body: "not found" };
    }
}

fn main() {
    std::http::Server {
        port: 8080,
        handler: Routes { },
        ready_signal: "READY"
    };
}
```

Run:

```sh
aperio build server/
./server/server &
# stdout prints "READY" when accept() is live
curl http://127.0.0.1:8080/
curl http://127.0.0.1:8080/health
```

## What just happened

- **`Routes`** is a regular locus. It declares
  `fn handle(req: Request) -> Response`, so it structurally
  satisfies the `std::http::Handler` interface. State
  (`hits`) lives in its `params` and survives across requests.
- **`std::http::Server { handler: Routes { } }`** — the
  Server's `handler:` field is typed `std::http::Handler` and
  is required (no default). The codegen coerces a locus value
  into the interface fat-pointer; you never write the cast
  yourself.
- **`Response.content_type`** defaults to `"text/plain"`;
  override per response.
- **`Response.headers`** is an optional CRLF-joined block of
  user-supplied headers (no trailing CRLF) — write
  `Response { status: 200, headers: "Set-Cookie: sid=" + sid,
  body: "..." }` to attach Set-Cookie / CORS / X-Custom-*
  lines without dropping down to a custom Stream writer.
  Empty `headers` (the default) reproduces the v0 wire bytes
  byte-for-byte. The companion `std::http::header(resp, name)`
  free-fn reads an attached header back off a Response, same
  case-insensitive shape as the Request-side lookup.
- **`ready_signal`** prints that exact line to stdout the
  moment `listen_socket` succeeds — before the first `accept()`
  call blocks. Pipe consumers (test oracles, shell scripts,
  supervisors) key off it: `./server | grep -m1 READY`.

## Routing

`Server` does no routing of its own. Match `req.method` +
`req.path` inside `handle`:

```aperio
fn handle(req: std::http::Request) -> std::http::Response {
    if req.method == "GET" && req.path == "/users"     { return self.list_users(); }
    if req.method == "POST" && req.path == "/users"    { return self.create_user(req); }
    if req.method == "GET" && std::str::index_of(req.path, "/users/") == 0 {
        let id = std::str::substring(req.path, 7, len(req.path));
        return self.show_user(id);
    }
    return std::http::Response { status: 404, body: "not found" };
}
```

There's no path-pattern DSL and no regex in stdlib — string
equality and prefix tests are the surface.

## Reading the request body

`req.body` is the raw bytes-as-String from the socket. Parse
it however you like. For JSON, the flat-object helpers cover
single-level shapes:

```aperio
fn create_user(req: std::http::Request) -> std::http::Response {
    let name = std::json::find_string_field(req.body, "name");
    let age  = std::json::find_int_field(req.body, "age");
    self.users.set(User { id: self.next_id(), name: name, age: age });
    return std::http::Response { status: 201, body: "" };
}
```

See [Read & write JSON](./json.md) for nested shapes and the
streaming Builder.

## Bounded vs forever runs

`max_accepts: N` caps the loop at N requests then exits. The
default `-1` runs forever (until SIGINT). Bounded runs are
useful for integration tests:

```aperio
std::http::Server {
    port: 8080,
    handler: Routes { },
    max_accepts: 100,        // exits after 100 requests
    ready_signal: "READY"
};
```

## What's NOT in `std::http`

- **No HTTPS server.** `std::http::Server` accepts plaintext
  HTTP/1.0 only — front it with a reverse proxy (nginx,
  Caddy, etc.) for TLS termination. The client side of TLS
  *is* available — `std::io::tls::*` ships a system-OpenSSL-
  backed client surface for outbound HTTPS calls (see the
  [stdlib reference](../reference/stdlib.md)).
- **No HTTP/2, no chunked transfer, no streaming bodies.**
  One request, one full `body: String`, one response. Set
  `Content-Length` correctly via the response's body length
  (Server does this for you).
- **No middleware / interceptors.** The Handler is the only
  hook. If you want logging on every request, do it inside
  `handle()` or compose a wrapper Handler locus.
- **No keep-alive.** Each request gets its own connection
  (Server sends `Connection: close`).

For production-grade HTTP serving, front Aperio with nginx or
similar. The stdlib server is sized for tooling, internal
services, and demos.

## See also

- [Read & write JSON](./json.md) — for request/response bodies.
- [Structured logging](./logging.md) — `Logger` instance on the
  Handler locus's params for per-request audit lines.
- [The bus](../concepts/the-bus.md) — handlers can publish to
  topics for fanout / metrics, with no extra coordination.
