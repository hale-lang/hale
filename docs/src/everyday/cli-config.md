# CLI & config

> **Coming from Python / Node?** No `argparse`, no `yargs`, no
> `dotenv`. Reading arguments and environment is a few direct
> calls under `std::env`; layering argv over env over defaults is
> a small `std::cli::Resolver`. Rich flag parsing (`--name=value`,
> subcommands) is library territory, not built into the language.

## Arguments and environment

```hale
fn main() {
    let n = std::env::args_count();        // includes the program name
    let first = std::env::arg(1);          // positional arg 1
    let port  = std::env::arg_or(2, "8080");  // with a default

    let home  = std::env::var("HOME");     // environment variable
    let debug = std::env::var_exists("DEBUG");
}
```

- `arg(0)` is the program name; user arguments start at `arg(1)`.
- `arg_or(i, default)` is the everyday form — no bounds-checking
  dance.
- `var(name)` reads an environment variable; `var_exists(name)`
  tests for one.

## Layered configuration

A common need: a setting should come from a command-line argument
if given, else an environment variable, else a built-in default.
`std::cli::Resolver` expresses that precedence directly:

```hale
fn main() {
    let cfg = std::cli::Resolver { prefix: "MYAPP" };

    // argv positional "port", else $MYAPP_PORT, else "8080"
    let port = cfg.get("port", "8080");
    let host = cfg.get("host", "127.0.0.1");

    println("listening on ", host, ":", port);
}
```

The resolver checks the argument, then the prefixed environment
variable (`MYAPP_PORT`), then the supplied default. Empty values
fall through to the next layer rather than counting as "set."

## Interactive terminal I/O

For a tool that draws to the terminal or reads keystrokes, a few
`std::` primitives cover the OS surface without an FFI dependency.

`std::term::is_tty(fd)` answers *"is this a terminal?"* — the usual
guard for whether to emit color:

```hale,fragment
let color = std::term::is_tty(2);   // fd 2 = stderr
```

`std::term::size()` returns a `TermSize { cols, rows }` record (and
`{0, 0}` when stdout isn't a tty). `std::term::RawMode` is a guard
locus that puts the terminal in raw mode for its lifetime — no line
buffering, no echo — and restores it on scope exit, and on a panic
or unhandled error too via an atexit backstop:

```hale
fn main() {
    let raw = std::term::RawMode { };       // birth: enter raw mode
    // ... read keys, draw frames ...
}                                           // dissolve: restore the terminal
```

For the bytes themselves, `std::io::stdin::read_byte(timeout_ms)`
polls one byte (`0..255`, `-1` on timeout, `-2` on EOF), and
`std::io::stdout::write_bytes(s)` does a raw, unbuffered write — it
`fflush`es first so it stays ordered with any `println` output:

```hale,fragment
while true {
    let b = std::io::stdin::read_byte(100);   // 100ms poll
    if b == -1 { continue; }                    // timeout: redraw, tick, …
    if b == -2 { break; }                       // EOF
    std::io::stdout::write_bytes("got a key\r\n");
}
```

These are primitives, not a TUI — key decoding and styling live in
a library on top of them.

## Where this fits

This is the boundary between the outside world and your program.
The idiomatic shape, building on the [app
locus](./locus-gently.md): `main` resolves configuration, then
constructs the app locus with it.

```hale
locus App {
    params { host: String = "127.0.0.1"; port: String = "8080"; }
    run() { println("listening on ", self.host, ":", self.port); }
}

fn main() {
    let cfg = std::cli::Resolver { prefix: "MYAPP" };
    App {
        host: cfg.get("host", "127.0.0.1"),
        port: cfg.get("port", "8080"),
    };
}
```

Configuration enters once, at the edge, and flows inward as
typed locus state — never read again from a global deep inside
the program. That keeps every setting owned by exactly one locus,
the rule from [The locus, gently](./locus-gently.md).

Next: seeing what your program is doing — [Logging](./logging.md).
