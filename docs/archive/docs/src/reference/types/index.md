# Types — overview

## Synopsis

Aperio's type system is invariant (no implicit subtyping),
nominal (types are identified by name, not structure), and
monomorphizing (generics resolve to per-instantiation
specializations at compile time). The compiler verifies every
expression's type at compile time; runtime type checks are
limited to deserialization and recovery dispatch.

## Type categories

| Category | Description | See |
|---|---|---|
| **Primitives** | `Int`, `Float`, `Decimal`, `Bool`, `String`, `Time`, `Duration`, `Bytes` | [primitives](./primitives.md) |
| **Compound** | Tuples, fixed-size arrays | [primitives](./primitives.md) |
| **Records** | User-defined `type T { ... }` | [structs-and-enums](./structs-and-enums.md) |
| **Enums** | `type T = enum { ... }`, plus built-in `Result<T,E>` and `Option<T>` | [structs-and-enums](./structs-and-enums.md) |
| **Loci** | Each `locus L { ... }` declaration introduces a locus type `L` | [loci](../loci/index.md) |
| **Perspectives** | `perspective P { ... }` declarations | [perspectives](./perspectives.md) |

Generics parameterize records, enums, loci, and functions.
See [generics](./generics.md).

## Type compatibility

### Invariance

Aperio types are *invariant*. `Result<Int, String>` is not a
subtype of `Result<Int, Object>`; there is no implicit
conversion. Where conversions are needed, they are explicit.

### Contract compatibility

Per **F.8**, when a parent declares `consume X: T` and a child
declares `expose X: T`, the typechecker verifies the child's
exposed type is the same as the parent's consumed type. (For
v0, "same as" is type equality.)

This is the typing-rule expression of vertical-only-flow at the
contract level.

## Mutability

Per **F.E**, bindings are immutable by default:

```aperio
let x = 0;        // immutable; x = 1 is a compile error
let mut y = 0;    // mutable; y = 1 is permitted
```

Mutability is a per-binding property, not a per-type property.
There is no `Mut<T>` wrapper. Locus parameter fields are
implicitly mutable through `self.x = ...` (per F.3 — a locus's
parameter struct is its mutable state bundle).

## See Also

- [Primitives](./primitives.md)
- [Structs and enums](./structs-and-enums.md)
- [Generics](./generics.md)
- [Perspectives and projection classes](./perspectives.md)
- [Locus declarations](../loci/index.md)
