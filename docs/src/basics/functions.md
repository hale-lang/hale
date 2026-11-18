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

The final expression is checked exactly as `return` would check it: against the return type (the success type of a `fallible(E)` function or method), with the same conversions — a set member becomes its set, an `Int` widens to a `Float`.

In a function with no return type, including a `() fallible(E)` one, `return` carries no value, not even a call that returns nothing: write `touch() or fail err;` and then `return;`, not `return touch() or fail err;`.

The last statement of a body that returns a value is its value too when it is a `match`, an `if … else` or a block: each arm's value is returned, whatever the type (an `Int`, a `String`, a record, an enum, a `Decimal`):

```hale
fn word(n: Int) -> String {
    match n { 1 -> "one", 2 -> "two", _ -> "many" }
}

fn sign(n: Int) -> Int {
    if n > 0 { return 1; } else if n < 0 { -1 } else { 0 }
}
```

- **Every path returns:** a body that returns a value must return one on every path that reaches its end. `hale check` refuses one that can fall off the end, at the function's name, and points at the path: an `if` with no `else`, a `while` loop that can finish, the `break` that leaves a `while true`, an arm whose last statement is not a value. A `while true` loop with no `break` never reaches the end, and neither does a path that ends in `return` or `fail`:

```text
main.hl:1:4: type error: fn `sign` returns `Int`, and its body can reach its end
without a value: this `if` has no `else`, so when its condition is false the body
ends after it. Return a value on that path, or end the body with its value (…)
    note: the path that reaches the end without a value at main.hl:2:5
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

## Generic Functions and the `Numeric` Bound

A function can take type parameters, and `<T: Numeric>` admits `Int`, `Float`, `Decimal` and `Duration`:

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

`Numeric` is the only type parameter bound in Hale; there is no user-defined trait bound syntax (declarations like `<T: Eq>` are rejected). A call takes each type argument from the argument in the position that names it, so `twice(2)` is the `Int` version and `twice(1.5)` the `Float` one. Generic records and loci, and the rules a generic call follows, are on the [Generics](../everyday/generics.md) page.

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

Any expression that evaluates to a fn value can be called directly: a call that returns one (`pick()(2)`), an array element (`handlers[i](req)`), a tuple part (`pair.0(x)`), or an `if` that picks one (`(if fast { dbl } else { tri })(5)`). A fn value can return any type, a `StringView` included, and so can an interface method.

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

A call through a function value is checked against its type as a direct call is checked against the function: each argument against its parameter (an `Int` argument widens to a `Float` parameter), and the argument count. A function type has no defaults, so a call through `f: fn(Int, Int) -> Int` passes both arguments.

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
