# Primitive types

## Synopsis

Eight primitive types plus two compound types (tuples and
fixed-size arrays). All primitive type names are PascalCase
identifiers recognized in type position only; they are not
reserved words.

## Numeric primitives

| Type | Width | Signed | Default literal | Example |
|---|---|---|---|---|
| `Int` | 8 bytes | yes | yes (integer literals) | `42`, `-7`, `1_000_000` |
| `Uint` | 8 bytes | no | suffix `u64` | `100u64` |
| `Float` | 8 bytes | n/a (IEEE 754) | yes (float literals) | `3.14`, `2.5e10` |
| `Decimal` | 16 bytes | yes (fixed-precision) | suffix `d` | `1.50d`, `100.40d` |

`Int` is the default type for unsuffixed integer literals.
`Float` is the default for unsuffixed literals with a decimal
point. `Decimal` requires the explicit `d` suffix to
disambiguate from `Float`.

No implicit conversion between numeric types. Use explicit
conversion functions where needed.

## Other primitives

| Type | Description | Width / shape |
|---|---|---|
| `Bool` | Boolean | 1 byte |
| `String` | UTF-8 byte sequence | 16 bytes (ptr + len), arena-resident |
| `Time` | Monotonic instant | 8 bytes |
| `Duration` | Time interval | 8 bytes |
| `Bytes` | Raw byte buffer | 16 bytes (ptr + len) |

### `String`

UTF-8 bytes stored in an arena. Literals are interned in a
static region; runtime concatenations / slicings land in the
caller's current arena.

```aperio
let s = "hello";
let n = len(s);             // byte length, returns Int
let g = "hi, " + name;      // concat; result in current arena
let eq = (s == "hello");    // byte-wise equality
let head = s[0..5];         // exclusive slice
let body = s[7..=11];       // inclusive slice
```

Slicing bounds are *clamped*, not panicking — out-of-range
indices produce a (possibly empty) substring.

### `Time` and `Duration`

```aperio
let t: Time = `2026-01-01T00:00:00Z`;
let d: Duration = 5s;
let later: Time = t + d;
let elapsed: Duration = time::monotonic() - t;
```

Time literals are backtick-delimited ISO 8601 strings;
duration literals use the `ns/us/ms/s/m/h` suffix (e.g.
`100ms`, `5s`, `1h`). `time::sleep(d)` and
`time::monotonic()` are stdlib helpers.

## Tuples

```text
tuple-type    ::= "(" type ("," type)+ ")"
tuple-literal ::= "(" expr ("," expr)+ ")"
```

```aperio
fn divmod(a: Int, b: Int) -> (Int, Int) {
    return (a / b, a % b);
}

let result = divmod(17, 5);
println(result.0, " ", result.1);

let (q, r) = divmod(23, 4);   // flat destructure
```

Tuples must have at least two elements (no unit `()`). Field
access is by 0-based numeric index (`.0`, `.1`, ...). Flat
destructure is supported in `let` and `match`; nested tuple
sub-patterns are not supported in v0.

## Fixed-size arrays

```text
array-type    ::= "[" type ";" integer-literal "]"
array-literal ::= "[" expr ("," expr)* "]"
```

```aperio
let nums: [Int; 5] = [10, 20, 30, 40, 50];
let xs = [1, 2, 3, 4];          // type [Int; 4] inferred
println(nums[0]);                // index access
for x in nums { println(x); }    // for-iteration
for i in 0..5 { println(nums[i]); }
```

The size `N` must be a compile-time integer literal. Element
type is inferred from the literal's first element. No
growable / dynamic arrays in v0; the substrate's region
allocator is wholesale-free, not per-object free.

Array storage lives in the enclosing arena. Index access is
bounds-checked; out-of-bounds reads emit a typed runtime
error (panic — distinct from `ClosureViolation`).

## See Also

- [Lexical structure — numeric literals](../lexical.md#numeric-literals)
- [Structs and enums](./structs-and-enums.md)
- [Memory model](../memory.md)
