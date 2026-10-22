# Package Management & `pond` Catalog

Hale's built-in standard library (`std::*`) covers core I/O, networking, HTTP, JSON, cryptography, logging, metrics, and the message bus. Ecosystem libraries—database drivers, migrations, NATS adapters, WebSockets, TUI frameworks, and LLM agent toolkits—live in **[`pond`](https://github.com/hale-lang/pond)**, the official contributed library repository.

---

## 1. Vendoring Dependencies (`hale.toml` & `hale fetch`)

Declare git dependencies in `hale.toml` pinned to a commit SHA:

```toml
[deps]
pond = { git = "https://github.com/hale-lang/pond", rev = "<commit-sha>" }
```

> [!TIP]
> Always pin `pond` to a recent commit SHA rather than the legacy `v0.8.0` tag; `pond` tracks the compiler closely.

Run `hale fetch` to clone each dependency into `vendor/<name>/` and record the exact commit in `hale.lock`:

```sh
hale fetch
```

- **No hidden transitive resolution:** Every external repository your project uses is listed explicitly in `[deps]` and locked in `hale.lock`.
- **Importing sub-libraries with native link flags:** When a `pond` library depends on another `pond` library that carries `[ffi]` link settings in its own `hale.toml` (for example, `pond/jobs` uses `pond/sqlite`), import both in your program so the compiler picks up `sqlite`'s link flags.

---

## 2. Importing Libraries (`import ... as ...;`)

Import a vendored or local seed directory using `import "<path>" as <alias>;`:

```hale,fragment
import "vendor/pond/sqlite" as sqlite;

main locus App {
    run() {
        let db = sqlite::Db { path: "app.db" };
    }
}
```

### Resolution & Scoping Rules

| Rule | Behavior |
|---|---|
| **Check/build the directory, not a single file** | In a multi-file seed, run `hale check .` or `hale build .`. Checking a single file (`hale check other.hl`) compiles that file standalone without sibling files' `import` declarations. |
| **Directory vs. file imports** | `import "../lib" as lib;` and `import "../lib/main" as lib;` both import the entire `../lib/` seed directory as one library. Importing any other `.hl` file (`import "../lib/helper" as h;`) imports that single file standalone. You cannot import both a directory and an individual file inside that same directory in one program. |
| **Aliases are seed-local namespaces** | An import alias (`sqlite::Db`) is a namespace, not a value binding: a function named `fn core()` and `import "../core" as core;` do not collide (`core::fn_name()` vs `core()`). However, keep aliases `snake_case` so they never collide with `PascalCase` `type` or `enum` names (`Type::Variant` takes precedence over `alias::Symbol`). |
| **Type identity across aliases** | Importing the same library under two aliases (`a` and `b`) refers to the exact same underlying types (`a::Row` is type-compatible with `b::Row`). |
| **Imported `main locus` is inert** | If an imported library directory contains its own `main locus` and `fn main()`, importing that library does **not** activate its `main locus`, `bindings`, or `placement`. Your application can still declare its own `main locus`. |
| **Stdlib requires `std::`** | Built-in standard library modules are called directly via `std::*` (e.g., `std::env::args_count()`, `std::time::sleep(1s)`) with no `import` statement. Omitting `std::` produces a compile error suggesting the `std::` prefix. |

---

## 3. The `pond` Library Catalog

All libraries below are imported from `vendor/pond/<path>` after running `hale fetch`.

### Persistence & Databases

| Import Path | Description |
|---|---|
| `vendor/pond/db` | Driver-agnostic database interface (`DbDriver`) and parameterized query builder (`Args` for `$1, $2, ...` placeholders). |
| `vendor/pond/pq` | PostgreSQL driver (`PgConn`) and fixed-size connection pool (`PgPool`) implementing `db::DbDriver`. |
| `vendor/pond/sqlite` | SQLite 3 connection and fallible query surface implementing `db::DbDriver`. |
| `vendor/pond/migrations` | Schema migration runner (`Migrator`) over any `db::DbDriver` (`up`, `down`, `steps`, `goto`). |
| `vendor/pond/jobs` | Persistent SQLite-backed job queue (`Queue`) and pinned worker pool (`Pool`, `Worker`). |

### Web, HTTP & Realtime Messaging

| Import Path | Description |
|---|---|
| `vendor/pond/sessions` | Stateless HMAC-signed cookie sessions (`session=<base64(payload)>.<base64(hmac)>`). |
| `vendor/pond/websocket` | RFC 6455 WebSocket client and server upgrade handler with ping/pong liveness deadlines. |
| `vendor/pond/realtime/nats` | NATS Core and JetStream client plus a `std::bus` transport adapter for cross-process topic delivery with at-least-once semantics. |
| `vendor/pond/http` | Legacy HTTP client pool (superseded by built-in `std::http`). |
| `vendor/pond/router` | Legacy HTTP router (superseded by built-in `std::http::Router`). |

### Observability, Supervision & OS Processes

| Import Path | Description |
|---|---|
| `vendor/pond/logfmt` | Additional `std::log` sinks implementing `std::text::Sink`, including `OtlpSink` (console and file sinks are built into `std::log`). |
| `vendor/pond/tracing` | Hierarchical span tracer (`Tracer`) that automatically mirrors the locus ownership tree. |
| `vendor/pond/supervisor` | Erlang/OTP supervision strategies built on top of Hale's `on_failure` (`restart`, `restart_in_place`, `bubble`). |
| `vendor/pond/subprocess` | Child process manager wrapping `std::process::run` and `spawn` with piped stdio and timeouts. |
| `vendor/pond/crypto` | Additional cryptographic helpers complementing `std::crypto`. |
| `vendor/pond/metrics` | Legacy Prometheus metrics registry (superseded by built-in `std::metrics` and `std::observe::Scrape`). |

### Terminal UI, AI Agents & Scientific Computing

| Import Path | Description |
|---|---|
| `vendor/pond/term` | Terminal capability detection (`is_tty`), ANSI SGR styling, raw-mode guard, and cursor/screen control over `std::term`. |
| `vendor/pond/tui` | Elm-architecture terminal UI runtime (model / update / view locus driven by a frame loop). |
| `vendor/pond/agent` | LLM agent toolkit (`agent/llm`, `agent/tools`, `agent/conversation`, `agent/embeddings`, `agent/sandbox`). |
| `vendor/pond/ml` | Neural network primitives (`ml/neural`). |
| `vendor/pond/math` | Matrix algebra and statistical functions (`math/matrix`, `math/stats`). |

> **Editor Grammar:** The Tree-sitter grammar for syntax highlighting lives in its own repository at [github.com/hale-lang/tree-sitter-hale](https://github.com/hale-lang/tree-sitter-hale).
