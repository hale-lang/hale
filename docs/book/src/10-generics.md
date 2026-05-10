# Generics

Aperio's generics let a single declaration — a `type`, a `locus`,
or a `fn` — work over a family of types. The compiler emits one
machine-code instance per concrete instantiation
(*monomorphization*), per the **F.1** commitment to runtime
performance over compile-time performance: generics impose no
runtime cost, but compile times grow with the generic surface.

This chapter covers what generic surface v0 ships, the `Numeric`
bound, the built-in `Result<T, E>` and `Option<T>`, and the
practical effect of monomorphization on how you write generic
code in Aperio today.

## Declaring generic parameters

Generic parameters appear in angle brackets after a declaration's
name:

```aperio
type Stack<T> {
    items: [T; 16];
    top: Int;
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

`T`, `E`, `U` and other single-letter type parameters are
conventional but not required — any PascalCase identifier in the
generic position binds that name as a type variable for the
declaration's body.

A type parameter is in scope inside the declaration's body for
field types, parameter types, return types, and bus payload
types. It is bound to a concrete type only at *use sites*, when
the declaration is referenced with concrete generic arguments.

## Constraints: the `Numeric` bound

The constraint syntax is `<T: Constraint>`. v0 admits two kinds
of constraint:

- **`Numeric`** — accepts `Int`, `Float`, `Decimal`, `Duration`.
  The compiler permits the conventional arithmetic operators
  (`+ - * /`), comparisons (`< > <= >= == !=`), and any
  Numeric-typed accumulator (`sum`, `count`, `mean`) inside the
  generic body.
- **`ProjectionClass`** — covered in
  [chapter 11](./11-perspectives.md).

```aperio
fn sum2<T: Numeric>(a: T, b: T) -> T {
    return a + b;
}

locus Tracker<T: Numeric> {
    params {
        running: T;
    }

    fn add(x: T) {
        self.running = self.running + x;
    }
}
```

Trying to instantiate `Tracker<String>` is a compile-time error —
`String` is not `Numeric`. The diagnostic names both the bound
and the offending type.

There is no general trait system in v0. `<T: SomeStruct>` and
`<T: SomeUserType>` are not permitted; concrete types as
constraints (`<T: Int>`) are also not permitted (use the type
directly). The two named constraints above are the v0 surface;
additional constraints may land if a workload demands them.

## `Result<T, E>` and `Option<T>`

Two generic enums are built into the language and do not require
a `type` declaration:

```aperio
// type Result<T, E> = enum { Ok(T), Err(E) };  -- built-in
// type Option<T>    = enum { Some(T), None };  -- built-in
```

You can use them directly in field types, parameter types, and
return types:

```aperio
type Holder {
    r: Result<Int, String>;
    o: Option<Int>;
}

fn divide(a: Int, b: Int) -> Result<Int, String> {
    if b == 0 {
        return Result_Int_String::Err("divide by zero");
    }
    return Result_Int_String::Ok(a / b);
}
```

A few details to flag:

### Construction uses the mangled monomorph name

The variant is constructed and matched not via the *template*
name but via its **mangled monomorph name**. For
`Result<Int, String>`, that name is `Result_Int_String`; the
`Ok` variant is constructed as `Result_Int_String::Ok(42)`.

```aperio
let r = Result_Int_String::Ok(42);
match r {
    Result_Int_String::Ok(n)  -> println("ok: ", n),
    Result_Int_String::Err(m) -> println("err: ", m),
}
```

This is how the codegen path keeps each instantiation a
distinct nominal type — `Result<Int, String>` and
`Result<Bool, String>` produce two separate enum types,
`Result_Int_String` and `Result_Bool_String`, with no
silent confusion between them. It is a v0 ergonomic that may
soften (the typechecker may eventually accept the template
name `Result::Ok` and resolve to the right monomorph by
context) — for now, write the mangled name explicitly.

### `Option<T>` follows the same shape

```aperio
let some_n = Option_Int::Some(7);
let none   = Option_Int::None;

match some_n {
    Option_Int::Some(n) -> println("some: ", n),
    Option_Int::None    -> println("none"),
}
```

### Shadowing is permitted but rarely needed

If you declare your own `type Result<T, E> = enum { ... }`, the
codegen distinguishes your monomorphs from the built-ins by the
mangled name — a user-declared `Result` with a different variant
set produces `Result_Int_String` from your declaration, not the
stdlib's. Shadowing is permitted but typically not what you
want; the built-ins are there because every Aperio program
benefits from a single shared shape for fallible and optional
values.

## User-declared generic types

Beyond the built-ins, declare your own generics with the same
syntax:

```aperio
type Stack<T> {
    items: [T; 16];
    top: Int;
}

type Pair<A, B> {
    first: A;
    second: B;
}
```

Construction and field access work as you'd expect. Each
concrete instantiation (`Stack<Int>`, `Pair<String, Decimal>`)
becomes a distinct nominal type at codegen, with its own struct
layout and its own machine-code lowerings of any methods.

> **v0 boundary.** Generic locus parameters with default values
> are partially supported — a generic param's default must
> resolve to a value the compiler can construct without
> additional type inference. Some defaulted-generic-param
> patterns may produce diagnostics; explicit construction at
> the call site is the workaround.

## Monomorphization in practice

Each concrete instantiation produces its own machine-code
instance:

- `sum2<Int>(1, 2)` and `sum2<Decimal>(1.0d, 2.0d)` compile to
  two separate functions in the emitted binary.
- `Result<Int, String>` and `Result<Bool, String>` produce two
  separate enum types with different storage layouts.
- `Tracker<Int>` and `Tracker<Decimal>` produce two separate
  locus types with different `running`-field widths.

The runtime cost is *zero* per use site: there is no virtual
dispatch, no boxing, no runtime type tag for the generic — every
call resolves to a direct call into the right specialization
the compiler picked at the use site. The build-time cost is
proportional to how many instantiations the program produces;
in practice, this is bounded by the program's actual variety,
not by the generic surface.

This is the **F.1** trade-off written into the type system:
*runtime perf over compile-time perf, behavior preserved.* If a
program defines a generic but never uses it on more than one
type, the compiler emits exactly one specialization. If it uses
it on a dozen types, twelve specializations land in the binary.

## What this chapter does not cover

- **`fn(T) -> U` as a parameter type** — function values
  passed as arguments — appears in
  [chapter 9 of stdlib](../../std/book/index.html) once
  higher-order helpers like `map` / `fold` ship.
- **Projection-class constraints** (`<T: ProjectionClass>`,
  `<T: Rich>`) — see [chapter 11](./11-perspectives.md).
- **Generic closures** — closures whose accumulator type is
  itself generic over a `Numeric` bound. They follow naturally
  from chapter 7 + this chapter; the m64 milestone wired them
  end-to-end.

The next chapter, **[Perspectives](./11-perspectives.md)**,
introduces the substrate's *other* generic surface —
projection classes (`Rich<T>` / `Chunked<T>` /
`Recognition<T>`) and the `ProjectionClass` constraint per
**F.2**, which lets the same coordination code dispatch
through three different representations of the same value.
