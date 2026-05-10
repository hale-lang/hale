# `apps/`

Production-shape Aperio programs, peer to `examples/`.

`examples/` is for pedagogical fragments — one concept per
file, ordered to scale up the language gradually. `apps/` is
for real programs that exercise the stdlib end-to-end.

The first app of this kind is the docs server (currently still
under `examples/docs-server/` for legacy reasons; will move
when convenient). The IDE, when its substrate ships, will live
here as `apps/aperio-ide/`.

## Adding a new app

1. Read `notes/agent-onboarding/app-dev-brief.md` first if you
   are an agent session.
2. Create `apps/<your-app-name>/` with at least:
   - `main.ap` — single source file (multi-file modules
     unshipped).
   - `README.md` — what it does, how to run it, what it
     doesn't do yet.
   - `FRICTION.md` — append-only log of moments the language
     got in your way. See the brief or
     `notes/aperio-friction.md` for the format.
3. Build and run via `aperio run apps/<your-app>/main.ap`.
4. End-to-end tests for the app go in
   `crates/aperio-codegen/tests/<your_app>.rs`, mirroring the
   pattern in `tests/docs_server.rs` and `tests/http_hello.rs`.

## What apps belong here

- Any non-trivial Aperio program that ships as a working
  artifact (server, daemon, CLI, viewer).
- Programs that compose three or more stdlib namespaces.
- Programs the project itself uses (the IDE, eventually a
  package browser, etc.).

What stays in `examples/`:

- One-concept demos (one stdlib feature, one language
  primitive).
- Onboarding-path code referenced from the book.
- Anything pedagogical first, useful second.
