# Control flow

> Choosing, repeating, and matching.

## `if` / `else`

```hale
if score >= 90 {
    println("A");
} else if score >= 80 {
    println("B");
} else {
    println("C");
}
```

`if` is also an *expression* — it produces a value, so you can
assign with it. In expression position it needs an `else`, and
both arms must produce the same type:

```hale
let grade = if score >= 90 { "A" } else { "B" };
```

Because an `if` is an expression, it can be an arm of another
`if` and the value flows out through both:

```hale
let band = if score >= 90 {
    if score >= 97 { "A+" } else { "A" }
} else {
    "B"
};
```

One small thing the compiler is strict about: an empty `if` body
won't parse. If you genuinely want a branch that does nothing,
put a comment in it or restructure the condition:

```hale
if done {
    // nothing to do yet
}
```

## `while` and `loop`

```hale
let mut i = 0;
while i < 5 {
    println(i);
    i = i + 1;
}
```

`loop { ... }` repeats forever until you `break`:

```hale
let mut n = 0;
loop {
    n = n + 1;
    if n >= 3 { break; }
}
```

`break` exits the nearest loop; `continue` skips to the next
iteration.

## `for`

`for` iterates over a range or a collection:

```hale
for i in 0..5 {
    println(i);            // 0 1 2 3 4
}
```

(`0..5` is exclusive of the upper bound; `0..=5` includes it.)
You'll use `for` over real collections once you meet lists and
maps in [Everyday programs](../everyday/collections.md).

## `match`

`match` compares a value against patterns and runs the first
that fits:

```hale
fn describe(n: Int) -> String {
    return match n {
        0       -> "zero",
        1       -> "one",
        _       -> "many",
    };
}
```

`_` is the wildcard — "anything else." Matches must be
*exhaustive*: the compiler rejects a `match` that doesn't cover
every possibility. For a `Bool` that means both `true` and
`false`; for open-ended types it means a `_` arm. This is a
safety feature — you can't forget a case and have it silently
fall through.

`match` shines on enums (a type that's one of several named
shapes), which you'll meet in
[Records & data](../everyday/records.md). The arms can bind the
data carried by each variant.

## Blocks have values

A `{ ... }` block's last expression — written without a trailing
`;` — is the block's value. That's why `if`/`match` can be used
as expressions, and why a function can end in a bare expression
instead of `return`. A block whose last item *does* end in `;`
has value `()`.

```hale
let label = {
    let base = compute();
    base + 1               // block evaluates to this
};
```

That's the whole control-flow surface. Next we look at working
with text: [Strings & text](./strings.md).
