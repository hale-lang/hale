# HTTP

> **Coming from Python / Node?** This is your Flask / Express
> moment — but instead of decorators or a routes table, you write
> a **handler locus**: a locus with a `handle(req) -> Response`
> method. `std::http::Server` runs the accept loop and calls your
> handler per request. Routing is a `match` on the path inside
> `handle`. (A fuller router with path params lives in the `pond`
> library catalog.)

## A server

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
    // A statement-position locus literal fires its lifecycle: this
    // runs the accept loop until the process is stopped.
    std::http::Server { port: 8080, handler: Api { } };
}
```

`hale build` it, run it, and `curl localhost:8080/health`. The
handler's `params` persist across requests — `self.hits` counts
them — because the `Api` locus is alive for the whole run.

## The pieces

- **`std::http::Request`** carries `method`, `path`, `version`,
  `body`, and headers (looked up case-insensitively). For a couple
  of routes you `match` / `if` on `method` and `path` right in the
  handler; past that, use the Router (below).
- **`std::http::Response`** needs at least `status` and `body`;
  `content_type` defaults to `text/plain`, and you can add custom
  `headers`.
- **`std::http::Server`** takes a `port` and a `handler`, then
  owns the listen-accept-parse-dispatch loop. `max_accepts: N`
  bounds it to N requests (handy for tests); the default runs
  until stopped. POST bodies are reassembled per `Content-Length`,
  so clients that write headers and body in separate segments
  (python's urllib, for one) work; requests are capped at 1 MiB
  (oversized declared bodies get a `413`).

## Routing

When the `if` ladder in a handler grows past a few routes, hand
the routing to `std::http::Router`: register patterns, get path
captures and query params extracted, and mount the router as the
server's handler —

```hale
locus Hello {
    fn handle(ctx: std::http::Context) -> std::http::Response {
        let who = std::http::path_param(ctx.params, "name");
        return std::http::Response { status: 200, body: "hi " + who };
    }
}

fn build_router() -> std::http::Router {
    let r = std::http::Router { };
    r.add("GET", "/hello/:name", Hello { });
    return r;
}

fn main() {
    std::http::Server { port: 8080, handler: build_router() };
}
```

`:name` segments capture (`path_param`), `?k=v` pairs are one
`query_param(ctx.params, "k")` away, the first matching route
wins (register specific patterns before general ones), and
anything unmatched hits an overridable 404. Middleware wraps the
chain in onion order:

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
// r.use(Cors { });
```

Each route's handler is its own locus, so per-route state lives
where it belongs (a `hits` counter on the route that counts, not
on a god-handler).

## A first taste of interfaces

How does `Server` know `Api` is a valid handler? `Server`'s
`handler` field has the type `std::http::Handler`, which is an
**interface** — a named set of required methods:

```hale
// (declared in the standard library)
interface Handler {
    fn handle(req: Request) -> Response;
}
```

Any locus that *has* a matching `handle` method satisfies
`Handler` — automatically, with no `implements` clause to write.
This is *structural* satisfaction: the shape is the contract. You
declared `Api` with the right method, so it's a `Handler`. (Go
programmers will recognize this; it's interfaces without the
`impl` ceremony.)

## Taking over the connection

Some protocols start as HTTP and then stop being HTTP — WebSocket
is the famous one. For those, a handler can *take over* the raw
connection instead of finishing the request/response cycle:

```hale
locus WsHandler {
    params { }
    fn handle(req: std::http::Request) -> std::http::Response {
        // req.conn_fd is the live socket. Hand it to whoever
        // will own the session (typically: publish it to a
        // session locus on its own pool).
        "ws.conn" <- Conn { fd: req.conn_fd };
        return std::http::Response {
            status: 101,
            headers: "Upgrade: websocket\r\nConnection: Upgrade",
            body: "",
            takeover: true
        };
    }
}
```

With `takeover: true` the server writes just the status line and
your headers — no `Content-Length`, no `Connection: close` — and
then *leaves the socket open and forgets it*. It's yours: read
and write it through the raw-fd `std::io::tcp` functions or a
borrowed `Stream { conn_fd: fd, owns_fd: false }`, and close it
when the session ends. Two things to remember: the server's 5s
receive timeout is still set on the fd (clear it with
`std::io::tcp::set_recv_timeout(fd, 0)` for a long-lived
session), and a takeover response without stashing `req.conn_fd`
leaks the connection.

## Calling out

Outbound requests are one call:

```hale
let resp = std::http::get("https://example.com") or raise;
println(std::str::from_bytes(resp.body));

let posted = std::http::post("http://api.local/things",
    std::bytes::from_string("{\"n\": 1}"), "application/json") or raise;
```

Both are `fallible(std::http::HttpError)` — address the error with
`or raise` / a substitute / an error-check fn, and branch on
`err.kind` (`connect_failed`, `bad_url`, …) when it matters.
Response bodies are **Bytes** (binary-safe; `std::str::from_bytes`
when you know it's text). For repeated calls to the same host,
`std::http::Client { keep_alive: true }` pools connections and
retries with backoff:

```hale
let c = std::http::Client { keep_alive: true, max_retries: 2 };
let r = c.get("http://api.local/health") or raise;
```

One placement note: **https calls block their thread** (TLS has no
async_io integration yet) — keep loci that make them on `pinned`
or an ordinary cooperative pool, not an `async_io` one.

That import line, the `bindings` that wire a server across
processes, and the lifecycle that lets a server shut down cleanly
on Ctrl-C are all next-level topics — but the handler you wrote
above doesn't change when you get there. The server *code* is
already complete; the surrounding tier just gives it more ways to
be deployed and supervised.

Next: the transports below HTTP — [UDP & TLS](./sockets.md).
