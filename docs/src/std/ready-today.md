# What you can build today

A capability-tier overview of the Aperio stdlib surface as of
m93. This page is the entry point for app-dev sessions and
should be skimmed before opening any other reference page.

Tiers:

- **Shipped** — usable now, has a reference page, has tests.
- **Workaround** — not directly supported, but a reasonable
  pattern exists in current source.
- **Blocked** — needs a future milestone; do not attempt yet.
  File a friction entry if your work needs it.

## I/O

| Capability | Tier | Notes |
|---|---|---|
| Read entire file | Shipped | `std::io::fs::read_file(path) -> String` |
| Write entire file | Shipped | `std::io::fs::write_file(path, content) -> Int` |
| File exists / size | Shipped | `file_exists`, `file_size` |
| Read raw bytes | Shipped | `std::io::fs::read_bytes(path) -> Bytes` (m89) |
| List directory entries | Shipped | `std::io::fs::list_dir(path) -> String` (newline-separated; m90) |
| Append to file | Workaround | `read_file` + concat + `write_file`. No O_APPEND surface. |
| Stream / line-by-line read | Blocked | No streaming `Reader` type yet. |
| Filesystem watch (inotify) | Blocked | m94 (planned). |
| `errno` disambiguation | Blocked | All errors collapse to `-1` / `false` / `""`. |

## Networking

| Capability | Tier | Notes |
|---|---|---|
| TCP listener (multi-accept) | Shipped | `std::io::tcp::Listener { host, port, max_accepts, on_connection }` (m83) |
| TCP stream send / recv | Shipped | `Stream.send(s)`, `Stream.recv(n)` (m81) |
| TCP send raw bytes | Shipped | `Stream.send_bytes(b)` (m89) |
| HTTP request parse | Shipped | `std::http::parse_request(raw) -> Request` (m84) |
| HTTP response write | Shipped | `std::http::write_response(stream, resp)` (m85) |
| Custom HTTP headers | Blocked | No header-map type. Phase 3 v1.0 follow-up. |
| `Connection: keep-alive` | Blocked | Each request closes. Phase 3 v1.0 follow-up. |
| HTTP bodies > 8 KB | Blocked | Single recv assumed. Phase 3 v1.0 follow-up. |
| TLS / HTTPS | Blocked | No TLS substrate. |
| UDP | Blocked | TCP only in v0. |
| WebSocket | Blocked | Not on the roadmap. |

## Process / environment

| Capability | Tier | Notes |
|---|---|---|
| Read argv | Shipped | `std::env::args_count()`, `std::env::arg(i)` |
| Read env vars | Shipped | `std::env::var(name)`, `std::env::var_exists(name)` |
| Process pid | Shipped | `std::process::pid()` |
| Process exit | Shipped | `std::process::exit(code)` |
| Spawn subprocess | Blocked | No `std::process::spawn` yet. |
| Embedded shell / PTY | Blocked | m98 (planned, IDE-driven). |

## Strings, numbers, bytes

| Capability | Tier | Notes |
|---|---|---|
| String length | Shipped | `len(s)` (bare-name builtin) |
| String concat | Shipped | `+` operator |
| Substring search | Shipped | `std::str::index_of(s, sub)` (m84) |
| Prefix / contains | Shipped | `starts_with(s, prefix)`, `contains(s, sub)` (bare-name) |
| Parse Int | Shipped | `std::str::parse_int(s)` + `can_parse_int` for disambiguation (m78) |
| Int to String | Shipped | `to_string(n)` (bare-name builtin) |
| Float to String | Shipped | `to_string(f)` (bare-name builtin) |
| Bytes type / length | Shipped | `Bytes` primitive + `len(b)` (m89) |
| Slice / split | Workaround | Hand-roll using `index_of` + char loop. No native split yet. |
| Regex | Blocked | No regex library. |
| Decimal arithmetic | Shipped | `Decimal` type with `1.50d` literals |

## Text / rendering

| Capability | Tier | Notes |
|---|---|---|
| Markdown → HTML (block-level) | Shipped | `std::text::md_to_html(md)` — ATX headings, paragraphs, fenced code, HTML escape (m91) |
| Markdown inline (bold/italic/code/links) | Blocked | Phase 4 v1.0 follow-up. |
| HTML escape | Shipped | Embedded in `md_to_html`; no standalone path-call yet. |
| Syntax highlighting | Blocked | Not on the roadmap. |

## Time

| Capability | Tier | Notes |
|---|---|---|
| Sleep | Shipped | `std::time::sleep(duration)` accepts `100ms`, `5s`, etc. |
| Monotonic clock | Shipped | `std::time::monotonic() -> Time` |
| Wall clock / formatting / parsing | Blocked | Phase 1 follow-up. |
| Time literals | Shipped | `` `2026-05-08T12:00:00Z` `` syntax |

## Testing

| Capability | Tier | Notes |
|---|---|---|
| Boolean assertion | Shipped | `std::test::assert(cond, msg)` (m87) |
| Int / String equality assertions | Shipped | `assert_eq_int`, `assert_eq_str` (m87) |
| Test runner CLI (`aperio test`) | Blocked | Phase 2 v1.0. Use `cargo test -p aperio-codegen` for now. |
| Compile-error tests | Blocked | `assert_rejects` not shipped. |
| Closure introspection asserts | Blocked | Phase 2 v1.0. |
| Property-based tests | Blocked | Explicitly deferred per spec. |
| Benchmarks | Blocked | Phase 2 v1.0 layer 3. |

## Logging

| Capability | Tier | Notes |
|---|---|---|
| Structured logging on the bus | Shipped | `std::log::Logger`, `std::log::LogEvent`, `std::log::StdoutSink` (m95) |
| Cascading namespaces | Shipped | `Logger { name, parent_path }` → publishes on `log.<full_path>` |
| Subtree-scoped subscribers | Shipped | `subscribe "log.app.**"` matches root + descendants (m94) |
| Bus subject wildcards (`**`) | Shipped | Trailing-only; matches zero+ remaining segments (m94) |
| Cross-process tailing from source | Blocked | `std::bus::expose` not yet shipped; substrate exists (m72) |
| WARN/ERROR → stderr | Blocked | No `eprintln` primitive yet. |
| Structured fields beyond `msg` | Blocked | Needs generic `Map` or fixed tuple array. |
| Log-level filtering at default sink | Blocked | `StdoutSink` prints everything; write a custom sink with `if e.level >= 2`. |

## Language-level

| Capability | Tier | Notes |
|---|---|---|
| Locus lifecycle (birth / accept / run / drain / dissolve) | Shipped | Core language since m1. |
| Bus pub/sub (typed subjects) | Shipped | Single-process and TCP transports. |
| Cross-process bus | Shipped | TCP framing in `lotus_tcp_*` (m72). |
| Closures (closure { ... } blocks) | Shipped | epoch / within / approx vocabulary. |
| Function pointers (`fn(T) -> R`) | Shipped | m80; required for Listener.on_connection. |
| Let-bound locus dissolve at scope-exit | Shipped | m82. |
| Function returns a locus | Blocked | Dissolve fires before caller binds. Language paper-cut. |
| Recovery primitives (bubble / restart / quarantine) | Shipped | Per spec recovery surface. |
| Schedule classes (cooperative / pinned) | Shipped | Per spec scheduling surface. |
| Generics (`List<T>`, `Map<K,V>`) | Blocked | Reserved keywords; no semantics yet. |
| Sum types in payloads / variant patterns | Blocked | Workaround: one bus subject per variant. |
| Multiple distinct accept types per locus | Blocked | One accept type per locus today. |
| Multi-file modules / `import` | Blocked | Single `main.ap` per program. |
| Trait / impl | Blocked | Reserved keywords; no semantics. |
| Async / await | Blocked | Reserved keywords; no semantics. Concurrency via loci + bus. |

## Apps that ship today (no new substrate)

These shapes are all buildable now:

- HTTP services (file/CRUD viewer, JSON API, doc browser — see
  `examples/docs-server/main.ap`).
- Static site generators (read `.md` → render → write `.html`).
- File-backed CLIs (env::args + io::fs + process::exit).
- TCP bus consumers / aggregators on custom subjects.
- Text / transformation tools (markdown block-level, str
  primitives).
- HTTP-fronted utilities (lint runners, log viewers, etc.) that
  shell out via existing tools.

## Apps blocked on substrate

Listed with the milestone that unblocks them:

- IDE / structural visualizer — m96 (graphics) onwards.
- Filesystem-watching dev servers — m94.
- Cross-program runtime observers — m95
  (`lotus.debug.*` instrumentation).
- HTTP keep-alive servers, header-rich APIs — Phase 3 v1.0.
- Multi-file Aperio programs of any kind — module support.
- MCP-tool-providing services — m99.

## Friction is a feature

If your app hits a tier "Blocked" or "Workaround" entry, log it
in `apps/<your-app>/FRICTION.md`. The compiler session reads
those logs and uses them to pick the next milestone. Your
friction is not noise; it is the prioritization signal. See the
agent-onboarding brief at
`notes/agent-onboarding/app-dev-brief.md` for the friction-log
format.
