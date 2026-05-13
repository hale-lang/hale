# Generics

## Synopsis

Generic parameters let a single declaration — `type`, `enum`,
`fn`, `locus` — work over a family of types. The compiler
emits one machine-code instance per concrete instantiation
(*monomorphization*), per the **F.1** commitment to runtime
performance over compile-time performance.

## Grammar

```text
generic-params ::= "<" generic-param ("," generic-param)* ">"
generic-param  ::= PascalCase-Ident (":" constraint)?
constraint     ::= "Numeric" | "ProjectionClass" | projection-class
projection-class ::= "Rich" | "Chunked" | "Recognition"

generic-args   ::= "<" type-expr ("," type-expr)* ">"
```

## Declaring generic parameters

```aperio
type Stack<T> {
    items: [T; 16];
    top: Int;
}

type Pair<A, B> {
    first: A;
    second: B;
}

fn first<T>(x: T) -> T {
    return x;
}

locus Compute<T: Numeric> {
    params {
        value: T;
    }
}
```

Type parameter names are PascalCase identifiers; single letters
(`T`, `E`, `U`) are conventional. A type parameter is in scope
inside the declaration's body for field types, parameter types,
return types, and bus payload types.

## Constraints

The constraint syntax is `<T: Constraint>`. v0 admits two
constraints:

### `Numeric`

Accepts `Int`, `Float`, `Decimal`, `Duration`. Inside a body
constrained `<T: Numeric>`, the compiler permits the
conventional arithmetic operators (`+ - * /`), comparisons
(`< > <= >= == !=`), and Numeric-typed accumulators (`sum`,
`count`, `mean`).

```aperio
fn sum2<T: Numeric>(a: T, b: T) -> T {
    return a + b;
}
```

Instantiating `Tracker<String>` (where `String` is not
`Numeric`) is a compile-time error.

### `ProjectionClass`

Per **F.2**, a built-in any-of-three constraint: `T` resolves
to one of `Rich`, `Chunked`, or `Recognition` at each call
site. See [perspectives](./perspectives.md).

```aperio
fn process<P: ProjectionClass, T>(input: P<T>) -> P<T> {
    // ...
}
```

### Other constraints

v0 does not admit:

- `<T: SomeStruct>` — no trait system in v0.
- `<T: Int>` — concrete types as constraints (use the type
  directly).

Future versions may add traits.

## Built-in generic enums

Two generic enums are built into the language and do not
require a `type` declaration:

```text
Result<T, E> = enum { Ok(T), Err(E) };
Option<T>    = enum { Some(T), None };
```

Use directly in field types, parameter types, return types:

```aperio
fn divide(a: Int, b: Int) -> Result<Int, String> {
    if b == 0 {
        return Result_Int_String::Err("divide by zero");
    }
    return Result_Int_String::Ok(a / b);
}
```

### Mangled monomorph names

Generic enum variants are constructed and matched via the
*mangled monomorph name*, not the template name. For
`Result<Int, String>`, the mangled name is
`Result_Int_String`:

```aperio
let r = Result_Int_String::Ok(42);
match r {
    Result_Int_String::Ok(n)  -> println("ok: ", n),
    Result_Int_String::Err(m) -> println("err: ", m),
}
```

This is a v0 ergonomic; future typechecker work may resolve
template names to monomorphs by context.

## Monomorphization

Per **F.1**, the compiler emits one machine-code instance per
concrete generic instantiation. `sum2<Int>(1, 2)` and
`sum2<Decimal>(1.0d, 2.0d)` compile to two separate functions
in the emitted binary; `Result<Int, String>` and
`Result<Bool, String>` produce two separate enum types with
different storage layouts.

Runtime cost per use site: zero. No virtual dispatch, no
boxing, no runtime type tag for the generic.

Build-time cost: proportional to the program's actual variety
of instantiations, not to the generic surface declared.

## Type inference

Type parameters are inferred from arguments where possible:

```aperio
let r = sum2(1, 2);          // T inferred as Int
let s = sum2(1.0d, 2.0d);    // T inferred as Decimal
```

For declarations whose type parameters cannot be inferred,
explicit annotation is required at the use site.

## See Also

- [Structs and enums](./structs-and-enums.md)
- [Perspectives and projection classes](./perspectives.md)
- [Monomorphization (mechanics)](../generics/index.md)
