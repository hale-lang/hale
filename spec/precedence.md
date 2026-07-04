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
| 1 | `..` `..=` | non-assoc | Range (parsed everywhere; *use* restricted to `for x in lo..hi` at typecheck) |
| 1 | `or` | right | Fallible disposition (v1.x-FORM-1; contextual). RHS is `raise` / `discard` / `fail <expr>` / another expression. |
| 0 | `=` `+=` `-=` `*=` `/=` `%=` `&=` `\|=` `^=` | right | Assignment |
| -1 | `<-` | non-assoc | Bus send (statement-shape only) |

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

### Fallible disposition (`or`) binds looser than everything except assignment

The `or` keyword is the fallible-disposition postfix (v1.x-
FORM-1). It is **contextual** — recognized only when it
appears as a postfix on an expression that the typechecker
has marked `Ty::Fallible`. Outside that position, `or` is an
ordinary identifier (so `let or = 5;` stays admissible).

It is **right-associative** so a chain reduces step by step:

```
a() or b() or raise
// parses as: a() or (b() or raise)
```

The RHS of `or` is one of:
- the contextual keyword `raise` (diverges via closure
  violation routing), or
- `discard` (swallow the error, substitute Unit; rejected at
  typecheck when the success type is non-Unit), or
- `fail <expr>` (diverge like `raise`, but with a fresh payload
  of the enclosing fallible fn's declared error type), or
- any expression of the success type (the substitute path;
  `err` is implicitly bound to the payload in this scope).

### Bus send is a statement, not an expression

`<-` does not nest in expressions. It appears only at statement
position:

```
"subject" <- value;
```

The left side is a string-literal subject (or an identifier
bound to a publish handle in a future revision); the right side
is any expression. The construct produces no value — there is no
`x = ("s" <- v)`. The level −1 row in the table is bookkeeping:
`<-` binds looser than every expression operator, so the parser
recognizes the leading expression in full before deciding the
statement is a send.

### Recovery primitives are statements, not operators

`restart`, `quarantine`, `bubble`, `violate`, etc. are
statement-level keywords; they do not participate in the
precedence table. They appear in the form:

```
restart(child);
quarantine(child) for 30s;
bubble(err);
violate fatal_io;             // F.27, v1.x-VIOLATE
violate fatal_io with detail; // optional payload
```

`violate` is divergent (same type-level shape as `fail` /
`bubble`); the typechecker treats it as `Never`. Its closure-
name argument is a bare identifier (not a parenthesized
argument list), and the optional `with <expr>` trailer carries
a user-shaped payload onto the synthesized `ClosureViolation`.

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

## Range and reserved precedence levels

- `..` / `..=` (range) — level 1, non-assoc. These **parse in any
  expression position**; v1 restricts their *use* to `for x in
  lo..hi`, so a range elsewhere is a typecheck error, not a parse
  error. `..` is exclusive, `..=` inclusive.

Future operators expected to land at specific levels:

- `??` (nil-coalesce) — level 2, right-assoc

`??` is a reserved token; using it in v0 is a parse error.

Note: `?` (try-propagation) was previously reserved at level
13 but has been **cut** from the roadmap. The fallible-
disposition operator `or` covers the same use case at the
expression level without re-introducing a parallel
upward-propagation mechanism at the value layer. See
`spec/design-rationale.md`.
