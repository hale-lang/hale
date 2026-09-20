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
semantics, use [`string_field`](#reading-a-field-that-may-be-null)
below, which names the shape it found.
`find_field_raw` returns the raw substring for a field, which is
how you reach into a nested object:

```hale,fragment
let inner = std::json::find_field_raw(doc, "address");
let city  = std::json::find_string_field(inner, "city");
```

The lookup matches the name **as a top-level key only** — a string
*value* elsewhere in the document that happens to repeat the key's
text can't shadow it, and a key that only exists deeper down isn't
found until you chain into its parent (as above). When you iterate
an object whose keys you *don't* know — a `dependencies` map, a
registry's `versions` — read each key with
`std::json::obj_key_string(it, doc)`: it decodes escapes the same
way `obj_value_string` does, which hand-slicing
`doc[it.key_start..it.key_end]` silently skips.

## Reading a field that may be null

`find_string_field` hands back a `String` whatever the value was,
which is convenient until the field is optional. `null` arrives as
the four-character string `"null"` — a perfectly good name — and
absent, empty, and a number all arrive as something a name-shaped
field will happily accept:

```hale,fragment
let a = std::json::find_string_field("{\"owner\": null}", "owner");     // "null"  (!)
let b = std::json::find_string_field("{\"owner\": \"null\"}", "owner"); // "null"
let c = std::json::find_string_field("{\"owner\": \"\"}", "owner");     // ""
let d = std::json::find_string_field("{}", "owner");                    // ""
let e = std::json::find_string_field("{\"owner\": 7}", "owner");        // "7"
```

`string_field` answers with the shape as well as the text, so you
decide instead of guessing:

```hale,fragment
let owner = std::json::string_field(body, "owner");
if owner.kind == "string" {
    println("owned by ", owner.text);
} else if owner.kind == "null" || owner.kind == "missing" {
    println("a root");                  // parentless — not named `null`
} else {
    println("owner must be a string or null");
}
```

It returns a `std::json::JsonString`, which is two fields:

- `kind` — one of `"string"`, `"null"`, `"missing"`, `"number"`,
  `"bool"`, `"array"`, `"object"`, `"invalid"`.
- `text` — the decoded string content (quotes stripped, escapes
  resolved) **only** when `kind` is `"string"`. Every other kind
  carries `""`, so forgetting to check `kind` gives you an empty
  name rather than a plausible wrong one.

`"missing"` means the document is an object with no such member.
`"invalid"` means the document is not an object at all, or the
member's value is not something the scanner can name (an
unterminated string, a bare `NaN`). A key repeated inside one
object resolves to the first one — the same member
`find_string_field` reads.

The kind is not string-specific. An `Int` or `Bool` read has the
same coercion problem (`null` becomes `0`, anything that isn't
`true` becomes `false`), and the same gate fixes it:

```hale,fragment
if std::json::string_field(body, "port").kind == "number" {
    let port = std::json::find_int_field(body, "port");
    println(port);
}
```

`find_string_field` is unchanged and stays permissive — existing
callers keep the behaviour they have.

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

`result()` hands back a copy, so it is a snapshot rather than a
window: keep building after it and the `String` you already took
does not change.

The Builder and both helpers accumulate into one growing byte
buffer, so the cost of a document is one pass over its own
bytes. You can assemble a multi-megabyte payload a field at a
time without watching memory climb.

## Escapes

`\uHHHH` decodes to the character it names, so an escaped
literal and a plain one are the same value:

```hale,fragment
let a = std::json::unescape_string("hale.v\\u0031");
println(a == "hale.v1");        // true
println(std::json::unescape_string("\\u00e9"));        // é
println(std::json::unescape_string("\\u4e2d"));        // 中
println(std::json::unescape_string("\\ud83d\\ude00")); // 😀
```

The last one is a *surrogate pair*: JSON has no way to write a
character above U+FFFF in one escape, so it writes two, and
`unescape_string` puts them back together.

Two inputs have no character to decode to, and both become `�`
(U+FFFD, the replacement character):

- **An unpaired surrogate** — half of a pair, with nothing to
  join it. It is not a character on its own.
- **`\u0000`** — a Hale `String` ends at its first zero byte, so
  a real NUL would silently cut the value short and let a prefix
  compare equal to the whole. A visible `�` is the safer answer.

Anything else that isn't a real escape passes through as it was
written — `\uZZZZ` stays `\uZZZZ` — on the principle that a
malformed byte somewhere in a document shouldn't cost you the
rest of it.

## Validating

The readers above are forgiving on purpose: `find_string_field`
answers with what it finds and doesn't mind a missing brace. When
the bytes come from somewhere you don't control — a request body,
a file on disk, a line of somebody's journal — ask first:

```hale,fragment
if !std::json::valid(body) {
    return Response { status: 400, body: "not JSON" };
}
```

`std::json::valid` is true for exactly one well-formed JSON
value, and nothing after it. Whitespace around the value is fine;
a second value, a stray brace or trailing text is not, and the
empty string is not a document. The number rules are JSON's, which
are stricter than most languages': `01`, `1.`, `.5`, `1e` and
`+1` are all refused, while `-0`, `1e5` and `-1.25e-3` are fine.

Most gates want a bit more than "is it JSON":

```hale,fragment
// A record: an object, with each field named once.
if !std::json::valid_object(row) { return false; }
```

`valid_object` is `valid` plus "the top level is an object whose
keys are unique and unescaped". Both extras are about
*agreement*, not taste. JSON's grammar permits
`{"a":1,"a":2}` — last-one-wins is a convention, not a rule — so
two readers of those bytes can disagree about the record. And
`{"\u0061":1}` spells the key `a`, but `find_int_field(row, "a")`
looks for the literal text `"a"` and won't find it: the field
would be present and unfindable at once. Refusing both up front
means everything downstream can trust what it reads.

Three inputs are refused even though a pedantic reading of the
spec allows them, and they are the same three the escape rules
above have no character for: text that isn't valid UTF-8, a lone
surrogate escape, and `\u0000`. A validator's job is admission,
so it says no rather than handing on a `�`.

Two limits are worth knowing, because both answer `false` rather
than working harder:

- **Nesting stops at 64 levels.** Deeper input is refused, not
  scanned — a document made of ten thousand `[` costs you a
  constant amount of work.
- **`valid_object` checks uniqueness over at most 64 top-level
  fields.** Past that it answers `false`. That bound is what lets
  it check keys without building anything: it compares them where
  they sit in your string. Plain `valid` has no such limit, and
  nested objects are only checked for syntax, so a map of a
  thousand keys is still checkable as JSON — walk its keys with
  `object_first` / `obj_key_eq` if you need them unique too.

Validating is a pass over the bytes and allocates nothing at all
— no substring, no set of keys, nothing left in the arena
afterwards — about 10 ms per megabyte. It is cheap enough to run
on every inbound message rather than hoping.

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
