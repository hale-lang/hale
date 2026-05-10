# Monomorphization

## Synopsis

Per **F.1**, the compiler emits one machine-code instance per
concrete generic instantiation. The runtime cost of a generic
use site is zero — no virtual dispatch, no boxing, no runtime
type tag — at the cost of larger build artifacts proportional
to the program's actual variety of instantiations.

This page covers the mechanics. The source-level surface lives
in [types/generics](../types/generics.md).

## Process

For each generic declaration, the compiler:

1. **Walks use sites.** Discovery finds every concrete
   instantiation across the program (bounded by the program's
   actual variety, not the generic surface declared).
2. **Mangles names.** For each instantiation, a unique
   monomorphized name is constructed by appending the concrete
   types: `Result<Int, String>` becomes `Result_Int_String`;
   `Stack<Decimal>` becomes `Stack_Decimal`.
3. **Synthesizes the specialization.** Each monomorph emits as
   its own type / fn in the IR, with concrete types
   substituted everywhere the generic parameter appears.
4. **Replaces use sites.** Every reference to the generic at
   a concrete instantiation rewrites to the mangled name.

## Example

Source:

```aperio
fn first<T>(x: T) -> T {
    return x;
}

fn main() {
    let a = first(42);          // T = Int
    let b = first("hello");     // T = String
}
```

Effective post-monomorphization:

```aperio
fn first_Int(x: Int) -> Int {
    return x;
}

fn first_String(x: String) -> String {
    return x;
}

fn main() {
    let a = first_Int(42);
    let b = first_String("hello");
}
```

The compiler generates the two specializations; the call sites
dispatch directly to the correct one.

## Generic enums and the mangled-name surface

For generic enums, the mangled name is *user-facing* in v0:
construction and pattern matching use `Result_Int_String::Ok`,
not `Result::Ok`. This is the v0 ergonomic. A future
typechecker pass may resolve template names to monomorphs by
context.

```aperio
type Result<T, E> = enum { Ok(T), Err(E) };  // template

let r = Result_Int_String::Ok(42);            // construct via mangled name
match r {
    Result_Int_String::Ok(n)  -> println("ok: ", n),
    Result_Int_String::Err(m) -> println("err: ", m),
}
```

## Built-in generic enums

`Result<T, E>` and `Option<T>` are built into the codegen.
The compiler injects the templates without requiring source
declarations; discovery still walks use sites and synthesizes
the necessary monomorphs.

```aperio
type Holder {
    r: Result<Int, String>;     // discovery sees Result<Int, String>
    o: Option<Int>;              // discovery sees Option<Int>
}
```

Synthesis produces `Result_Int_String` and `Option_Int`; their
constructors and match arms work as for user-declared generics.

## User-shadowing built-ins

If the user declares `type Result<T, E> = ...` with a
different variant set, the codegen distinguishes their
monomorphs from the built-ins by the mangled name. Shadowing
is permitted but rarely useful.

## Build-time cost

Generic surface itself imposes no compile cost. Each
*concrete instantiation* adds one specialization to the
emitted IR. A program that uses `Result<Int, String>` in 100
places produces one specialization; a program that uses
`Result` with 10 different `<T, E>` pairs produces 10.

## Runtime cost

Zero per use site. Every call resolves to a direct function
call into the right specialization the compiler picked at the
use site. There is no dispatch table, no type erasure, no
boxing.

## See Also

- [Types — generics](../types/generics.md)
- [Structs and enums](../types/structs-and-enums.md)
- [Perspectives and projection classes](../types/perspectives.md)
