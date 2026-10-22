# Binding C Libraries (`@ffi("c")`)

Hale binds external C libraries directly through `@ffi("c")` function declarations, a small C glue file when needed, and linker flags in `hale.toml` or on the CLI. No compiler modifications or external code generators are required to call native libraries.

```hale,fragment
@ffi("c") fn doubler_double(x: Int) -> Int;

fn main() {
    println(doubler_double(21)); // 42
}
```

## Declaring Foreign Functions

Annotate a bodiless top-level function with `@ffi("c")` to emit an unmangled LLVM `declare` symbol with the exact function identifier:

```hale,fragment
@ffi("c") fn sqlite3_libversion_number() -> Int;
@ffi("c") fn raylib_init_window(w: Int, h: Int, title: String);
```

By convention, prefix FFI function names with the target library's name (`raylib_`, `sqlite3_`) to avoid collisions in the flat C symbol namespace.

### Signature Restrictions

An `@ffi("c")` declaration must adhere to the C ABI boundary rules:
- **No `fallible(E)` channel**: C functions cannot raise directly on Hale's error channel. Return an integer or sentinel value from the C shim and wrap it in a regular Hale function that returns `fallible(E)` (see [Fallible Functions](../basics/fallible.md)).
- **No default parameter values or generics**: Every parameter must be explicitly typed and passed at each call site.

## Type Mapping Across the C Boundary

Only portable ABI types may cross an `@ffi("c")` boundary:

| Hale Type | C ABI Representation | Notes |
|---|---|---|
| `Int` | `int64_t` | Signed 64-bit integer. |
| `Float` | `double` | IEEE-754 64-bit floating point. |
| `Bool` | `int32_t` | `0` for `false`, `1` for `true`. |
| `Duration` / `Time` | `int64_t` | Signed 64-bit nanoseconds. |
| `String` | `const char *` | NUL-terminated UTF-8 byte string. |
| `Bytes` | `const void *` / `void *` | Pointer to `[int64_t len][uint8_t payload...]`. |
| User `type` | `struct Foo *` | Pointer to a layout-matching C struct (`sret` when returned by value). |
| `()` | `void` | Valid as a return type only. |

> [!WARNING]
> `Decimal`, `Uint`, and fixed-size arrays (`[T; N]`) are rejected on `@ffi("c")` signatures at compile time. Pass numeric components or `Bytes` buffers instead.

## Compiling C Glue and Linking Libraries

Write any thin C wrapper functions in a standard `.c` file:

```c
/* glue.c */
#include <stdint.h>

int64_t doubler_double(int64_t x) {
    return x * 2;
}
```

### CLI Build Flags (`--csrc` and `--link`)

For single-project builds, pass C source files with `--csrc` and system libraries (`-l<name>`) with `--link`:

```sh
hale build mydir/ --csrc glue.c
hale build mydir/ --csrc raylib_glue.c --link raylib
```

### Package Configuration (`hale.toml`)

For reusable binding packages, declare the C sources and libraries in `hale.toml` so downstream consumers do not need to pass CLI flags manually:

```toml
[ffi]
csrc = ["glue.c"]
link = ["raylib"]
```

When an application `import`s your binding seed, `hale build`, `hale run`, `hale test`, and `hale replay` compile `csrc` and pass `link` flags automatically:
- **Deduplication**: If a user also passes `--csrc glue.c` on the CLI for a file already listed in `hale.toml`, the compiler deduplicates the path and compiles the file once.
- **Direct-import scope**: Only `[ffi]` sections from packages directly imported by the entry seed are collected automatically. If your entry imports library `A`, which transitively imports C binding `B`, repeat `B`'s `[ffi]` entries in the entry's `hale.toml` (or pass `--csrc` / `--link` on the CLI).

## Memory Ownership & Arena Conventions

Memory crossing the C boundary follows strict caller-ownership rules:

1. **Borrowed input pointers**: Every pointer passed from Hale to C (`String`, `Bytes`, or user `type` struct pointer) is owned by the caller's arena or scratch region and is read-only. The C function **must not** retain the pointer after returning; if the C library needs to keep the data, it must `malloc` and copy the bytes during the call.
2. **Returning heap data to Hale**: When a C function needs to allocate a `String`, `Bytes`, or struct to return to Hale, it must allocate directly from the caller's active arena:
   ```c
   void *ptr = lotus_arena_alloc(lotus_caller_arena_or_global(), size, align);
   ```
   Values allocated in the caller's arena obey Hale's normal scratch and locus lifetime rules without manual `free` calls.
3. **No non-local jumps**: C++ exceptions and `setjmp` / `longjmp` must never unwind across an `@ffi("c")` call boundary.

For the low-level ABI specification—including the `sret` struct-return convention and exact `BytesView` header layout—see `spec/ffi.md`. For packaging conventions when publishing C bindings to [Pond](../libraries.md), see `agents/binding-packages.md`.

> [!NOTE]
> When targeting WebAssembly (`--target wasm32`), use `@ffi("js")` to import functions from the JavaScript host and `@export` to expose Hale loci or functions to the browser. See [WebAssembly & the Browser](./webassembly.md).
