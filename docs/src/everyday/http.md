# HTTP

`std::http` provides an HTTP/1.1 server, request router, streaming SSE and chunked-body writers/readers, and an outbound HTTP/HTTPS client. An HTTP server is configured by passing a handler locus—any locus implementing `fn handle(req: std::http::Request) -> std::http::Response`—to `std::http::Server`.

```hale
locus Api {
    params { hits: Int = 0; }

    fn handle(req: std::http::Request) -> std::http::Response {
        if req.path == "/health" {
            return std::http::Response {
                status: 200, body: "ok\n", content_type: "text/plain"
            };
        }
        self.hits = self.hits + 1;
        return std::http::Response {
            status: 200,
            body: f"hello — hit #{self.hits}\n",
            content_type: "text/plain"
        };
    }
}

fn main() {
    std::http::Server { port: 8080, handler: Api { } };
}
```

Compile and run the server with `hale build`, then query `curl localhost:8080/health`. Because `Api` stays alive for the lifetime of `Server`, its `params` (`self.hits`) persist across requests.

---

## 1. Quickstart HTTP Server

### Core Server Types

| Type / Interface | Key Fields & Methods | Description |
|---|---|---|
| `std::http::Request` | `method`, `path`, `version`, `body`, `conn_fd` | Parsed inbound HTTP request. Headers are queried case-insensitively. (`std::http::parse_request` parses raw requests into this type.) |
| `std::http::Response` | `status`, `body`, `content_type`, `headers`, `takeover`, `takeover_raw` | Outbound HTTP response. Requires `status` and `body`; `content_type` defaults to `"text/plain"`. |
| `std::http::Server` | `port`, `handler`, `name`, `max_accepts`, `max_connections`, `max_request_bytes`, `too_large` | Owns the listen-accept-parse-dispatch loop. Reassembles `POST` bodies by `Content-Length` across TCP segments. |
| `std::http::Handler` | `fn handle(req: Request) -> Response;` | Structural interface satisfied automatically by any locus declaring a matching `handle` method. |

### `std::http::Server` Configuration

- **`max_accepts`**: Bounds the server to `N` requests before returning (useful in tests). By default, the server runs until stopped.
- **`max_request_bytes`**: Caps the total size of a request including request line, headers, and body (`1 MiB` default, up to `1 GiB` maximum).
- **`too_large`**: When a request's declared `Content-Length` exceeds `max_request_bytes`, the server replies immediately with `too_large` without reading the request body. Defaults to a plain-text `413 payload too large` response, and can be overridden (for example, `too_large: std::http::Response { status: 413, content_type: "application/json", body: "{\"error\":\"too large\"}" }`).

### Structural `Handler` Interface

`Server`'s `handler` parameter has type `std::http::Handler`:

```hale
// (declared in the standard library)
interface Handler {
    fn handle(req: Request) -> Response;
}
```

Hale interfaces use structural satisfaction: any locus that defines `fn handle(req: std::http::Request) -> std::http::Response` satisfies `std::http::Handler` without an explicit `impl` declaration. Conversely, a parameter typed as `Handler` exposes only `h.handle(req)`—accessing concrete locus fields (such as `h.routes`) is a compile-time error.

---

## 2. Concurrent Serving

When started directly in `fn main()`, on the `main` pool, `pinned`, or on a plain `cooperative(pool = io)` pool, `std::http::Server` processes **one request at a time** (macOS also runs one request at a time, as `async_io` pools are Linux-only).

To serve requests concurrently, declare `std::http::Server` as a param on your `main locus` and place it on a `cooperative` pool with `where async_io`:

```hale
locus Api {
    params { hits: Int = 0; }

    fn handle(req: std::http::Request) -> std::http::Response {
        if req.path == "/slow" {
            std::time::sleep(1s);   // parks this request coroutine only
        }
        self.hits = self.hits + 1;
        return std::http::Response { status: 200, body: f"hit #{self.hits}\n" };
    }
}

main locus App {
    params {
        web: std::http::Server = std::http::Server { port: 8080, handler: Api { } };
    }
    placement { web: cooperative(pool = web) where async_io; }
    run() {
        while !self.draining { std::time::sleep(100ms); }
    }
}

fn main() {
    App { };
}
```

### Concurrency & State Semantics on `async_io`

- **Per-connection coroutines**: Each accepted connection runs on its own coroutine. When a handler parks (`std::time::sleep`, socket read/write, or outbound HTTP request), the pool executes other ready requests and continues accepting new connections.
- **Connection limits (`max_connections`)**: Up to `max_connections` (`256` by default) requests execute concurrently. Up to `max_connections` additional connections wait in the queue; connections beyond that limit are closed unserved.
- **Single-threaded interleaving**: Every request dispatches to the same `Api` locus on a single OS thread. Two `handle` calls never execute simultaneously, and code between park points runs uninterrupted. However, state may change across a park point (`sleep` or I/O call)—always re-read mutable `self` fields after parking rather than relying on values cached before the park.
- **No blocking calls**: Synchronous file I/O or `std::process::run` blocks the pool's underlying OS thread and stalls all concurrent requests on that pool. `hale check` emits a warning if a blocking call is reachable from a `where async_io` locus.

### Stopping a Server (`std::io::tcp::stop`)

Because cross-pool method calls (such as `self.web.shutdown()`) are disallowed, stop a server from another pool by assigning it a unique `name` (defaults to `"127.0.0.1:<port>"`; duplicate active names fail at birth) and calling the free function `std::io::tcp::stop(name)`:

```hale,fragment
// In App params:
// web: std::http::Server = std::http::Server { port: 8080, name: "web", handler: Api { } };
std::io::tcp::stop("web");
```

`std::io::tcp::stop` closes the listening socket and waits for in-flight requests to finish before the server's `run()` returns. `SIGTERM` and `Ctrl-C` trigger the same graceful drain automatically. Note that `main`'s `run()` returning does not terminate an active server; a `main locus` that only constructs server loci does not need a `run()` block unless it performs additional coordination.

### Sharded Listener & Worker Pools (`Acceptor` + `Connections`)

`std::http::Server` combines a connection listener and a connection consumer in a single locus. For multi-core scaling, split them explicitly: place a single `std::io::tcp::Acceptor` on a `pinned` thread and distribute connections round-robin across multiple `std::io::tcp::Connections` shards on separate `where async_io` pools:

```hale
locus Api {
    params { tag: String = ""; }
    fn handle(req: std::http::Request) -> std::http::Response {
        return std::http::Response { status: 200, body: self.tag };
    }
}

main locus App {
    params {
        // Declare Connections consumers before Acceptor so subscribers exist before accept begins
        zero: std::io::tcp::Connections = std::io::tcp::Connections {
            listener: "web", shard: 0,
            handler: std::http::Responder { handler: Api { tag: "zero" } }
        };
        one: std::io::tcp::Connections = std::io::tcp::Connections {
            listener: "web", shard: 1,
            handler: std::http::Responder { handler: Api { tag: "one" } }
        };
        acceptor: std::io::tcp::Acceptor = std::io::tcp::Acceptor {
            port: 8080, name: "web", shards: 2
        };
    }
    placement {
        zero: cooperative(pool = zero) where async_io;
        one: cooperative(pool = one) where async_io;
        acceptor: pinned;
    }
    run() {
        while !self.draining { std::time::sleep(100ms); }
        std::io::tcp::stop("web");
    }
}

fn main() {
    App { };
}
```

- **Handler adaptation**: `std::http::Responder` wraps an HTTP `Handler` into a connection consumer. Any locus declaring `fn serve(c: std::io::tcp::Accepted)` (taking ownership of `c.fd`) can also be passed as the `handler` of `Connections`.
- **Zero connection leaks**: Any connection not claimed by a consumer is closed by `Acceptor`, satisfying `require no_silent_loss(topic __TcpAcceptedT)` in program `claims`. Each connection runs in a transient child arena that reclaims on close.
- **Unix domain sockets**: `std::io::tcp::Acceptor { path: "/run/app.sock" }` listens on a Unix socket (named `"unix:/run/app.sock"`, removing the socket file on dissolve) and feeds the exact same `Connections` consumers. The API runtime's `http::Rpc`, `mcp::Rpc`, `unix::Rpc`, `grpc::Rpc`, and `ws::Hub` listeners use this same architecture (see [The API surface](../services/api.md)).
- **Bus-subscribing connection children**: If per-connection work requires a locus that subscribes to the bus, declare `accept` and `release` on the handler to spawn it as a flow child rather than constructing it inline inside `serve` (loci constructed and dissolved inside a method have their deliveries queued for the main thread).

For raw TCP protocols below HTTP, `std::io::tcp::Listener` dispatches each accepted socket as a `std::io::tcp::Stream` to an `on_connection` function and closes the stream when the function returns (`max_accepts` defaults to `1`; pass `-1` to serve until stopped):

```hale
fn echo(s: std::io::tcp::Stream) {
    let line = s.recv(1024) or "";
    s.send(line) or discard;
}

main locus App {
    params {
        echo: std::io::tcp::Listener = std::io::tcp::Listener {
            port: 9000, max_accepts: -1, on_connection: echo
        };
    }
    placement { echo: cooperative(pool = echo) where async_io; }
    run() {
        while !self.draining { std::time::sleep(100ms); }
    }
}

fn main() {
    App { };
}
```

---

## 3. Routing

Hale supports two routing patterns:
1. **`std::http::Router`**: Dispatches routes to free functions or separate per-route handler loci, with support for middleware chains.
2. **`std::http::build_context` + `std::http::is_route`**: Dispatches routes directly to methods on a single stateful locus.

### Multi-Handler Routing (`std::http::Router`)

For stateless endpoints, register a free function with `router.add_fn`:

```hale,fragment
fn greet(ctx: std::http::Context) -> std::http::Response {
    let who = std::http::path_param(ctx.params, "name");
    return std::http::Response { status: 200, body: "hi " + who };
}

router.add_fn("GET", "/hello/:name", greet);
```

For stateful endpoints, register a handler locus with `router.add`. Because `Router` borrows handler and middleware loci without owning them, **every registered handler and middleware locus must outlive the router**. Declare them as `params` fields on the locus that owns the `Router` and register them in `birth()`:

```hale
locus Hello {
    fn handle(ctx: std::http::Context) -> std::http::Response {
        let who = std::http::path_param(ctx.params, "name");
        return std::http::Response { status: 200, body: "hi " + who };
    }
}

locus Routes {
    params {
        hello: Hello = Hello { };
        router: std::http::Router = std::http::Router { };
    }
    birth() {
        self.router.add("GET", "/hello/:name", self.hello);
    }
    fn handle(req: std::http::Request) -> std::http::Response {
        return self.router.dispatch(req);
    }
}

fn main() {
    std::http::Server { port: 8080, handler: Routes { } };
}
```

> [!IMPORTANT]
> Passing a temporary handler literal into a router that escapes the current function—or registering a handler constructed inside a loop body—would leave a dangling reference when the frame or loop iteration reclaims its storage. `hale check` rejects both patterns at compile time.

### Path Captures, Query Strings, and Middleware

- **Path parameters (`:name`)**: Extracted via `std::http::path_param(ctx.params, "name")`, automatically percent-decoded (`/hello%20world` yields `"hello world"`).
- **Query parameters (`?k=v`)**: Extracted via `std::http::query_param(ctx.params, "k")`, percent-decoded with `+` converted to space.
- **Route precedence**: `add` and `add_fn` share a single route list evaluated in first-match-wins order. Register specific routes before wildcard/parameterized routes. Unmatched requests return a customizable `404` response.
- **Middleware (`router.use`)**: Wraps route handlers in onion order. A middleware locus implements `before` and `after`:

```hale
locus Cors {
    fn before(ctx: std::http::Context) -> std::http::Context { return ctx; }
    fn after(ctx: std::http::Context, resp: std::http::Response) -> std::http::Response {
        return std::http::Response {
            status: resp.status, content_type: resp.content_type,
            headers: "Access-Control-Allow-Origin: *", body: resp.body
        };
    }
}
// In Routes params: `cors: Cors = Cors { };`
// In Routes birth(): `self.router.use(self.cors);`
```

### Single-Locus Routing (`build_context` + `is_route`)

When multiple endpoints share a single piece of locus state (such as a database connection or session table), dispatch directly inside `handle` using `std::http::build_context` and `std::http::is_route`:

```hale
locus Api {
    params { hits: Int = 0; }

    fn handle(req: std::http::Request) -> std::http::Response {
        let ctx = std::http::build_context(req);
        if std::http::is_route(ctx, "GET", "/users") {
            return self.list(ctx);
        }
        if std::http::is_route(ctx, "GET", "/users/:id") {
            return self.show(ctx);
        }
        return std::http::Response { status: 404, body: "not found" };
    }

    fn list(ctx: std::http::Context) -> std::http::Response {
        self.hits = self.hits + 1;
        return std::http::Response { status: 200, body: "users" };
    }

    fn show(ctx: std::http::Context) -> std::http::Response {
        let id = std::http::path_param(ctx.params, "id");
        return std::http::Response { status: 200, body: "user " + id };
    }
}

fn main() {
    std::http::Server { port: 8080, handler: Api { } };
}
```

- `build_context(req)` splits the query string into `ctx.params.qs`.
- `is_route(ctx, method, pattern)` runs the router matcher, populating `:name` captures in `ctx.params` on a match and clearing captures on a miss.
- Always read `std::http::path_param(ctx.params, "id")` **after** `is_route` matches. Because records are value types, binding `let p = ctx.params;` before the `if` ladder copies an empty capture table (`hale check` warns on this bug).
- Linear route matching costs ~90 ns per tested route (~1 µs for 10 routes) and pairs naturally with [replicated placement](../services/concurrency.md) (`pinned(..., replicas = K)`), giving each replica its own independent `Api` instance and state.

---

## 4. Streaming Responses & Connection Takeover

### Server-Sent Events (`std::http::SseWriter`)

To stream Server-Sent Events (`text/event-stream`), construct an `std::http::SseWriter` with `req.conn_fd`, emit events with `w.event(...)`, and return `w.end()`:

```hale
locus Countdown {
    fn handle(req: std::http::Request) -> std::http::Response {
        let w = std::http::SseWriter { conn_fd: req.conn_fd };
        let mut n = 3;
        while n > 0 {
            w.event(std::http::SseEvent { event: "tick", data: to_string(n) }) or discard;
            std::time::sleep(1s);
            n = n - 1;
        }
        w.event(std::http::SseEvent { event: "done", data: "liftoff" }) or discard;
        return w.end();
    }
}
```

- Constructing `SseWriter` immediately flushes the HTTP response headers.
- Each `w.event(SseEvent { ... })` writes a single chunked frame to the client. `SseEvent` accepts `data` (multi-line strings are split into one `data:` line per line), plus optional `event`, `id` (returned by reconnecting clients as `Last-Event-ID`), and `retry` (`retry: 5s`).

### Chunked Byte Streams (`std::http::BodyWriter`)

For raw streaming bodies such as newline-delimited JSON (`application/x-ndjson`) or log tails, use `std::http::BodyWriter`:

```hale,fragment
let w = std::http::BodyWriter { conn_fd: req.conn_fd, content_type: "application/x-ndjson" };
w.chunk("{\"step\": 1}\n") or discard;
w.chunk("{\"step\": 2}\n") or discard;
return w.end();
```

Both `SseWriter` and `BodyWriter` use HTTP/1.1 chunked transfer encoding. `w.event` and `w.chunk` fail only when the client disconnects—use that failure to break out of your loop and `return w.end();`, which writes the terminating chunk and closes the connection.

> [!TIP]
> Always place streaming handlers on a `where async_io` pool so parked streams do not block other connections, and set `std::io::tcp::set_send_timeout(req.conn_fd, 10s)` before streaming so stalled clients cannot block writes indefinitely.

### Connection Takeover (`takeover` and `takeover_raw`)

For protocol upgrades (such as WebSockets) or deferred responses written by another locus, a handler can take ownership of `req.conn_fd`:

| Response Flag | What `Server` Writes | Caller Responsibility |
|---|---|---|
| `takeover: true` | Status line + custom `headers` (no `Content-Length` or `Connection: close`) | Read/write the upgraded protocol on `req.conn_fd` and call `std::io::tcp::close_fd(fd)` when done |
| `takeover_raw: true` | Nothing (`status`, `headers`, and `body` are ignored) | Write the complete raw HTTP response (including `HTTP/1.1 200 OK\r\n...`) and close `req.conn_fd` |

```hale
type Conn { fd: Int; }
topic WsConn { payload: Conn; subject: "ws.conn"; }

locus WsHandler {
    bus { publish WsConn; }
    fn handle(req: std::http::Request) -> std::http::Response {
        WsConn <- Conn { fd: req.conn_fd };
        return std::http::Response {
            status: 101,
            headers: "Upgrade: websocket\r\nConnection: Upgrade",
            body: "",
            takeover: true
        };
    }
}
```

For `takeover_raw: true`, return `std::http::Response { status: 0, body: "", takeover_raw: true }` and later write the full HTTP frame from the owning locus:

```hale,fragment
std::io::tcp::send_fd(fd, std::bytes::from_string(
    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")) or discard;
std::io::tcp::close_fd(fd);
```

When taking over a connection (via raw-fd helpers `send_fd`, `recv_into`, `close_fd` or a borrowed `Stream { conn_fd: fd, owns_fd: false }`), observe four rules:
1. **Clear the receive timeout**: The server's 5s receive timeout remains on `fd`; call `std::io::tcp::set_recv_timeout(fd, 0)` for long-lived sessions.
2. **Bound send timeouts**: Call `std::io::tcp::set_send_timeout(fd, 200ms)` before entering a push loop so one stalled client cannot block fan-out.
3. **Never drop `req.conn_fd`**: Returning a takeover response without saving and eventually closing `req.conn_fd` leaks the socket descriptor.
4. **Bound deferred responses**: Under `takeover_raw`, the client waits on a silent socket until you write, so enforce a timeout on deferred work.

---

## 5. HTTP Client & Streaming Reads

### One-Shot Requests & Pooled `Client`

Use `std::http::get` and `std::http::post` for one-shot requests, or `std::http::Client` for connection pooling and automatic retries:

```hale,fragment
// One-shot GET and POST (30s default deadline)
let resp = std::http::get("https://example.com") or raise;
println(std::str::from_bytes(resp.body));

let posted = std::http::post(
    "http://api.local/things",
    std::bytes::from_string("{\"n\": 1}"),
    "application/json"
) or raise;

// Pooled client with retries and custom timeout
let c = std::http::Client { keep_alive: true, max_retries: 2, timeout_ms: 5000 };
let r = c.get("http://api.local/health") or raise;
```

- **Error handling (`HttpError`)**: Client calls are `fallible(std::http::HttpError)` with `err.kind` (`"connect_failed"`, `"recv_failed"`, `"bad_url"`, `"retries_exhausted"`, etc.) and `err.detail`.
- **Binary-safe bodies**: `resp.body` is `Bytes`; convert to text with `std::str::from_bytes(resp.body)`.
- **URL encoding & IPv6**: Escape URL parameters with `std::url::encode` and decode with `std::url::decode` or `std::url::decode_query` (where `+` decodes to a space). Bracket IPv6 hosts in URLs (`"http://[::1]:8080/health"`):

```hale,fragment
let q = std::url::encode("tea & cake");            // "tea%20%26%20cake"
let r = std::http::get("http://api.local/search?q=" + q) or raise;
let back = std::url::decode_query("tea+%26+cake"); // "tea & cake"
```

- **Timeouts (`timeout_ms`)**: Defaults to `30000` (30s) on both `Client` and one-shot `get`/`post`/`request`. Bounds the entire operation (TCP dial, TLS handshake, request write, response headers and body read, and all retries). A truncated body at deadline is always an error. Set `timeout_ms: 0` to disable the timeout for large downloads.
- **5xx Retry behavior**: One-shot `get` returns `5xx` responses directly. `Client` retries `5xx` responses up to `max_retries`; if all retries fail, it raises `HttpError` with `err.kind == "retries_exhausted"` and populates `err.status`, `err.headers`, and `err.body` from the final `5xx` response (`err.status == 0` if no response was received).
- **`async_io` integration**: On a `where async_io` pool, TCP connect, TLS handshake, and socket reads/writes park only the calling coroutine. (DNS hostname resolution still runs synchronously on the worker thread; dial IP addresses or isolate slow resolvers on a dedicated pool.)

### Streaming Client Reads (`SseReader` and `BodyReader`)

To consume SSE streams or chunked responses incrementally as they arrive, use `std::http::SseReader` or `std::http::BodyReader`:

```hale,fragment
let url = std::http::parse_url("https://feeds.local/events") or raise;
let feed = std::http::SseReader { req: std::http::ClientRequest { url: url, body: b"" } };
while true {
    let e = feed.event() or { break; };
    println(e.event, ": ", e.data);
}
```

- Constructing `SseReader` or `BodyReader` sends the request and reads the response status and headers immediately into `feed.body.status` and `feed.body.headers`.
- `feed.event()` yields the next `SseEvent` (`data`, `event`, `id`, `retry`); `BodyReader`'s `chunk()` yields the next `Bytes` chunk.
- **A long download in `run()`**: `let c = body.chunk() or { break; }; h.update(c); out.write_bytes(c) or { break; };` in `run()`'s own loop holds one chunk at a time, however long the body: each iteration's chunk is freed when the next iteration starts ([Memory & Lifetime](../systems/memory.md#per-iteration-loop-regions)). Keeping each chunk in a binding declared before the loop would keep them all.
- **Bounded-wait reads (`event_within(ms)` and `chunk_within(ms)`)**: When a streaming relay also needs to service a control channel or cancellation signal between events, `feed.event_within(ms)` (or `body.chunk_within(ms)`) waits at most `ms` milliseconds and fails with `err.kind == "idle"` if nothing arrived, leaving the stream open and any partially received event buffered for the next read. The reader's overall `timeout_ms` still bounds the entire stream. Build the reader with `deferred: true` and the wait for the upstream's response head is bounded the same way: birth sends nothing, the first `event_within(ms)` opens the exchange and fails `idle` while the head has not come (or call `open_within(ms)`, which answers `true` once the head is read). The dial itself is still bounded by `timeout_ms`.
- **A gated stream (`bearer:`)**: a download or a feed behind a token takes the vault credential itself, `std::http::BodyReader { req: ..., bearer: self.key }` (or on `SseReader`), and sends it as `Authorization: Bearer <token>`. The credential writes that header line onto the wire on its sealed side, so the token is never a `String` the program can read, deferred reader or not. A credential that resolves to nothing fails the opening with `send_failed` and nothing is sent. `hale check` refuses a `reveal_text()` into the `req` of a reader built with `deferred`, which would keep the header readable in `req` until the opening.
- **Redirects (`follow_redirects`)**: a reader hands a 3xx back as it is unless its request sets `follow_redirects: n`, the most redirects it follows (a download URL that answers 302 to a CDN, say). The reader's `status` and `headers` are then the final target's. It follows the same scheme or an upgrade to https, never https to http (`err.kind == "redirect_refused"`), and fails `too_many_redirects` past `n`. When the redirect moves to another origin, the `Authorization` header and the `bearer` are not sent there. The whole-body `get`/`post`/`request` calls still return a 3xx as it is.
- When the stream finishes, `event()` / `chunk()` fails with `err.kind == "end"` on clean completion or `"recv_failed"` on an interrupted connection or deadline expiry. `timeout_ms` defaults to `30000` (set `timeout_ms: 0` for long-lived streams).

### End-to-End Example: SSE Relay

Combining `SseReader` and `SseWriter` creates a streaming proxy that inspects the upstream status before forwarding events as they arrive:

```hale
locus Relay {
    params {
        upstream: String = "https://models.local/v1/chat/completions";
    }
    fn handle(req: std::http::Request) -> std::http::Response {
        let url = std::http::parse_url(self.upstream) or std::http::Url { };
        let up = std::http::SseReader {
            req: std::http::ClientRequest {
                method: "POST",
                url: url,
                headers: "Content-Type: application/json",
                body: std::bytes::from_string(req.body),
            },
            timeout_ms: 0,
        };
        if up.body.status != 200 {
            return std::http::Response {
                status: 502,
                body: "upstream answered " + to_string(up.body.status)
            };
        }
        let w = std::http::SseWriter { conn_fd: req.conn_fd };
        while true {
            let e = up.event() or { break; };
            w.event(e) or { break; };
        }
        return w.end();
    }
}
```

Next: [UDP & TLS](./sockets.md).
