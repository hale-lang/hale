# spec/ffi.md — Foreign-function interface (`@ffi("c")`)

User-extensible bindings to external C-ABI libraries. Library
authors declare extern symbols in `.hl` source via an `@ffi("c")`
annotation; the compiler emits LLVM `declare` for the signature
and the linker resolves against C source files supplied at build
time. No stdlib expansion is required to bind a new library.

## Syntax

```hale
@ffi("c") fn raylib_init_window(w: Int, h: Int, title: String) -> ();
@ffi("c") fn raylib_should_close() -> Bool;
@ffi("c") fn raylib_clear_background(c: Color) -> ();
```

Grammar:

```
ffi_annotation ::= '@' 'ffi' '(' STRING ')'
ffi_fn_decl    ::= ffi_annotation 'fn' Ident '(' params ')' ('->' type_expr)? ';'
```

The annotation precedes the `fn` keyword. The fn body MUST be
absent — the declaration terminates with `;`. The compiler
synthesizes an empty body internally so downstream passes keep
the same `FnDecl` shape; user code MAY NOT write a `{...}` block.

The ABI string is the literal `"c"` (native C-ABI binding) or
`"js"` (a WASM host import — see [§ WASM host interface](#wasm-host-interface)).
Any other ABI string is rejected at parse time.

## Position

`@ffi("c")` is valid only on **top-level free fn declarations**.
The annotation is rejected on:

- Locus methods (`locus L { fn ...; }`).
- Mode bodies (`mode bulk { ... }`, `mode harmonic { ... }`, ...).
- Perspective method signatures.
- Interface method signatures.
- Closure declarations.

The position restriction matches the substrate's expectation that
the C-ABI boundary crosses at top-level program scope only; locus
and perspective methods carry implicit Hale-side context
(`self`, scratch arena, lifecycle hooks) that doesn't translate
to C.

## Restrictions

An `@ffi("c")` fn declaration MUST NOT be:

- **Generic.** Type parameters require monomorphization; the
  C-ABI boundary is monomorphic by definition. Declare separate
  `@ffi` fns per type if needed.
- **Fallible.** `fallible(E)` is an Hale internal channel; C
  functions report failure via error sentinels in the return
  value, and the Hale wrapper above translates to `fallible(E)`
  if exposed to user code.
- **Defaulted.** Parameter defaults are not portable across the
  C-ABI boundary; the wrapper layer applies defaults before the
  call.

The parser rejects all three with a diagnostic at the annotation
or marker position.

## Type marshalling

The typechecker validates `@ffi("c")` parameter and return types
against a portable subset. LLVM lowers each Hale type to a
matching C-ABI representation at the call boundary:

| Hale type | LLVM type | C type | Notes |
|---|---|---|---|
| `Int` | `i64` | `int64_t` | 64-bit signed throughout. |
| `Float` | `double` | `double` | 64-bit IEEE 754. |
| `Bool` | `i32` | `int32_t` | Hale's i1 zero-extends to i32 at the call, truncates back at the return. Avoids C `_Bool` cross-platform ambiguity. |
| `String` | `ptr` | `const char *` | NUL-terminated. Caller owns; callee MUST NOT retain past the call. A `StringView` does **not** implicitly coerce to a `String` parameter — it isn't NUL-terminated, so a `char*`-expecting callee would `strlen` past its end. Pass a view as a `StringView` parameter (→ `lotus_view_t`, length-carrying) or materialize it first via `std::str::clone`. |
| `Bytes` | `ptr` | `void *` (header) | Points at Hale's `[int64 len][payload]` header — callee uses `lotus_bytes_len(p)` / `lotus_bytes_data(p)` (declared in `lotus_arena.h`) to inspect. Caller owns. |
| `BytesView` / `StringView` | `{ ptr, i64 }` (struct by value) | `lotus_view_t` | 16-byte F.30b view layout. C glue MAY use `lotus_view_data` to recover the payload pointer + length. |
| `Duration` / `Time` | `i64` | `int64_t` | Both are 64-bit nanosecond counts under the hood. |
| `()` (unit) | `void` | `void` | Return-position only — declared as `-> ()` or omitted entirely. Empty-tuple return type accepted but normalized to `()`. |
| User struct (`type T { ... }`) | `ptr` | `const T *` (param) / `T *out` (sret return) | Passed by pointer at the boundary; struct returns use a hidden sret first arg (see User-type structs section below). Layout match is the library author's responsibility. |

Reserved at Stage 1 (typecheck rejects with a clear diagnostic):

- `Decimal` — i128 mantissa with platform-variable ABI. Marshal as
  `Int` (raw mantissa) or `Float` (lossy conversion) at the
  Hale side; the wrapper handles the scale.
- `Uint` — Hale-internal type; declare as `Int` at the FFI
  signature.
- Projections / fixed-size arrays / tuples — no portable C struct
  layout for these v0 shapes.
- `fallible(E)` — internal channel; see Restrictions above.
- Function-pointer types — wrap as a struct/handle at the C side.
- `LocusRef`, `Cell` — Hale-internal.

### User-type structs

User-type structs (`type Color { r: Int = 0; ... }`) are passed
**by pointer** at the C-ABI boundary, not by value. The Hale
side already stores user structs as heap pointers, so the natural
mapping is `ptr` at the LLVM level. C glue authors write:

```c
// Param-position: const T * (or T * if the callee mutates).
void raylib_clear_background(const Color *c) {
    ClearBackground((::Color){
        (uint8_t)c->r, (uint8_t)c->g,
        (uint8_t)c->b, (uint8_t)c->a,
    });
}
```

Struct returns use **sret-style**: Hale allocates the return
slot in the caller's arena and passes a pointer as a hidden first
argument. The LLVM-level fn signature is `void foo(T *out,
<user args>)`; the C glue writes the struct into `*out`:

```c
// Return-position: hidden T *out first param, returns void.
void vec3i_scale(Vec3i *out, const Vec3i *v, int64_t k) {
    out->x = v->x * k;
    out->y = v->y * k;
    out->z = v->z * k;
}
```

The Hale-side call expression
```hale
let scaled = vec3i_scale(v, 10);
```
sees the sret slot's pointer as its result — same value-shape
as any other struct-returning expression. The sret transformation
is hidden from user code; only the C glue author sees it.

**Why pointer + sret instead of by-value:** SysV / Win64 / aarch64
all classify struct-by-value differently based on size. A
portable implementation would need a per-platform ABI-lowering
pass. The pointer convention sidesteps that entirely — every
target lowers `ptr` the same way — at the cost of one
dereference per arg on the C side. For the workloads Hale is
shaped for (locus methods, bus dispatch, FFI to system
libraries), that cost is negligible compared to the portability
win.

**Layout contract:** the Hale struct's field order + types must
match the C struct on the other side. The library author
guarantees this. Future spec iteration may add a compile-time
layout-assertion mechanism (`@ffi_layout("c")` on the `type`
decl); today the contract is documented but not machine-checked.

## Calling convention

`@ffi` fns differ from regular Hale free fns at the LLVM ABI
level:

- **No implicit `__caller_arena` first parameter.** Regular free
  fns receive the caller's `current_arena_ptr()` as an implicit
  prefix; `@ffi` fns do not.
- **No fallible sret slots.** `@ffi` fns can't be `fallible(E)`,
  so the sret-pair the substrate emits for fallible returns is
  absent.
- **No monomorphization.** `@ffi` fns can't be generic.

The LLVM symbol name is the literal Hale fn name as written.
There is no `__std_*` mangling, no per-import alias prefix, no
generic-instantiation suffix. The library author's C glue
exports a function with that exact name; the linker resolves
directly.

## Lifetime rules

The Hale-side caller of an `@ffi` fn owns every pointer it
passes. The C-side callee MUST:

- NOT retain `String` / `Bytes` / view pointers past the call
  boundary. If C needs persistent storage, it must copy into its
  own malloc'd memory.
- NOT free or write through any pointer received from Hale.
  Arena-owned pointers are read-only at the C side.

If a C function needs to RETURN heap-allocated `String` or
`Bytes`, the convention matches stdlib primitives that allocate
return values: call `lotus_arena_alloc(lotus_caller_arena_or_global(),
size, align)` to land the storage in the caller's arena, then
return the pointer. The caller's arena outlives the C-side
function frame, so the returned pointer survives.

Exceptions MUST NOT cross the FFI boundary. C code that fails
returns an error sentinel (NULL, -1, etc.); the Hale-side
wrapper translates to a `fallible(E)` shape if the error needs
to propagate.

## Build surface

The `hale build` CLI accepts repeatable flags that thread the
library author's C glue + link surface through to clang:

```
hale build mydir/ --link raylib --csrc pond/raylib/glue.c \
                    --link curl   --csrc pond/curl/glue.c
```

- `--link <name>` — appended as `-l<name>` to the clang link
  line. The system's dynamic linker resolves at runtime.
- `--csrc <path>` — passed directly to clang as a translation
  unit compiled alongside the C runtime. The library author's
  `.c` glue file goes here. May be repeated for multiple files.

Both flags are optional; programs that don't use `@ffi`
declarations don't need either.

### `hale.toml [ffi]` auto-pickup (Stage 2)

When `hale build` resolves an `import` against a directory
that contains an `hale.toml`, it reads the file's `[ffi]`
section and appends those values to the build's link surface
automatically. `hale run` and `hale replay` build with the same
options, from the same function, and `hale test` and `hale bench`
apply the same pickup. Library authors ship:

```toml
# pond/raylib/hale.toml
[ffi]
link = ["raylib"]
csrc = ["glue.c"]
```

Consumers then just `import`:

```hale
// myapp/main.hl
import "vendor/raylib" as ray;

fn main() {
    let w = ray::Window { width: 1280, height: 720 };
    ...
}
```

`hale build myapp/` reads `vendor/raylib/hale.toml`, picks
up `link=["raylib"]` + `csrc=["glue.c"]`, and threads them
through to the clang invocation. The CLI flags from the prior
section still work as additive overrides (CLI first, then toml-
sourced). A flag naming what an imported package's `[ffi]`
names — a `--csrc` with the canonical path of a package's `csrc`,
a `--link` of a library a package links — is a no-op, not an
error: the package's entry stands and the flag's is dropped, so
the C file is compiled once and the options (and the execution
identity) are the flagless invocation's. This holds for `hale
build`, `hale run` and `hale replay` alike. Single-file imports
(`import "helpers"` → `helpers.hl`) have no companion toml and
contribute nothing.

De-duplication: a lib referenced under two aliases or via
multiple files in the same seed contributes its FFI flags once
per unique resolved directory.

Transitive FFI is NOT walked at Stage 2: only the entry's
top-level imports are scanned for `hale.toml`. If a directly-
imported lib itself imports another `@ffi`-using lib, the
transitive lib's `[ffi]` must be re-declared (or surfaced via
manual `--link` / `--csrc`) at the entry. Resolved if a workload
surfaces the need.

## Library-author surface

A binding library typically ships:

1. A `.hl` file with `@ffi("c") fn ...;` declarations + the
   user-facing Hale wrapper (locus, types, idiomatic
   signatures).
2. A `.c` file exporting the C-side symbols declared in the
   `.hl`. Often a thin shim from Hale's snake_case to upstream
   C naming.
3. (Stage 2) An `hale.toml [ffi]` section declaring
   `link = [...]` and `csrc = [...]`.

Example skeleton (pond/raylib):

```hale
// pond/raylib/raylib.hl
@ffi("c") fn raylib_init_window(w: Int, h: Int, title: String) -> ();
@ffi("c") fn raylib_close_window() -> ();

locus Window {
    params { width: Int = 1280; height: Int = 720; title: String = ""; }
    birth()    { raylib_init_window(self.width, self.height, self.title); }
    dissolve() { raylib_close_window(); }
}
```

```c
// pond/raylib/glue.c
#include <stdint.h>
#include "raylib.h"
void raylib_init_window(int64_t w, int64_t h, const char *t) {
    InitWindow((int)w, (int)h, t);
}
void raylib_close_window(void) { CloseWindow(); }
```

```toml
# pond/raylib/hale.toml (Stage 2)
[ffi]
link = ["raylib"]
csrc = ["glue.c"]
```

## Diagnostic surface

Parser errors:

- `expected ; (an @ffi fn declaration has no body), got LBrace`
  — body block written after the signature; convert to `;`.
- `unsupported FFI ABI "<x>" — Stage 1 accepts only "c"`
- `\`@ffi\` fn must not be generic — the C-ABI boundary is
  monomorphic`
- `\`@ffi\` fn must not be \`fallible(...)\` — C functions
  return an error sentinel, the Hale wrapper above translates
  to \`fallible(E)\` if needed`
- `expected \`fn\` after \`@ffi(...)\` annotation`
- `expected \`fn\` after \`@export\` annotation`
- `\`@export\` and \`@ffi\` are mutually exclusive — an FFI import
  is not a module export`

Typecheck errors:

- `\`@ffi\` fn \`<name>\` parameter \`<p>\` has type Decimal —
  Decimal (i128) has platform-variable ABI; marshal as Int/Float
  at the Hale side instead`
- `\`@ffi\` is only valid on top-level free fns at Stage 1, not
  on locus methods`

Codegen errors:

- `@ffi fn \`<name>\` parameter \`<p>\`: type <T> is not yet
  wired for FFI codegen at Stage 1` — user-type structs, arrays,
  etc. fall here.
- `@ffi fn \`<name>\`: parameter defaults are not supported
  across the C-ABI boundary`
- `@export fn \`<name>\`: fallible exports are not supported yet
  (wasm entry-inversion v1)`

## WASM host interface

On the `wasm32` target (`hale build --target wasm32`, or a program that
declares `target wasm { }`) the foreign boundary is the JavaScript
host rather than a C library. The same `@ffi` machinery serves the
inbound direction, and a dual annotation `@export` serves the
outbound direction.

### Package `[ffi]` under wasm32

A package's `[ffi] csrc` translation units **are compiled for wasm32**
and linked into the module (#213, 2026-08-03). Before that they were
silently skipped: `link_wasm` was invoked without the build options,
so every `@ffi("c")` symbol a package defined in C surfaced as an
undefined `env` import, `--allow-undefined` swallowed it, and the
generated JS loader stubbed unknown imports with `() => 0`. The build
reported success and every such call returned 0 forever.

Two constraints follow from the wasm build being **freestanding**:

- **No libc sysroot.** The runtime is compiled with bare
  `--target=wasm32` against a forward-declared shim, not wasi-sdk.
  There are no system headers, so a translation unit that
  `#include`s `<string.h>` will not compile for this target. That is
  now a build error naming the file, rather than a silently missing
  symbol.
- **`[ffi] link = [...]` is rejected.** It names a system dynamic
  library, and wasm has neither a dynamic linker nor system
  libraries. Dropping it silently would reproduce the same failure one
  level up, so it is an error that points at `csrc` as the
  alternative. It is a property of the program, the configuration and
  the target, never of the machine: `hale build` reports it before any
  tool is looked up (a machine without clang meets this refusal, not a
  missing compiler), and `hale check --target wasm32` reports it too,
  as a record against the package manifest's `link` line
  (``lib/glue/hale.toml:5:1: error: `[ffi] link = ["m"]` cannot be
  satisfied on wasm32 — …``). A `--link <lib>` flag, which `hale check`
  takes as `hale build` does, is the same input, refused naming the
  flag.

One sharp edge worth stating, because linking the C is what exposes
it: **Hale's `Int` is 64-bit, so a C declaration must use `long long`,
not `int`.** A mismatch links, and then traps at call time with a wasm
`signature_mismatch`. That is louder than the native ABI, which would
quietly truncate — and far louder than the previous behaviour, where
the symbol was a stub and the mismatch could not be observed at all.

### The `target` declaration + stdlib gating

The program opts into the wasm backend with a top-level `target`
declaration whose name is **`wasm`** (or the alias **`browser_js`** —
both select the same backend and gating):

```
target wasm { }
```

**Top-level is part of the rule, not a habit.** A `target`
declaration inside a `module { }` is a parse error (GH #901,
`spec/semantics.md` § "Declarations inside `module { }`"): every
consumer of a target reads the program's own item list, so one
declared at depth used to be accepted and then ignored — the same
program that this section gates reported `ok` with its `target wasm
{ }` one brace deeper.

**The effective target.** `hale check`, `hale build` and the editor
act on one target, for analysis and for the artifact alike:

- an explicit `--target` (on `hale build`, and on `hale check`, which
  takes the same flag) is the effective target;
- with no `--target`, a written `target wasm { }` / `target browser_js
  { }` selects **wasm32**: `hale build` of such a program emits the
  wasm module and its loader, exactly as `--target wasm32` does;
- with neither, the host.

An explicit `--target` of another class than a written declaration's
is refused at the declaration, on `hale check` and `hale build` alike:
``this program declares `target wasm`, and is being checked for
`<triple>`: build it with `--target wasm32`, or drop the declaration``.
The editor takes no `--target`, so it shows what `hale check` and `hale
build` without one compute. `hale run` and `hale replay` execute what
they build, and a declared program builds a module this host cannot
execute, so they refuse it, as they refuse `--target wasm32`.

The portable stdlib (`std::str`, `std::bytes`, `std::json`,
`std::math`, `std::text`, …) works unchanged. The **POSIX-backed
namespaces are rejected at typecheck** under this target — the browser
sandbox has no syscalls — with the diagnostic ``error: `std::...` is
unavailable under `target wasm`: <reason>`` (``under `--target
wasm32` `` when the configuration, not a declaration, put the program
under wasm32). The gated set is exactly the table below, rendered from
the compiler's capability matrix (`hale_types::capability`).

**Every use, not every spelling.** The refusal covers every way a
program reaches a gated namespace, read off the resolved program
(`hale_types::capability::uses`), not the text of its calls:

- a call spelled with the stdlib path, refused at the call with the
  wording above;
- a construction of a stdlib locus (`std::io::tcp::Listener { … }`,
  in a body or a params initializer): its lifecycle runs, so it is
  refused at the literal;
- a method on a handle (`conn.recv(64)` with `conn:
  std::io::tcp::Stream`), resolved through the receiver's type;
- a call into another seed through its import alias (`c::stamp()`),
  refused at that call when the seed's fn reaches the namespace;
- a construction of another seed's locus (`lib::Kid { }`), refused at
  the literal when what its existence runs reaches the namespace: its
  lifecycle, its params initializers and its `on_failure` handler — the
  same requirements that refuse the locus declared in the program's own
  sources at the use inside it.

A use reached through a chain is worded ``error: `std::<namespace>` is
unavailable under `target wasm`: <reason> — witness: `<use>` → … →
`<primitive>` ``, naming each link. The program's own sources are the
horizon: a fn the program writes is judged at the use in its own body,
once, and the calls to it carry nothing; a fn of an imported seed or of
the stdlib is judged at the call that crosses into it, and a locus of
one at its construction. The horizon relocates a refusal; it never
erases a requirement. A type that only
names a gated namespace — a parameter, field or return typed
`std::io::tcp::Stream` — is not a use. A call through a local bound by
`let` to a fn or a stdlib path is that fn's use, the local a link of the
witness (`let f = pid; f()` → `` `f` → `pid` → `std::process::pid` ``).
In a fn's, a hook's or a method's body, a call through a function value
the local does not name — a function-typed parameter, a computed callee,
a local bound to anything but a fn (`let f = self.g; f()`) or reassigned
(a local a `while` or `for` loop reassigns anywhere in its condition or
body is unresolved for the whole loop and after it, since a call ahead of
the assignment runs, on the next iteration, the value it stored) —
reaches the program's function values of its type (spec/verification.md,
"An indirect call reaches the program's function values"), each that
fn's use, the local a link of the witness (`` `lib::via` → `f` →
`lib::pid` → `std::process::pid` ``). A call whose requirements cannot
be established (a method on a receiver whose type is not known; such a
call no function value of the program can be; and, in the program's own
or another seed's params initializer or `on_failure` handler, any call
through a local function value bound to anything but a fn, or
reassigned, by the same loop rule) is refused under wasm32 as ``cannot
establish what `<callee>` requires on wasm32: <why>``, since an unknown
requirement is never an admission there; on the native targets it is
admitted.

<!-- capability-matrix: wasm32 stdlib (generated by hale_types::capability::render_markdown; do not edit) -->

| Rejected path | Browser substitute |
|---|---|
| `std::io::tcp` | a WebSocket bus adapter (`ws://`) |
| `std::io::udp` | (no raw UDP in the browser) |
| `std::io::tls` | the browser does TLS transparently for `wss://` / `https://` |
| `std::io::fs`, `std::io::file` | `fetch` via an `@ffi("js")` host import, or a bus message |
| `std::io::h2` | a WebSocket bus adapter (`ws://`), or an `@ffi("js")` host import |
| `std::io::stdin`, `std::io::stdout` | `println(...)` (the loader routes it to the host console) |
| `std::term` | (no terminal in the browser) |
| `std::process` | (no OS process control) |
| `std::http` | (server is built on raw TCP) |
| `std::env` | configuration handed in through an `@ffi("js")` host import or an `@export` fn's arguments |
| `std::io::mirror` | (no shared memory in the browser) |
| `std::io::sockopt` | (no sockets in the browser) |
| `std::io::unix` | a WebSocket bus adapter (`ws://`), or an `@ffi("js")` host import |
| `std::ring` | (no shared memory in the browser) |
| `std::time` | a host clock (`performance.now`, `Date.now`) or timer through an `@ffi("js")` host import |
| `std::ts` | parsing on the host, through an `@ffi("js")` host import |

<!-- /capability-matrix -->

The **in-process typed bus is fully available** under `target wasm`:
`topic` declarations and `bus { publish … }` / `bus { subscribe … }`
across loci lower the same way they do natively — a `Subject <-
payload` is delivered to every matching in-module subscriber's handler,
payload-copied through the synthesized `__serialize_T` / `__deserialize_T`
wire codec. Those codecs follow the `lotus_serialize_fn` /
`lotus_deserialize_fn` ABI (`ssize_t(const void *, …, size_t)`), whose
`ssize_t` / `size_t` widths are **target-pointer-width** — i32 on wasm32,
i64 on the native 64-bit targets — so the runtime's `lotus_bus_dispatch`
indirect call matches the codec on both. The *cross-process /
network* transports need syscalls the sandbox does not have, and the
module runs on its host's one thread, so under wasm32 the check refuses
every `bindings { }` entry (`unix`, `shm_ring`, an adapter) at the
binding, and every placement that asks for a thread of its own or a
pool's — `pinned`, `cooperative(pool = X)` with X other than `main`,
`where async_io` — at the placement entry. (Before, a `unix` or
`shm_ring` binding and a pool were admitted and never ran, and a
`pinned` locus or an adapter failed late in wasm-ld.)

Reach the outside world through `@ffi("js")` host imports and the
inbox/state seam below instead.

### `@ffi("js")` — host imports (host → into Hale's callees)

`@ffi("js") fn name(...);` declares a function the **JS loader**
provides at instantiation (a wasm `env` import), e.g.:

```
target wasm { }
@ffi("js") fn console_log(msg: String);
@ffi("js") fn draw_line(x1: Float, y1: Float, z1: Float,
                        x2: Float, y2: Float, z2: Float);
```

Marshalling: `Float` passes directly as a JS `number` (f64). `Int`
and `Duration` are i64 internally, but at the **`@ffi("js")`** boundary
they marshal as **f64 (JS `number`), not i64 (which crosses as a JS
`BigInt`)** — the host handler receives a plain number, with no
`Number(x)` dance, and an `Int`-returning host import accepts a plain
JS number back (the runtime `sitofp`s before the call and `fptosi`s the
return). The trade-off is f64's 53-bit integer range: an `Int` whose
magnitude exceeds 2^53 loses precision across this boundary — pass such
values as a `String`/`Bytes` payload instead. (This is **only**
`@ffi("js")`. `@ffi("c")` keeps i64 — on wasm those resolve to linked
runtime C symbols that genuinely expect i64.) `String`/`Bytes` pass as
a pointer into wasm linear memory (the loader reads them with a
`TextDecoder` over the module's `memory`). The generated `.mjs` loader
supplies a built-in `console_log` plus the libm set
(`sin`/`cos`/`tan`/`sqrt`/… mapped to JS `Math.*`, so `std::math` works
under wasm with no app glue); an app wires its own imports through
`run(glue)`. Position and the generic / defaulted restrictions are the
same as `@ffi("c")`.

Only the wasm32 loader supplies a `js` import. On a native target an
`@ffi("js")` declaration is refused at the declaration, called or not
(the declaration is the use): ``` `@ffi("js")` fn `name` is a host import
of the wasm32 loader, and this program is built for `<triple>`: … ```,
where it used to become an undefined symbol at the native link, and only
once something called it.

### What a wasm32 module imports

The link keeps every symbol nothing defines as an `env` import
(`wasm-ld --allow-undefined`). The generated loader supplies an import
from its writers (the libc output functions `println` lowers to,
`console_log` and the libm set) or from the app's `run(glue)`, and
stubs any other with `() => 0`. The runtime's thread, pool, mailbox and
transport paths a wasm32 module cannot run (the target refuses
`pinned`, every pool but `main` and every transport binding) are
compiled out of it, the runtime's own `fprintf(stderr, …)` messages
are no-ops there, and codegen emits no observation probe for a target
that is neither recorded nor replayed. So a module imports the loader's
writers and the program's declared `@ffi("js")` names, and today two
more, from generated code:

- `dprintf`: what `eprint` and `eprintln` lower to, and the report of a
  violation no handler absorbs;
- `fflush`: the flush of stdout before that report.

The loader has no stderr writer, so it stubs both with `() => 0`: on
wasm32 `eprintln` writes nothing, and an unabsorbed violation's message
is dropped before its `exit(1)` traps the module.

An `@ffi("c")` name no `[ffi] csrc` defines is imported too, and runs as
`() => 0` under the generated loader. A host that instantiates the
module with its own imports object supplies every one of these names.
`crates/hale-codegen/tests/wasm_import_backstop.rs` holds every module
the wasm tests build to this list. It checks linkage only: an operation
the runtime lowers to an inline stub imports nothing, so passing it does
not show the operation does anything.

### `@export` — exports (Hale → callable by the host)

Two forms; both are wasm-only (a **no-op on the native target**) and
both produce a wasm module export the host calls by its literal name.

**`@export fn name(...) { ... }`** — a top-level free fn. Unlike
`@ffi` it has a real Hale body. It is valid only on top-level free fns
(same position rule as `@ffi`), is **not** `@ffi` (mutually exclusive
— an import is not an export), and is **not** `fallible(E)` (v1 — the
host has no error channel).

**`@export locus L { ... }`** — the persistent singleton "app." At
most one per program. It is instantiated **once** (birth runs; it is
never dissolved), and each of its non-fallible `fn` methods becomes a
wasm export the host calls (`inst.exports.<method>()`). State lives in
the locus's params — ordinary Hale fields that survive across calls
because the singleton persists. The locus **must not define `run()`**
(it is host-driven via its methods, not a cooperative run loop):
under wasm32 the check refuses the `run()` it writes with ``@export locus
`L` must not define `run()` — a wasm singleton is host-driven via its
`@export` methods, not a cooperative run loop``; natively the locus is
an ordinary one. `fallible` methods stay internal (not exported).

### Entry-inversion run-model

A program built with `@export` runs **inverted**: instead of a
blocking `main`, the host drives the exports. The compiler synthesizes
and exports **`_hale_start()`**, which creates a **persistent** program
arena (and bus queue) that is *not* torn down — and, for an `@export
locus`, instantiates the singleton there and stashes its pointer. The
generated loader calls `_hale_start` once at instantiation and then the
host calls the exports (e.g. one per `requestAnimationFrame`). A
program with no `fn main` is valid when it has any `@export` and its
effective target is wasm32; on a native target its entry is its `fn
main`, so the check refuses it at its first `@export`: ``a program with
no `fn main` is an export-only module, which needs wasm32: declare
`target wasm { }` or build with `--target wasm32` ``. If `_hale_start`
is present the loader does **not** call `main` (its create-then-destroy
of the arena would clobber the persistent one).

**`--wrap-main` (browser-playground entry synthesis).** A bare
`fn main` program is not the `@export` shape a wasm build needs. The
`hale build … --target wasm32 --wrap-main` flag synthesizes it *on the
parsed AST*: when the program has a top-level `fn main()` and no
`@export` entry, it replaces `fn main` with an
`@export locus __Main { birth() { <main's body> } }` (routing the body
through the `_hale_start` path) and injects a `target wasm { }` gate if
absent. Because it operates on the AST — not the source text — every
diagnostic keeps the user's original line/col (no offset) and a `{`/`}`
inside a string or comment can't mis-wrap it. It is **wasm-only and
opt-in**: a hard error unless the effective target of the written
sources is wasm32 — `--target` in any wasm32 spelling, or a written
declaration; the declaration the flag injects never selects the target
— since there is no native entry-inversion to wrap. It is never implied
by the target (a wasm program
may legitimately keep a bare `fn main` exported as `main`), and a no-op
when an explicit `@export` entry already exists (prefer-explicit).

Holding state across calls:

- **`@export locus` (preferred):** state is the locus's fields,
  mutated in one method and read in another — plain Hale, no
  marshalling. This is the natural shape for a browser client.
- **`@export fn` (lower-level):** each call's allocations are released
  on return, so cross-call state goes through the runtime **host seam**
  — `@ffi("c") fn lotus_wasm_state_set(b: Bytes);` /
  `lotus_wasm_state_get() -> Bytes;` deep-copies a packed `Bytes` blob
  into its own arena so it survives.

Inbound messages use the seam in either model:
`lotus_wasm_alloc(n)` / `lotus_wasm_set_inbox(len)` (wasm exports the
host calls to write bytes in) + `@ffi("c") fn lotus_wasm_inbox() ->
Bytes;` (Hale reads them and parses with `std::json` / `std::bytes`).

## Cross-references

- `notes/ffi-design.md` — design memo capturing the agreement
  the Stage 1 surface graduated from, plus the Stage 2/3 staging
  plan still pending implementation.
- `spec/stdlib.md` — `std::*` paths are NOT the only way to
  bind C libraries; this spec is the user-extensible alternative.
- `spec/runtime.md` — the C-runtime helpers (`lotus_bytes_*`,
  `lotus_arena_alloc`, `lotus_caller_arena_or_global`, etc.)
  that library authors typically call from C glue.
- `docs/src/systems/webassembly.md` — the pedagogical companion to
  the WASM host interface above (the browser-client walkthrough:
  loader `run(glue)`, the inbox, the `@export locus` game loop).


## C→Hale re-entry: `@export fn` on native (Crumb batch 2, 2026-07-27)

The inverse direction of `@ffi`: an `@export fn` free fn is
emitted as an **unmangled C-ABI symbol** on native targets (the
same annotation wasm entry-inversion already used; the internal
arena-ABI implementation is renamed `__hale_impl_<name>` and the
literal name is claimed by a C-callable wrapper). C code — an
engine registering host functions, a callback-taking library —
links the symbol and calls straight into Hale.

**Marshalling** is the same FFI-portable set as imports, in the
same C shapes: `Int` ↔ `int64_t`, `Float` ↔ `double`, `Bool` ↔
`bool`, `String` ↔ `const char*`, `Bytes` ↔ lotus blob pointer
(`lotus_bytes_len`/`lotus_bytes_data` are linkable for C-side
access). Typecheck rejects non-portable params/returns, defaulted
params (fixed C arity), and fallible exports (no C error
channel).

**The v1 contract is same-thread re-entry**: the callback must
fire while Hale is inside an in-flight `@ffi` call on that same
thread (the engine-host-function shape — JS calls a host fn
during `crumb_js_run_file`). Codegen publishes the call site's
arena in the caller-arena TLS around every `@ffi` call
(save/set/restore, nesting-safe); the export wrapper's prologue
picks it up, so the re-entered body composes with the established
context — bus publishes dispatch, eager loci instantiate and
dissolve, all on the thread that owns them. Entry from a foreign
thread (or outside any in-flight `@ffi` call) aborts with a
pointed diagnostic naming this section; cross-thread entry is the
documented follow-up.

**Invariants the re-entered code inherits** (spelled out per the
Crumb request): re-entry runs on the calling thread's context —
the same single-threaded interiority as the locus/fn that made
the `@ffi` call. Publishes enqueue exactly as they would from
that caller; a re-entered export must not assume it runs on main
unless the `@ffi` call site does. `JSValue`-style by-value
foreign structs never cross — trampolines convert to the
marshalling set C-side (the pond/sqlite handles-in/scalars-out
pattern).
