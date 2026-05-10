# The Aperio Standard Library — Roadmap

Aperio's stdlib is in active development as the v1.x build-out. The doc
server you're reading these pages on (eventually — currently mdbook
serves them) will be one of the first programs that synthesizes the
stdlib end-to-end.

## Phases

### Phase 1: Foundations

- `std::io::tcp` — TCP listen / connect / accept / send / recv. Extends
  the substrate transport from AF_UNIX to AF_INET / AF_INET6.
- `std::io::fs` — read_file, read_dir, file_size, file_exists.
- `std::time` — extends `time::sleep` / `time::monotonic` with `time::now`,
  formatting, parsing.

### Phase 2: Test framework

- `std::test` — assertions, test runner (`aperio test path/`).
- `std::test::fake` — fake time, fake bus, fake fs.
- `std::test::trace` — record/replay bus events for regression tests.

### Phase 3: HTTP server

- `std::http::request` — parse request line + headers.
- `std::http::response` — build response (status, headers, body).
- `std::http::server` — accept loop on a TcpListener.
- `std::http::router` — path-pattern dispatch.

### Phase 4: Text processing

- `std::text::markdown` — CommonMark subset parser.
- `std::text::html` — escape, build, pretty-print.
- `std::text::highlight` — per-language syntax highlighter.

### Phase 5: Synthesis

- `examples/docs-server/main.ap` — the Aperio doc server, written in
  Aperio, serving the Aperio docs.

## Status

Currently: **placeholder.** No stdlib code has shipped yet. This page
exists so SUMMARY.md resolves cleanly under `mdbook build`. Each Phase
above is its own multi-milestone arc with its own plan.
