# Reference

This guide is the tour. The **canonical contract** — what the
compiler actually enforces — lives in the `spec/` directory at
the repository root. When the guide and the spec disagree, the
spec wins; when you need the exact rule, an edge case, or a
diagnostic's meaning, go there.

## The spec, by topic

| You want | Read |
|---|---|
| The formal grammar | [`spec/grammar.ebnf`](https://github.com/hale-lang/hale/blob/main/spec/grammar.ebnf) |
| Lexical structure, literals, operators | [`spec/tokens.md`](https://github.com/hale-lang/hale/blob/main/spec/tokens.md) |
| Operator precedence & associativity | [`spec/precedence.md`](https://github.com/hale-lang/hale/blob/main/spec/precedence.md) |
| Operational semantics (lifecycle, bus, recovery, fallible) | [`spec/semantics.md`](https://github.com/hale-lang/hale/blob/main/spec/semantics.md) |
| The type system | [`spec/types.md`](https://github.com/hale-lang/hale/blob/main/spec/types.md) |
| Memory: regions, capacity slots, projection classes | [`spec/memory.md`](https://github.com/hale-lang/hale/blob/main/spec/memory.md) |
| The form library (`vec` / `hashmap` / `ring_buffer`) | [`spec/forms.md`](https://github.com/hale-lang/hale/blob/main/spec/forms.md) |
| The always-loaded runtime | [`spec/runtime.md`](https://github.com/hale-lang/hale/blob/main/spec/runtime.md) |
| The standard library surface | [`spec/stdlib.md`](https://github.com/hale-lang/hale/blob/main/spec/stdlib.md) |
| Idiomatic patterns, the seven shapes, correctness + speed rules | [`spec/styleguide.md`](https://github.com/hale-lang/hale/blob/main/spec/styleguide.md) |
| The FFI contract — C (`@ffi("c")`) and the WASM host interface (`@ffi("js")` / `@export`) | [`spec/ffi.md`](https://github.com/hale-lang/hale/blob/main/spec/ffi.md) |
| Dependencies & vendoring | [`spec/packages.md`](https://github.com/hale-lang/hale/blob/main/spec/packages.md) |
| Project layout & imports | [`spec/projects.md`](https://github.com/hale-lang/hale/blob/main/spec/projects.md) |
| How tests are written and run | [`spec/testing.md`](https://github.com/hale-lang/hale/blob/main/spec/testing.md) |
| Why the current surface is shaped this way | [`spec/design-rationale.md`](https://github.com/hale-lang/hale/blob/main/spec/design-rationale.md) |
| The design-decision log (the F-series: commitments, rejects, sketches) | [`spec/decisions.md`](https://github.com/hale-lang/hale/blob/main/spec/decisions.md) |
| Internal codename legend (milestone / workstream tags) | [`spec/glossary.md`](https://github.com/hale-lang/hale/blob/main/spec/glossary.md) |

## Two more anchors

- **[`AGENTS.md`](https://github.com/hale-lang/hale/blob/main/AGENTS.md)** — the load-bearing prompt for
  agents writing `.hl`. It condenses the six idiomatic patterns,
  the "what's not in the language" reflexes, and the formal
  design model into one file. Excellent for a human, too.
- **Working programs** — `crates/hale-codegen/tests/fixtures/examples/`
  holds 83 small per-feature programs, numbered. Reading a few
  near your target shape is the fastest way to see real,
  compiling Hale.

## Toolchain commands

| Command | Does |
|---|---|
| `hale init [dir]` | bootstrap a project: `hale.toml`, a hello-world seed, a first test, `.gitignore` — non-destructive, fills in only what's missing |
| `hale run <file/dir>` | compile + run (fast feedback) |
| `hale build <file/dir>` | compile to a native binary |
| `hale check` | parse + typecheck only |
| `hale test` | run `*_test.hl` |
| `hale fetch` | clone & pin git dependencies |
| `hale fmt` | canonical formatter |
| `hale verify` | `check`, but any advisory fails too (the CI gate) |
| `hale bench` | run `*_bench.hl` (ns/op, allocs/op) |
| `hale doc` | API reference from `///` comments |
| `hale fleet` | compose topology artifacts across binaries: `check` / `dump` / `sign` / `attest` / `keygen` |
| `hale node <name>` | express a fleet plan's instances on one machine, from the record |
| `hale targets` | the targets this compiler can name, and which of them it can build |
| `hale lsp` | the language server, in the compiler binary |
| `hale mcp` | the MCP server, in the compiler binary |

Every one of them answers `--help` (or `-h`) as its first argument with
its own flags, its input shape and where its output goes. `hale build
--help` is where the built binary's path is written down — `build` has
no `-o`, so a file target becomes `./app` and a directory target becomes
`myapp/myapp`, inside the directory.

Flags may stand on either side of the target. The first argument that
is not a flag is the target, so `hale build --dev app.hl` and `hale
build app.hl --dev` are the same command, as they already were for
`check`. `run` follows the same rule up to the target and then stops:
everything after it is the program's own argv, which is why `hale run
--observe app.hl` puts the flag in front.

`check` and `verify` follow every `import`, so a parse failure anywhere
in the import graph — a library you import, or one it imports — fails
them both, reported at that file's own line and column. Neither ever
reports success on a tree `hale build` would refuse.

Every command that follows imports reports it the same way. `build`,
`run`, `test`, `bench` and `replay` print the same
`path:line:col: kind: message`, with the offending line and a caret
under it, positioned in the file that holds the mistake — so the
line number you read is the line number you open, whichever command
found it.

The `path` is the file's canonical one: absolute, symlinks resolved,
never a `../` form, however you reached the file or spelled the
target. One file has one name, so a script can compare what two
commands said about it.

A refusal that comes from the *backend* rather than the front end
reads the same way. Some things `hale check` accepts cannot be
lowered — a generic argument the code generator has no name for, a
toolchain piece the program needs and the build cannot find — and
each of those is `path:line:col: codegen error: message`, with the
line and the caret, from `build`, `run`, `test`, `bench` and
`replay` alike. When such a refusal has no particular place in your
source to point at, every one of them prints the message on its own,
with no position invented for it.

With `--json` a parse error is reported exactly like a type error: one
NDJSON record on stdout carrying the file, line, column and message,
with `"kind":"parse error"`. Whatever fails, the machine-readable
stream says why — so for a gate reading it, an empty stream means the
seed is clean and nothing else does.

That holds for the inputs themselves, not just for what is in them: a
target that does not exist, or a file `check` cannot open — one of the
seed's own, or one reached through an `import` — is a record with
`"kind":"io error"`, the path in `"file"` and the operating system's
error in `"message"`. It carries `"line":0` and `"col":0`, since a
file that never opened has no text to point into. In text mode you
read the same sentence you always did, on stderr.
