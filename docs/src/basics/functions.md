# Functions

Functions in Hale are declared with `fn`, explicitly typed parameters, and an optional return type `-> T`. Every top-level function in a directory (seed) is visible to all other `.hl` files in that directory without `import` statements or `pub` visibility modifiers.

```hale
fn add(a: Int, b: Int) -> Int {
    return a + b;
}

fn greet(name: String, punctuation: String = "!") {
    println("hello, ", name, punctuation);
}

fn main() {
    let sum = add(2, 3);
    greet("world");
    greet("world", "?");
    println("sum = ", sum);
}
```

## Function Declarations and Return Values

A function signature specifies the parameter names and types, followed by `-> ReturnType`. If `-> ReturnType` is omitted, the function returns the unit type `()`.

- **Explicit parameter types:** Parameter types are always required; Hale does not infer parameter types across function boundaries.
- **Trailing commas:** Both parameter lists and call-site argument lists allow a trailing comma (`add(2, 3,)`).
- **Returning values:** Use `return expr;` to return a value (including early returns), or omit the semicolon on the final expression in the function body:

```hale
fn double(n: Int) -> Int {
    n * 2          // final expression without `;` is returned
}
```

## Default Parameter Values

Parameters may declare default values using `= expr`, allowing callers to omit trailing arguments:

```hale
fn pow(base: Int, exp: Int = 2) -> Int {
    let mut acc = 1;
    for _ in 0..exp { acc = acc * base; }
    return acc;
}

fn main() {
    println(pow(3));      // 9  (exp defaults to 2)
    println(pow(2, 5));   // 32 (exp overridden)
}
```

### Rules for Default Parameters

| Rule | Behavior |
|---|---|
| **Trailing suffix only** | All defaulted parameters must appear after all required parameters. |
| **Call-site evaluation** | Default expressions are evaluated at each call site that omits the argument, not once at function definition. |
| **Module-level scope only** | Defaults may reference literals, top-level `const`s, free functions, types, enum variants, and builtins. They cannot reference caller locals, function pointers, or earlier parameters. |
| **No constant shadowing at call site** | If a default reads `const K`, a caller that defines a local `let K` cannot omit that parameter; rename the local or pass the argument explicitly. |
| **Locus construction & bus sends** | A default that constructs a locus builds it in the caller's pool context (which can trigger cross-pool spawn checks). Publishing (`<-`) is forbidden inside a default expression; call a helper method instead. |
| **Locus method support** | User-defined `fn` methods on a locus support default parameters. Bus-subscribed handlers and mode bodies (`bulk`, `harmonic`, `resolution`) do not allow defaults. |

## Generics and the `Numeric` Bound

Generic functions and types can constrain a type parameter with `: Numeric`, which accepts `Int`, `Float`, `Decimal`, and `Duration`:

```hale
fn twice<T: Numeric>(x: T) -> T {
    return x + x;
}

fn main() {
    println(twice(2));        // 4
    println(twice(1.5));      // 3
    println(twice(50ms));     // 100000000ns
}
```

`Numeric` is the only type parameter bound in Hale; there is no user-defined trait bound syntax (declarations like `<T: Eq>` are rejected).

When a generic locus (such as `Holder<T>`) calls a generic function with `self.value` (or a local bound from `self.value`), the compiler monomorphizes the call for each concrete instantiation (`Holder<Int>`, `Holder<String>`). Default expressions in `params` use the declaring locus's type arguments.

## Function Pointers (`fn(T) -> U`)

Functions are first-class values with type `fn(ParamTypes) -> ReturnType`. You can pass named functions as arguments, store them in variables, tuples, or arrays, and return them from functions:

```hale
fn apply_twice(f: fn(Int) -> Int, x: Int) -> Int {
    return f(f(x));
}

fn inc(n: Int) -> Int { return n + 1; }

fn main() {
    println(apply_twice(inc, 10));    // 12
}
```

### Function Value Capabilities and Restrictions

Hale function values are thin function pointers with no environment capture. There is no inline closure syntax (`|x| x + captured`).

| Callable Kind | Valid as `fn(...)` Value? | Details & Workaround |
|---|---|---|
| Top-level free `fn` | Yes | Passed directly by name (`inc`). |
| Stdlib function written in Hale | Yes | Passed by path (`std::url::encode`). |
| Compiler intrinsic / inline stdlib (`std::math::abs`, `len`) | No | No address to take; wrap in a one-line helper `fn my_abs(x: Int) -> Int { return abs(x); }`. |
| Capturing closure | No | Pass context explicitly as arguments or store state in a `locus`. |
| Locus method (`self.inc`) | No | Requires a `self` receiver; call the method directly or pass a free function. |
| Fallible function (`-> T fallible(E)`) | No | Indirect calls cannot attach an `or` clause; wrap in an infallible helper (`fn parse_or(s: String) -> Int { return parse(s) or 0; }`). |
| Function returning a fresh locus (`fn make() -> Worker { return Worker { }; }`) | Yes | Caller owns the newly constructed locus. If the return value is discarded (`f();`), the locus dissolves immediately at `;`. |
| Function returning a passed-in locus (`fn pick(w: Worker) -> Worker { return w; }`) | No | Cannot alias or forward an existing locus through a function pointer; call directly instead. |

### Identifier Shadowing in Calls

A parameter or local variable whose type is a function pointer (`f: fn(Int) -> Int`) shadows any top-level function or builtin with the same name (`len`, `max`, etc.) inside that scope. By contrast, a non-function local variable (`let max = 10;`) does not shadow the builtin `max(1, 2)` when called as a function.

## Seed Scope and Builtin Function Names

All files in a directory belong to the same seed and share a flat, order-independent top-level scope:

```hale
fn celsius_to_f(c: Float) -> Float { return c * 9.0 / 5.0 + 32.0; }
fn f_to_celsius(f: Float) -> Float { return (f - 32.0) * 5.0 / 9.0; }
```

### Reserved Builtin Call Forms

The compiler resolves bare calls to its built-in functions before checking user declarations. Because a user-defined free function with the same name would never be called, declaring a free `fn` with any of the following names is a compile error:

- **I/O and formatting:** `print`, `println`, `eprint`, `eprintln`, `to_string`
- **Numeric casts and math:** `Int`, `Float`, `abs`, `min`, `max`
- **String and inspection:** `len`, `starts_with`, `contains`
- **Verification and closures:** `sum`, `prod`, `check_closures`

> [!NOTE]
> These names are restricted **only** for top-level free functions. You can freely declare locus methods with these names (such as `fn len()` or `fn sum()` on a locus) because method calls are dispatched through a receiver (`b.len()`).
>
> The `bounded[T; N]` intrinsics (`count`, `clear`, `truncate`, `push`, `at`, `set`) are dispatched only when the first argument is a `bounded[T; N]`. You may declare your own free functions with those names, and a signature matching a specific `bounded[T; N]` (such as `fn count(xs: bounded[Int; 8]) -> Int`) overrides the intrinsic for that exact type and capacity.

Next: [Control Flow](./control-flow.md).
