# Standard library

Aperio's stdlib ships bundled with every binary — no separate
install, no manual import for stdlib namespaces (just inline
`std::*` paths in your code). This page indexes the shipped
surface. The authoritative phase-by-phase history lives at
[`spec/stdlib.md`](https://github.com/aperio-lang/aperio/blob/main/spec/stdlib.md).

## Two shapes

The stdlib comes in two structurally distinct shapes, with a
clear rule for which is which:

### Path-call dispatch

Inline calls through `std::*` paths that route directly to C
runtime primitives. No `.ap` source backing them — they're
extern bridges into `lotus_*` C functions:

```aperio
let pid     = std::process::pid();
let content = std::io::fs::read_file("config.toml");
let n       = std::str::parse_int("42");
```

Namespaces with path-call shape:

| Namespace | Surface |
|---|---|
| `std::process` | `pid()`, `exit(code)` |
| `std::env` | `args_count()`, `arg(i)`, `arg_or(i, default)`, `var(name)`, `var_exists(name)` |
| `std::time` | `monotonic()` → Duration, `sleep(d)` |
| `std::str` | `parse_int` / `can_parse_int` / `parse_float` / `can_parse_float`, `index_of`, `lower` / `upper`, `trim`, `replace`, `repeat`, `pad_left` / `pad_right`, `from_bytes`, `builder_new` / `builder_append` / `builder_len` / `builder_finish` |
| `std::bytes` | `at(b, i) -> Int fallible(IndexError)`, `slice(b, lo, hi)`, `from_string(s)` |
| `std::text` | byte-class predicates `is_alpha`, `is_digit`, `is_alnum`, `is_whitespace`, `is_word_char` (`fn(Int) -> Bool`); `tokenize_words_into(s, target_vec)` populates a `@form(vec) of String` with lowercased word tokens |
| `std::io::fs` | `read_file`, `write_file`, `write_file_append`, `read_bytes`, `file_size`, `mkdir`, `list_dir`, `list_dir_count`, `list_dir_at` — all return `fallible(IoError)` (`kind: String`, `errno: Int`, `path: String`). `file_exists(path) -> Bool` is the only non-fallible predicate |
| `std::io::stdin` | `read_line() -> String`, `read_line_status() -> Int` |
| `std::io::tcp` | path-call entry points `listen_socket(host, port) -> Int fallible(IoError)`, `connect(host, port) -> Int fallible(IoError)`, `accept_one(listen_fd) -> Int fallible(IoError)`, `close_fd(fd)` (infallible) |
| `std::math` | `sqrt`, `exp`, `log`, `floor`, `ceil`, `pow` |
| `std::ts` | tree-sitter bindings (Go grammar shipped) |

Path-call surfaces are appropriate for *value-shaped*
operations that don't need lifecycle. A file read returns
bytes; a math op returns a number; argv access returns a
string. No locus required.

### Namespace lotus

When the operation has a lifetime — a stream that's open
across multiple reads, a sink that has setup and teardown — the
stdlib provides a **namespace lotus**: an Aperio-sourced locus
under `runtime/stdlib/`. You instantiate it the same way you
instantiate any other locus:

```aperio
let l = std::io::tcp::Listener {
    host: "127.0.0.1",
    port: 8080,
    on_connection: my_handler,
};
```

Namespaces with namespace-lotus shape:

| Namespace | Loci / interfaces shipped |
|---|---|
| `std::io::tcp` | `Listener` (multi-accept loop, dispatch via `on_connection: fn(Stream)`), `Stream` (per-connection handle with `send` / `send_bytes` / `recv` / `recv_bytes` methods) |
| `std::http` | `Request` and `Response` types, `parse_request`, `write_response`, case-insensitive `header` lookup, `Server` locus (wraps accept-recv-parse-dispatch-write; supplies single `handler: fn(Request) -> Response` callback) |
| `std::text` | `md_to_html`, `base64::encode` / `decode`, `Sink` interface with `StdoutSink` / `StringSink` / `FileSink` implementations (note: the byte-class predicates + `tokenize_words_into` are path-call surface, listed in the previous table) |
| `std::cli` | `Resolver` for argv parsing |
| `std::iter` | `Lines` iterator over text |
| `std::json` | `Builder` for JSON output; free-fn helpers `escape_string` / `unescape_string` (RFC 8259), `find_string_field` / `find_int_field` / `find_bool_field` (flat-object field lookup), `ArrayIter` + `array_first` / `array_next` (flat-array iteration). No nested-tree shape at v1 |
| `std::lang` | `Morpheme`, `Vocabulary`, etc. for language utilities |
| `std::log` | `Logger`, `LogEvent`, `StdoutSink` (subscribes to `log.**`) |
| `std::yaml` | YAML parsing surface |
| `std::test` | `assert(cond, msg)`, `assert_eq_int`, `assert_eq_str` |

Source for namespace-lotus stdlib lives at
[`crates/aperio-codegen/runtime/stdlib/`](https://github.com/aperio-lang/aperio/tree/main/crates/aperio-codegen/runtime/stdlib).
Read it directly — it's idiomatic Aperio that exercises every
pattern Concepts covers.

## Built-in identifiers (no path needed)

A handful of functions and types are always in scope without
any `std::*` qualification:

| Name | Purpose |
|---|---|
| `print`, `println`, `eprint`, `eprintln` | stdout / stderr output |
| `len(x)` | length of String / Bytes / array |
| `to_string(x)` | format any printable value to String |
| `min(a, b)`, `max(a, b)`, `abs(x)` | numeric helpers |
| `starts_with(s, prefix)`, `contains(s, needle)` | string predicates |
| `sum(expr)`, `prod(expr)` | reductions (also closure-test primitives) |
| `Int(x)` | explicit Float → Int narrowing (truncate toward zero) |

Primitive types (`Int`, `Uint`, `Float`, `Decimal`, `String`,
`Bool`, `Time`, `Duration`, `Bytes`) are valid only in type
position.

## Form-synthesized types

When any locus in your program uses `@form(...)`, the
resolver injects companion error types into the top scope:

| Form | Synthesized type | Fields |
|---|---|---|
| `@form(vec)` | `IndexError` | `kind: String`, `index: Int`, `len: Int` |
| `@form(hashmap)` | `KeyError` | `kind: String` (also surfaces `IndexError` for `key_at` / `entry_at`) |
| `@form(ring_buffer)` | `EmptyError` | `kind: String` |
| `std::io::*` | `IoError` | `kind: String`, `errno: Int`, `path: String` |

The form-method surface synthesizes more than fallibility — see
[`spec/forms.md`](https://github.com/aperio-lang/aperio/blob/main/spec/forms.md)
for the full per-form table. Quick reference for what's on each:

| Form | Synthesized methods |
|---|---|
| `@form(vec)` | `push`, `get`, `set`, `pop`, `len`, `is_empty`, `sort`, `sort_by`, `sort_desc_by` |
| `@form(hashmap)` | `set`, `get`, `has`, `remove`, `len`, `is_empty`, `key_at`, `entry_at`, `bump` |
| `@form(ring_buffer)` | `push -> Bool`, `pop`, `len`, `is_full` |

You can reference these as ordinary types — pattern-match
them in `match`, declare fn parameters typed by them,
construct them in fallback expressions.

## What's NOT in stdlib

Aperio's stdlib follows Go's batteries-included approach:
table-stakes functionality ships. Specifically *not* in
stdlib (and intended for the
[`aperio-lang/pond`](https://github.com/aperio-lang/pond)
contrib monorepo or third-party):

- ML / learning libraries
- Database drivers (Postgres, MySQL, ...)
- Web frameworks beyond basic HTTP
- Image / audio / video processing
- Cloud SDKs (AWS, GCP, ...)
- GUI / TUI frameworks beyond what `std::io::tcp` enables
- Cryptography beyond TLS basics
- Compression formats beyond gzip (used internally by HTTP)

Aperio also doesn't have parametric collection types in
stdlib — no `Vec<T>` / `Map<K, V>` / `Set<T>` / `Option<T>` /
`Result<T, E>` as user-facing tagged enums. Storage is
locus-shaped via `@form(...)`. See
[Capacity & storage](../concepts/capacity-storage.md) for
the rationale.

## Reading order

If you're writing application code and want to discover
what's available, the productive order is:

1. **Skim this page** to know what namespaces exist.
2. **Read the spec section**
   ([`spec/stdlib.md`](https://github.com/aperio-lang/aperio/blob/main/spec/stdlib.md))
   for the namespace you need; it's the authoritative
   surface.
3. **Read the namespace-lotus source** for any lotus you'll
   use — it's a few hundred lines per namespace, and it's
   the clearest documentation of how the surface composes.
