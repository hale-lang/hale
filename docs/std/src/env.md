# `std::env`

Process environment + argv access. m77 ships four functions:
two for command-line arguments (`args_count`, `arg`) and two
for environment variables (`var`, `var_exists`). All four
follow the path-call shape — no locus wrapping; the values
they expose are process-level singletons captured once at
startup.

## How argv reaches Aperio

Codegen lifted main's signature from `i32 @main()` to
`i32 @main(i32 argc, ptr argv)` and emits a call to
`lotus_env_init` in main's prelude that stashes both into
process-static globals. `std::env::args_count` and
`std::env::arg(i)` read those globals back. The capture
happens once, before any user code runs in `main()`, so
even loci instantiated at the top of `main()` can reach
argv from inside `birth()`.

## Functions

### `std::env::args_count`

#### Synopsis

```aperio
fn args_count() -> Int
```

Returns the count of command-line arguments, including the
binary path at index 0. So a program invoked as
`./demo a b c` reports `args_count() == 4`.

#### Examples

```aperio
fn main() {
    let n = std::env::args_count();
    if n < 2 {
        println("usage: demo <arg>");
    } else {
        println("processing ", n - 1, " arg(s)");
    }
}
```

### `std::env::arg`

#### Synopsis

```aperio
fn arg(i: Int) -> String
```

Returns argv[i] as a String. Out-of-range indices (negative
or `>= args_count()`) return the empty String — the C
runtime hands back a stable empty-string sentinel rather
than dereferencing past argv.

#### Examples

```aperio
fn main() {
    let n = std::env::args_count();
    let mut i: Int = 1;
    while i < n {
        println("arg[", i, "]=", std::env::arg(i));
        i = i + 1;
    }
}
```

### `std::env::var`

#### Synopsis

```aperio
fn var(name: String) -> String
```

Returns the value of environment variable `name`, or the
empty String if unset. Use `var_exists` to disambiguate
"unset" from "set to empty string."

#### Examples

```aperio
fn main() {
    let home = std::env::var("HOME");
    println("home=", home);
}
```

### `std::env::var_exists`

#### Synopsis

```aperio
fn var_exists(name: String) -> Bool
```

Returns `true` if `name` is set in the process environment,
`false` otherwise.

#### Examples

```aperio
fn main() {
    if std::env::var_exists("APERIO_DEBUG") {
        println("debug mode");
    }
}
```

## Limitations

- **No string-to-int parsing**: `arg(i)` returns a String;
  numeric arguments need a `std::str::parse_int` (or similar)
  primitive that hasn't shipped yet. A future follow-up.
- **No env iteration**: there's no `std::env::vars()` returning
  every key/value pair. The variable-length-output story
  (mirrors the deferred `read_dir` question) waits on its
  own design pass.
- **No `setenv` / `unsetenv`**: env access is read-only at v0.
  Programs that need to mutate their environment for child
  processes (when `std::process::spawn` lands) will need
  explicit modification primitives at that point.
- **argv is read-once**: the captured pointers stay valid
  for the program's lifetime, but later modification by
  `setenv`-style calls (which we don't have yet) would
  invalidate them. Don't store `arg(i)` results past
  hypothetical future env-mutation calls.

## See Also

- [Roadmap](./roadmap.md) — Phase 1+ stdlib build-out plan.
- [`std::process`](./process/index.md) — sibling
  process-introspection module.
