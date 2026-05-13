# Structs and enums

## Synopsis

User-defined record types (structs) and tagged-union types
(enums) are introduced with the `type` keyword. Structs are
ordered records with named fields; enums are sums of named
variants, each optionally carrying positional payload fields.

## Grammar

```text
type-decl ::= struct-decl | enum-decl
struct-decl ::= "type" PascalCase-Ident generic-params? "{"
                  field-decl (";" field-decl)* ";"?
                "}"
enum-decl ::= "type" PascalCase-Ident generic-params? "=" "enum" "{"
                  variant-decl ("," variant-decl)* ","?
              "}"

field-decl   ::= snake_case-Ident ":" type-expr
variant-decl ::= PascalCase-Ident ("(" type-expr ("," type-expr)* ")")?
```

## Structs

A struct declaration introduces a named record:

```aperio
type Point {
    x: Int;
    y: Int;
}

type Greeting {
    text: String;
    sender: String;
    priority: Int;
}
```

Construct with `T { field: value, ... }`. All declared fields
must be supplied; field order in the literal does not have to
match the declaration:

```aperio
let p = Point { x: 3, y: 4 };
let q = Point { y: 8, x: p.x + 10 };
```

Field access is `.name`:

```aperio
println(p.x, " ", p.y);
let r = Point { x: p.x * 2, y: p.y * 2 };
```

Structs are passed and stored by *value*. Assigning a struct
to a let-binding copies it (subject to arena rules — see
[memory model](../memory.md) for the per-locus arena
semantics).

## Enums

An enum is a tagged union — exactly one of several named
variants, each optionally carrying typed payload fields.

### No-payload variants

```aperio
type Light = enum { Red, Yellow, Green };
```

Construct: `Light::Red`. Match arms by variant name:

```aperio
fn next(l: Light) -> Light {
    match l {
        Light::Red    -> Light::Green,
        Light::Green  -> Light::Yellow,
        Light::Yellow -> Light::Red,
    }
}
```

No-payload enums are represented as a 32-bit tag at runtime;
plain value semantics, no allocation.

### Payload variants

Variants may carry positional payload fields:

```aperio
type Event = enum {
    Tick(Int),
    Trade(Decimal, Int),
    Halt,
};

type Result = enum {
    Ok(Int),
    Err(String),
};
```

Construct with positional arguments: `Event::Tick(7)`,
`Event::Trade(99.95d, 100)`, `Event::Halt`,
`Result::Ok(42)`, `Result::Err("oops")`.

Match destructures the payload into bindings:

```aperio
match e {
    Event::Tick(0)            -> println("tick zero"),
    Event::Tick(n)            -> println("tick #", n),
    Event::Trade(price, size) -> println("trade ", size, " @ ", price),
    Event::Halt               -> println("halt"),
}
```

Literal sub-patterns (`Event::Tick(0)`) match a specific
value before more general arms in the same match.

Payload enums are stored as a pointer to a `{ tag, body }`
struct allocated in the current arena; the body is sized to
the largest variant's payload.

## Exhaustiveness (F.18)

The typechecker enforces match exhaustiveness on enums. A
match that omits a variant is a compile-time error unless a
wildcard arm (`_ -> ...`) is present:

```aperio
match l {
    Light::Red   -> "stop",
    Light::Green -> "go",
    // Compile error: missing variant Light::Yellow
}

match l {
    Light::Red   -> "stop",
    Light::Green -> "go",
    _            -> "wait",   // ok
}
```

## Generics

Both struct and enum declarations may take generic parameters.
See [generics](./generics.md).

## Built-in generic enums

`Result<T, E>` and `Option<T>` are built into the language; no
declaration is needed. See [generics](./generics.md#built-in-generic-enums).

## Memory layout

Structs are flat C-style records, fields laid out in
declaration order, alignment per the field types. No padding
beyond what alignment requires.

Enums:
- No-payload: 4-byte tag (i32).
- Has-payload: pointer to `{ i32 tag, [N x i8] body }` where
  N is the size of the largest variant's payload. Allocation
  in the current arena.

## See Also

- [Primitives](./primitives.md)
- [Generics](./generics.md)
- [Memory model](../memory.md)
- [Glossary — locus](../glossary.md#locus)
