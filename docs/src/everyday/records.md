# Records & data

> **Coming from Python / Node?** Where you'd reach for a dict or
> an object literal to pass structured data around, Hale uses a
> named `type` — a fixed-shape record with typed fields. It's
> closer to a TypeScript `interface` / a Python `@dataclass` than
> to a free-form dict: the shape is declared, and the compiler
> checks it.

## Records — `type`

```hale
type Player {
    id:    String;
    name:  String;
    score: Int;
}
```

Construct with a struct literal, naming each field:

```hale
let p = Player { id: "p1", name: "Ada", score: 0 };
println(p.name);                  // field access with .
```

Records are pure data: you pass them by value, read their
fields, and compare them. They carry no behavior and no
lifecycle. Fields can have defaults, so callers can omit them:

```hale
type Config { host: String = "127.0.0.1"; port: Int = 8080; }

let c = Config { port: 9000 };    // host defaults
```

Records nest, and they're what travels on the bus and in and out
of functions. When a record starts wanting *methods*, that's the
signal to promote it to a [locus](./locus-gently.md).

## Arrays

A fixed sequence of one type is an array. `[T]` is a slice (a
view of some elements); `[T; N]` is a fixed-length array:

```hale
type Match { players: [Player]; }     // a slice of Players

let xs = [1, 2, 3];                    // an array literal
let zeros = [0; 8];                    // eight zeros
```

For a sequence that *grows*, you want a `@form(vec)` list from
the [previous chapter](./collections.md), not a bare array.

## Tuples

A quick, unnamed grouping of a few values:

```hale
let pair = (1, "one");
```

Reach for a `type` once the grouping has meaning worth naming;
tuples are for the throwaway case.

## Enums — one of several shapes

An enum is a value that is exactly one of a set of named
variants — a tagged union / sum type:

```hale
type Light = enum { Red, Yellow, Green };

fn next(l: Light) -> Light {
    return match l {
        Light::Red    -> Light::Green,
        Light::Green  -> Light::Yellow,
        Light::Yellow -> Light::Red,
    };
}
```

Construct a variant with `EnumName::Variant`, and use `match` to
branch on it — exhaustively, so you can't forget a case.

Variants can carry data:

```hale
type Event = enum {
    Tick(Int),
    Trade(Decimal, Int),
    Halt,
};

fn handle(e: Event) {
    match e {
        Event::Tick(0)            -> println("tick zero"),
        Event::Tick(n)            -> println("tick #", n),
        Event::Trade(price, size) -> println("trade ", size, " @ ", price),
        Event::Halt               -> println("halt"),
    }
}
```

The match arms *bind* the payload — `Tick(n)` pulls the integer
out as `n`. You can also match a literal sub-pattern (`Tick(0)`)
ahead of the general one. This is the idiomatic way to model
"the message is one of these kinds, each with its own data" —
and it pairs naturally with the typed bus at the next level.

> Enums fill the role of `Option<T>` / `Result<T, E>` from other
> languages when you want a closed set of outcomes as data. For
> the "this call failed" case specifically, prefer the
> [`fallible`](../basics/fallible.md) channel — it's the
> purpose-built tool and the compiler enforces handling.

Next: reading and writing the world — [Files](./files.md).
