# Your First Program

A complete Hale command-line program can be written with top-level functions and `fn main()`, or structured around an **App locus** that encapsulates configuration parameters and lifecycle execution.

```hale
fn c_to_f(c: Float) -> Float {
    return c * 9.0 / 5.0 + 32.0;
}

fn main() {
    // arg(0) is the binary name; arg(1) is the first CLI argument.
    let raw = std::env::arg_or(1, "20");

    let celsius = std::str::parse_float(raw) or {
        eprintln("not a number: ", raw);
        return;
    };

    let f = c_to_f(celsius);
    println(raw, "C = ", to_string(f), "F");
}
```

## Running and Building

Use `hale check` to typecheck the file, `hale run` to compile and execute it in one step, or `hale build` to produce a standalone native binary:

```sh
hale check temp.hl
hale run temp.hl 100      # prints: 100C = 212F
hale run temp.hl          # defaults to 20: 20C = 68F
hale build temp.hl        # produces native executable
```

## Anatomy of the Program

| Call / Construct | Role |
|---|---|
| `std::env::arg_or(1, "20")` | Reads CLI argument at index `1`, or returns `"20"` if omitted (`std::env::args_count()` and `std::env::arg(i)` provide indexed access). |
| `std::str::parse_float(raw) or { ... }` | Parses `raw` as a `Float`. On invalid input, the `or` block logs to `stderr` via `eprintln` and returns early from `main`. |
| `fn c_to_f(c: Float) -> Float` | Stateless calculation implemented as a top-level free function. |
| `println(raw, "C = ", to_string(f), "F")` | Concatenates and prints its arguments to `stdout`. |

## Promoting `main` to an App Locus

While small scripts only need free functions and `fn main()`, larger Hale programs encapsulate their state and entry point in an **App locus**:

```hale
fn c_to_f(c: Float) -> Float {
    return c * 9.0 / 5.0 + 32.0;
}

locus TempApp {
    params {
        raw: String = "20";
    }

    run() {
        let celsius = std::str::parse_float(self.raw) or {
            eprintln("not a number: ", self.raw);
            return;
        };
        println(self.raw, "C = ", to_string(c_to_f(celsius)), "F");
    }
}

fn main() {
    TempApp { raw: std::env::arg_or(1, "20") };
}
```

- **`params { ... }`** declares the locus's typed configuration and state fields, read via `self.raw`.
- **`run() { ... }`** (declared without `fn`) is a lifecycle hook driven by the runtime after the locus is constructed.
- **Statement-position instantiation (`TempApp { ... };`)** constructs the locus, runs its lifecycle (`birth` -> `run` -> `dissolve`), and reclaims its memory at the semicolon.

Next: [The Locus, Gently](../everyday/locus-gently.md).
