# When a Call Can Fail

Hale handles recoverable, value-level errors through `fallible(E)` function signatures and mandatory `or` clauses at every call site. Unlike exceptions, errors never propagate invisibly across stack frames; unlike Rust's `Result<T, E>` or Go's `(T, error)`, a fallible return is not a wrapper value that can be stored or ignored—it must be resolved immediately where the call is made.

```hale
type ParseError { kind: String; input: String; }

fn parse_count(s: String) -> Int fallible(ParseError) {
    if !std::str::can_parse_int(s) {
        fail ParseError { kind: "not_int", input: s };
    }
    return std::str::parse_int(s) or 0;
}

fn main() {
    let valid   = parse_count("42") or 0;
    let invalid = parse_count("banana") or -1;
    println(f"valid={valid} invalid={invalid}");
}
```

## Declaring `fallible(E)` and Raising Errors

Append `fallible(E)` after a function's return type (or directly after the parameter list if the function returns `()`) to declare that it may fail with error payload `E`:

```hale,fragment
fn write_report(path: String, body: String) fallible(IoError) {
    std::io::fs::write_file(path, body) or raise;
}
```

Inside a `fallible(E)` function, `fail <expr>;` exits the function immediately along its error channel with `<expr>` of type `E`.

> [!NOTE]
> **Standard Library Error Types:** If your program declares a record with the same name as a built-in stdlib error (`ParseError`, `IoError`, `CryptoError`, `IndexError`, `KeyError`, `CapacityError`, or `ClosureViolation`), your declaration unifies with the stdlib's type across the entire program and must include all fields required by the stdlib (for example, `kind: String; input: String;` on `ParseError`). Use a distinct name for custom error shapes.

## Addressing Fallible Calls with `or`

Every call to a `fallible(E)` function must be followed immediately by an `or` clause. Calling a fallible function bare (`parse_count(s);`), passing it directly as an argument, or matching on it (`match parse_count(s) { ... }`) is a compile error in `hale check`, `hale verify`, and `hale build`.

| `or` Form | Syntax Example | Behavior |
|---|---|---|
| **Fallback value** | `parse_count(s) or 0` | Substitutes a default value of the success type (`err` is in scope inside the fallback expression). |
| **Fallback block** | `parse_count(s) or { log(err.kind); return; }` | Runs a block with `err: E` in scope. In value position, the block must evaluate to `T` or diverge (`return`, `break`, `continue`, `fail`, `violate`). |
| **Propagate (`raise`)** | `parse_count(s) or raise` | Returns `E` from the enclosing `fallible(E)` function. In an infallible function (such as `main`), halts the process with `Hale panic: unhandled E escaping main locus`. |
| **Handler function** | `parse_count(s) or handle_err(err)` | Calls a helper `fn(E) -> T` with `err` to compute a substitute value (or trigger structural shutdown). |
| **Translate (`fail`)** | `parse_count(s) or fail AppErr { msg: err.kind }` | Fails the enclosing `fallible(AppErr)` function with a translated error payload. |
| **Ignore (`discard`)** | `cache.put(k, v) or discard;` | Drops the error. Valid anywhere for `()` functions, or at statement position when discarding both value and error. |

### Examples of Each `or` Disposition

```hale
type AppErr { msg: String; }

fn default_port(e: ParseError) -> Int {
    eprintln("bad port (", e.input, "), defaulting to 8080");
    return 8080;
}

fn read_port(raw: String) -> Int fallible(AppErr) {
    return std::str::parse_int(raw) or fail AppErr { msg: err.kind };
}

fn main() {
    let p1 = std::str::parse_int("9000") or 8080;
    let p2 = std::str::parse_int("bad") or default_port(err);
    let p3 = read_port("3000") or {
        eprintln("fatal: ", err.msg);
        return;
    };
    std::io::fs::mkdir("tmp_cache") or discard;
    println(p1, " ", p2, " ", p3);
}
```

### Statement vs. Value Position Rules

- **Statement-position `or` blocks:** When a fallible call stands as a complete statement (`parse_count(s) or { log(err.kind); };`), its success value is discarded, so the `or` block does not need to produce a value of the success type.
- **No binding `()` calls:** A fallible function whose success type is `()` cannot be bound to a `let` variable (`let ok = std::io::fs::write_file(p, s) or { };` is rejected); call it as a statement instead.
- **No `or discard` in value bindings:** Writing `let n = parse_count(s) or discard;` is a compile error because `n` would have no value on failure; use `or <fallback>`, `or raise`, or call `parse_count(s) or discard;` as a statement.

## Chaining and Fallible Recovery Handlers

`or` clauses associate right-to-left, allowing you to chain fallback attempts before a final default or `raise`:

```hale,fragment
let id = parse_count(primary) or parse_count(fallback) or 0;
```

When a recovery expression in an `or` clause can itself fail:

- **User-defined fallible functions** implicitly propagate their error to the enclosing `fallible` function (`db_read(k) or self.rebuild(k)`).
- **Stdlib functions, generic functions, interface methods, and `@form` methods** used inside an `or` arm require an explicit inner `or` disposition:

```hale
fn load(primary: String, backup: String) -> String fallible(IoError) {
    return std::io::fs::read_file(primary)
        or (std::io::fs::read_file(backup) or raise);
}
```

## Fallible Methods in Interfaces

Interface method signatures may declare `fallible(E)`:

```hale
type StoreError { kind: String; }

interface Store {
    fn put(key: String) -> Int fallible(StoreError);
}

locus StrictStore {
    fn put(key: String) -> Int fallible(StoreError) {
        if key == "" { fail StoreError { kind: "empty key" }; }
        return len(key);
    }
}

fn store_or_zero(s: Store, key: String) -> Int {
    return s.put(key) or 0;
}
```

A locus whose `put` method is infallible still satisfies `Store` (since it never exercises the error path). A locus whose `put` fails with a different error type, or declares `fallible` when the interface method is infallible, fails typechecking.

## Two Failure Channels: Why Lifecycle Hooks Cannot Be `fallible(E)`

Hale separates failures into two orthogonal channels:

1. **Value channel (`fallible(E)` + `or`):** Caller-addressed operational errors on free functions and user-declared `fn` methods on a locus.
2. **Structural channel (`↑` — closure tests, `violate`, and `on_failure`):** Component-level invariant violations and supervision policies, covered in [When Things Fail](../services/failure.md).

> [!IMPORTANT]
> **`fallible(E)` is rejected on substrate-invoked surfaces:** lifecycle hooks (`birth`, `run`, `dissolve`), mode bodies (`bulk`, `harmonic`, `resolution`), closure-assertion bodies, and bus-subscribed handlers (`subscribe Topic as handler`).
>
> Because the runtime invokes these hooks directly rather than a caller frame in your code, there is no call site to attach an `or` clause. Inside a lifecycle hook or bus handler, resolve fallible calls with `or <fallback>`, `or { ... }`, `or discard`, or bridge to the structural channel using an error-check method that executes `violate`.

Next: [Your First Program](./first-program.md).
