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
  `body`, and headers (looked up case-insensitively). You
  `match` / `if` on `method` and `path` to route.
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

## Calling out

The standard library ships the server. For an HTTP *client* —
making outbound requests, with connection pooling and TLS — reach
for the `http/client` library in [pond](../libraries.md):

```hale
import "vendor/pond/http/client" as http;
// let resp = http::get("https://example.com") or raise;
```

That import line, the `bindings` that wire a server across
processes, and the lifecycle that lets a server shut down cleanly
on Ctrl-C are all next-level topics — but the handler you wrote
above doesn't change when you get there. The server *code* is
already complete; the surrounding tier just gives it more ways to
be deployed and supervised.

Next: the transports below HTTP — [UDP & TLS](./sockets.md).
