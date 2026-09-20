# Libraries (pond)

The standard library covers the substrate — I/O, time, strings,
JSON, HTTP, crypto, the bus. Everything else — web stacks,
databases, observability — lives in **pond**, the contributed
library catalog: <https://github.com/hale-lang/pond>.

*Many lotus grow in a pond.* Each library is a directory of `.hl`
loci you vendor into your project.

## Using one

Declare it in `hale.toml`, fetch it, import it:

```toml
[deps]
pond = { git = "https://github.com/hale-lang/pond", tag = "v0.1.0" }
```

```sh
hale fetch
```

```hale
import "vendor/pond/router" as router;
```

Everything the library declares is then reachable as
`router::Name`, and a qualified literal — `router::Config { ...
}` — is typechecked against the library's own declaration:
misspell a field and `hale check` says so, the same as for a
type you declared yourself.

So is a qualified type written as an *annotation*. `let c:
router::Config = "dev";` is a type error naming `router::Config`,
and so is the same type in a parameter, a return, a struct field,
a `params` field or a `capacity` slot — the library's declaration
is the type, and `c.timeuot` is a misspelled field rather than
something the checker shrugs at. Import the same library under two
aliases and you still have one type: what `a::Config` builds fits
where `b::Config` is wanted.

The one thing to know is *when*: this needs the whole program, so
run `hale check` on the seed (`hale check .`) rather than on a
single file. Checking one file of a multi-file seed leaves an
imported type opaque, because the `import` line it needs may be in
a file you did not hand it. `build`, `run` and `test` always see
the whole thing.

`hale fetch` clones each dependency into `vendor/<name>/` and
pins the resolved commit in `hale.lock`. Pond's "no transitive
dependencies in v1" rule means every package your program pulls
in is visible in your lockfile — if a library uses another, you
vendor both explicitly.

## The alias is not a value

The name after `as` is a namespace, not a binding, so it can be
the same as a fn you declare:

```hale
import "../core" as core;

fn core(args: String) -> String { return "[" + args + "]"; }

fn go() -> String {
    return core::greet("mid") + " " + core("x");
}
```

`core::greet` goes through the import; `core("x")` calls your fn.
Paths take the alias, bare names take the value — and it reads the
same way when someone else imports *your* seed.

Types are the exception: `Name::Thing` is also how you write an
enum variant, so if you name an alias after one of your own types
the type wins and the alias becomes unreachable in path position.
Aliases are lower-case by convention, which keeps them out of the
way of type names.

An alias also belongs to the seed that declares it: a library you
vendor may call something `u` while your own program calls a
different library `u`, and each `u::f()` means the one its own seed
imported. It follows that a seed can only use the aliases it
declares itself — `u::f()` written in a seed with no `import … as
u;` is a check error that names the seed which does declare `u`,
rather than quietly borrowing that seed's import.

## One library, however you spell it

A library is a directory, and `main.hl` is that directory's entry
file rather than a library of its own — so `import "../lib/main"`
and `import "../lib"` name the same library. Both spellings see
every file of the seed, and both resolve to one set of symbols, so
a value your app builds as `lib::Config` is the same type the
library's own signatures mean by it. Spell it whichever way in
whichever file; there is one library either way.

Any *other* single file is its own small library: `import
"../lib/helper"` brings in `helper.hl` and nothing else.

## When the path names nothing

A path that resolves to none of those places fails the check where
you wrote it:

```text
/tmp/app/main.hl:1:8: type error: could not resolve import `../nowhere` (tried /tmp/app/../nowhere.hl, /tmp/app/../nowhere/, and workspace-root/../nowhere/)
    import "../nowhere" as nowhere;
           ^^^^^^^^^^^^
```

The three paths in the message are the three places the compiler
looked, in order, so a typo and a library you have not vendored yet
look different at a glance. The caret is under the string itself —
the thing to change — and the file it names is the file holding the
`import`, which for a two-hop failure is the library's file rather
than yours.

`build`, `run` and `test` print that same line, and `hale check
--json` carries it as one record with the file, line and column in
it, so an editor opens the `import` rather than the top of the
program.

If the path *does* resolve but there is nothing to compile there —
a vendored directory with no `.hl` files in it, a checkout that
did not finish — the report names that directory instead. The
import was found; reading what it named is what failed.

## When the name after the alias names nothing

An `import` that resolves does not make every `alias::Name` you go
on to write real. The compiler answers the path itself, where you
wrote it:

```text
/tmp/app/main.hl:4:13: type error: `zz::f`: `zz` is not an import or a type of this seed
/tmp/app/main.hl:6:13: type error: `b::Greeting` is not declared by the library imported as `b`; `b` provides: Config, Route, serve
```

The first is a head that is nothing at all — no `import … as zz;`
in this seed, and no type of your own called `zz`: a typo, or an
import you meant to add. The second is your own alias with a name
the library does not declare; when a spelling is close the message
says "did you mean", and when none is it lists what the library
provides.

Both hold in every position a path can stand in — a type
annotation, a call, a `Name { }` literal, a const, an enum variant
— and a `bindings { }` entry naming a topic nothing declares has
always reported itself the same way. `build`, `run` and `test`
print the same finding, so a gate that runs `hale check` and a
build that runs later agree about the program.

A third shape belongs here: a stdlib call with the `std::` prefix
left off. `env::args_count()` is not an import of anything, and the
message says what it is — "`env::args_count` is unresolved — did you
mean `std::env::args_count`?" — at the call rather than at the end of
a build. (`time::sleep` and `time::monotonic` are the exception the
compiler still answers unprefixed; they are old spellings, and
`std::time::` is the one to write.)

The *when* is the one above again: this needs the whole program.
`hale check <one file>` stays quiet about a qualified path, because
the `import` line that would answer it may be in a file you did not
hand it. `hale check .` and every build path see the whole seed.

## The catalog

**Persistence & data**

| Library | Provides |
|---|---|
| `db` | Driver-agnostic database surface: the `DbDriver` interface + `Args` bind-parameter list for parameterized (`$1, $2, …`) queries. Pick a backend (`pq`, `sqlite`) at the `DbDriver` slot. |
| `pq` | PostgreSQL driver — `PgConn` plus `PgPool`, a fixed-size fd connection pool that itself satisfies `db::DbDriver`. |
| `sqlite` | SQLite connection + fallible query surface. |
| `migrations` | Schema migration runner (up/down); builds to a `migrate` binary. |
| `jobs` | SQLite-backed job queue (`Queue`) + a pinned-worker pool. |

**Web**

| Library | Provides |
|---|---|
| `http` | HTTP client (`http/client`) over `std::io` — request/response building atop the socket primitives, for libraries that need an HTTP client without the full `std::http` server surface. |
| `router` | HTTP router over `std::http` — method + path-param routes, middleware chain. |
| `sessions` | Stateless, HMAC-signed cookie sessions (`session=<base64(payload)>.<base64(hmac)>`). |
| `websocket` | Synchronous, owner-driven RFC 6455 WebSocket client (suggested alias `ws`); a passive wrapper your own `run()` loop drives. |

**Observability & supervision**

| Library | Provides |
|---|---|
| `logfmt` | Alternative `std::log` sinks wearing the `std::text::Sink` shape — file with rotation, structured output. |
| `metrics` | Counter / gauge / histogram primitives + a Prometheus text-format renderer and `/metrics` endpoint. |
| `tracing` | Span tree mirroring the locus tower — one `Tracer` per app; spans nest with locus instantiation. |
| `supervisor` | Erlang/OTP supervision-tree strategies grafted onto Hale's `on_failure` + `restart` / `restart_in_place` / `bubble`. |

**Primitives & composition**

| Library | Provides |
|---|---|
| `crypto` | SHA-256, HMAC-SHA256, hex encode/decode, constant-time compare, CSPRNG. |
| `subprocess` | Spawn + manage child processes (suggested alias `sub`) — wraps the `std::process` spawn / wait / pipe primitives. |
| `tower` | Run several independent locus trees ("towers") under one process, each with its own root and lifecycle. |

**Terminal & UI**

| Library | Provides |
|---|---|
| `term` | Tier-0 terminal infrastructure — capability/`is_tty` probes, SGR styling, raw-mode guard, cursor + screen control over `std::term`. |
| `tui` | An Elm-shaped TUI runtime: write a locus with model/update/view, the runtime drives the frame loop, input, and rendering. |

**AI & numeric**

| Library | Provides |
|---|---|
| `agent` | LLM-agent toolkit — `agent/{llm, tools, conversation, embeddings, sandbox}`: a client surface, a tool-registry, conversation state, and a sandboxed execution path. |
| `ml` | Neural-network primitives (`ml/neural`). |
| `math` | Numeric helpers — `math/{matrix, stats}`. |

> The tree-sitter grammar that drives editor highlighting lives at
> [tree-sitter-hale](https://github.com/hale-lang/tree-sitter-hale)
> (it moved out of pond — it's developer tooling, not a library you
> `import`). The `_util` directory holds internal helper libs
> consumed by other pond libs, not imported directly by apps.

Pond is where the ecosystem grows: if a protocol, parser, or
shape is too useful to rewrite per project but doesn't belong in
the language, it lands here.
