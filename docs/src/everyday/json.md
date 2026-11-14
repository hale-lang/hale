# JSON

Hale provides two ways to work with JSON: compile-time generated struct codecs (`Type::from_json` and `Type::to_json`) for fixed schemas, and the `std::json` module for dynamic field extraction, streaming output construction, cursor iteration, and input validation.

```hale
type Order {
    id: Int      `json:"id"`;
    price: Int   `json:"px"`;
    qty: Float   `json:"sz"`;
    active: Bool `json:"on"`;
    side: String `json:"side"`;
    currency: String = "USD";
}

fn parse_and_echo(body: String) -> String fallible(JsonError) {
    let order = Order::from_json(body) or raise;
    return Order::to_json(order);
}

fn main() {
    let raw = "{\"id\":7,\"px\":100,\"sz\":2.5,\"on\":true,\"side\":\"buy\"}";
    let out = parse_and_echo(raw) or "";
    println(out);
}
```

## API Summary

| API | Signature / Type | Description |
|---|---|---|
| `Type::from_json` | `fn(String) -> Type fallible(JsonError)` | Single-pass parser generated for structs with `json:"..."` field tags |
| `Type::to_json` | `fn(Type) -> String` | Infallible serializer generated for structs with `json:"..."` field tags |
| `std::json::string_field` | `fn(String, String) -> JsonString` | Inspects a top-level field's JSON type (`kind`) and decoded string (`text`) |
| `std::json::find_string_field` | `fn(String, String) -> String` | Extracts a top-level field as `String` (returns `""` if missing; coerces non-strings) |
| `std::json::find_int_field` | `fn(String, String) -> Int` | Extracts a top-level field as `Int` (returns `0` if missing or non-numeric) |
| `std::json::find_bool_field` | `fn(String, String) -> Bool` | Extracts a top-level field as `Bool` (returns `false` unless `true`) |
| `std::json::find_field_raw` | `fn(String, String) -> String` | Returns the raw JSON substring of a top-level field (for nested objects/arrays) |
| `std::json::array_first` / `array_next` | `ArrayIterSpan` cursor | Iterates elements of a JSON array (`it.element`, `it.done`) |
| `std::json::object_first` / `obj_key_string` | `ObjectIterSpan` cursor | Iterates key-value pairs of a JSON object with dynamic keys |
| `std::json::Builder` | Streaming builder locus | Incrementally constructs well-formed JSON objects and arrays |
| `std::json::escape_string` / `unescape_string` | `fn(String) -> String` | Escapes and unescapes JSON string content (including `\uHHHH` and surrogate pairs) |
| `std::json::valid` | `fn(String) -> Bool` | Zero-allocation strict JSON validator (up to 64 nesting levels) |
| `std::json::valid_object` | `fn(String) -> Bool` | Validates a JSON object with up to 64 unique, unescaped top-level keys |

---

## Typed Struct Codecs (`from_json` and `to_json`)

When parsing or serializing a known schema, annotate `type` fields with `` `json:"<key>"` `` metadata tags. The compiler generates a single-pass parser (`Type::from_json`) and serializer (`Type::to_json`). If a field omits the `json:` tag, the field's identifier is used as the JSON key.

```hale,fragment
type Addr {
    city: String `json:"city"`;
    zip: Int     `json:"zip"`;
}

type Person {
    name: String `json:"name"`;
    home: Addr   `json:"home"`;
    role: String = "member";      // optional: uses default when key is absent
}

let p = Person::from_json(body) or raise;
println(p.home.city);

let encoded = Person::to_json(p);
let roundtrip = Person::from_json(encoded) or raise;
```

### Decoding & Encoding Rules

- **Single-pass scanning**: `Type::from_json(s) -> Type fallible(JsonError)` walks the JSON object once, dispatches matched keys to their target fields, and skips unknown keys (including nested objects or arrays under unknown keys).
- **Required vs. optional fields**: A missing field raises `JsonError { kind: "missing_field", field }` (naming the missing field) unless the field declaration includes a default value, of any shape a field default may have: a literal (`role: String = "member"`, `n: Int = -1`), an enum variant (`side: Side = Side::Buy`), a record literal (`inst: Instrument = Instrument { sym: "Y", tick: 2 }`, its own fields' defaults filling what it leaves out). The codec builds the default when the key is absent, exactly as the record's own literal would. A key sent as `null` is a key absent: the field takes that same default, whatever its shape (`"n": null` reads `-1`, `"side": null` reads `Side::Buy`, `"inst": null` reads the declared `Instrument { sym: "Y", tick: 2 }`), and a field with no default is `missing_field`. Only a `std::json::Value` field holds `null` as its text.
- **Error handling**: Because `from_json` is `fallible(JsonError)`, callers must address errors with `or raise`, `or <fallback>`, or an error handler. `Type::to_json(value)` is infallible and always returns a `String`.
- **Supported field types**: what an api surface's JSON codec carries of a field ([The API surface](../services/api.md)):
  - **Scalars**: `Int`, `Float`, `Bool`, `String`, and `Decimal<S>`; a unit-dialect scalar (an identity, a range, a quantity) as its `Int`; a `Time` as its RFC 3339 text; a `Duration` as its nanoseconds; a `std::json::Value` as its text.
  - **Exact decimals**: `Decimal<S>` serializes as an exact JSON number (`19.99`, without float approximation) and parses at scale `S`, any JSON number spelling included: an exponent is applied exactly (`1e-7` at scale 8 is `0.0000001`, `1.50E+1` is `15`). A JSON string holding a number parses the same way, so an API that quotes its prices (`{"price": "97000.50"}`) and one that sends numbers both decode, with no float on the way. Numbers with more fractional digits than `S` (the exponent applied), a mantissa past i128, and spellings that are not JSON numbers (`+1`, `.5`, `1.`, `01`, or a string holding one of those, a space or anything but the number) are rejected with `wrong_type` rather than truncated or read as zero. To *write* a Decimal as a string, tag the field `` `json:"price,string"` `` (or `` `json:",string"` `` to keep the field's name): `to_json` then writes `"97000.5"`. `string` is the one tag option, and it is a Decimal's (a field or a list of them); the check refuses any other.
  - **Enums**: a payload-free variant is its name as a JSON string (`"Buy"`), read back by name; a name no variant has is `wrong_type` for the field. A variant with values is written as the api codec writes it (`{"Circle": 1.5}`).
  - **Nested records**: Any field whose type is another record of the seed is parsed and serialized recursively, by that record's own codec (its keys its fields' tags, else their names), whether or not it declares a `json:` tag of its own.
  - **Lists**: a `bounded[T; N]` or `[T; N]` of any of those is a JSON array; more elements than `N` is `too_long`, fewer than a fixed array's `too_short`, and an absent `bounded` field is empty.
- **Positional rows**: many APIs send a table's rows as arrays (`[[1688666400, "2990.5", "3010", 100.5], ...]`). Mark the row's record `@positional` and it crosses as an array of its fields in declaration order, as a field, a list's element or a whole value; `@positional` gives the record `from_json` / `to_json` without a tag:

  ```hale,fragment
  @positional type Candle { t: Int; open: Decimal<2>; close: Decimal<2>; vol: Decimal<4>; }
  @positional type Level { px: Decimal<2>; qty: Decimal<4>; orders: Int = 1; }
  type Book { sym: String `json:"symbol"`; bids: bounded[Level; 50]; candles: bounded[Candle; 500]; }

  let book = Book::from_json(body) or raise;  // {"symbol": "X", "bids": [["3000.40", "0.5", 1], ...], ...}
  ```

  Each element is read as its field would be, so one row mixes kinds (a Decimal from a string beside an `Int` from a number). Elements past the last field are ignored, as unknown keys are, so a row that gains a column keeps decoding; a row that ends early leaves the rest absent: a field with a default takes it, and any other is `missing_field` named `<Type>.<field>` (`Level.qty`), the way every error inside a row names its field. Every field is an element (no presence pairs), and a positional record's tags name no key. A served `api` surface does not carry one: read and write positional rows with the codec calls.
- **Through an import**: `pricing::Observation::from_json(text)` and `pricing::Observation::to_json(v)` name the library's declaration, as `pricing::Observation { … }` does; the library's own `Observation::to_json` is the same codec.
- **The check is the gate**: a tagged record with a field the codec does not carry (`Bytes`, an unbounded `[T]`, a tuple), in it or in a record it reaches, has no codec, and `hale check` refuses the call naming the field: `` `Export::from_json`: `Export` has a field `raw: Bytes`, which the JSON codec does not carry ``. A call on a record with no `json:` tag is refused with the tag to write, and a call on an enum, or with two arguments, is refused too. A program that checks builds.

> [!NOTE]
> `Type::from_json` does not support unbounded `[T]` fields because general sequences in Hale are locus-owned (`@form(vec)`). Declare the field `bounded[T; N]` for a list with a known cap; to read a JSON array of unknown length with `std::json`, extract the raw array slice with `find_field_raw` (if nested), iterate it with [`array_first` / `array_next`](#iterating-arrays-and-dynamic-objects), and `push` elements into a locus collection.

---

## Dynamic Field Extraction (`std::json`)

For ad-hoc queries or configuration fragments where you only need a few fields, extract fields directly by key name:

```hale,fragment
let doc = "{\"name\": \"Ada\", \"age\": 36, \"active\": true}";

let name   = std::json::find_string_field(doc, "name");   // "Ada"
let age    = std::json::find_int_field(doc, "age");       // 36
let active = std::json::find_bool_field(doc, "active");   // true
```

- **Top-level matching**: Field lookups match top-level keys only. A string value elsewhere in the document cannot shadow a key, and nested keys are not matched until you extract their parent object.
- **Zero-value defaults**: Missing fields return the target type's zero value (`""`, `0`, `false`).
- **Nested objects**: Call `std::json::find_field_raw` to retrieve the raw JSON substring of a nested object, then query fields on that substring:

```hale,fragment
let inner = std::json::find_field_raw(doc, "address");
let city  = std::json::find_string_field(inner, "city");
```

### Handling Nullable and Optional Fields (`string_field`)

`find_string_field` is permissive and coerces non-string tokens into strings, while `find_int_field` and `find_bool_field` coerce `null` or mismatched types to `0` and `false`:

| Input JSON | `find_string_field(doc, "owner")` | `string_field(doc, "owner")` |
|---|---|---|
| `{"owner": "Ada"}` | `"Ada"` | `JsonString { kind: "string", text: "Ada" }` |
| `{"owner": null}` | `"null"` | `JsonString { kind: "null", text: "" }` |
| `{"owner": ""}` | `""` | `JsonString { kind: "string", text: "" }` |
| `{}` | `""` | `JsonString { kind: "missing", text: "" }` |
| `{"owner": 7}` | `"7"` | `JsonString { kind: "number", text: "" }` |

When you need to distinguish `"null"` from `null`, or verify a field's type before reading it, use `std::json::string_field(doc, key) -> std::json::JsonString`:

```hale,fragment
let owner = std::json::string_field(body, "owner");
if owner.kind == "string" {
    println("owned by ", owner.text);
} else if owner.kind == "null" || owner.kind == "missing" {
    println("unassigned");
} else {
    println("owner must be a string or null");
}
```

`JsonString` contains two fields:
- `kind`: One of `"string"`, `"null"`, `"missing"`, `"number"`, `"bool"`, `"array"`, `"object"`, or `"invalid"`.
- `text`: Decoded string content (quotes stripped, escapes resolved) when `kind == "string"`, and `""` for all other kinds.

`"missing"` means the document is a JSON object without that key. `"invalid"` means the document is not an object or the field value is syntactically malformed (such as an unterminated string or bare `NaN`). Duplicate keys within the same object resolve to the first occurrence.

You can also use `string_field` to guard numeric or boolean reads:

```hale,fragment
if std::json::string_field(body, "port").kind == "number" {
    let port = std::json::find_int_field(body, "port");
    println(port);
}
```

`JsonString`, `ArrayIterSpan`, `ObjectIterSpan`, and `JsonFieldRange` are statically checked structs in `std::json`, so misspelled field names fail at compile time.

---

## Iterating Arrays and Dynamic Objects

### Array Iteration (`ArrayIterSpan`)

Walk a JSON array using `std::json::array_first` and `std::json::array_next`:

```hale,fragment
let arr = "[10, 20, 30]";
let mut it = std::json::array_first(arr);
while !it.done {
    let n = std::str::parse_int(it.element) or 0;
    println(n);
    it = std::json::array_next(it);
}
```

`array_first` returns an `ArrayIterSpan` containing the first `element` substring and `done: Bool`. `array_next` advances the cursor.

### Dynamic Object Iteration (`ObjectIterSpan`)

When iterating a JSON object whose keys are not known ahead of time (such as a `dependencies` or `versions` map) with `object_first`, read each key using `std::json::obj_key_string(it, doc)` and values using `obj_value_string`. Unlike manual slicing (`doc[it.key_start..it.key_end]`), `obj_key_string` decodes JSON escape sequences in keys. To compare a key without allocating, use `obj_key_eq`.

---

## Building JSON (`std::json::Builder`)

`std::json::Builder` is a streaming JSON writer that tracks open object/array scopes, escapes strings, and inserts commas automatically into a single growing byte buffer:

```hale,fragment
let b = std::json::Builder { };
b.begin_object();
b.string_field("name", "Ada");
b.int_field("age", 36);
b.bool_field("active", true);
b.end_object();
let out = b.result();   // {"name": "Ada", "age": 36, "active": true}
```

| Builder Method | Description |
|---|---|
| `begin_object()` / `end_object()` | Opens and closes a JSON object (`{` ... `}`) |
| `begin_array()` / `end_array()` | Opens and closes a JSON array (`[` ... `]`) |
| `begin_object_field(name)` | Writes `"name": {` and opens a nested object scope |
| `begin_array_field(name)` | Writes `"name": [` and opens a nested array scope |
| `string_field(name, value)` | Writes `"name": "escaped_value"` |
| `int_field(name, value)` | Writes `"name": 123` |
| `bool_field(name, value)` | Writes `"name": true` or `false` |
| `string_value(value)` | Writes a quoted, escaped string element inside an array |
| `field(name, raw_json)` | Writes `"name": <raw_json>` for pre-formatted JSON fragments |
| `result() -> String` | Returns an independent copy of the accumulated JSON string |

### String Escaping and Unescaping

For manual string escaping, use `std::json::escape_string(s)` and `std::json::unescape_string(s)`:

```hale,fragment
let a = std::json::unescape_string("hale.v\\u0031");
println(a == "hale.v1");                               // true
println(std::json::unescape_string("\\u00e9"));        // é
println(std::json::unescape_string("\\u4e2d"));        // 中
println(std::json::unescape_string("\\ud83d\\ude00")); // 😀
```

`unescape_string` resolves standard escapes (`\"`, `\\`, `\n`, `\t`, etc.), `\uHHHH` hex escapes, and UTF-16 surrogate pairs for code points above `U+FFFF`.

- **Unpaired surrogates** and **`\u0000`** decode to `` (`U+FFFD`). Because Hale strings are NUL-terminated internally, replacing `\u0000` with `U+FFFD` prevents silent string truncation. Both are valid JSON, so `valid` and `valid_object` accept them, and every field reader (`find_string_field`, `string_field`, the object cursor, `from_json`) decodes them the same way.
- **Unrecognized escapes** (such as `\uZZZZ`) pass through unchanged.

---

## Validating Untrusted Input

Because `find_*_field` helpers are forgiving by design, validate untrusted payloads at system boundaries before extracting fields:

```hale,fragment
if !std::json::valid(body) {
    return Response { status: 400, body: "not JSON" };
}

if !std::json::valid_object(row) {
    return Response { status: 400, body: "expected flat record object" };
}
```

| Validator | Guarantees & Limits |
|---|---|
| `std::json::valid(doc)` | Exactly one well-formed JSON value (RFC 8259 numbers: rejects `01`, `1.`, `.5`, `1e`, `+1`; accepts `-0`, `1e5`, `-1.25e-3`), valid UTF-8, no trailing tokens (any `\uXXXX` escape is accepted, `\u0000` and lone surrogates included), maximum nesting depth of **64 levels**. Zero heap allocations (~10 ms/MB). |
| `std::json::valid_object(doc)` | All guarantees of `valid(doc)`, plus the top-level value must be an object with at most **64 top-level keys**, all keys must be **unique**, and no key may contain escape sequences (preventing `"\u0061"` vs `"a"` lookup mismatches). |

> [!TIP]
> For JSON maps with more than 64 top-level keys, validate syntax with `std::json::valid(doc)` and iterate keys with `object_first` / `obj_key_eq`. For high-throughput binary formats, see [Zero-copy bus](../systems/zero-copy-bus.md) and [Performance](../systems/performance.md).

Next: [HTTP](./http.md).
