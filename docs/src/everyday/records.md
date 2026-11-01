# Records & Data

Hale models structured data with nominal `type` records, tagged `enum` unions, and type-tagged set unions (`A | B | C`). Every data type is checked at compile time, copied by value with automatic copy-on-write optimization, and matched exhaustively with `match`.

```hale
type Book    { id: Int; title: String; }
type Trade   { qty: Int; px: Decimal<2>; }
type Inbound = Book | Trade;

fn describe(msg: Inbound) -> String {
    return match msg {
        Book(b)  -> f"book #{b.id}: {b.title}",
        Trade(t) -> f"trade {t.qty} @ {t.px}",
    };
}

fn main() {
    let m: Inbound = Trade { qty: 10, px: 42.50d };
    println(describe(m));
}
```

## Data Declarations Overview

| Construct | Syntax Example | Description |
|---|---|---|
| **Record (`type`)** | `type Player { id: String; score: Int = 0; }` | Fixed-layout product type with optional field defaults |
| **Enum (`enum`)** | `type Event = enum { Tick(Int), Halt };` | Tagged union with unit or payload-carrying variants |
| **Set union (`\|`)** | `type Inbound = Book \| Trade \| Unknown;` | Union of existing struct/enum types where the member type is the tag |
| **Type alias** | `type Cents = Int;` / `type IntBox = Box<Int>;` | Transparent synonym for an existing type or concrete generic instantiation |
| **Generic record** | `type Box<T> { value: T = 0; }` | Template monomorphized per concrete type argument (`Box<Int>`) |
| **Fixed array / slice** | `[Int; 8]` / `[Player]` | Inline fixed-length array (`[0; 8]`) or slice view |
| **Tuple** | `let pair = (1, "one");` | Lightweight anonymous grouping |

## Records (`type`)

Declare a record at the top level of a file with `type Name { ... }` and construct it with a struct literal:

```hale
type Config {
    host: String = "127.0.0.1";
    port: Int    = 8080;
}

fn main() {
    let c = Config { port: 9000 };    // `host` uses its default "127.0.0.1"
    println(c.host, ":", c.port);
}
```

- **Compile-time field verification:** Unknown field names, missing required fields (fields without defaults), or mismatched field types fail compilation at the literal—including for cross-seed records constructed as `alias::Config { ... }`.
- **Top-level only:** `type` declarations must appear at the top level outside any `locus` body.
- **No loci inside records:** Because records are pure values with no lifecycle hooks, a `type` field (or `enum` variant) cannot hold a `locus` or a function pointer whose signature names a locus (`fn() -> Worker`). Store loci inside another locus's `params` instead. (A record field typed by an `interface` is permitted because it is a non-owning reference.)

## Value Copy Semantics and Mutation

### Independent Value Copies and Copy-on-Write

Binding a record, `String`, `Bytes`, tuple, or array from a field or variable (`let saved = self.row;` or `copy = saved;`) gives the binding an independent logical copy of the value:

```hale,fragment
let saved = self.row;
self.row = Row { };           // `saved` retains the previous value
let mut copy = saved;
copy.score = 9;               // `saved.score` is unchanged
```

Under the hood, the compiler avoids copying whenever no mutation can alias the read, and performs a copy on the first write into a local derived from a field, `match` payload, `for` element, or tuple.

### In-Place Read-Modify-Write Optimization

To update a nested array or field inside a locus, either assign directly (`self.row.scores[0] = 9;`) or read into a local, mutate, and write it back:

```hale,fragment
let mut scores = self.row.scores;
scores[0] = 9;                 // `self.row.scores` is unchanged until written back
self.row.scores = scores;      // optimized to an in-place write with zero allocations
```

When a local read from a field is mutated and written back to the same field with no intervening reads of that field, the compiler updates the field's storage in place without allocating or copying.

### Reference Semantics for Parameters, `@form(vec)` Iteration, and Locus Handles

| Target | Mutation Behavior |
|---|---|
| **Record function parameter** (`fn bump(r: Row)`) | Passed by reference to the caller's value; `r.scores[0] = 9;` mutates the caller's record. |
| **`@form(vec)` loop variable** (`for x in v.items`) | Visits the vec's cells in place; writing through `x` updates the stored element. |
| **Locus field via handle** (`l.name = s;`) | Copies `s` into locus `l`'s arena and recycles the replaced value so repeated external writes never grow `l`'s arena. |

A method that writes into a record parameter (`fn put(x: Bag, p: Bytes) { x.b = p; }`) writes the caller's record, so the value is kept as long as that record, never only for the call, whether the method is `fallible(E)` or not (and when it then fails). The method may pass the record on to another writer, directly or through a block, `if` or `match`, and the same holds at every step. A record a field of `self` holds (`put_free(self.bag, p)`) is kept in the locus's arena. One call can hand a writer only one owner's records. `self.put_two(self.held, x, p)`, or `self.put(if c { self.held } else { x }, p)`, mixes the locus's record with the caller's and is rejected. Copy one side into a local record first (`let mine = Bag { b: x.b };`) and hand that, or write the held record through `self`.

Such a write is not recycled when the method returns, and does not replace the old value in place the way `self.bag.b = p` does. Handing a writer `self.bag` once per message, or once per pass of a `run()` loop, keeps every value until the locus dissolves, and `hale check` reports the call as an unbounded allocation. Write the held record through `self` instead, or hand the writer a record the loop body builds.

## Arrays (`[T; N]`) and Tuples

Use `[T; N]` for fixed-length inline arrays, `[T]` for slice views, and `(a, b)` for small ad-hoc tuples:

```hale,fragment
type Match { players: [Player]; }      // slice field

let xs    = [1, 2, 3];                 // [Int; 3]
let zeros = [0; 8];                    // [Int; 8] filled with 0
let pair  = (1, "one");                // tuple
```

For growable lists, use a `@form(vec)` locus or `bounded[T; N]` ([Lists & Maps](./collections.md)).

## Type Aliases (`type Name = Target;`)

A type alias introduces a transparent synonym for an existing type:

```hale
type Cents = Int;

fn price_of(n: Cents) -> Cents { return n * 2; }

fn main() {
    let c: Cents = 250;
    println(price_of(c));
}
```

Because `Cents` is transparent, it is completely interchangeable with `Int` (including in struct construction `RowAlias { id: 1 }` and enum patterns `LightAlias::Red`). If you need the compiler to reject mixing two integer types, use `distinct Int` ([Units, End to End](../units.md)) or a single-field struct (`type Cents { v: Int; }`).

### Type Alias Rules

1. **Literal construction requires a record or locus target:** If `type Cents = Int;`, writing `Cents { }` is a compile error.
2. **No cyclic aliases:** `type A = B; type B = A;` is rejected.
3. **Concrete generic aliases only:** Generic aliases (`type Twin<T> = Pair<T>;`) are not supported, but aliasing a concrete instantiation (`type IntBox = Box<Int>;`) is supported and supplies the type arguments for `let b: IntBox = Box { value: 1 };` (for both generic records and generic loci).

## Enums (`enum`) and Exhaustive `match`

An `enum` defines a tagged union whose variants may be unit tags or carry positional payloads:

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

fn main() {
    handle(Event::Tick(0));
    handle(Event::Trade(19.99d, 5));
}
```

- **Construction and formatting:** Construct variants with `EnumName::Variant` (`Event::Halt`, `Event::Tick(3)`). Printing an enum outputs its declared name and payload (`Event::Tick(3)`), even when imported via a library alias.
- **Exhaustive matching:** `match` arms can match literal sub-patterns (`Event::Tick(0)`) before variable bindings (`Event::Tick(n)`), and must cover every variant in the `enum`.
- **`enum` vs. `fallible(E)`:** Use `enum` for domain states and sum-type messages; use `fallible(E)` ([When a Call Can Fail](../basics/fallible.md)) for functions that can fail.

## Set Types (`A | B | C`)

When each alternative is already a standalone `type` record or `enum`, declare a **set type** using `|` instead of wrapping each record in a redundant enum variant:

```hale
type Book    { id: Int; title: String; }
type Trade   { qty: Int; px: Float; }
type Unknown { raw: String; }
type Inbound = Book | Trade | Unknown;

fn decode(n: Int) -> Inbound {
    if n == 0 { return Book { id: 7, title: "Kim" }; }
    if n == 1 { return Trade { qty: 2, px: 1.5 }; }
    return Unknown { raw: "?" };
}

fn describe(m: Inbound) -> String {
    return match m {
        Book(b)    -> "book " + b.title,
        Trade(t)   -> "trade of " + to_string(t.qty),
        Unknown(u) -> "unknown " + u.raw,
    };
}
```

- **Type-as-tag coercion:** A `Book`, `Trade`, or `Unknown` value automatically coerces to `Inbound` wherever `Inbound` is expected (return values, function arguments, typed `let` bindings, or struct fields).
- **Target type required for mixed branches:** An unannotated `if` or `match` whose arms produce `Book` and `Trade` (`let m = if n == 0 { Book { ... } } else { Trade { ... } };`) fails typechecking because the compiler does not guess which set type joins them; annotate the binding (`let m: Inbound = ...`) or return from a function declared `-> Inbound`.
- **Members must be declared structs or enums:** Primitive types like `Int` cannot appear directly in a set; wrap them in a struct (`type Count { n: Int; }`).
- **Wire representation and seams:** Across JSON APIs, a set serializes as a tagged object keyed by the member's type name (`{"Trade": {"qty": 2, "px": 1.5}}`). At the service layer, a [seam](../services/seams.md) can dispatch each member of a set to a dedicated handler method and verify exhaustiveness at compile time.

## Generic Records and Loci (`<T>`)

Both `type` records and `locus` declarations can be parameterized by type arguments:

```hale
type Box<T> {
    value: T = 0;
}

locus Cache<K, V> {
    params {
        cap: Int = 1;
    }
}

fn main() {
    let b: Box<Int> = Box { value: 1 };
    let c: Cache<Int, String> = Cache { cap: 2 };
    println(b.value, " ", c.cap);
}
```

### Monomorphization and Type-Argument Rules

- **Explicit type context required:** In `let b: Box<Int> = Box { value: 1 };`, the struct literal spells the template name `Box` while the concrete type argument `<Int>` comes from the surrounding type context (a `let` annotation, function return type `-> Box<Int>`, enclosing struct field, assignment target, or `[Box<Int>; 2]` array annotation). Writing `let b = Box { value: 1 };` without a type annotation is a compile error.
- **Supported type arguments:** Any user-defined `type` record and all built-in types (`Int`, `Float`, `Bool`, `String`, `Duration`, `Decimal`, `Time`, `Bytes`, `BytesView`, `BytesMut`, `StringView`) are valid type arguments. (`Uint` has no lowered representation and is rejected; use `Int`.)
- **Name mangling:** Each instantiation compiles to a distinct specialized type named `Template_Arg1_Arg2` (such as `Box_Int` or `Cache_Int_String`). Inside `Box<T>`, the identifier `T` always resolves to the type parameter even if the seed also declares a top-level type named `T`.

Next: [Files](./files.md).
