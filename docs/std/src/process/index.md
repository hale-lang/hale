# `std::process`

Process-level introspection and control. Phase 1 ships a single
function — `pid` — as the proof symbol that the magic-`std::*`-path
resolver works end-to-end. The rest of the module fills in across
later phases.

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
