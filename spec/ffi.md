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
automatically. Library authors ship:

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
sourced); duplicates are tolerated. Single-file imports
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

On the `wasm32` target (`hale build --target wasm32`; the program
declares `target wasm { }`) the foreign boundary is the JavaScript
host rather than a C library. The same `@ffi` machinery serves the
inbound direction, and a dual annotation `@export` serves the
outbound direction.

### The `target` declaration + stdlib gating

The program opts into the wasm backend with a top-level `target`
declaration whose name is **`wasm`** (or the alias **`browser_js`** —
both select the same backend and gating):

```
target wasm { }
```

The portable stdlib (`std::str`, `std::bytes`, `std::json`,
`std::math`, `std::text`, …) works unchanged. The **POSIX-backed
namespaces are rejected at typecheck** under this target — the browser
sandbox has no syscalls — with the diagnostic ``error: `std::...` is
unavailable under `target wasm`: <reason>``. The gated set
(`wasm_unavailable_stdlib`) is exactly:

| Rejected path | Browser substitute |
|---|---|
| `std::io::tcp` | a WebSocket bus adapter (`ws://`) |
| `std::io::udp` | (no raw UDP in the browser) |
| `std::io::tls` | the browser does TLS transparently for `wss://` / `https://` |
| `std::io::fs`, `std::io::file` | `fetch` via an `@ffi("js")` host import, or a bus message |
| `std::io::stdin`, `std::io::stdout` | `println(...)` (the loader routes it to the host console) |
| `std::term` | (no terminal in the browser) |
| `std::process` | (no OS process control) |
| `std::http` | (server is built on raw TCP) |

The **in-process typed bus is fully available** under `target wasm`:
`topic` declarations and `bus { publish … }` / `bus { subscribe … }`
across loci lower the same way they do natively — a `Subject <-
payload` is delivered to every matching in-module subscriber's handler,
payload-copied through the synthesized `__serialize_T` / `__deserialize_T`
wire codec. Those codecs follow the `lotus_serialize_fn` /
`lotus_deserialize_fn` ABI (`ssize_t(const void *, …, size_t)`), whose
`ssize_t` / `size_t` widths are **target-pointer-width** — i32 on wasm32,
i64 on the native 64-bit targets — so the runtime's `lotus_bus_dispatch`
indirect call matches the codec on both. Only the *cross-process /
network* transports (`shm_ring`, `unix`, and CONNECT-role bindings) are
unavailable in the sandbox, since they need syscalls.

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
(it is host-driven via its methods, not a cooperative run loop);
`fallible` methods stay internal (not exported).

### Entry-inversion run-model

A program built with `@export` runs **inverted**: instead of a
blocking `main`, the host drives the exports. The compiler synthesizes
and exports **`_hale_start()`**, which creates a **persistent** program
arena (and bus queue) that is *not* torn down — and, for an `@export
locus`, instantiates the singleton there and stashes its pointer. The
generated loader calls `_hale_start` once at instantiation and then the
host calls the exports (e.g. one per `requestAnimationFrame`). A
program with no `fn main` is valid when it has any `@export`; if
`_hale_start` is present the loader does **not** call `main` (its
create-then-destroy of the arena would clobber the persistent one).

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
