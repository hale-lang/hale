# CLI & config

Hale provides command-line argument and environment variable access in `std::env`, layered configuration resolution in `std::cli::Resolver`, and low-level terminal control in `std::term`. Idiomatic Hale programs resolve configuration once in `fn main()` and pass the resulting values into the application locus's `params`.

```hale
locus App {
    params {
        host: String = "127.0.0.1";
        port: Int    = 8080;
    }
    run() {
        println("listening on ", self.host, ":", self.port);
    }
}

fn main() {
    let cfg = std::cli::Resolver {
        env_prefix: "MYAPP_",
        argv_keys:  "host\nport\n",
    };
    App {
        host: cfg.get("host", "127.0.0.1"),
        port: cfg.get_int("port", 8080),
    };
}
```

---

## API Summary

| Function / Locus | Signature / Fields | Description |
|---|---|---|
| `std::env::args_count` | `fn() -> Int` | Total argument count including the program name (`arg(0)`) |
| `std::env::arg` | `fn(Int) -> String` | Positional argument at index `i` (`arg(1)` is the first user argument) |
| `std::env::arg_or` | `fn(Int, String) -> String` | Positional argument at index `i`, or `default` if out of bounds |
| `std::env::var` | `fn(String) -> String` | Reads environment variable `name` (returns `""` if unset) |
| `std::env::var_exists` | `fn(String) -> Bool` | Returns `true` if environment variable `name` is defined |
| `std::cli::Resolver` | `env_prefix`, `argv_keys`, `get`, `get_int` | Resolves settings across `argv` -> prefixed env var -> default value |
| `std::term::is_tty` | `fn(Int) -> Bool` | Returns `true` if file descriptor `fd` (`1` = stdout, `2` = stderr) is a TTY |
| `std::term::size` | `fn() -> TermSize` | Returns `TermSize { cols, rows }` (`{0, 0}` if stdout is not a TTY) |
| `std::term::RawMode` | Guard locus | Enters raw terminal mode on `birth` and restores terminal state on `dissolve` |
| `std::io::stdin::read_byte` | `fn(Int) -> Int` | Polls one byte from stdin with `timeout_ms` (`0..255`, `-1` timeout, `-2` EOF) |
| `std::io::stdout::write_bytes` | `fn(String)` | Flushes stdout and performs an unbuffered write |

---

## Command-Line Arguments & Environment (`std::env`)

Read positional command-line arguments and environment variables directly through `std::env`:

```hale
fn main() {
    let n     = std::env::args_count();       // includes program name at arg(0)
    let first = std::env::arg(1);             // positional arg 1
    let port  = std::env::arg_or(2, "8080");  // positional arg 2 with fallback

    let home  = std::env::var("HOME");        // environment variable value
    let debug = std::env::var_exists("DEBUG");
}
```

- `arg(0)` is the executable name; user-supplied arguments start at `arg(1)`.
- Prefer `arg_or(i, default)` over manual `args_count()` bounds checks when reading optional positional arguments.
- `var(name)` returns `""` when `name` is unset; use `var_exists(name)` to distinguish an unset variable from one set to `""`.

---

## Layered Configuration (`std::cli::Resolver`)

`std::cli::Resolver` layers positional command-line arguments over prefixed environment variables and built-in defaults:

```hale
fn main() {
    let cfg = std::cli::Resolver {
        env_prefix: "MYAPP_",
        argv_keys:  "host\nport\n",
    };

    // Precedence: argv[1] ("host") -> $MYAPP_HOST -> "127.0.0.1"
    let host = cfg.get("host", "127.0.0.1");
    // Precedence: argv[2] ("port") -> $MYAPP_PORT -> 8080
    let port = cfg.get_int("port", 8080);

    println("listening on ", host, ":", port);
}
```

### Resolution Rules

1. **Command-line argument (`argv_keys`)**: Each newline-terminated key in `argv_keys` maps to a positional argument starting at `arg(1)` (line 1 -> `arg(1)`, line 2 -> `arg(2)`).
2. **Environment variable (`env_prefix`)**: If the positional argument was not provided, the resolver checks `<env_prefix><UPPERCASE_KEY>` (for example, `MYAPP_HOST`).
3. **Default value**: If neither argument nor environment variable is present, `get` / `get_int` returns the default argument.

- **Empty values**: A layer counts as set if the argument is present or the environment variable is defined, even if its value is `""`. For `get`, an explicitly empty variable returns `""`. For `get_int`, an empty or unparseable value falls back to the supplied `Int` default instead of failing.
- **Boundary ownership**: Resolve all configuration in `fn main()` and pass the typed values into your [application locus](./locus-gently.md) `params`. Avoid calling `std::env::var` deep inside domain loci so every configuration value has a single owner.

---

## Interactive Terminal I/O (`std::term`)

For CLI tools that format terminal output or read raw keystrokes, `std::term` and `std::io` provide OS terminal primitives without requiring C FFI bindings.

### TTY Detection and Dimensions

Use `std::term::is_tty(fd)` to check whether an output stream (`1` for stdout, `2` for stderr) is attached to an interactive terminal before emitting ANSI escape codes:

```hale,fragment
let color = std::term::is_tty(2);         // fd 2 = stderr
let dim   = std::term::size();            // TermSize { cols, rows } ({0, 0} if not a TTY)
```

### Raw Terminal Mode & Unbuffered Keystroke Polling

`std::term::RawMode` is a scoped guard locus: its `birth` disables line buffering and input echo, and its `dissolve` restores the original terminal attributes on scope exit (backed by an `atexit` handler that also restores the terminal on panic or unhandled failure):

```hale
fn main() {
    let raw = std::term::RawMode { };       // birth: enter raw mode
    while true {
        let b = std::io::stdin::read_byte(100);   // poll stdin for up to 100ms
        if b == -1 { continue; }                  // -1: timeout (redraw, tick, etc.)
        if b == -2 { break; }                     // -2: EOF
        std::io::stdout::write_bytes("got a key\r\n");
    }
}                                           // dissolve: restore terminal settings
```

- `std::io::stdin::read_byte(timeout_ms)` reads a single byte (`0..255`), returning `-1` if `timeout_ms` elapses without input or `-2` on `EOF`.
- `std::io::stdout::write_bytes(s)` flushes stdio (`fflush`) before writing raw bytes so unbuffered terminal draws stay ordered with `println` calls.

Next: [Logging](./logging.md).
