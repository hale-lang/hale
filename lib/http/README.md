# lib/http — vendored snapshot of pond/http/client

Vendored from `pond/http/client` at iris Phase 4 landing.
Used by `AnthropicBackend` for HTTPS POSTs to the Anthropic
Messages API.

This is a fork — no upstream sync. If `pond/http/client` gets
useful changes, cherry-pick by hand.

## Surface (per pond's README)

```hale
import "lib/http" as http;

let r = http::get("https://example.com/")              or raise;
let r = http::post(url, body, content_type)            or raise;
let r = http::request(http::Request { … })             or raise;

// Or with the long-lived Client (retry + connection pool):
let c = http::Client { };
```

## Files

- `types.hl` — `Url`, `Request`, `Response`, `HttpError` types
- `url.hl` — URL parsing
- `wire.hl` — HTTP/1.1 wire format (request / response framing)
- `client.hl` — `get` / `post` / `request` free fns + `Client` locus

## Known limitation vs. iris's eventual needs

**Synchronous request-response only.** No streaming receive
for SSE. Anthropic responses arrive in full once the body
completes; mid-stream proposal extraction isn't possible
through this surface. Phase 4 ships against the non-streaming
shape; SSE upgrade is future work (needs an extension to
this library *or* a thin custom SSE layer over
`std::io::tls::*` directly).
