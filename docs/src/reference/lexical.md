# Lexical structure

## Synopsis

Aperio source is UTF-8 text. The lexer produces a stream of
tokens — keywords, identifiers, literals, operators, and
punctuation. Whitespace and comments are insignificant beyond
acting as token separators (the language does not use indentation
for structure).

## Comments

```text
// single-line comment to end-of-line
/* delimited block comment, not nestable */
```

Block comments do not nest in v0.

## Identifiers

```text
identifier ::= letter (letter | digit | "_")*
letter     ::= "A".."Z" | "a".."z" | "_"
digit      ::= "0".."9"
```

Conventions (writer's discipline, not enforced by the lexer):

- Locus types use `PascalCase` ending in `L` (`HelloL`,
  `CoordinatorL`).
- Plain `type` declarations use `PascalCase` (`Greeting`,
  `Observation`).
- Function names, parameter names, field names use
  `snake_case`.
- Enum variants use `PascalCase` (`Light::Red`).

## Keywords

Reserved words. Cannot appear as identifiers in any position.

```text
locus  type  enum  perspective  fn  let  mut
params  contract  expose  consume  bus  subscribe  publish  as  of
closure  epoch  birth  run  drain  dissolve  accept  on_failure
match  if  else  while  for  in  return  break  continue  yield
schedule  cooperative  pinned  core
import  true  false
```

The five lifecycle keywords (`birth`, `run`, `drain`,
`dissolve`, `accept`) are reserved in any position.

The four recovery primitives (`restart`, `restart_in_place`,
`quarantine`, `bubble`) are *not* keywords — they are
runtime-resolved built-in functions called from `on_failure`
bodies.

## Predefined type names

Per **F.15**, primitive type names are PascalCase identifiers
recognized in type position only. They are *not* reserved
words; they may appear as expression-position identifiers (path
prefixes etc.) without conflict.

```text
Int  Uint  Float  Decimal  Bool  String  Time  Duration  Bytes
```

## Numeric literals

### Integer

```text
integer ::= digit+ ("_" digit+)*
optional-suffix ::= "i32" | "i64" | "u32" | "u64"
```

Examples: `0`, `42`, `1_000_000`, `-7`, `100u64`. Default
type: `Int` (8 bytes, signed).

### Float

```text
float ::= integer "." integer ("e" integer)?
optional-suffix ::= "f32" | "f64"
```

Examples: `3.14`, `2.5e10`, `1.0`. Default type: `Float`
(8 bytes, IEEE 754 double).

### Decimal

```text
decimal ::= integer "." integer "d"
```

Examples: `1.50d`, `100.40d`, `0.001d`. The `d` suffix
disambiguates from `Float`. Aperio does not implicitly convert
between `Float` and `Decimal`. Decimal semantics match the
`shopspring/decimal` Go library.

## String literals

```text
string ::= "\"" character* "\""
escape ::= "\"" | "\\" | "n" | "t" | "r" | "0" | "x" hex hex | "u{" hex+ "}"
```

Strings are UTF-8 byte sequences at runtime. The literal
itself is interned in a static region; runtime concatenations
and slicings land in the current arena.

## Time and duration literals

```text
duration ::= integer ("ns" | "us" | "ms" | "s" | "m" | "h")
time     ::= "`" iso8601-string "`"
```

Examples:

```aperio
let d: Duration = 5s;
let f: Duration = 100ms;
let t: Time = `2026-01-01T00:00:00Z`;
```

Time literals use the backtick-delimited ISO 8601 form. The
runtime parses them at compile time; an invalid time literal
is a compile error.

## Operators and punctuation

See `spec/precedence.md` in the source tree for the full
precedence and associativity table. Token forms:

```text
arithmetic   ::= "+" | "-" | "*" | "/" | "%"
comparison   ::= "<" | ">" | "<=" | ">=" | "==" | "!="
logical      ::= "&&" | "||" | "!"
bitwise      ::= "&" | "|" | "^" | "~" | "<<" | ">>"
range        ::= ".." | "..="
assignment   ::= "=" | "+=" | "-=" | "*=" | "/=" | "%="
                  | "&=" | "|=" | "^="
closure-only ::= "~~"
bus-only     ::= "<-"
path         ::= "::"
member       ::= "."
delimiters   ::= "(" ")" "{" "}" "[" "]" "," ";"
                  ":" "->" "=" "<" ">" "@"
```

The `~~` operator (approximate equality) is permitted only
inside a closure assertion body; it is a parse error
elsewhere.

The `<-` operator (bus send) is statement-position only; it
does not nest in expressions.

## See Also

- [Types — overview](./types/index.md)
- [Expressions](./expressions/index.md)
- [Glossary](./glossary.md)
