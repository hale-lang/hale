# `std::process`

Process-level introspection and control. Phase 1 shipped `pid`
as the proof symbol for the magic-`std::*`-path resolver; m79
added `exit` to the same module. The rest of the surface fills
in across later milestones.

## Functions

### `std::process::pid`

#### Synopsis

```aperio
fn pid() -> Int
```

Returns the process identifier of the running Aperio program.

#### Grammar

A namespaced call expression with no arguments:

```ebnf
pid_call ::= "std" "::" "process" "::" "pid" "(" ")"
```

#### Semantics

- Lowers to a libc `getpid()` call. POSIX defines `getpid` as
  always succeeding for the calling process — there is no error
  path.
- The return is `pid_t` (i32 on Linux); Aperio sign-extends to
  `Int` (i64). All real OS pids are positive and well under the
  i32 ceiling, so the extension never observably truncates.
- The pid is stable for the lifetime of the process. Two calls
  from the same program return the same value.
- The pid is **not** unique across program runs; the OS may reuse
  pids freely after a process exits. Programs that need durable
  identity must persist their own.

#### Examples

```aperio
fn main() {
    println("pid=", std::process::pid());
}
```

```aperio
locus L {
    birth() {
        let p = std::process::pid();
        if p > 0 {
            println("running as pid ", p);
        }
    }
}
fn main() { L { }; }
```

#### See Also

- [Roadmap](../roadmap.md) — Phase 1+ stdlib build-out plan.
- `spec/stdlib.md` (in the language repo) — path-resolution
  semantics, the m71 dispatcher, and design principles.

### `std::process::exit`

#### Synopsis

```aperio
fn exit(code: Int)
```

Terminates the process with the given exit code. Does not
return. Statement-position only — using it as an expression
errors at compile time.

#### Semantics

- Lowers to a libc `exit()` call with the user-supplied code
  truncated to i32 (POSIX exit codes are 8 bits anyway; the
  truncation is observationally equivalent to passing
  `code & 0xff`).
- Code after `std::process::exit(...)` lowers into a fresh
  basic block that's unreachable at runtime but well-formed
  in IR. A future control-flow milestone may diagnose it
  as dead code.
- The standard convention applies: `0` for clean exit;
  non-zero for failure. The shell sees `n & 0xff`.

#### Examples

```aperio
fn main() {
    if std::env::args_count() < 2 {
        println("usage: tool <port>");
        std::process::exit(2);
    }
    println("running");
}
```
