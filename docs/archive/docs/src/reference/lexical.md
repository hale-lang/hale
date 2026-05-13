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

### Contextual keywords

A few words are recognized as keywords **only inside specific
syntactic contexts**; outside those contexts they lex as
ordinary identifiers and may appear in any identifier position
(fn names, variable bindings, struct fields, etc.).

| Word     | Active inside              | Spec |
|----------|-----------------------------|------|
| `approx` | `closure { ... }` body — assertion long-form (`a approx b within e`); equivalent to `~~` | `spec/tokens.md`; F.10-style narrowing, 2026-05-11 |
| `within` | `closure { ... }` body — tolerance clause of an assertion | same |

So:

```aperio
fn approx(a: Float, b: Float, eps: Float) -> Bool {
    // `approx` is a perfectly legal fn name outside closure
    // bodies — and inside this body it is just a binding.
    return a - b < eps;
}

locus L {
    closure tolerance {
        // Inside a closure body, `approx` / `within` are
        // recognised as assertion long-form keywords.
        epoch tick;
        self.signal approx self.target within 0.01;
    }
}
```

Pre-2026-05-11 these were lexer-level reserved words; the
narrowing was made to free up natural math-shaped helper
names (`approx`, `within` as a fn).

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
string  ::= "\"" character* "\""
fstring ::= "f\"" (text-part | interpolation)* "\""
escape  ::= "\"" | "\\" | "n" | "t" | "r" | "0" | "x" hex hex | "u{" hex+ "}"
```

Strings are UTF-8 byte sequences at runtime. The literal
itself is interned in a static region; runtime concatenations
and slicings land in the current arena.

### F-strings (v1.x-10)

Prefixing a string literal with `f` enables interpolation:
`{ expression }` inside the body is evaluated and inserted
into the result via `to_string`. Plain double-quoted strings
keep their old semantics — `{` and `}` are ordinary characters
there, so existing source containing literal `{...}` content
(JSON snippets, placeholder tokens, etc.) is unchanged.

```aperio
let name = "world";
let n = 42;
println(f"hello {name}, n={n}");
//        ↑              ↑
//        interpolation pieces lower to:
//        "hello " + to_string(name) + ", n=" + to_string(n)
```

Inside `{...}`:

- The body is parsed as a regular Aperio expression
  (arithmetic, field access, function calls all work).
- `{{` and `}}` are literal braces inside the surrounding
  literal text.
- An inline string literal uses escaped quotes:
  `f"got: {std::str::upper(\"abc\")}"`. The lexer tracks
  quote state via `\"` toggles so `{` / `}` inside the inner
  string don't perturb depth counting.
- A literal `"` inside the nested string isn't supported at
  v1 (would require triple-escape and conflicts with the
  `\"` boundary marker).
- Empty interpolation `{}` is a lex error.

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
