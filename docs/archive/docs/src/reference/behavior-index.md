# Behavior index

A consolidated index of every runtime, language, and stdlib
behavior in Aperio with implementation status. Use this page
to:

- See at a glance what's reachable today vs. blocked on a
  future milestone.
- Cross-reference a feature you're about to use against its
  status before relying on it.
- Audit drift — every shipped behavior has a milestone tag, so
  this page is the single source of truth for "is this real?"

For a category-grouped, app-developer-oriented capability
matrix, see [`docs/std/src/ready-today.md`](../std/ready-today.md).
For phase planning and roadmap, see
[`docs/std/src/roadmap.md`](../std/roadmap.md).

## Status legend

| Marker      | Meaning                                                       |
|-------------|---------------------------------------------------------------|
| **Shipped** | Fully working, tested, documented. Safe to use.               |
| **Partial** | Shipped with significant caveats — see Notes.                 |
| **Planned** | Specific milestone identified, not yet implemented.           |
| **Blocked** | Needs a design pass, generics, or upstream work first.        |
| **Reserved**| Keyword reserved by the lexer, no semantics yet.              |
| **Legacy**  | Parsed but ignored downstream, or deprecated form.            |

The **Since** column is the milestone in which the behavior was
introduced (or last materially changed). Blank when not yet
shipped.

---

## Lexical & literals

| Behavior                                             | Status   | Since | Notes                                                                  |
|------------------------------------------------------|----------|-------|------------------------------------------------------------------------|
| ASCII source (UTF-8 file encoding, ASCII outside literals/comments) | Shipped | m1 | Per spec/tokens.md.                                                |
| Line comments `// ...`                               | Shipped  | m1    |                                                                        |
| Block comments `/* ... */`                           | Shipped  | m1    | Do not nest in v0.                                                     |
| Doc comments `///` and `/** */`                      | Shipped  | m1    | Preserved by lexer, attached to following decl.                        |
| Identifiers `[a-zA-Z_][a-zA-Z0-9_]*`                 | Shipped  | m1    | Case conventions enforced by lints, not lexer.                         |
| Integer literals (decimal, `0x`, `0o`, `0b`, `_` separators) | Shipped | m1 |                                                                        |
| Float literals (decimal + exponent)                  | Shipped  | m1    |                                                                        |
| Decimal literals (`1.50d`)                           | Shipped  | m1    | Backed by shopspring/decimal-shape semantics.                          |
| String literals (`"..."`, `r"..."`, `"""..."""`)     | Shipped  | m1    | NUL-terminated in memory; embedded NULs truncate.                      |
| Bytes literals `b"..."`                              | Partial  | m89   | Lexer + parser support; **codegen rejects** `Literal::Bytes`. Use `read_bytes` to produce a `Bytes` value. |
| Boolean literals (`true`, `false`)                   | Shipped  | m1    |                                                                        |
| Nil literal (`nil`)                                  | Shipped  | m1    | Absent value of an option type; distinct from 0 / "".                  |
| Time literals (`` `2026-05-08T12:00:00Z` ``)         | Shipped  | m1    | ISO-8601 between backticks.                                            |
| Duration literals (`100ms`, `5s`, `1h30m`)           | Shipped  | m1    | Compound forms permitted.                                              |

## Types

| Behavior                                  | Status   | Since | Notes                                                                  |
|-------------------------------------------|----------|-------|------------------------------------------------------------------------|
| `Int`                                     | Shipped  | m1    | i64 lowering.                                                          |
| `Uint`                                    | Partial  | m1    | Declared in PrimType; lowered as i64. Distinct unsigned arithmetic pending. |
| `Float`                                   | Shipped  | m1    | f64 lowering.                                                          |
| `Decimal`                                 | Shipped  | m1    | Fixed-precision; literals via `d` suffix.                              |
| `String`                                  | Shipped  | m1    | NUL-terminated in memory; binary use requires `Bytes`.                 |
| `Bool`                                    | Shipped  | m1    |                                                                        |
| `Bytes`                                   | Shipped  | m89   | `[i64 len][u8 data]` blob; binary-safe. Literals not lowered (use `read_bytes`). |
| `Time`                                    | Shipped  | m1    | Per `std::time::*`.                                                    |
| `Duration`                                | Shipped  | m1    | Duration literals + arithmetic.                                        |
| `type T { field: U; ... }` records        | Shipped  | m1    | Plain data, no lifecycle.                                              |
| `type T = enum { ... }`                   | Shipped  | m43   | Variants without payloads.                                             |
| Enum variants with payloads               | Shipped  | m45   | Construction OK; pattern match limited (see Match).                    |
| Generic types `<T, E>`                    | Partial  | m68   | Monomorph mangling works (e.g. `Result_Int_String::Ok`); template-level inference limited. |
| Generic `List<T>` / `Map<K,V>`            | Blocked  | —     | Reserved syntax, no semantics. Pending generics arc.                   |
| Function-pointer types `fn(T) -> R`       | Shipped  | m80   | Types only; no inline closures-as-values.                              |
| Tuples                                    | Shipped  | m26   | `(a, b)`, destructuring in `let`.                                      |
| Arrays `[T; N]`                           | Shipped  | m21   | Fixed-size; no growable list yet.                                      |
| Type ascription on `let`                  | Shipped  | m1    | Optional; usually elided.                                              |

## Bindings & expressions

| Behavior                                                        | Status   | Since | Notes                                                                  |
|-----------------------------------------------------------------|----------|-------|------------------------------------------------------------------------|
| Immutable `let x = ...`                                          | Shipped  | m1    | Reassignment is a compile-time error.                                  |
| Mutable `let mut x = ...`                                        | Shipped  | m50   | `x = newval` permitted within scope.                                   |
| Top-level `const` declarations                                   | Shipped  | m1    |                                                                        |
| Assignment to immutable binding                                  | Shipped  | m50   | Diagnostic with "immutable" message.                                   |
| Field assignment `x.field = ...` through immutable head          | Shipped  | m50   | Mutates state, doesn't rebind.                                         |
| Numeric operators `+ - * / %`                                    | Shipped  | m1    |                                                                        |
| Comparison operators `== != < > <= >=`                           | Shipped  | m1    |                                                                        |
| Logical operators `&& || !`                                      | Shipped  | m1    |                                                                        |
| Bitwise operators `& \| ^ << >> ~`                               | Shipped  | m1    |                                                                        |
| Compound assign `+= -= *= /= %= &= \|= ^=`                       | Shipped  | m1    |                                                                        |
| `String + String` concat                                         | Shipped  | m1    |                                                                        |
| `String + Int/Float/Decimal` (auto `to_string`)                  | Blocked  | —     | Currently a type error. Workaround: `s + to_string(n)`. Friction-flagged in apps/tcp-echo. |
| Range expression `start..end`                                    | Shipped  | m23   | Used in `for i in 0..n`.                                               |
| Match — literals + wildcard                                      | Shipped  | m15   |                                                                        |
| Match — exhaustiveness check                                     | Shipped  | m15   | Fires on Bool, Int (with wildcard), enums.                             |
| Match — enum variant patterns (no payload)                       | Shipped  | m43   |                                                                        |
| Match — enum variant patterns with payload bindings              | Partial  | m45   | Works for some shapes; broader pattern matching deferred.              |
| String slicing `s[start..end]`                                   | Shipped  | m27   | Byte-wise.                                                             |
| Array indexing `a[i]`                                            | Shipped  | m21   |                                                                        |
| Tuple destructuring                                              | Shipped  | m26   | `let (a, b) = pair();`                                                 |

## Functions

| Behavior                                          | Status   | Since | Notes                                                                  |
|---------------------------------------------------|----------|-------|------------------------------------------------------------------------|
| Free `fn name(args) -> T { ... }`                 | Shipped  | m1    |                                                                        |
| Default parameter values                          | Shipped  | m24   | `fn f(n: Int = 5)`.                                                    |
| Multiple return via tuples                        | Shipped  | m26   | `fn pair() -> (Int, String)`.                                          |
| Returning a Bytes value                           | Shipped  | m89   |                                                                        |
| Returning a locus value (handle-style)            | Blocked  | —     | Dissolve fires before caller binds. Language paper-cut.                |
| Bare `return;` from void fn                       | Shipped  | m1    |                                                                        |
| `return expr;` from value fn                      | Shipped  | m1    |                                                                        |
| Function pointers as values                       | Shipped  | m80   | Pass named functions; no inline closures.                              |
| Indirect call through fn-pointer field            | Shipped  | m80   | Used by `Listener.on_connection`.                                      |
| Methods on a locus (`fn name(self, args)`)        | Shipped  | m81   | Stdlib loci use this; user loci typically prefer the bus.              |
| Trailing comma in fn param list                   | Blocked  | —     | Parser rejects. Language paper-cut.                                    |

## Loci — declaration & lifecycle

| Behavior                                              | Status   | Since | Notes                                                                  |
|-------------------------------------------------------|----------|-------|------------------------------------------------------------------------|
| `locus L { ... }` declaration                         | Shipped  | m1    |                                                                        |
| `params { x: T = default; ... }`                      | Shipped  | m1    | Mutable from inside lifecycle methods (`self.x = ...`).                |
| Locus instantiation `L { x: 1 }`                      | Shipped  | m1    | Statement position fires birth → run → drain → dissolve back-to-back. |
| Let-bound locus literal `let l = L { ... }`           | Shipped  | m82   | Dissolve deferred to enclosing fn scope-exit.                          |
| `birth()` lifecycle method                            | Shipped  | m1    |                                                                        |
| `run()` lifecycle method                              | Shipped  | m1    |                                                                        |
| `drain()` lifecycle method                            | Shipped  | m1    |                                                                        |
| `dissolve()` lifecycle method                         | Shipped  | m1    |                                                                        |
| `on_failure(child, err)`                              | Shipped  | m32   | Recovery hook; alternative to dissolve.                                |
| `accept(c: ChildT)` (single accept type)              | Shipped  | m2    |                                                                        |
| Multiple distinct `accept` signatures per locus       | Blocked  | —     | F.11 v0 limitation. Workaround: split supervisor.                      |
| Block-level deferred-dissolve frames                  | Blocked  | —     | m82 ships fn-level only; per-iteration cleanup uses helper-fn.         |

## Loci — contract

| Behavior                            | Status   | Since | Notes                                                          |
|-------------------------------------|----------|-------|----------------------------------------------------------------|
| `contract { expose x: T; ... }`     | Shipped  | m2    |                                                                |
| `contract { consume x: T; ... }`    | Shipped  | m2    | Type checker validates child exposes ↔ parent consumes.        |
| Contract on parent without `accept` | Shipped  | m2    | Diagnostic — `consume` requires accepting children.            |
| `inferred` keyword                  | Reserved | —     | Reserved word for future contract inference.                   |

## Loci — bus

| Behavior                                                       | Status   | Since | Notes                                                                  |
|----------------------------------------------------------------|----------|-------|------------------------------------------------------------------------|
| `bus { publish "subject" of type T; }`                         | Shipped  | m1    |                                                                        |
| `bus { subscribe "subject" as handler of type T; }`            | Shipped  | m1    | Sibling `fn handler(e: T)` runs once per delivered event.              |
| Bus send `subject <- payload;`                                 | Shipped  | m1    | String subject expression; payload type checked against publish decl. |
| Static-literal subject required                                | Shipped  | m1    | Default rule: `<-` rejects non-literal subjects unless wildcard publish exists. |
| Computed subject via wildcard publish                          | Shipped  | m94   | Declaring `publish "x.**" of type T` authorizes any computed subject of type T. |
| Wildcard subscribe (`subscribe "x.**"`)                        | Shipped  | m94   | Trailing `**` matches zero+ remaining segments.                        |
| Wildcard at non-trailing position                              | Blocked  | —     | `log.**.error` rejects; only end-of-pattern supported in v0.           |
| Single-segment wildcard `*`                                    | Blocked  | —     | Not in v0; only `**` (trailing zero+).                                 |
| Bus ordering: subscribers register at `birth()`                | Shipped  | m1    | Instantiate subscribers before publishers.                             |
| Cross-process bus over AF_UNIX SEQPACKET                       | Shipped  | m57   | Substrate.                                                             |
| Cross-process bus over TCP                                     | Shipped  | m72   | Internal 8-byte LE length-prefix framing on `lotus_tcp_*`.             |
| Cross-process bus configuration from `.ap` source              | Blocked  | —     | Today via deployment config. `std::bus::expose` planned (m97).         |
| Sync transport (`SyncDispatch`)                                | Shipped  | m1    | Default; immediate fanout.                                             |
| Ring-buffer transport (`RingBuffer`)                           | Shipped  | m26   | LMAX-style; opt-in per subject.                                        |

## Closures

| Behavior                                              | Status   | Since | Notes                                                                  |
|-------------------------------------------------------|----------|-------|------------------------------------------------------------------------|
| `closure name { ... }` block                          | Shipped  | m3    |                                                                        |
| `expr ~~ value within tolerance`                      | Shipped  | m3    | Approximate-equal assertion.                                           |
| `epoch tick` / `epoch dissolve` / `epoch <duration>`  | Shipped  | m3    |                                                                        |
| `persists_through(quarantine)`                        | Shipped  | m32   |                                                                        |
| `resets_on(...)`                                      | Shipped  | m38   |                                                                        |
| Closure-block accumulators (`sum`, `prod`, etc.)      | Shipped  | m41   | Vocabulary only inside closure blocks.                                 |
| Inline closures-as-values                             | Blocked  | —     | Use named fns + fn pointers instead.                                   |

## Recovery

| Behavior                          | Status   | Since | Notes                                                          |
|-----------------------------------|----------|-------|----------------------------------------------------------------|
| `bubble`                          | Shipped  | m32   | Re-raises to parent's `on_failure`.                            |
| `restart`                         | Shipped  | m32   |                                                                |
| `restart_in_place`                | Shipped  | m38   | In-locus restart without dissolution.                          |
| `quarantine`                      | Shipped  | m33   |                                                                |
| `reorganize`                      | Reserved | —     | Reserved word; no v0 semantics.                                |

## Schedule classes

| Behavior                                          | Status   | Since | Notes                                                                  |
|---------------------------------------------------|----------|-------|------------------------------------------------------------------------|
| `schedule cooperative`                            | Shipped  | m16   | Default; runs on the cooperative scheduler.                            |
| `schedule pinned`                                 | Shipped  | m18   | Spawns a pthread; needs `-lpthread` (auto-linked).                     |
| `yield;` cooperative yield point                  | Shipped  | m17   | Lowers to bus-queue drain.                                             |

## Modes & projections

| Behavior                                          | Status   | Since | Notes                                                          |
|---------------------------------------------------|----------|-------|----------------------------------------------------------------|
| `mode <Name> { ... }` block                       | Shipped  | m4    |                                                                |
| Mode params with defaults                         | Shipped  | m50   |                                                                |
| `bulk` / `harmonic` / `resolution`                | Shipped  | m4    | Mode keywords.                                                 |
| `projection`, `rich`, `chunked`, `recognition`    | Shipped  | m14   | Projection-class keywords.                                     |
| `stable_when` (perspective)                       | Shipped  | m14   |                                                                |
| `serialize_as` (perspective)                      | Shipped  | m14   |                                                                |

## Memory

| Behavior                                  | Status   | Since | Notes                                                          |
|-------------------------------------------|----------|-------|----------------------------------------------------------------|
| Per-locus arena (region)                  | Shipped  | m19   | Allocated on locus instantiation, freed on dissolve.           |
| Lazy global payload arena                 | Shipped  | m70   | For cross-process String + Bytes that outlive the call frame.  |
| Per-fn arena                              | Shipped  | m46   | Returned values escape via the lazy global arena.              |
| Manual heap pointers (`Box<T>`, `Rc<T>`)  | Blocked  | —     | Not in v0; arenas + bus do the work.                           |

## Match — exhaustiveness specifics

| Behavior                                              | Status   | Since | Notes                                                                  |
|-------------------------------------------------------|----------|-------|------------------------------------------------------------------------|
| Bool match (true/false)                               | Shipped  | m15   |                                                                        |
| Int match (with required wildcard)                    | Shipped  | m15   |                                                                        |
| Enum variant match                                    | Shipped  | m43   |                                                                        |
| Generic enum monomorph match (e.g. `Result_Int_String::Ok`) | Shipped | m68 |                                                                        |
| Variant payload destructuring                         | Partial  | m45   | Some shapes work; broader pattern matching deferred.                   |

## Module system / source organization

| Behavior                                  | Status   | Since | Notes                                                          |
|-------------------------------------------|----------|-------|----------------------------------------------------------------|
| Magic `std::*` path resolution            | Shipped  | m71   | Only recognized prefix; codegen routes to mangled stdlib names. |
| Multi-file user projects                  | Blocked  | —     | Single `main.ap` per program.                                  |
| `import "..."` legacy syntax              | Legacy   | —     | Parses, ignored downstream. Stripped from active examples.     |
| `use` / `mod` / `pub`                     | Reserved | —     | None of these exist.                                           |

## Reserved (no v0 semantics)

| Keyword         | Notes                                                                  |
|-----------------|------------------------------------------------------------------------|
| `trait`         | Reserved; compose via lifecycle + bus instead.                         |
| `impl`          | Reserved.                                                              |
| `async`         | Reserved; concurrency via loci + bus + schedule classes.               |
| `await`         | Reserved.                                                              |
| `macro`         | Reserved.                                                              |
| `where`         | Reserved (generic constraints).                                        |
| `with`          | Reserved.                                                              |
| `inferred`      | Reserved (contract inference).                                         |
| `reorganize`    | Reserved (recovery primitive).                                         |

---

# Stdlib — by namespace

## Built-in functions (no path needed)

| Name           | Type                                       | Status   | Since | Notes                                                          |
|----------------|--------------------------------------------|----------|-------|----------------------------------------------------------------|
| `print(...)`   | variadic                                   | Shipped  | m1    | Stdout, no newline.                                            |
| `println(...)` | variadic                                   | Shipped  | m1    | Stdout, with newline.                                          |
| `len(s)`       | `(String) -> Int`, `(Bytes) -> Int`, `(Array) -> Int` | Shipped | m1 | Byte length for String/Bytes; element count for arrays.       |
| `to_string(n)` | `(Int / Float / Decimal) -> String`        | Shipped  | m28   |                                                                |
| `min(a, b)`    | `(Int, Int) -> Int`, `(Float, Float) -> Float` | Shipped | m29 |                                                                |
| `max(a, b)`    | as `min`                                   | Shipped  | m29   |                                                                |
| `abs(n)`       | `(Int) -> Int` / `(Float) -> Float`        | Shipped  | m29   |                                                                |
| `starts_with(s, p)` | `(String, String) -> Bool`            | Shipped  | m38   |                                                                |
| `contains(s, sub)`  | `(String, String) -> Bool`            | Shipped  | m38   |                                                                |
| `eprintln(...)` | variadic, to stderr                       | Blocked  | —     | Friction-flagged. WARN/ERROR sink routing waits on this.       |

## Closure-block-only vocabulary

| Name        | Status  | Notes                                                          |
|-------------|---------|----------------------------------------------------------------|
| `sum`       | Shipped | Inside `closure { ... }` only.                                 |
| `prod`      | Shipped | Inside `closure { ... }` only.                                 |
| `length`    | Shipped | Inside `closure { ... }` only.                                 |
| `empty`     | Shipped | Inside `closure { ... }` only.                                 |

## `std::process`

| Item                       | Status   | Since | Notes                                              |
|----------------------------|----------|-------|----------------------------------------------------|
| `pid() -> Int`             | Shipped  | m71   |                                                    |
| `exit(code: Int)`          | Shipped  | m79   | Short-circuits dissolve cascade.                   |
| `args() -> [String]`       | Blocked  | —     | Use `std::env::args_count` + `arg(i)` for now.     |
| `spawn(...)`               | Blocked  | —     | No subprocess primitive in v0.                     |

## `std::env`

| Item                                 | Status   | Since | Notes                                              |
|--------------------------------------|----------|-------|----------------------------------------------------|
| `args_count() -> Int`                | Shipped  | m77   |                                                    |
| `arg(i: Int) -> String`              | Shipped  | m77   | argv[i]; empty string if out of range.             |
| `var(name: String) -> String`        | Shipped  | m77   | Empty string if unset.                             |
| `var_exists(name: String) -> Bool`   | Shipped  | m77   |                                                    |

## `std::str`

| Item                                          | Status   | Since | Notes                                              |
|-----------------------------------------------|----------|-------|----------------------------------------------------|
| `parse_int(s: String) -> Int`                 | Shipped  | m78   | Base 10, signed; strict trailing-char check.       |
| `can_parse_int(s: String) -> Bool`            | Shipped  | m78   | Disambiguates "0" from parse failure.              |
| `index_of(s: String, sub: String) -> Int`     | Shipped  | m84   | Byte-wise search.                                  |
| `split` / `replace` / `trim` / `to_lower`     | Blocked  | —     | Hand-roll with `index_of` + slicing for now.       |
| `int_to_str` (path-call alias)                | Blocked  | —     | Use bare-name `to_string(n)`.                      |

## `std::time`

| Item                                | Status   | Since | Notes                                              |
|-------------------------------------|----------|-------|----------------------------------------------------|
| `sleep(d: Duration)`                | Shipped  | m79   | Cooperative yield point.                           |
| `monotonic() -> Time`               | Shipped  | m79   |                                                    |
| `now() -> Time` (wall clock)        | Blocked  | —     |                                                    |
| Time formatting / parsing           | Blocked  | —     |                                                    |

## `std::io::tcp`

| Item                                                             | Status   | Since | Notes                                                          |
|------------------------------------------------------------------|----------|-------|----------------------------------------------------------------|
| `Listener` locus (multi-accept + on_connection callback)         | Shipped  | m83   | `host`, `port`, `max_accepts`, `on_connection: fn(Stream)`.     |
| `Stream` locus (let-bound, scope-bound dissolve)                 | Shipped  | m81   |                                                                |
| `Stream.send(msg: String)`                                       | Shipped  | m81   | NUL-truncates; use `send_bytes` for binary.                    |
| `Stream.send_bytes(b: Bytes)`                                    | Shipped  | m89   | Length-preserving.                                             |
| `Stream.recv(max: Int) -> String`                                | Shipped  | m81   | Single recv; up to `max` bytes.                                |
| `Stream.recv_bytes(max: Int) -> Bytes`                           | Planned  | —     | Mirror of `send_bytes`. Friction-flagged in apps/tcp-echo.     |
| `Stream::connect(host, port)` (constructor)                      | Blocked  | —     | Use lower-level `__connect` primitive for now.                 |
| AF_INET6 (IPv6)                                                  | Blocked  | —     | AF_INET only in v0.                                            |
| TLS / HTTPS                                                      | Blocked  | —     | No TLS substrate.                                              |
| Listener bind-readiness primitive                                | Blocked  | —     | Tests retry-connect from client side.                          |

## `std::io::fs`

| Item                                                         | Status   | Since | Notes                                                          |
|--------------------------------------------------------------|----------|-------|----------------------------------------------------------------|
| `read_file(path: String) -> String`                          | Shipped  | m75   | Empty string on error.                                         |
| `write_file(path: String, content: String) -> Int`           | Shipped  | m75   | **Truncates**; no append.                                      |
| `file_exists(path: String) -> Bool`                          | Shipped  | m75   |                                                                |
| `file_size(path: String) -> Int`                             | Shipped  | m75   | -1 on error.                                                   |
| `read_bytes(path: String) -> Bytes`                          | Shipped  | m89   | Binary-safe.                                                   |
| `list_dir(path: String) -> String`                           | Shipped  | m90   | Newline-separated; no recursion.                               |
| `mkdir(path: String) -> Bool`                                | Planned  | —     | Friction-flagged in apps/ssg.                                  |
| `append_file(path, content) -> Int`                          | Planned  | —     | Friction-flagged in apps/log-router.                            |
| `write_bytes(path, payload: Bytes) -> Int`                   | Planned  | —     | Counterpart to `read_bytes`.                                   |
| `read_dir(path) -> [String]`                                 | Blocked  | —     | Pending generic `List<T>`.                                     |
| Streaming reads (line-by-line, large files)                  | Blocked  | —     | No `Reader` type.                                              |
| Filesystem watch (inotify)                                   | Blocked  | —     | m96 planned; was the IDE plan's fs::watch.                     |
| `errno` disambiguation                                       | Blocked  | —     | All errors collapse to -1 / false / "".                        |

## `std::http`

| Item                                                                | Status   | Since | Notes                                                          |
|---------------------------------------------------------------------|----------|-------|----------------------------------------------------------------|
| `Request { method, path, version, body }`                            | Shipped  | m84   | No header surface in v0.                                       |
| `Response { status, content_type, body }`                            | Shipped  | m85   | Fixed header set on the wire.                                  |
| `parse_request(raw: String) -> Request`                              | Shipped  | m84   | Single-recv assumed (≤ 8 KB).                                  |
| `write_response(s: Stream, r: Response)`                             | Shipped  | m85   | `Connection: close` hardcoded.                                 |
| Custom request/response headers                                      | Blocked  | —     | Phase 3 v1.0; needs header-map type.                           |
| `Connection: keep-alive`                                             | Blocked  | —     | Phase 3 v1.0; needs request-handling loop.                     |
| Bodies > 8 KB                                                        | Blocked  | —     | Phase 3 v1.0; needs streaming reassembly.                      |
| Router / path dispatch                                               | Blocked  | —     | Hand-roll with `==` / `starts_with` / `index_of` for now.      |
| Content-type-by-extension                                            | Blocked  | —     | Phase 3 v1.0.                                                  |
| HTTPS / TLS                                                          | Blocked  | —     | No TLS substrate.                                              |

## `std::text`

| Item                                                        | Status   | Since | Notes                                                          |
|-------------------------------------------------------------|----------|-------|----------------------------------------------------------------|
| `md_to_html(md: String) -> String`                          | Shipped  | m91   | Block-level: ATX headings, paragraphs, fenced code, escape.    |
| Inline markdown (`**bold**`, `*italic*`, `` `code` ``, `[a](b)`) | Blocked | —    | Phase 4 v1.0.                                                  |
| `html_escape(s: String) -> String` (standalone)             | Blocked  | —     | Internal to `md_to_html` for now.                              |
| Lists, blockquotes, setext headings                         | Blocked  | —     | Phase 4 v1.0+.                                                 |
| Reference-style links / raw HTML                            | Blocked  | —     | Phase 4 v1.0+.                                                 |
| Syntax highlighting                                         | Blocked  | —     | Not on the roadmap.                                            |

## `std::test`

| Item                                                                | Status   | Since | Notes                                                          |
|---------------------------------------------------------------------|----------|-------|----------------------------------------------------------------|
| `assert(cond: Bool, msg: String)`                                   | Shipped  | m87   | Pass = exit 0 silent; fail = stdout `ASSERTION FAILED:` + exit 1. |
| `assert_eq_int(actual, expected, msg)`                              | Shipped  | m87   |                                                                |
| `assert_eq_str(actual, expected, msg)`                              | Shipped  | m87   |                                                                |
| `assert_neq_*` siblings                                             | Blocked  | —     | Phase 2 v1.0. Use `assert(a != b, ...)` for now.               |
| `assert_rejects` (compile-error tests)                              | Blocked  | —     | Phase 2 v1.0; needs compiler-level surface.                    |
| `assert_closure(name, tolerance)`                                   | Blocked  | —     | Phase 2 v1.0.                                                  |
| `aperio test` CLI runner                                            | Blocked  | —     | Phase 2 v1.0; use `cargo test -p aperio-codegen` today.        |
| Fake time / fake bus / fake fs                                      | Blocked  | —     | Phase 2 v1.0+.                                                 |
| Property-based testing                                              | Blocked  | —     | Explicitly deferred per spec.                                  |
| Benchmarks (`@bench`, `aperio bench`)                               | Blocked  | —     | Phase 2 v1.0 layer 3.                                          |

## `std::log`

| Item                                                  | Status   | Since | Notes                                                          |
|-------------------------------------------------------|----------|-------|----------------------------------------------------------------|
| `LogEvent { level: Int, msg: String, path: String }`  | Shipped  | m95   |                                                                |
| `Logger { name, parent_path }` locus                  | Shipped  | m95   | Cascading namespace; methods info/warn/error/debug/trace.      |
| `StdoutSink` locus (`subscribe "log.**"`)             | Shipped  | m95   | All levels to stdout.                                          |
| Levels as enum (variant-pattern dispatch)             | Blocked  | —     | Pending enum-variant pattern support.                          |
| Custom sinks subscribing to sub-tree patterns         | Shipped  | m95   | E.g. `subscribe "log.app.db.**"`.                              |
| WARN/ERROR routing to stderr                          | Blocked  | —     | Needs `eprintln` primitive.                                    |
| Cross-process tailing from `.ap` source               | Blocked  | —     | Needs `std::bus::expose` (m97 planned).                        |
| Structured fields beyond `msg`                        | Blocked  | —     | Needs generic `Map` or fixed tuple array.                      |
| Default sink filtering by level                       | Blocked  | —     | Custom sinks can do `if e.level >= 2`.                         |

---

# Phase 6 substrate (planned)

The IDE's design forces a multi-milestone substrate roadmap.
m94 + m95 are already sealed; the rest are pre-committed but
not yet shipped. See `notes/aperio-ide-design.md` for the full
plan.

| Milestone | Surface                                          | Status   |
|-----------|--------------------------------------------------|----------|
| m94       | Bus subject wildcards                            | Shipped  |
| m95       | `std::log`                                       | Shipped  |
| m96       | `std::fs::watch::{create, next, close}`          | Planned  |
| m97       | `std::bus::expose` (cross-process from source)   | Planned  |
| m98       | Runtime debug instrumentation (`lotus.debug.*`)  | Planned  |
| m99       | `std::graphics`                                  | Planned  |
| m100      | `std::ui`                                        | Planned  |
| m101      | `std::shell`                                     | Planned  |
| m102      | `std::mcp`                                       | Planned  |
| m103      | `std::aperio` (compiler self-introspection)      | Planned  |

---

# Source-of-truth notes

This page is updated as part of any milestone that ships new
surface or changes status. If you add a new namespace or
change a behavior's status, edit this file in the same commit.
A drift-checker pass should diff this index against the spec
and stdlib bundled source on a regular cadence.

When in doubt about a behavior's status:

- **Spec authority:** `spec/grammar.ebnf` + `spec/types.md` +
  `spec/semantics.md`.
- **Implementation authority:** the test suite under
  `crates/aperio-codegen/tests/` and `crates/aperio-runtime/`.
  If a test exists and passes, the behavior ships.
- **Friction signal:** `notes/aperio-friction.md` and per-app
  `apps/<name>/FRICTION.md` files capture real moments where
  the surface fell short.
