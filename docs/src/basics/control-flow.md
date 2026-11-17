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
| Value match | `match expr { pat -> res, _ -> def }` | Yes | Must be exhaustive; `_` is the wildcard pattern; `p1 \| p2 -> res` matches either |
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

To take a text apart, loop over one of the text sources, `std::str::split(s, sep)`, `std::str::lines(s)` or `std::str::words(s)` (covered in [Strings](strings.md)). A `String` itself is not something to loop over: `for c in s` is refused, since a part could be a byte, a character, a line or a word.

```hale,fragment
for line in std::str::lines("a=1\nb=2\n") {
    println(line);         // a=1, b=2
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
- **Flat patterns:** each part of a tuple pattern, like each value of a variant's pattern, is a binding, `_` or a literal (`(0, b) -> …`, `Shape::Line(l) -> …`). A nested pattern there (`((1, 2), 3)`, `(Light::Red, 3)`) is refused by `hale check`: bind the part and match the binding inside the arm.

### Matching strings, and or-patterns

A `String` matches against string literals, and an arm may list several patterns separated by `|`; the arm runs when any one of them matches. This is the way to dispatch on a word: one decision, the subject named once, and a `_` arm that says what happens to everything else.

```hale
fn usage(verb: String) -> String {
    return match verb {
        "assign" | "unassign" -> verb + " <name> reviews <kind>:<node>",
        "close" -> "close <name>",
        _ -> "unknown verb",
    };
}
```

Alternatives work for every pattern form: literals (`1 | 2 | 3`), enum variants with or without payloads, set members, tuples. When the alternatives bind names, every alternative binds the same names at the same types, so the arm's body can use them whichever one matched:

```hale
type CtlError = enum { Invalid(String), NotFound(String), Conflict(String), NotReady(Int) };

fn problem(e: CtlError) -> String {
    return match e {
        CtlError::Invalid(p) | CtlError::NotFound(p) | CtlError::Conflict(p) -> p,
        CtlError::NotReady(h) -> f"not ready after {h} checks",
    };
}
```

- **Exhaustiveness** counts each alternative, so `Color::Red | Color::Green -> …, Color::Blue -> …` covers a three-color enum without `_`.
- **A guard** after the alternatives (`(x, _) | (_, x) if x > 0 -> x`) covers the whole arm. Alternatives are tried left to right, and when the guard is false for one alternative's bindings, the next alternative is tried.
- **Top level only:** `|` separates a whole arm's patterns. `E::A(1 | 2)` is refused; write `E::A(1) | E::A(2)`.
- An alternative an earlier arm or alternative already covers is never tried, and `hale check` warns about it.
- Inside a guard or an arm's body, `|` is still the bitwise-or operator.

A ladder of `if verb == "assign" { … } else if verb == "close" { … }` tests is the same decision spelled longer. `hale check --warn-string-match` (or `hale verify --warn-string-match`) points out three or more such tests in a row on one string and suggests the `match`.

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

A block whose last item is a `match` (no `;` after its closing brace) takes the match's value where the block is used as one — a `let`'s block, an arm of a `match` or `if` expression, the body of a function that returns a value ([Functions](./functions.md#function-declarations-and-return-values)) — so an arm can bind a local and then match on it. A trailing `match` whose every arm `return`s makes the block a diverging one: fine as a match arm, as an `if` arm beside one that yields the value, or as an `or { … }` substitute, but where the block alone must supply a value (a `let`'s block, a call argument, a condition, an operand of `&&`), `hale check` refuses it, as it refuses `let x = { return 1; };`:

```hale,fragment
let label = match kind {
    Kind::A -> {
        let m = n + 1;
        match m { 1 -> "one", _ -> "many" }
    },
    Kind::B -> "b",
};
```

Conversely, a standalone statement terminated by `;` must perform an action—such as a function or method call (whose return value, if any, is discarded), a locus instantiation, or an `or` clause on a fallible call. Writing a pure value expression as a statement (`total + 1;` or `p.name;`) fails compilation with `this expression's value is unused`.

Next: [Strings & Text](./strings.md).
