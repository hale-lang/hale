# JSON

> **Coming from Python / Node?** There's no `JSON.parse` that
> hands you a dynamic object you index freely. Hale's `std::json`
> is field-oriented: you ask a JSON string for a named field and
> a type (`find_string_field`, `find_int_field`, …), and you
> build output with a streaming `Builder`. At v1 it's tuned for
> flat objects and arrays — the common shapes for config and wire
> messages.

## Reading

Pull individual fields out of a JSON string by name:

```hale,fragment
let doc = "{\"name\": \"Ada\", \"age\": 36, \"active\": true}";

let name   = std::json::find_string_field(doc, "name");    // "Ada"
let age    = std::json::find_int_field(doc, "age");        // 36
let active  = std::json::find_bool_field(doc, "active");    // true
```

Missing fields come back as the type's zero value (`""`, `0`,
`false`) rather than failing — so for "is this really present?"
semantics, check with the raw accessor or validate upstream.
`find_field_raw` returns the raw substring for a field, which is
how you reach into a nested object:

```hale,fragment
let inner = std::json::find_field_raw(doc, "address");
let city  = std::json::find_string_field(inner, "city");
```

## Parsing into a type

Pulling fields one by one rescans the document per field. When you have
a fixed shape, tag the fields with their JSON keys and the compiler
generates a single-pass parser for you:

```hale,fragment
type Order {
    id: Int      `json:"id"`;
    price: Int   `json:"px"`;     // JSON key differs from the field name
    qty: Float   `json:"sz"`;
    active: Bool `json:"on"`;
    side: String `json:"side"`;
    currency: String = "USD";     // optional: default fills a missing key
}

let o = Order::from_json(body) or raise;
println(o.price);
```

`Type::from_json(s) -> Type fallible(JsonError)` walks the object once,
dispatches each key to the matching field, and reads the value by the
field's declared type — no per-field rescan, and unmatched keys (and
nested objects/arrays under them) are skipped. The `json:"<key>"` tag
sets the JSON key; without it the field name is the key.

A **missing field raises** `JsonError`, naming the field — *unless* the
field declares a default (`currency: String = "USD"`), in which case the
default fills it. Because `from_json` is `fallible`, you must address it
(`or raise`, `or <fallback>`, …) like any other fallible call.

A field whose type is **another `json:`-tagged struct** is parsed
recursively — nest as deep as you like, and a missing field anywhere
raises with that field's name:

```hale,fragment
type Addr   { city: String `json:"city"`; zip: Int `json:"zip"`; }
type Person { name: String `json:"name"`; home: Addr `json:"home"`; }

let p = Person::from_json(body) or raise;
println(p.home.city);
```

The same tags drive the reverse direction — `Type::to_json(value)`
serializes back to a JSON string (numbers and bools bare, strings escaped,
nested structs recursed), so `from_json` / `to_json` round-trip:

```hale,fragment
let body = Order::to_json(o);          // -> {"id":7,"px":...}
let o2   = Order::from_json(body) or raise;
```

`to_json` is not fallible — serialization always succeeds.

The tag is general `key:"value"` metadata — `json:` is one consumer;
other keys are free for future tools.

Fields must be scalars — `Int` / `Float` / `Bool` / `String` — or nested
`json:`-tagged structs. **Array fields are not supported**, by design:
Hale sequences are locus-owned (there is no heap-owning value list to put
in a struct). To read a JSON array, walk it with the [array
cursor](#arrays) and `push` each element into a `@form(vec)` cell on a
locus — `from_json` handles the flat/nested record shape, arrays stay an
explicit, locus-owned step.

## Arrays

Walk a JSON array with the iterator pair:

```hale,fragment
let arr = "[10, 20, 30]";
let mut it = std::json::array_first(arr);
while !it.done {
    let n = std::str::parse_int(it.element) or 0;
    println(n);
    it = std::json::array_next(it);
}
```

`array_first` returns an iterator with the first `element` and a
`done` flag; `array_next` advances it.

## Writing

The `Builder` is a streaming assembler — it tracks open scopes
and inserts separators for you, so you can't produce malformed
JSON by forgetting a comma:

```hale,fragment
let b = std::json::Builder { };
b.begin_object();
b.field("name", "Ada");
b.int_field("age", 36);
b.bool_field("active", true);
b.end_object();
let out = b.result();      // {"name":"Ada","age":36,"active":true}
```

Nest objects and arrays by pairing `begin_*` / `end_*`. String
values are escaped per the JSON spec automatically; if you need
to escape or unescape a string by hand, `std::json::escape_string`
and `unescape_string` are there.

## When the shape is deep

`std::json` at v1 is built for flat objects and top-level arrays
— the great majority of config files and API messages. For
deeply-nested documents you walk level by level with
`find_field_raw`, treating each nested object as its own flat
document. If you're parsing a genuinely complex or
performance-critical format, the [zero-copy binary
techniques](../systems/zero-copy-bus.md) and the systems-tier
[performance](../systems/performance.md) chapter cover building
your own parser over `Bytes`.

Next: serving and calling over the network — [HTTP](./http.md).
