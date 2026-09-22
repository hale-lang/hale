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

```hale,fragment
let p = Player { id: "p1", name: "Ada", score: 0 };
println(p.name);                  // field access with .
```

Records are pure data: you pass them by value, read their
fields, and compare them. They carry no behavior and no
lifecycle. Fields can have defaults, so callers can omit them:

```hale,fragment
type Config { host: String = "127.0.0.1"; port: Int = 8080; }

let c = Config { port: 9000 };    // host defaults
```

A literal is checked against the declaration: a field name that
isn't there, or a value of the wrong type, is a compile error —
never a silently defaulted field. That holds for a record you
imported from another seed too, where you spell the type
`alias::Config { ... }`.

**A binding is a copy.** `let saved = self.row;` copies the record
into the frame — its Strings and Bytes cloned, nested records copied,
a locus handle inside it left as a handle — the way a returned record
already was. Replace the field afterwards and `saved` still reads
what it read; write through a `let mut copy` and the original is
untouched. A literal or a call result is bound as it is, since it was
already yours. Assignment is the same: `copy = saved` copies too, and
a write through one local never shows up in another.

```hale,fragment
let saved = self.row;
self.row = Row { };           // saved is unchanged
let mut copy = saved;
copy.score = 9;               // saved.score is unchanged
```

Records nest, and they're what travels on the bus and in and out
of functions. When a record starts wanting *methods*, that's the
signal to promote it to a [locus](./locus-gently.md).

A `type` is always declared at the **top level** — beside your
`fn`s and `locus`es, never inside a locus body. It's in scope
everywhere in the program, including inside every locus, so a
locus that needs a record of its own declares it just above
itself. A `type` written inside a locus body is an error at the
`type` keyword.

## Arrays

A fixed sequence of one type is an array. `[T]` is a slice (a
view of some elements); `[T; N]` is a fixed-length array:

```hale,fragment
type Match { players: [Player]; }     // a slice of Players

let xs = [1, 2, 3];                    // an array literal
let zeros = [0; 8];                    // eight zeros
```

For a sequence that *grows*, you want a `@form(vec)` list from
the [previous chapter](./collections.md), not a bare array.

## Tuples

A quick, unnamed grouping of a few values:

```hale,fragment
let pair = (1, "one");
```

Reach for a `type` once the grouping has meaning worth naming;
tuples are for the throwaway case.

## Aliases — a second name for a type

`type Name = Type;` gives an existing type another name:

```hale
type Cents = Int;

fn price_of(n: Cents) -> Cents { return n * 2; }

fn main() {
    let c: Cents = 250;
    println(price_of(c));
}
```

An alias is **transparent**: `Cents` *is* `Int`, not a new type
wrapped around one. The two are interchangeable in both
directions, with no conversion and no wrapper — so an alias buys
you a name that reads better at the call site, and nothing else.

That cuts both ways. Because the alias adds no type of its own,
it also adds no safety: nothing stops you passing a plain `Int`
where `Cents` is expected. If you want the compiler to keep two
integers apart, give each a record of its own (`type Cents { v:
Int; }`).

Transparent includes *building* the value. With `type Row2 =
Row;`, `Row2 { id: 1 }` builds a `Row` — the same record the
target's own name builds, checked against the target's fields.
The same goes for an enum: with `type Light2 = Light;`,
`Light2::Red` is `Light::Red`, in a match arm as well as in an
expression.

Three small rules:

* The alias has to end at a type you can build. `type Cents =
  Int;` names a primitive, so `Cents { }` means nothing and is
  refused — as is `[Row; 2]`, or a tuple.
* An alias chain has to end somewhere. `type A = B; type B = A;`
  is a type error.
* An alias takes no type parameters of its own. `type Twin<T> =
  Pair<T>;` is refused at the `<` with *generic type aliases are
  not supported*; name a concrete instantiation instead (`type
  IntPair = Pair<Int>;`), which is allowed and stays transparent.

A word on what may stand in those angle brackets. A generic record
is compiled once per instantiation, under a name built from the
arguments — `Pair<Int>` becomes `Pair_Int` — so each argument has
to be something that can be spelled in a name. Any record of your
own can, and so can every primitive but one:

```text
Int  Float  Bool  String  Duration  Decimal  Time
Bytes  BytesView  BytesMut  StringView
```

`Uint` is the exception, because it has no representation of its
own yet (it is recognised and lowered nowhere), so `Pair<Uint>` is
refused at the argument with the supported list named — where you
wrote it, not later from the backend. Use `Int`.

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

Printing one gives you the spelling you wrote: `println(l)` on a
`Light` prints `Light::Red`, and a variant with a payload prints
`Event::Tick(3)`. An enum that came from a library prints the same
way — under the name its *declaration* uses, without the alias you
imported it as.

> Enums fill the role of `Option<T>` / `Result<T, E>` from other
> languages when you want a closed set of outcomes as data. For
> the "this call failed" case specifically, prefer the
> [`fallible`](../basics/fallible.md) channel — it's the
> purpose-built tool and the compiler enforces handling.

## Records with a type parameter

A record can leave one of its field types open:

```hale
type Box<T> {
    value: T = 0;
}
```

`Box` on its own is a *template*, not a type — there is no `Box`
to build. `Box<Int>` is the type, and the compiler makes one real
record per set of arguments you use.

The literal still spells the template name, and takes the
arguments from whatever declares the type at that spot:

```hale
fn main() {
    let b: Box<Int> = Box { value: 1 };   // the annotation says Int
    println(b.value);
}
```

A declared return type does the same job (`fn make() -> Box<Int> {
return Box { value: 4 }; }`), as does a declared field — both at
its default and at a literal that fills it (`Outer { inner: Box {
value: 9 } }`).

What does *not* work is leaving it to the compiler to guess:

```hale,fragment
let b = Box { value: 1 };     // error: `Box` is a generic type
```

The field value being an `Int` is not enough — nothing says `T` is
`Int` rather than something an `Int` could become, so Hale asks you
to write it. Getting the count wrong is an error too: `Box<Int,
String>` reports *generic type `Box` takes 1 type argument, not 2*
at the annotation.

A **locus** can take type parameters the same way, and its
`params` are substituted just like a record's fields:

```hale
locus Cache<K, V> {
    params {
        cap: Int = 1;
    }
}

fn main() {
    let c: Cache<Int, String> = Cache { cap: 2 };
    println(c.cap);
}
```

Behind the scenes the compiler calls that instance's type
`Cache_Int_String`, and you will see the name in a diagnostic. For
a record you may write that name yourself — `Box_Int { value: 1 }`
is the same type as `Box<Int>` — but for a locus it is the
compiler's name only: build one through `Cache` with the arguments
on the binding.

Next: reading and writing the world — [Files](./files.md).
