# Libraries (pond)

The standard library covers the substrate — I/O, time, strings,
JSON, HTTP, crypto, the bus. Everything else — web stacks,
databases, observability — lives in **pond**, the contributed
library catalog: <https://github.com/hale-lang/pond>.

*Many lotus grow in a pond.* Each library is a directory of `.hl`
loci you vendor into your project.

## Using one

Declare it in `hale.toml`, fetch it, import it:

```toml
[deps]
pond = { git = "https://github.com/hale-lang/pond", tag = "v0.1.0" }
```

```sh
hale fetch
```

```hale
import "vendor/pond/router" as router;
```

`hale fetch` clones each dependency into `vendor/<name>/` and
pins the resolved commit in `hale.lock`. Pond's "no transitive
dependencies in v1" rule means every package your program pulls
in is visible in your lockfile — if a library uses another, you
vendor both explicitly.

## The catalog

**Persistence & data**

| Library | Provides |
|---|---|
| `db` | Driver-agnostic database surface: the `DbDriver` interface + `Args` bind-parameter list for parameterized (`$1, $2, …`) queries. Pick a backend (`pq`, `sqlite`) at the `DbDriver` slot. |
| `pq` | PostgreSQL driver — `PgConn` plus `PgPool`, a fixed-size fd connection pool that itself satisfies `db::DbDriver`. |
| `sqlite` | SQLite connection + fallible query surface. |
| `migrations` | Schema migration runner (up/down); builds to a `migrate` binary. |
| `jobs` | SQLite-backed job queue (`Queue`) + a pinned-worker pool. |

**Web**

| Library | Provides |
|---|---|
| `http` | HTTP client (`http/client`) over `std::io` — request/response building atop the socket primitives, for libraries that need an HTTP client without the full `std::http` server surface. |
| `router` | HTTP router over `std::http` — method + path-param routes, middleware chain. |
| `sessions` | Stateless, HMAC-signed cookie sessions (`session=<base64(payload)>.<base64(hmac)>`). |
| `websocket` | Synchronous, owner-driven RFC 6455 WebSocket client (suggested alias `ws`); a passive wrapper your own `run()` loop drives. |

**Observability & supervision**

| Library | Provides |
|---|---|
| `logfmt` | Alternative `std::log` sinks wearing the `std::text::Sink` shape — file with rotation, structured output. |
| `metrics` | Counter / gauge / histogram primitives + a Prometheus text-format renderer and `/metrics` endpoint. |
| `tracing` | Span tree mirroring the locus tower — one `Tracer` per app; spans nest with locus instantiation. |
| `supervisor` | Erlang/OTP supervision-tree strategies grafted onto Hale's `on_failure` + `restart` / `restart_in_place` / `bubble`. |

**Primitives & composition**

| Library | Provides |
|---|---|
| `crypto` | SHA-256, HMAC-SHA256, hex encode/decode, constant-time compare, CSPRNG. |
| `subprocess` | Spawn + manage child processes (suggested alias `sub`) — wraps the `std::process` spawn / wait / pipe primitives. |
| `tower` | Run several independent locus trees ("towers") under one process, each with its own root and lifecycle. |

**Terminal & UI**

| Library | Provides |
|---|---|
| `term` | Tier-0 terminal infrastructure — capability/`is_tty` probes, SGR styling, raw-mode guard, cursor + screen control over `std::term`. |
| `tui` | An Elm-shaped TUI runtime: write a locus with model/update/view, the runtime drives the frame loop, input, and rendering. |

**AI & numeric**

| Library | Provides |
|---|---|
| `agent` | LLM-agent toolkit — `agent/{llm, tools, conversation, embeddings, sandbox}`: a client surface, a tool-registry, conversation state, and a sandboxed execution path. |
| `ml` | Neural-network primitives (`ml/neural`). |
| `math` | Numeric helpers — `math/{matrix, stats}`. |

> `heron` (the tree-sitter grammar that drives editor tooling) also
> lives in pond, but it's developer tooling, not a vendored runtime
> library you `import`. The `_util` directory holds internal helper
> libs consumed by other pond libs, not imported directly by apps.

Pond is where the ecosystem grows: if a protocol, parser, or
shape is too useful to rewrite per project but doesn't belong in
the language, it lands here.
