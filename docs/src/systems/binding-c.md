# Binding C

> **Coming from Rust / C++?** This is `extern "C"` with a thin,
> hand-written wrapper — no `bindgen`, no build-script codegen.
> You declare the C symbols you need with `@ffi("c")`, ship a
> small glue `.c` file, and name the link flags in `hale.toml`.
> The compiler emits LLVM `declare`s and the linker resolves them.
> No compiler change is needed to bind a new library.

## Declaring a foreign function

An `@ffi("c")` annotation on a bodiless top-level function
declares an external C symbol:

```hale
@ffi("c") fn doubler_double(x: Int) -> Int;

fn main() {
    println(doubler_double(21));     // 42
}
```

The LLVM symbol name is the function name verbatim — no mangling
— so the linker matches it directly against your C. Convention:
prefix FFI names with the library identifier
(`raylib_init_window`, `sqlite3_open`) to keep the global C
namespace tidy.

## Type marshalling

Only a portable subset crosses the boundary; the mapping is
fixed:

| Hale | C |
|---|---|
| `Int` | `int64_t` |
| `Float` | `double` |
| `Bool` | `int32_t` (0 / 1) |
| `Duration` / `Time` | `int64_t` (nanoseconds) |
| `String` | `const char *` (NUL-terminated) |
| `Bytes` | pointer to `[int64 len][payload]` |
| user `type` | pointer to a layout-matching struct |
| `()` | `void` (return only) |

`Decimal` and fixed-size arrays are *not* portable across FFI —
the compiler rejects them at the boundary. Function declarations
also can't be generic or `fallible(E)`; a C function reports
errors with a sentinel, and your Hale wrapper translates that
sentinel into the [`fallible`](../basics/fallible.md) channel.

## The glue and the build

Write the C side as an ordinary translation unit:

```c
/* glue.c */
#include <stdint.h>
int64_t doubler_double(int64_t x) { return x * 2; }
```

Build, naming the C source (and any libraries to link):

```sh
hale build mydir/ --csrc glue.c
hale build mydir/ --csrc raylib_glue.c --link raylib
```

For a reusable binding library, declare the surface in
`hale.toml` so consumers don't pass flags by hand:

```toml
[ffi]
csrc = ["glue.c"]
link = ["raylib"]
```

A downstream project then just `import`s the binding and builds
normally; the FFI flags thread through automatically.

## Lifetime rules across the boundary

The boundary is read-only for arena-owned memory, and the rule is
simple: **the caller owns every pointer; the callee must not
retain it past the call.** If the C side needs to keep data, it
`malloc`s and copies. If it returns heap data back to Hale, it
allocates into the caller's arena via
`lotus_arena_alloc(lotus_caller_arena_or_global(), size, align)`
so the value lives by Hale's rules. Exceptions / `longjmp` must
not cross the boundary.

This is the whole FFI story — declare, glue, link. The full
contract (struct-return `sret` convention, the exact view layout
for `BytesView`) is in `spec/ffi.md`. Binding libraries
conventionally live in [pond](../libraries.md); the
`agents/binding-packages.md` brief covers the recommended file
layout.

> **On the wasm target,** `@ffi("c")` has a sibling: `@ffi("js")`
> declares a function the JavaScript loader provides instead of a
> linked C symbol, and `@export` sends Hale functions *out* to the
> host. Same declare-and-bind shape, different boundary — see
> [WebAssembly & the browser](./webassembly.md).

Next: running it in production — [Operations &
debugging](./operations.md).
