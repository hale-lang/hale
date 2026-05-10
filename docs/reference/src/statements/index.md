# Statements

## Synopsis

Statements have effect; they do not (in general) produce
values. Aperio's statement surface is conventional plus two
substrate-specific forms (`yield`, `<-`) and the lifecycle
methods declared inside locus bodies.

## Statement forms

| Form | Description |
|---|---|
| `let pattern = expr;` | Binding (immutable) |
| `let mut name = expr;` | Mutable binding |
| `name = expr;` | Assignment to mutable binding or `self.field` |
| `name += expr;` (and other compound forms) | Compound assignment |
| `expr;` | Expression statement (effect, value discarded) |
| `if cond { ... } else { ... }` | Conditional |
| `while cond { ... }` | Loop |
| `for name in iter { ... }` | Iteration over arrays, ranges, `self.children` |
| `break;` | Exit innermost loop |
| `continue;` | Skip to next iteration |
| `return;` / `return expr;` | Function/method return |
| `yield;` | Cooperative cell boundary (see [scheduling](../runtime.md)) |
| `"subject" <- expr;` | Bus send |
| `Type { ... };` | Locus / type construction at statement position |

## `let` bindings

```aperio
let x = 0;          // immutable binding
let mut y = 0;      // mutable binding
let (a, b) = pair;  // tuple destructure
let z: Decimal = 1.5d;  // explicit type annotation
```

Per **F.E**, bindings are immutable by default. Reassigning an
immutable binding is a compile-time error.

## Locus instantiation

Constructing a locus type at statement position attaches it as
a child of the enclosing locus (or as an anonymous root child
of `main`'s implicit locus):

```aperio
fn main() {
    HelloL { greeting: "hi" };   // attaches to main's implicit locus
}

locus ParentL {
    run() {
        ChildL { };               // attaches to ParentL
    }
}
```

See [locus declarations](../loci/index.md) for the full
attachment rules.

## `for` and ranges

```aperio
for i in 0..10 { /* ... */ }       // exclusive
for i in 0..=10 { /* ... */ }      // inclusive

let xs = [1, 2, 3, 4];
for x in xs { /* ... */ }

for child in self.children { /* ... */ }  // population iteration
```

## Compound assignment

```aperio
let mut n = 0;
n = 1;       // simple
n += 5;      // compound
n -= 2;
n *= 3;
n /= 2;
n %= 10;
n &= 0xFF;
n |= 0x01;
n ^= 0xAA;
```

## Bus send (`<-`)

Statement-only. The left side is a string-literal subject the
locus has declared `publish` for; the right side is any
expression of the declared type:

```aperio
"trellis.observation" <- Observation {
    value_low: 100.0d,
    value_high: 100.05d,
    timestamp: time::monotonic(),
};
```

`<-` does not nest in expressions and produces no value.

## `yield`

Statement-only. Inserts an explicit cooperative cell boundary
in a long inner loop. See [runtime](../runtime.md).

## Match as statement

`match` may appear as a statement when its arms produce no
value:

```aperio
match e {
    Event::Tick(n)   -> println("tick #", n),
    Event::Halt      -> { /* multi-statement arm */ },
    _                -> { },
}
```

## See Also

- [Expressions](../expressions/index.md)
- [Locus declarations](../loci/index.md)
- [Bus dispatch](../bus/index.md)
- [Runtime](../runtime.md)
