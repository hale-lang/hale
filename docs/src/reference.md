# Language & CLI Reference

This chapter provides a quick-reference cheat sheet of Hale's syntax, decorators, CLI subcommands, diagnostic formats, and links to the normative language specifications in `spec/`.

---

## 1. Syntax & Decorator Cheat Sheet

### Top-Level Declarations

| Syntax | Purpose |
|---|---|
| `type Name { field: Type = default; }` | Value record (struct) passed by value |
| `enum Name { VariantA, VariantB(Int), VariantC { x: Int } }` | Tagged union (sum type) |
| `type Union = TypeA \| TypeB;` | Set type (message union for `seam` pipelines) |
| `interface Name { fn method(x: Int) -> String; }` | Structural interface satisfied by any locus with matching methods |
| `perspective Name { field: Type; }` | Cross-process projection schema |
| `topic Name { payload: Type; keyed_by field; }` | Typed message bus channel |
| `locus Name { ... }` | Stateful component owning an arena, `params`, methods, and lifecycle |
| `main locus Name { ... }` | Process entrypoint declaring `placement`, `bindings`, and `claims` |
| `fleet locus Name { ... }` | Multi-binary fleet topology declaring host `placement`, `bindings`, and `claims` |
| `seam name(x: In) -> Out { ... }` / `seam { ... }` | Linear typed pipeline across methods and loci |
| `api Name { @rpc fn m(req: Req) -> Resp; }` | Typed RPC/streaming surface definition |
| `group name = { LocusA, alias::* };` | Named set of loci/functions for `claims` |
| `constitution Name [extends Base] { ... }` | Reusable set of architectural `claims` |
| `dimension` / `unit` / `scale` / `quantity` | Compile-time dimensional analysis and unit types |
| `import "path/to/seed" as alias;` | Cross-seed library import |

### Decorator Reference

| Decorator | Target | Meaning |
|---|---|---|
| `@form(vec\|hashmap\|set\|ring_buffer\|lru_cache\|heap)` | `locus` | Specializes the locus as an arena-backed collection over its `capacity` slot |
| `@repo(RecordType)` | `locus` | Enforces keyed CRUD interface (`get`, `create`, `put`, `remove`) and state effects (`reads(T)`, `writes(T)`, etc.) |
| `@idempotent` / `@not_idempotent` | `fn` on `@repo(T)` | Declares retry safety on custom repository mutators |
| `@sealed` | `locus` | Restricts read/write access to `params` fields to the locus's own methods |
| `@supervised` | `locus` | Requires every child locus in its subtree to have an `on_failure` policy |
| `@phase_effects(birth: {...}, run: {...})` | `locus` | Restricts which effect classes each lifecycle phase may reach |
| `@effects(none\|only\|is\|publish\|causes\|depends: {...})` | `fn` | Declares or restricts transitive side effects |
| `@no_syscall` / `@no_block` / `@deterministic` / `@no_ffi` / `@no_publish` / `@no_spawn` / `@no_recursion` / `@no_panic` | `fn` | Shorthand effect restrictions |
| `@hot` | `fn` | Promotes hot-path allocation advisories inside the function to errors |
| `@budget(alloc_per_call = N)` | `fn` | Hard compile-time ceiling on arena allocations per call (`0` = zero-alloc) |
| `@bounded_by(input_name)` | `fn` / hook | Documents external input bounding an allocation loop; silences unbounded-alloc advisory |
| `@unbounded` | `fn` / hook | Acknowledges intentional unbounded growth; silences unbounded-alloc advisory |
| `@secret` | `fn` param | Lint flagging direct logging or bus publishing of the parameter within the body |
| `@rpc` | `fn` in `api` / `locus` | Marks a method as an exposed RPC operation |
| `@not_found` / `@conflict` / `@invalid` / `@forbidden` / `@unavailable` | `type` / `enum` variant | Maps an `api` error type or variant to a standard transport status code |
| `@ffi("c")` / `@ffi("js")` | `fn` | Binds an external C symbol or JavaScript host import |
| `@export` | `fn` / `locus` | Exports a function or locus to the WebAssembly host |

---

## 2. Toolchain CLI Reference

Every subcommand accepts `--help` (or `-h`) as its first argument.

### Build, Run & Check

| Command | Description |
|---|---|
| `hale init [dir]` | Non-destructively scaffold `hale.toml`, an entry `.hl` file, a first test, and `.gitignore` |
| `hale run [flags] <file\|dir> [args...]` | Compile and run immediately (everything after `<file\|dir>` is passed as program `argv`) |
| `hale build [flags] <file\|dir> [-o <path>]` | Compile to a native binary (`./<name>` or `<dir>/<dir>`) or `.wasm` + `.mjs` (`--target wasm32`) |
| `hale check [flags] <file\|dir>` | Parse, typecheck, and evaluate effects, placement, memory bounds, and claims |
| `hale verify <file\|dir>` | CI gate: runs `hale check` and fails on any warning or advisory |
| `hale test <file\|dir>` | Build and execute `*_test.hl` files |
| `hale bench <file\|dir>` | Build and execute `*_bench.hl` benchmarks (`ns/op`, `allocs/op`) |
| `hale fmt <file\|dir>` | Canonical zero-config formatter (100-column structural line breaking; skips `vendor/`, dot-dirs, and directories containing `.hale-fmt-frozen`) |
| `hale doc [<file\|dir> \| --stdlib]` | Render documentation from `///` comments or print the standard library reference |
| `hale fetch` | Clone and pin git dependencies from `hale.toml` into `vendor/` and `hale.lock` |
| `hale inputs <seed>` | List every source file read by a build of `<seed>`, following all `import` edges |
| `hale targets` | List all target triples known to the compiler and their support status |

### API Surfaces, Clients & Inspection

| Command | Description |
|---|---|
| `hale describe <endpoint \| file>` | Print a served exposure's schema, members, streams, digest, and caller roles |
| `hale call <endpoint> <member> [json]` | Invoke an exposed RPC method over Unix socket or HTTP and print the JSON response |
| `hale watch <ws://hub> <topic>` | Subscribe to a `ws::Hub` stream and print one JSON line per frame |
| `hale admin <endpoint>` | Launch a local web UI over a served exposure's description |
| `hale api export --surface <S> [--out <dir> \| --check <dir> \| --tools-for <roles>]` | Export OpenAPI, JSON Schema, MCP tool definitions, and contract digest for `api` surface `<S>` |
| `hale api client --surface <S> --lang hale\|ts [--out <file> \| --check <file>]` | Generate a typed Hale or TypeScript client for `api` surface `<S>` |
| `hale api import <openapi-or-mcp> --as <S> [--out <file>]` | Generate an `api S from openapi\|mcp "..."` block and record types from an OpenAPI 3 JSON/YAML file/URL or MCP endpoint |
| `hale api drift [<file\|dir>] [--surface <S>] [--document]` | Verify imported API surfaces against their upstream OpenAPI/MCP specs (exit 1 on drift) |

### Topology, Fleets, Observability & Editor Servers

| Command | Description |
|---|---|
| `hale fleet <check\|dump\|sign\|attest\|keygen>` | Verify, export, sign, or attest multi-binary `fleet locus` topologies |
| `hale deploy render <compose\|kubernetes> <file\|dir>` | Render `docker-compose.yml` or Kubernetes manifests (`Deployment`, `Service`, `ConfigMap`) from a `fleet locus` |
| `hale node <name>` | Run a host agent that supervises fleet instances assigned to `<name>` |
| `hale topology graph <file\|dir>` | Render the program's locus and bus topology graph |
| `hale model diff <old.json> <new.json>` | Diff two exported topology/application model snapshots |
| `hale iris [port]` | Launch the embedded live observer UI (or run `hale run --observe <target>`) |
| `hale replay <recording> <file\|dir> [--diff \| --feed]` | Deterministically replay a recorded execution and report divergences |
| `hale lsp` | Start the Language Server Protocol server over stdio |
| `hale mcp [--app <endpoint>]` | Start the MCP server over stdio (optionally exposing a running app's API surface as tools) |
| `hale dna <verb>` | Operate a DNA organism (`dev`, `run`, `work`, `task`, `status`, `ledger`, `board`, `review`, `body`, etc.) — see [DNA Reference](./dna/reference.md) |
| `hale habitat <verb>` | Operate the local/remote multi-record control plane (`up`, `down`, `graft`, `status`, `face`, `host`, `check`) — see [Habitat](./habitat.md) |

---

## 3. CLI Flag Rules & Diagnostic Format

### Flag Ordering and Output Paths

- **Output binary location (`-o` / `--out`):** By default, `hale build app.hl` writes `./app` and `hale build myapp/` writes `myapp/myapp`. Pass `-o target/seeds/myapp/myapp` to place build artifacts outside your source tree. For `--target wasm32`, `-o path/app.wasm` writes both `app.wasm` and `app.mjs` at that destination.
- **Flag positioning:** For `build`, `check`, `verify`, `test`, and `bench`, flags may appear before or after the target path (`hale build --dev app.hl` and `hale build app.hl --dev` are identical). For `hale run`, compiler flags must appear **before** the target path (`hale run --observe app.hl arg1 arg2`), because all arguments after the target path are forwarded to the program's `argv`.
- **Shared build flags on `run` and `replay`:** `hale run` and `hale replay` accept `--dev`, `--target-cpu`, `--link`, and `--csrc`. Recording identities include these flags so that a `--dev` recording must be replayed with `hale replay --dev`. Report-only flags (`--locality-report`, `--target-cache`, `--strict`, `--wrap-main`, `-o`) apply only to `hale build`.

### Canonical Diagnostic Format

All commands (`check`, `verify`, `build`, `run`, `test`, `bench`, `replay`) follow all `import` edges and emit diagnostics in a uniform format using canonical, symlink-resolved absolute paths:

```text
/abs/path/to/file.hl:line:col: <kind>: <message>
    <source line>
    ^^^^
```

- **Backend/codegen errors:** Constructs accepted by the typechecker that fail during lowering emit `path:line:col: codegen error: <message>` (or a positionless message if not tied to a single span).
- **Standard library effect notes:** When an `@effects` violation occurs inside a `std::*` body called by your code, the primary error points to your call site and attaches a `note: the '<effect>' effect happens here (in the standard library, <file>:<line>:<col>)`.
- **NDJSON mode (`hale check --json`):** Emits one JSON object per line on `stdout` (`{"file": "...", "line": N, "col": N, "severity": "...", "kind": "...", "message": "..."}`) covering parse errors, type errors, I/O errors (`"kind": "io error"`, `line: 0`, `col: 0`), and stdlib notes (`file: ""`, `line: 0`, `col: 0`). Empty `stdout` means zero findings.

---

## 4. Normative Language Specifications (`spec/`)

When you need the formal grammar or exact compiler rules, consult the `spec/` directory in the repository:

| Specification | Contents |
|---|---|
| [`spec/grammar.ebnf`](https://github.com/hale-lang/hale/blob/main/spec/grammar.ebnf) | Formal EBNF grammar |
| [`spec/tokens.md`](https://github.com/hale-lang/hale/blob/main/spec/tokens.md) | Lexical structure, literals, block strings, operators |
| [`spec/precedence.md`](https://github.com/hale-lang/hale/blob/main/spec/precedence.md) | Operator precedence and associativity table |
| [`spec/types.md`](https://github.com/hale-lang/hale/blob/main/spec/types.md) | Type system, subtyping, set types, and generics |
| [`spec/semantics.md`](https://github.com/hale-lang/hale/blob/main/spec/semantics.md) | Operational semantics: lifecycle, bus, supervision, `fallible(E)`, `release` |
| [`spec/memory.md`](https://github.com/hale-lang/hale/blob/main/spec/memory.md) | Arena hierarchy, scratch regions, capacity slots, projection classes |
| [`spec/forms.md`](https://github.com/hale-lang/hale/blob/main/spec/forms.md) | `@form` collection semantics (`vec`, `hashmap`, `set`, `ring_buffer`, `lru_cache`, `heap`) |
| [`spec/stdlib.md`](https://github.com/hale-lang/hale/blob/main/spec/stdlib.md) | Standard library (`std::*`) API surface and effect classifications |
| [`spec/runtime.md`](https://github.com/hale-lang/hale/blob/main/spec/runtime.md) | C runtime (`lotus`) architecture, schedulers, and atomics |
| [`spec/styleguide.md`](https://github.com/hale-lang/hale/blob/main/spec/styleguide.md) | Idiomatic patterns, memory rules, hot-path speed rules, and enforcement ladder |
| [`spec/ffi.md`](https://github.com/hale-lang/hale/blob/main/spec/ffi.md) | C FFI (`@ffi("c")`) and WebAssembly host ABI (`@ffi("js")`, `@export`) |
| [`spec/packages.md`](https://github.com/hale-lang/hale/blob/main/spec/packages.md) & [`spec/projects.md`](https://github.com/hale-lang/hale/blob/main/spec/projects.md) | `hale.toml`, vendoring (`hale fetch`), seed layout, and `import` resolution |
| [`spec/testing.md`](https://github.com/hale-lang/hale/blob/main/spec/testing.md) | `hale test` and `hale bench` execution rules |
| [`spec/verification.md`](https://github.com/hale-lang/hale/blob/main/spec/verification.md) | Exhaustive catalog of static analyses and compile-time checks |
| [`spec/registry.md`](https://github.com/hale-lang/hale/blob/main/spec/registry.md) | Compiler graph registry (`ApplicationModel` producers and consumers) |
| [`spec/habitat.md`](https://github.com/hale-lang/hale/blob/main/spec/habitat.md) | `hale habitat` control plane specification |
| [`spec/design-rationale.md`](https://github.com/hale-lang/hale/blob/main/spec/design-rationale.md) & [`spec/decisions.md`](https://github.com/hale-lang/hale/blob/main/spec/decisions.md) | Numbered architectural decision records (`F.1`–`F.40`) |
| [`AGENTS.md`](https://github.com/hale-lang/hale/blob/main/AGENTS.md) | Concise language & `%DESIGN v1` reference for AI coding agents and humans |
