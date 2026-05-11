# The Embuement

> Orientation brief for a new app-dev session in Aperio.
>
> *Aperio is the spell of spellcasting. The compiler is the wand.
> A locus is the invariant form every spell must take. You are a
> caster who has just arrived; this brief is the embuement that
> lets you cast. Read it before writing a line.*

## Read this first

You have **zero training data** on Aperio. The language was
written recently; no model has seen it during pretraining. Your
priors are wrong by default. Treat every syntactic guess as
suspect until you have read working source. **Read first, then
write.** When in doubt, grep `examples/` — it is the corpus.

The closest neighbours are Rust and Go. You will reflexively
reach for their idioms. Most of those idioms do not exist here.
The counter-hallucination list below is not optional reading.

## What you are doing here

You are an app-dev session. You build an Aperio program in
`apps/<your-app-name>/`. You may use anything in `std::*`. You
**do not modify the compiler** (`crates/aperio-codegen/`,
`crates/aperio-runtime/`, `crates/aperio-syntax/`,
`crates/aperio-types/`) — that is the leading-edge compiler
session's territory. If you need a primitive that does not
exist, **log it in `FRICTION.md` and either work around it or
stop and ask** — do not invent substrate.

A sibling document, `notes/aperio-friction.md` (and its per-app
peers in `apps/<name>/FRICTION.md`), is how your friction
reaches the compiler session. Your friction directly shapes the
next milestone. This is the load-bearing reason your work
matters: you are not a downstream consumer, you are the forcing
function.

## The minimum mental model

**One sentence.** A locus is a unit of design with a lifecycle
(birth → accept → run → drain → dissolve), a contract (what it
exposes / consumes), and a bus (typed pub/sub on string
subjects). An Aperio program is a tree of loci.

**File extension.** `.ap`. Source is ASCII-only outside string
literals and comments. UTF-8 file encoding.

**Statements end with semicolons.** Always. Newlines are not
terminators.

**Bindings are immutable by default.** `let x = 1;` is final.
`let mut x = 1; x = 2;` to reassign.

**`self`** refers to the enclosing locus's params and exposed
state. Only legal inside lifecycle / mode / closure blocks of
that locus.

**There is no module system.** No `use`, no `import`, no
`from x import y`, no `mod`, no `pub`. The only namespace
mechanism is the magic `std::*` prefix. You call stdlib
functions by their full path, inline, every time:

```aperio
let s = std::io::fs::read_file("config.toml");
let n = std::str::parse_int(std::env::arg(1));
```

Yes, every call site repeats the path. Yes, that is the
intended ergonomics for v0. Do not paper over it with a
helper-fn alias unless the program genuinely needs one.

**Locus instantiation runs the lifecycle.** A bare struct
literal at statement position runs `birth()` and (for non-let
bindings) `dissolve()`:

```aperio
SomeLocus { param: value };           // fire-and-forget
let l = SomeLocus { param: value };   // dissolve fires at fn scope-exit
```

The let-binding form (m82) is what makes `Stream`, `Listener`,
and friends usable.

**Bus send is `<-`.** Subscribe is declarative in the locus
body. The send operator looks like Erlang's `!`:

```aperio
"ide.source.changed" <- SourcePathEvent { path: "x.ap" };
```

Subscribe form: declared inside the locus body, paired with a
sibling handler fn. The handler runs once per delivered event:

```aperio
locus LoggerL {
    bus {
        subscribe "log.event" as on_event of type LogEvent;
    }
    fn on_event(e: LogEvent) {
        println("[", e.level, "] ", e.msg);
    }
}
```

**Bus ordering: subscribers must be born before publishers.**
v0 dispatches at registration order; subscriptions register at
the locus's `birth()`. Instantiate subscriber loci first in
`main()`; publisher loci last. If a publisher's `birth()` fires
events before its subscribers exist, those events are dropped.

**`return` works as expected.** Bare `return;` exits a void fn;
`return expr;` exits a value fn. Used freely in lifecycle
methods, bus handlers, and free fns.

## Counter-hallucination list

Things that **do not exist** in Aperio v0. Do not write them.

| You will reach for | It does not exist. Use instead |
|---|---|
| `import` / `use` / `from x` | Magic `std::*` paths, called inline. |
| `pub fn` / `pub struct` | All top-level items are visible in the file. No visibility modifiers. |
| `async` / `await` / `Future` | Plain functions. Concurrency comes from loci + the bus + schedule classes, not coroutines. |
| `trait` / `impl` / interfaces | Reserved keywords, but no semantics yet. Compose loci by lifecycle + bus, not by trait dispatch. |
| `match` patterns on enum variants | Enum variants exist; pattern-matching on payloads is deferred. Workaround: one bus subject per variant. |
| `Vec<T>` / `Box<T>` / `Rc<T>` | No. Arrays exist (`[T; N]` style) but no growable list yet. No heap pointers in source. |
| `Option<T>` / `Result<T, E>` | No. Functions return sentinels (`-1`, `""`, `false`, `nil`) plus a sibling boolean if disambiguation matters. See `std::str::parse_int` + `can_parse_int`. |
| Closures as values | Function pointers exist (`fn(T) -> R`); inline closures-as-values do not. Pass named functions. **A fn-pointer callback (e.g., `Listener.on_connection`) cannot capture surrounding state** — if your callback needs context, either reconstruct it inside the callback (cheap loci like `Logger` are fine to re-instantiate) or route the context through the bus. |
| `let x: T = ...;` (type ascription) | Yes for fn params, yes for `let mut x: Int = -1`, but you usually elide it. The parser is fine either way. |
| `var` / `final` / `const T` keywords | `let` (immutable) and `let mut` (mutable). `const` exists at top level only. |
| Multi-file projects | Single `main.ap` per app. Multi-file module support is planned, not shipped. |
| Method syntax on builtin types | `len(s)`, `to_string(n)` — function call form, not `s.len()`. |
| Locus methods called on locus var | Only via stdlib types that explicitly support it (`Stream.send(msg)`, `Stream.recv(n)`). User loci communicate via the bus. |
| Trailing commas in fn param lists | Parser rejects them. Bites everyone once. |
| `printf`-style format strings | `print` and `println` take any-number-of-args and concatenate. `println("got ", n, " items")`. |

## Lifecycle, in pictures

Every locus runs through some subset of these in order:

```
birth()       once, when instantiated. Setup goes here.
  |
  v
accept(c: ChildT)    zero or more times. Children land here.
  |
  v
run()         once, after birth + any synchronous accepts.
  |           Long-running work (loops, listeners) lives here.
  v
drain()       once, when shutdown begins. Stop accepting work.
  |
  v
dissolve()    once, at end of life. Release resources.
on_failure()  fires instead of dissolve if a closure fails.
```

Statement-position literals `SomeLocus { ... };` run all of
this back-to-back. Let-bound literals defer `dissolve()` to
fn-scope exit (the m82 rule) so the binding is usable in
between.

## What is shipped (your toolbox)

Stdlib namespaces you can use right now:

- `std::io::fs::{read_file, write_file, file_exists, file_size, read_bytes, list_dir}`
- `std::io::tcp::{Listener, Stream}` — Listener accepts many; Stream `.send()` / `.send_bytes()` / `.recv()`
- `std::http::{Request, Response, parse_request, write_response}`
- `std::text::md_to_html(md: String) -> String` (block-level only)
- `std::test::{assert, assert_eq_int, assert_eq_str}`
- `std::env::{args_count, arg, var, var_exists}`
- `std::str::{parse_int, can_parse_int, index_of}`
- `std::time::{sleep, monotonic}`
- `std::process::{pid, exit}`
- `std::log::{Logger, LogEvent, StdoutSink}` — structured logging on the bus with cascading namespaces (m95). See `docs/std/src/log.md`.

Built-in functions, no path needed: `print`, `println`, `len`,
`to_string`, `min`, `max`, `abs`, `starts_with`, `contains`.
(So you write `starts_with(path, "/usr")` and `len(s)`, not
`std::str::starts_with(...)` or `s.len()`.)

Closure-block-only vocabulary: `sum`, `prod`, `length`, `empty`.
These appear inside `closure { ... }` bodies as primitives over
the closure's tracked stream; they are not general-purpose
functions. If you are not writing a closure block, ignore them.

Type primitives: `Int`, `Uint`, `Float`, `Decimal`, `String`,
`Bool`, `Time`, `Duration`, `Bytes`. Capitalized; available in
type position only — `time::sleep` works as a path because
lowercase `time` is not reserved.

User-defined `type` records (no lifecycle) are fine for
payloads:

```aperio
type Point { x: Int; y: Int; }
```

## What is *not* shipped (the work-around-or-flag list)

If your program needs any of these, log a friction entry:

- Filesystem watch (inotify / fsevents)
- Generics (`List<T>`, `Map<K,V>`)
- Sum types in payloads / pattern-matching on enum variants
- Multiple distinct accept types in one locus
- HTTP keep-alive, custom request headers, header maps
- HTTP bodies > 8 KB (single recv assumed)
- Multi-file source (until module support lands)
- Filesystem errno disambiguation (only `-1` / `false` / `""`)
- Inline markdown formatting (`**bold**`, `*italic*`, links)
- Graphics, UI, embedded shell, MCP server
- Compiler self-introspection (`std::aperio::parse` etc.)

Flag, don't fake. Faking a missing primitive in your `.ap`
source means future-you (or another agent) won't know to lift
it into the compiler.

## How to read existing source

Reading order for a cold start:

1. **`examples/hello-world/main.ap`** — the smallest legal
   program. One locus, one lifecycle method, one builtin call.
2. **`examples/01-locus-with-run/main.ap`** through
   **`examples/05-bus/main.ap`** — these scale up the
   primitives one at a time. Skim, don't deep-read.
3. **`examples/http-hello/main.ap`** — small, complete,
   real. Composes locus + lifecycle + fn-pointer callback +
   stdlib HTTP. ~80 lines.
4. **`examples/docs-server/main.ap`** — the Phase 5 capstone.
   ~200 lines composing seven stdlib namespaces. The largest
   real Aperio program in the repo and the closest analogue to
   what you will be writing.
5. **`docs/std/src/`** — reference. Per-namespace pages with
   Synopsis / Semantics / Examples. Authoritative for surface
   you can call.
6. **`docs/reference/src/`** — language reference. Authoritative
   for syntax and semantics. Long; consult on demand, not in
   one sitting.
7. **`docs/book/src/`** — the human-style tutorial. Useful if
   the reference's prescriptive register is hard to extract a
   mental model from.

The grimoire (`docs/grimoire/src/`) is a vibes-first onboarding
path written in meta-spell register. You are an agent; you
will get more out of `docs/book/` and `docs/reference/`.

## Running and testing

The CLI has two execution modes — **read this carefully**, several
cold-start sessions have lost time to it:

```
aperio run   apps/<your-app>/main.ap [args]   # interpreter
aperio build apps/<your-app>/main.ap          # native binary at ./main
```

**`aperio run` (interpreter) does NOT support qualified-name
struct/locus literals** like `std::http::Request { ... }`,
`std::log::Logger { ... }`, etc. — it errors with "qualified-
name struct/locus literals not yet implemented." If your
program uses any path-qualified stdlib type (most non-trivial
programs do), you must use `aperio build` and then run the
produced binary.

```
# Recommended pattern:
target/debug/aperio build apps/<your-app>/main.ap
./main                                    # binary lands in cwd
```

**Stale-CLI gotcha.** `target/debug/aperio` only updates when
`aperio-cli` rebuilds. If `crates/aperio-codegen` or
`crates/aperio-runtime` was changed (by anyone), you may be
running a stale CLI that was built against the old lowering —
silent miscompile is possible (subscribers may be quietly
dropped, etc.). When in doubt, run via cargo to force freshness:

```
cargo run -p aperio-cli --bin aperio -- build apps/<your-app>/main.ap
```

This is slower (cargo checks freshness on every invocation) but
guaranteed up-to-date.

For end-to-end tests of your app, mirror
`crates/aperio-codegen/tests/docs_server.rs` or
`tests/http_hello.rs`: a Rust harness spawns the compiled
binary, exercises it, and asserts on observable behavior. Keep
your app tests in your app's directory if it has a `tests/`
subdir, else add to the codegen crate's `tests/` with a clear
filename. Run via `cargo test -p aperio-codegen <pattern>`.

## The friction-log contract

`apps/<your-app>/FRICTION.md` is append-only. The compiler
session reads it. Format:

```
## YYYY-MM-DD <short tag>

**Tried:** <one sentence: what you wanted to write>
**Hit:** <one sentence: what happened — error, missing primitive, etc.>
**Workaround:** <one sentence: what you did instead, or "blocked">
**Why it matters:** <one sentence: what feature this gates, or "minor papercut">
```

Examples of valid entries:

- "Tried to declare `accept(c: ChildA)` and `accept(c: ChildB)`
  in one locus. Hit: parser rejects multiple accept signatures.
  Workaround: split the supervisor in two. Why it matters:
  blocks any locus that supervises heterogeneous children."
- "Tried `let names: [String] = std::io::fs::list_dir(p);`. Hit:
  `list_dir` returns newline-separated `String`, not `[String]`.
  Workaround: split on `\n` manually. Why it matters: every
  caller does the same split — wants a `[String]` overload."

What is **not** a friction entry: a bug in your own code, a
lint you disagree with, a stylistic preference. The bar is
"the language got in the way of writing what should be a
correct program."

## Hard guardrails

- **Do not edit `crates/`.** That is the compiler.
- **Do not invent stdlib paths** that are not listed under
  "What is shipped" above. The codegen will reject unknown
  `std::*` paths.
- **Do not skip semicolons** to "match the example" — every
  example already has them; if yours appears not to, you are
  misreading.
- **Do not assume a feature works because it's in the
  reference's grammar.** The grammar describes the language;
  the implementation is a subset. When in doubt, write the
  smallest test program first.
- **Do not silently work around missing features.** Log them.
- **Do not commit your `apps/<name>/` work without a
  `README.md`** explaining what the app does, how to run it,
  and (importantly) what it doesn't do yet.

## First-step protocol

1. Read this brief. Re-read the counter-hallucination table.
2. Read the four reference programs (`hello-world`,
   `http-hello`, `docs-server`, plus one of `01-` through
   `05-`).
3. Skim the `std::*` reference pages for the namespaces you
   expect to need.
4. **Propose a small target to the human** before writing
   code. Shape: "I propose to build `<app>` in `apps/<app>/`.
   It will use `<list of std namespaces>`. Estimated friction
   areas: `<two or three things you suspect>`. OK to start?"
5. Wait for OK.
6. Build. Log friction as you hit it.
7. When the app works end-to-end (or hits a wall), write its
   `README.md` and a friction-summary entry pointing back to
   the per-app FRICTION log.

## Conventions worth pinning

- **Aperio = the language.** **lotus = the runtime substrate.**
  C symbols stay `lotus_*` by design; that is not a typo, do
  not "fix" it. The repo directory is currently `lotus-lang/`
  for historical reasons; that is not a typo either.
- **fitter/applier** is a project name that uses Aperio. The
  financial-trading domain has been generalized: types are
  `Observation / Kernel / Action / Receipt`; loci are
  `FitterL / ApplierL`.
- **"The ancient texts"** is the in-universe attribution
  convention for the framework's mathematical pedigree. Use it
  if the moment calls for it; don't go looking for citations.
- **No marketing language.** "Powerful," "elegant," "blazing,"
  etc. are out per `docs/STYLE.md`. Same applies to your
  README.

## When you are stuck

1. Re-read the counter-hallucination table. Most stuck-points
   come from a smuggled-in idiom.
2. Grep `examples/` for the construct you are trying to write.
   If no example uses it, suspect it does not exist.
3. Write the smallest possible program that exhibits your
   issue. Run it. Read the compiler's error. The error
   messages are usually accurate.
4. If still stuck, log a friction entry with the smallest
   repro and stop. The compiler session can act on a clear
   repro; it cannot act on prose frustration.

---

You have what you need. The spell can be cast.
