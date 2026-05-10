# The Aperio Standard Library — Roadmap

The stdlib's v1.x build-out unfolds in five phases. Phases 1
through 5 are sealed as of m93. Phase 6 (the substrate plan
forced by the Aperio IDE) is the next arc.

For "what can I actually use today," see
[What you can build today](./ready-today.md).

## Phase 1 — Foundations (sealed, m71 → m76)

- `std::io::tcp` — multi-accept Listener + Stream send/recv.
  Substrate: `lotus_tcp_*` over AF_INET with internal length-prefix
  framing.
- `std::io::fs` — `read_file`, `write_file`, `file_exists`,
  `file_size`, plus `read_bytes` (m89) and `list_dir` (m90).
- `std::env` — `args_count`, `arg`, `var`, `var_exists`.
- `std::str` — `parse_int`, `can_parse_int`, `index_of` (m84).
- `std::time` — `sleep`, `monotonic` (under the `std::*` namespace).
- `std::process` — `pid`, `exit`.

Magic `std::*` path resolver in codegen; no module system.

## Phase 2 — Test framework (v0.1 sealed, m87 → m88)

- `std::test::assert(cond, msg)`
- `std::test::assert_eq_int(actual, expected, msg)`
- `std::test::assert_eq_str(actual, expected, msg)`

Test-runner contract: pass = exit 0 silent; fail = non-zero +
`"ASSERTION FAILED: …"` on stdout. Sufficient for self-tests
written in Aperio that drive real Aperio behavior.

**v1.0 still on the table:** `aperio test` CLI runner,
`assert_rejects` (compile-error tests), `assert_closure` (closure
introspection), `assert_neq_*` siblings, benchmarks. Property-based
testing explicitly deferred per spec.

## Phase 3 — HTTP server (sealed, m83 → m86)

- `std::http::Request` and `std::http::Response` — record types.
- `std::http::parse_request(raw) -> Request`
- `std::http::write_response(stream, response)`
- Multi-accept TCP Listener composes via `on_connection: fn(Stream)`.

**v1.0 still on the table:** header-map type, `Connection: keep-alive`,
streaming bodies > 8 KB, listener bind-readiness primitive,
content-type-by-extension dispatch.

## Phase 4 — Text processing (v0.1 sealed, m91)

- `std::text::md_to_html(md) -> String` — block-level markdown:
  ATX headings, paragraphs joined by single space, fenced code
  blocks, HTML escape (`&` / `<` / `>`).

**v1.0 still on the table:** inline formatting (`**bold**`,
`*italic*`, `` `code` ``, `[text](url)`).

## Phase 5 — Synthesis (sealed, m92)

`examples/docs-server/main.ap` — ~200 lines of Aperio composing
seven stdlib namespaces into a real HTTP server that lists and
renders markdown docs from a configured directory. Demonstrates
the stdlib end-to-end.

## Phase 6 — Substrate for the IDE (m94 onwards, planned)

Forcing function: an Aperio IDE that renders the locus tree of
running programs. Each milestone exposes a `std::*` namespace
useful to every Aperio program, not just the IDE.

| #     | Name                          | Surface                                                              |
| ----- | ----------------------------- | -------------------------------------------------------------------- |
| m94   | Filesystem watch              | `std::fs::watch::{create, next, close}`                              |
| m95   | Runtime debug instrumentation | Runtime publishes `lotus.debug.*` when `APERIO_DEBUG` env-enabled    |
| m96   | 3D graphics                   | `std::graphics::{scene, camera, mesh, transform, animate}`           |
| m97   | UI panels                     | `std::ui::{panel, label, button, text_input, ...}`                   |
| m98   | Embedded shell                | `std::shell::{pty_spawn, pty_read, pty_write, terminal_widget}`      |
| m99   | MCP server                    | `std::mcp::{serve_http, tool, register_tool}`                        |
| m100  | Compiler self-introspection   | `std::aperio::{parse, typecheck, ast_walk}`                          |

Detailed plan: `notes/aperio-ide-design.md` (in the repo).

## How priorities are set

The next milestone is chosen from friction signals — moments where
real Aperio programs hit something missing. Sources:

- `notes/aperio-friction.md` — global friction log.
- `apps/<name>/FRICTION.md` — per-app logs.
- The IDE design plan's own pre-emptive friction list (above).

If you are an agent session writing an Aperio program, your
friction log is the prioritization signal. See
`notes/agent-onboarding/app-dev-brief.md`.
