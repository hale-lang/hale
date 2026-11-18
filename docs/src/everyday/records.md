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
| **Type alias** | `type Cents = Int;` / `type UserId = String;` | Transparent synonym for an existing type |
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
- **A record cannot hold itself by value:** `type Node { next: Node; }` (or `A` holding a `B` that holds an `A`) has no smallest value, and `hale check` refuses it at the field that closes the ring. Hold children in a `bounded[Node; N]` (it starts empty), or in an enum variant beside one that holds none (`type Link = enum { End, Next(Node) };`), or keep them elsewhere and hold a key.
- **A record field's default names its type:** write `meta: Meta = Meta { };`, not `meta: Meta = { };`. A bare `{ }` is an empty block, which produces no value, and `hale check` refuses it at the default and spells the literal (`Meta { }`).
- **Top-level only:** `type` declarations must appear at the top level outside any `locus` body.
- **No loci inside records:** Because records are pure values with no lifecycle hooks, a `type` field (or `enum` variant) cannot hold a `locus` or a function pointer whose signature names a locus (`fn() -> Worker`). Store loci inside another locus's `params` instead. (A record field typed by an `interface` is permitted because it is a non-owning reference.)

### Optional Fields (`x?: T`)

A field marked `?` may be absent. `x?: T` declares `x` and, right after it, a real `Bool` field `has_x` that says whether `x` holds a value; `x?: T clearable` adds `clear_x` as well, for a PATCH field whose `null` means "clear it":

```hale
type Ticker {
    symbol: String;
    last?: Decimal<8>;
    expires_at?: Time clearable;
    limit?: Int = 100;
}

fn main() {
    let mut t = Ticker { symbol: "BTC" };   // `last` left out: absent
    println(t.has_last);                    // false
    t.last = 97000.5d;                      // a write sets the flag
    println(t.has_last);                    // true
    println(t.limit, t.has_limit);          // 100 false: absent, at its default
}
```

- **A write sets the flag:** a literal that spells `x`, an assignment to it through any path (`t.last = v`, `self.row.last = v`, `rows[i].last = v`), a write into a part of it, and a `push` or `set` into an optional `bounded` field all set `has_x` (and clear `clear_x`). The flags are ordinary fields, so a program may also read and write them by name.
- **Leaving it out builds it absent:** `x` then holds its default, or its type's zero (`0`, `""`, `false`, the epoch, an empty list, an enum's first variant, a record of zeros). A type with no zero (a tuple, a point, `Bytes`) needs a default, and `hale check` says so at the field.
- **One shape on the wire:** the flags are exactly the hand-written presence pair (`x: T; has_x: Bool;`) and triple the JSON and protobuf codecs read, so a record converted from the pair to `x?` crosses the wire, describes itself and hashes byte for byte as before (see [JSON](json.md) and [The API surface](../services/api.md)).
- **Only record fields:** Hale has no optional type. `?` on a parameter, a local or a locus param, and `T?` after any type, are refused; a value that may be missing in a computation is `fallible(E)` ([Fallible values](../basics/fallible.md)).

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
3. **Concrete generic aliases only:** An alias can name one instantiation of a [generic](./generics.md) record or locus (`type OrderReply = Envelope<Order>;`), and `let r: OrderReply = Envelope { data: o };` then takes its type arguments from the alias. An alias cannot take parameters itself: `type Reply<T> = Envelope<T>;` is refused.

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

- **Construction and formatting:** Construct variants with `EnumName::Variant` (`Event::Halt`, `Event::Tick(3)`). Printing an enum outputs its declared name and payload (`Event::Tick(3)`), even when imported via a library alias. A variant the enum does not declare (`Event::Tik`) is a type error from `hale check` wherever it is written, a value, a constructor call or a `match` pattern, with the nearest variant suggested.
- **Exhaustive matching:** `match` arms can match literal sub-patterns (`Event::Tick(0)`) before variable bindings (`Event::Tick(n)`), and must cover every variant in the `enum`.
- **`enum` vs. `fallible(E)`:** Use `enum` for domain states and sum-type messages; use `fallible(E)` ([When a Call Can Fail](../basics/fallible.md)) for functions that can fail.

### Names, Parsing and the Variant List

Don't write a name function and a parse function for each enum. The compiler writes four for every enum `E`:

```hale
type Lifecycle = enum { Active, Archived };

fn load(status: String) -> Lifecycle {
    // an unknown name fails with ParseError { kind: "unknown_variant", input }
    return Lifecycle::parse(status) or Lifecycle::Active;
}

fn main() {
    for l in Lifecycle::variants {                      // [Lifecycle; 2], declaration order
        println(Lifecycle::index(l), " ", Lifecycle::name(l)); // 0 Active, 1 Archived
    }
}
```

- **`E::name(v)`** is the variant's external name, its values ignored. **`E::index(v)`** is its position in the declaration, from 0. Both work on any enum but a generic one.
- **`E::parse(s)`** is the variant `s` names exactly, and fails with `ParseError { kind: "unknown_variant", input: s }` otherwise; write `or raise`, or `or` a default. **`E::variants`** is a value (no parentheses) holding every variant in declaration order. Both need variants that carry no values: on an enum like `Event` above, `hale check` refuses them and names the variant that carries values.
- **One external name:** a variant's name is its `json:` tag's when it has one (``Sell `json:"sell"` ``), so `name`, `parse` and the [JSON codecs](./json.md) always agree. With a catch-all, ``Other(String) `json:",other"` ``, `parse` cannot fail: an unknown name becomes `Other(s)`, and `name(Other(s))` is `s`. `E::variants` leaves that catch-all out.
- **`@wire_case`:** spell every variant in one case without tagging each: `@wire_case(snake) type Phase = enum { InProgress, Done };` makes the names `in_progress` and `done`. The cases are `snake`, `screaming_snake` (`IN_PROGRESS`), `kebab` (`in-progress`) and `lower` (`inprogress`), and a variant's own `json:` tag still wins.
- **Printing is unchanged:** `println(Lifecycle::Archived)` still shows `Lifecycle::Archived`; ask for `Lifecycle::name(v)` where you want the external name. To change how a record or a variant reads to people, give it a `@display("…")` template; see [Strings & Bytes](../basics/strings.md). The external name is the wire's and the template is the reader's, and neither changes the other.
- **A variant named `name` keeps its meaning:** if an enum declares a variant called `name`, `parse`, `variants` or `index`, `E::name` is that variant, and the other three are still the enum's.

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

- **Type-as-tag coercion:** A `Book`, `Trade`, or `Unknown` value automatically coerces to `Inbound` wherever `Inbound` is expected (return values, a function body's final expression included, function arguments, typed `let` bindings, or struct fields), and on the error channel of a `fallible(Inbound)` function: `fail`, `or fail err` and `or raise` of a member are the set's ([Converting Errors for `or raise`](../basics/fallible.md#converting-errors-for-or-raise)).
- **Bare names are for sets only:** A set's `match` names a member by its type alone (`Book(b)`). An `enum`'s `match` always names the enum (`Shape::Line(l)`); `Line(l)` against an enum, or against a generic fn's type parameter, is a type error.
- **Mixed branches are not joined into a set:** The arms of an `if` or `match` whose values are different members (`Book { ... }` in one arm, `Trade { ... }` in the other) are not joined into `Inbound`, even under an annotated binding (`let m: Inbound = if ...`) or as the value of a `return` from a function declared `-> Inbound`: `hale check` refuses the expression ("if-expression arms have mismatched types"). Return from each branch instead (`if n == 0 { return Book { ... }; } return Trade { ... };`), or give each arm the set's type, a call to a function returning `Inbound` (`let m = if n == 0 { book() } else { trade() };`).
- **Members must be declared structs or enums:** Primitive types like `Int` cannot appear directly in a set; wrap them in a struct (`type Count { n: Int; }`).
- **Wire representation and seams:** Across JSON APIs, a set serializes as a tagged object keyed by the member's type name (`{"Trade": {"qty": 2, "px": 1.5}}`). At the service layer, a [seam](../services/seams.md) can dispatch each member of a set to a dedicated handler method and verify exhaustiveness at compile time.

## Generic Records

A record can take type parameters (`type Envelope<T> { data: T; request_id: String = ""; }`), and so can a locus or a `fn`. See [Generics](./generics.md).

Next: [Generics](./generics.md).
