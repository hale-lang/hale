# Operator precedence and associativity

Operators are listed from **highest** precedence (binds tightest)
to **lowest** (binds loosest). Operators on the same row have the
same precedence; their associativity is given in the right column.

| Level | Operators | Associativity | Notes |
|-------|-----------|---------------|-------|
| 14 | `()` `[]` `.` `::` | left | Call, index, field access, path |
| 13 | unary `-` `!` `~` | right | Unary minus, logical not, bitwise not |
| 12 | `*` `/` `%` | left | Multiplicative |
| 11 | `+` `-` | left | Additive |
| 10 | `<<` `>>` | left | Bit shifts |
| 9 | `&` | left | Bitwise and |
| 8 | `^` | left | Bitwise xor |
| 7 | `\|` | left | Bitwise or |
| 6 | `<` `>` `<=` `>=` | non-assoc | Ordering comparison |
| 5 | `==` `!=` | non-assoc | Equality |
| 4 | `~~` | non-assoc | Approximate equality (closure context only) |
| 3 | `&&` | left | Logical and |
| 2 | `\|\|` | left | Logical or |
| 1 | `..` `..=` | non-assoc | Range (reserved; not yet permitted) |
| 0 | `=` `+=` `-=` `*=` `/=` `%=` `&=` `\|=` `^=` | right | Assignment |

## Notes

### Comparison and equality are non-associative

`a < b < c` is a parse error. Use `a < b && b < c`. This avoids
the chained-comparison surprises common in C and matches Rust's
choice.

### Approximate equality binds at level 4

The `~~` operator is permitted **only inside a `closure` block's
assertion clause**. Elsewhere it is a parse error. It is
non-associative; `a ~~ b ~~ c` is invalid.

A closure assertion has the form:

```
expression ~~ expression within expression
```

The `within` clause is part of the assertion syntax, not an
operator at this precedence level. See `grammar.ebnf` for the
production.

### Generic arguments shadow comparison

The lexer emits the same tokens for `<` `>` regardless of context.
The parser disambiguates: when a `<` follows an identifier in a
type-expression context (after `:`, `->`, in a generic-args
position), it begins a generic-args list. Otherwise it is a
comparison.

This is the same approach Rust takes (with the famous turbofish
disambiguator `::<>` reserved for ambiguous expression contexts).
v0 does not provide turbofish; we'll add it if needed.

### Member access vs. path

- `.` accesses a field or method of a runtime value:
  `foo.bar`, `book.bid_side()`.
- `::` accesses a name through a path: module, type, perspective:
  `messages::Book`, `Strategy::default()`.

### Recovery primitives are statements, not operators

`restart`, `quarantine`, `bubble`, etc. are statement-level
keywords; they do not participate in the precedence table. They
appear in the form:

```
restart(child);
quarantine(child) for 30s;
bubble(err);
```

### Lifecycle and locus member declarations are not expressions

`birth`, `accept`, `run`, `drain`, `dissolve`, `on_failure`,
`mode`, `closure`, `contract`, `params`, `bus` introduce
declaration-level constructs inside a locus body and never
appear in expression position.

## Examples

```
a + b * c                  // a + (b * c) -- level 12 binds tighter than 11
a == b && c == d           // (a == b) && (c == d) -- level 5 over level 3
a < b == c                 // parse error: < is non-assoc with ==
-a.field                   // -(a.field) -- access binds tighter than unary minus
foo<T>(x)                  // generic instantiation, not (foo < T) > (x)
```

## Reserved precedence levels

Future operators expected to land at specific levels:

- `..` / `..=` (range) — level 1, non-assoc
- `?` (try / propagation) — level 13, postfix
- `??` (nil-coalesce) — level 2, right-assoc

These are reserved tokens; using them in v0 is a parse error.
