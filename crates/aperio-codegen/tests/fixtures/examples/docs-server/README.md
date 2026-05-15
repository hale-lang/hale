# docs-server

Phase 5 capstone — a real, end-to-end HTTP server in Aperio
that lists and renders markdown files from a configurable
directory.

This is the program every prior milestone was building toward:

| Milestone | Contribution                                           |
|-----------|--------------------------------------------------------|
| m71-m76   | Stdlib substrate: TCP, fs, env                         |
| m80       | First-class function values (`fn(T) -> R`)             |
| m81-m83   | Stream + Listener + multi-accept callbacks             |
| m84-m86   | HTTP request parsing + response writing                |
| m89       | `Bytes` type for binary I/O                            |
| m90       | `list_dir` for index enumeration                       |
| m91       | Markdown → HTML rendering                              |

Composed: an Aperio program that does real work — a docs
viewer you can `curl` against — in roughly 200 lines of code.

## Run

```
cargo run --bin aperio -- run examples/docs-server/main.ap
# Custom port + docs dir:
cargo run --bin aperio -- run examples/docs-server/main.ap 9000 ./spec
# Bounded accepts (useful for tests):
cargo run --bin aperio -- run examples/docs-server/main.ap 8080 ./spec 5
```

Then in another terminal:

```
curl http://127.0.0.1:8080/             # index
curl http://127.0.0.1:8080/types.md     # render one file
```

## Routes

| Path       | Behavior                                              |
|------------|-------------------------------------------------------|
| `GET /`    | HTML index of `.md` files in the docs directory.     |
| `GET /X.md`| Render `docs_dir/X.md` to HTML in a styled shell.    |
| anything else | 404 Not Found.                                    |

## Path safety

The server rejects:

- Paths containing `..` (basic traversal protection).
- Nested paths like `/a/b.md` — only basenames within the
  configured directory.
- Anything not ending in `.md` doesn't appear in the index.

## v0 limits

- One `recv` per request — assumes the request fits in 8 KB.
  Real-world browser GETs are well within that.
- `Connection: close` hardcoded; no keep-alive.
- Single-threaded — each request handled before the next
  accept.
- Markdown supports headings, paragraphs, fenced code blocks,
  HTML escaping. Inline formatting (bold/italic/links) lands
  in m92.
- Static `.md` files only. Binary assets would compose with
  m89's `Bytes` + a content-type-by-extension dispatch.

## Why this is the capstone

Every Aperio invariant gets exercised:

- Locus lifecycle (m82) — the per-connection Stream dissolves
  cleanly between accepts so fds don't bleed.
- Function pointers (m80) — `on_connection: fn(Stream)` is
  how the user-supplied handler reaches the accept loop.
- Stdlib composition — the handler is 30 lines of Aperio
  composing seven distinct stdlib namespaces (`std::http::*`,
  `std::io::tcp::*`, `std::io::fs::*`, `std::env::*`,
  `std::str::*`, `std::text::*`, plus core builtins).
- Locus-all-the-way-down — every helper is plain Aperio,
  not a special-case primitive.

The same code shape that handles three connections in a test
keeps working when the doc server runs forever.
