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
untouched. A literal is bound as it is, since it was already yours, and
so is a call's result — unless the function hands back a part of what
you passed it (`fn first(r: Row) -> Cell { return r.cells[0]; }`), which
is then a value read from a place like any other. Assignment is the same: `copy = saved` copies too, and
a write through one local never shows up in another.

The same holds for a `String`, `Bytes`, tuple or array you read out of
a field — `let name = self.row.name;`, a `match` payload of a field, the
array a `for` walks, a value a `match` or `if` carries out of one: it
keeps what it read, even across a method that replaces the field, and
a local rebound inside a loop keeps it into the next iteration. The compiler only pays for a copy where such a
write can come before a use; anywhere else the binding reads the
field's own bytes.

```hale,fragment
let saved = self.row;
self.row = Row { };           // saved is unchanged
let mut copy = saved;
copy.score = 9;               // saved.score is unchanged
```

And the other way round: a local you read out of a place is yours to
write. An array bound from a field, an element of one, a `match`
payload, a `for` element, a tuple's element — writing into it, at any
depth, never changes the field, the record local or the parameter it
came from. The compiler copies the value at the first write into it
(once: later writes go to the copy), so a local you only read costs
nothing. To change the field, write the field (`self.row.scores[0] =
9`), or set the local back:

```hale,fragment
let mut scores = self.row.scores;
scores[0] = 9;                 // self.row.scores is unchanged here
self.row.scores = scores;      // and now it holds the write
```

A part you assign into a local is the same: after `h.scores =
self.row.scores;` the local holds the field's array, and the first write
through `h.scores` copies it, so the field never sees it.

That last shape — read a field, write the local, store it back whole,
with nothing reading the field between — copies nothing at all: the
write lands in the field's own storage, which the store keeps. A
parameter is different: it is the caller's value by reference, and a
function that writes through its parameter (`fn bump(r: Row) {
r.scores[0] = 9; }`) changes what the caller passed. A `for x in
v.items` walk over a `@form(vec)` visits the vec's own cells, so a
write through `x` changes the vec, as before.

A locus is different again. A store into a locus's field from outside
its methods, through a handle (`fn put(l: Ledger, s: String) { l.name
= s; }`, `let l = Ledger { }; l.name = s;`, `h.child.name = s`,
`ledgers[i].name = s`), is the locus's own store, exactly as if a method
of it had written `self.name = s`. The value is copied into the locus,
so the locus and your `s` never share a block, whatever wrote the value
(a variable, an `if`, a call). The value it replaces is recycled, so a
free fn that keeps storing into a long-lived locus does not grow it.

Records nest, and they're what travels on the bus and in and out
of functions. When a record starts wanting *methods*, that's the
signal to promote it to a [locus](./locus-gently.md).

A `type` is always declared at the **top level** — beside your
`fn`s and `locus`es, never inside a locus body. It's in scope
everywhere in the program, including inside every locus, so a
locus that needs a record of its own declares it just above
itself. A `type` written inside a locus body is an error at the
`type` keyword.

A record cannot hold a **locus** either. A field (or an enum
variant) whose type is a locus — `type Layer { w: Model; }` — is an
error at the field: a record is copied by value and has no
lifecycle, so it can neither own the locus nor keep it alive. Keep
the locus in a locus: a `params` field of the locus that uses it,
or a parameter. (A field typed by an *interface* is fine: it is a
handle on a locus someone else owns.)

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

An alias of a generic instantiation can also supply a literal's type
arguments. With `type IntBox = Box<Int>;`, write
`let b: IntBox = Box { value: 1 };`. This works for generic loci too,
including imported templates: a qualified annotation or an alias of
one selects the concrete instance and its child layouts.

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

The parameter belongs to the template. In `Box<T>`, `T` still means
the supplied type argument even if the seed also declares a type,
locus or alias named `T`.

Behind the scenes the compiler calls that instance's type
`Cache_Int_String`, and you will see the name in a diagnostic. For
a record you may write that name yourself — `Box_Int { value: 1 }`
is the same type as `Box<Int>` — but for a locus it is the
compiler's name only: build one through `Cache` with the arguments
on the binding.

Next: reading and writing the world — [Files](./files.md).
