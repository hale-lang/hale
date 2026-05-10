# Expressions

## Synopsis

Expressions produce values. Aperio expressions are
conventional — literals, identifier references, operators,
calls, field/index access, struct/enum/tuple/array
construction, `match`, `if`/`else` as expression — with two
non-conventional forms reserved for substrate use:

- `~~` (approximate equality, closure-only)
- `<-` (bus send, statement-only — not a true expression)

## Categories

| Category | Forms |
|---|---|
| Literals | integer, float, decimal, string, time, duration, bool |
| Names | identifiers, paths (`Module::name`, `Type::variant`), `self.field`, `self.method()` |
| Construction | `Type { field: value, ... }`, `Type::Variant`, `Type::Variant(args)`, tuple `(a, b)`, array `[1, 2, 3]` |
| Access | `value.field`, `value.0`, `value[index]`, `value[lo..hi]`, `value[lo..=hi]` |
| Arithmetic | `+ - * / %` (Numeric only) |
| Comparison | `< > <= >= == !=` (non-associative) |
| Logical | `&& || !` (short-circuiting) |
| Bitwise | `& | ^ ~ << >>` (integer only) |
| Range | `..` (exclusive), `..=` (inclusive) |
| Match | `match scrutinee { pattern -> arm-expr, ... }` |
| Conditional | `if cond { ... } else { ... }` (as expression or statement) |
| Call | `function(args)`, `value.method(args)` |

## Operator precedence

See `spec/precedence.md` in the source tree for the full
table. Headlines:

- **Comparison and equality are non-associative** — `a < b < c`
  is a parse error. Use `a < b && b < c`.
- **Generic arguments shadow comparison** — `<` after an
  identifier in type-expression context begins a generic-args
  list, not a comparison. The parser disambiguates by context.
- **Bus send is a statement, not an expression** — `<-` does
  not nest in expressions; it appears only at statement
  position.

## `match` as expression

```aperio
let label = match x {
    0     -> "zero",
    1..=9 -> "single digit",
    _     -> "other",
};
```

Each arm produces a value of the same type. Patterns include
literal, wildcard `_`, binding, range, tuple, struct
destructure, and enum constructor. Match exhaustiveness is
checked per **F.18**.

## `if` as expression

```aperio
let kind = if x > 0 { "positive" } else { "non-positive" };
```

When used as an expression, both branches must produce values
of the same type. When used as a statement (no value
required), either branch may be absent.

## Method calls and path access

- `value.method(args)` — runtime method dispatch through the
  value's known type.
- `Module::name` / `Type::variant` — path access through a
  namespace or to a constant / variant.

`.` and `::` bind tighter than every other operator (precedence
level 14, left-associative).

## See Also

- [Statements](../statements/index.md)
- [Types — overview](../types/index.md)
- [Closures (the `~~` operator)](../closures/index.md)
- [Bus dispatch (the `<-` operator)](../bus/index.md)
