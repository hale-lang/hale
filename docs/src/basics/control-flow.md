# Control Flow

Hale provides `if`/`else` branches, exhaustive `match` expressions, `while` loops, and `for` range/collection loops. Both `if` and `match` are expressions that can return values directly when all arms produce the same type.

```hale
fn classify(n: Int) -> String {
    return match {
        n < 0   -> "negative",
        n == 0  -> "zero",
        n < 10  -> "small",
        else    -> "large",
    };
}

fn main() {
    for i in 0..=3 {
        let parity = if i % 2 == 0 { "even" } else { "odd" };
        println(f"{i}: {parity} ({classify(i)})");
    }
}
```

## Control-Flow Summary

| Construct | Syntax | Can Produce a Value? | Key Rules |
|---|---|---|---|
| Conditional | `if cond { ... } else { ... }` | Yes (when `else` is present) | Both arms must have the same type in expression position |
| Value match | `match expr { pat -> res, _ -> def }` | Yes | Must be exhaustive; `_` is the wildcard pattern |
| Condition match | `match { c1 -> r1, else -> def }` | Yes | Evaluates boolean guards top-to-bottom; `else` is the catch-all |
| Conditional loop | `while cond { ... }` | No (`()`) | Use `while true { ... }` for unbounded loops; exit with `break;` |
| Iterator loop | `for x in 0..n { ... }` | No (`()`) | `a..b` is half-open (`[a, b)`); `a..=b` is inclusive (`[a, b]`) |
| Block expression | `{ stmt; expr }` | Yes (if final expression omits `;`) | Final expression without `;` becomes the block's value |

## `if` / `else`

`if` branches on a `Bool` condition and chains with `else if` and `else`:

```hale,fragment
if score >= 90 {
    println("A");
} else if score >= 80 {
    println("B");
} else {
    println("C");
}
```

### `if` as an Expression

When an `if` has an `else` branch and both branches evaluate to the same type, it can be used as an expression:

```hale,fragment
let grade = if score >= 90 { "A" } else { "B" };

let band = if score >= 90 {
    if score >= 97 { "A+" } else { "A" }
} else {
    "B"
};
```

> [!NOTE]
> When an `if` condition ends in a bare identifier, an **empty** `{ }` body (`if done { }`) is parsed as a struct literal `done { }`, even if the braces contain a comment. If you need an empty branch body, parenthesize the condition: `if (done) { }`.

## `while` and `for` Loops

Use `while` to repeat a block while a condition holds, or `while true` for an explicit loop terminated by `break;`:

```hale,fragment
let mut i = 0;
while i < 5 {
    println(i);
    i = i + 1;
}

let mut n = 0;
while true {
    n = n + 1;
    if n >= 3 { break; }
}
```

Inside any loop, `break;` exits the innermost loop immediately, and `continue;` skips to the next iteration.

Use `for` to iterate over integer ranges or collections (covered in [Collections](../everyday/collections.md)):

```hale,fragment
for i in 0..5 {
    println(i);            // 0, 1, 2, 3, 4 (exclusive upper bound)
}

for i in 0..=5 {
    println(i);            // 0, 1, 2, 3, 4, 5 (inclusive upper bound)
}
```

## Pattern Matching (`match`)

`match` compares a value against a list of patterns using `->` arms and runs the first matching arm:

```hale
fn describe(n: Int) -> String {
    return match n {
        0 -> "zero",
        1 -> "one",
        _ -> "many",
    };
}
```

- **Exhaustiveness:** Every `match` is checked for exhaustiveness at compile time. Matching a `Bool` requires both `true` and `false`; matching an `enum` or set type ([Records & Data](../everyday/records.md)) requires covering every variant; matching open-ended types (`Int`, `String`) requires a `_` wildcard arm. Because `match` is exhaustive, a function can end with a `match` statement whose arms all `return`.
- **Statement vs. expression:** `match` works as both a statement and an expression. Arms can be single expressions or `{ ... }` blocks (block-bodied arms still take a trailing comma).

### Condition `match` (Scrutinee-less `match`)

Omit the value after `match` to test a sequence of boolean conditions top-to-bottom, using `else` as the catch-all arm:

```hale,fragment
let size = match {
    n < 10  -> "small",
    n < 100 -> "medium",
    else    -> "large",
};
```

Scrutinee-less `match` is shorthand for `match true { _ if n < 10 -> ..., _ -> ... }` and is ideal when routing or classifying across multiple predicates that return the same type:

```hale,fragment
return match {
    std::http::is_route(ctx, "GET", "/users")     -> self.list(),
    std::http::is_route(ctx, "GET", "/users/:id") -> self.show(id),
    else                                          -> not_found(),
};
```

## Block Values and Statement Rules

A `{ ... }` block evaluates to its final expression when that expression omits the trailing semicolon `;`:

```hale,fragment
let label = {
    let base = compute();
    base + 1               // no semicolon: block evaluates to `base + 1`
};
```

A block whose final item ends with `;` has no value (`()`) and cannot be used where a value is expected.

Conversely, a standalone statement terminated by `;` must perform an action—such as a function or method call (whose return value, if any, is discarded), a locus instantiation, or an `or` clause on a fallible call. Writing a pure value expression as a statement (`total + 1;` or `p.name;`) fails compilation with `this expression's value is unused`.

Next: [Strings & Text](./strings.md).
