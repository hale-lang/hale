# Lexical structure

This document specifies the lexical layer of lotus: the set of
tokens the lexer produces and feeds to the parser. The formal
grammar in `grammar.ebnf` is defined over these tokens.

## Source encoding

Source files are UTF-8 encoded. File extension: `.lt`.

Outside string literals and comments, only ASCII characters are
permitted. The framework's mathematical primitives are spelled
out as ASCII names; the renderer can produce Unicode for human
display, but the source is ASCII-only. This commitment serves the
agent-first authorship principle (no symbol-input friction) and
simplifies tooling.

## Whitespace

- Spaces, tabs, carriage returns, and newlines are whitespace.
- Whitespace separates tokens but is otherwise insignificant.
- No off-side rule (no Python-style indentation).
- Newlines are not statement terminators; semicolons are required.

## Comments

- Line comment: `// ...` to end of line.
- Block comment: `/* ... */`. Block comments do not nest in v0.
- Doc comment: `///` (line) or `/** */` (block) attached to the
  following declaration. Doc comments are preserved by the lexer
  for tooling consumption.

## Identifiers

- Match: `[a-zA-Z_][a-zA-Z0-9_]*`.
- Case conventions are enforced by lints, not the lexer:
  - `PascalCase` for type names, locus names, perspective names.
  - `snake_case` for variables, fields, function names.
  - `SCREAMING_SNAKE_CASE` for constants.
- Reserved words (see below) are not legal identifiers.

## Reserved words (keywords)

### Declaration keywords

```
locus           perspective     type            const
fn              import          export          module
```

### Locus member keywords

```
params          contract        bus
```

### Lifecycle keywords

```
birth           accept          run             drain
dissolve        on_failure
```

### Mode keywords

```
mode            bulk            harmonic        resolution
```

### Projection-class keywords

```
projection      rich            chunked         recognition
```

### Closure keywords

```
closure         epoch           within          approx
persists_through resets_on
```

### Recovery primitives

```
restart         restart_in_place    quarantine      reorganize
bubble
```

### Contract keywords

```
expose          consume         inferred
```

### Bus keywords

```
subscribe       publish         on              of
```

### Perspective keywords

```
stable_when     serialize_as
```

### Statement / expression keywords

```
let             if              else            match
for             in              while           return
break           continue        true            false
nil             tier            self
```

`self` is meaningful only inside a lifecycle block, mode block,
or closure block. It refers to the enclosing locus's own params
and contract-exposed state. Outside such a block, `self` is a
parse error.

### Type keywords (built-in primitives)

```
int             uint            float           decimal
string          bool            time            duration
bytes
```

### Reserved for future use (not yet legal)

```
trait           impl            async           await
yield           macro           where           with
```

## Operators

### Arithmetic

```
+   -   *   /   %
```

### Comparison

```
==  !=  <   >   <=  >=
```

### Logical

```
&&  ||  !
```

### Bitwise

```
&   |   ^   <<  >>  ~
```

### Assignment

```
=   +=  -=  *=  /=  %=  &=  |=  ^=
```

### Closure / approximation

```
~~          equivalent to `approx`; tests value approximate-equal
            within a stated tolerance band. Used in closure tests.
```

### Member access / call / index

```
.   ::  (   )   [   ]
```

### Type / generic

```
<   >   ->  =>  :   ::
```

(Note: `<` and `>` are overloaded between comparison and generic
arguments. The parser disambiguates contextually.)

### Punctuation

```
;   ,   {   }
```

### Reserved (no v0 meaning)

```
@   #   $   ?   ??  ?:
```

## Literals

### Integer literals

- Decimal: `0`, `42`, `1_000_000` (underscores permitted as digit
  separators).
- Hexadecimal: `0xFF`, `0x1A_2B`.
- Octal: `0o755`.
- Binary: `0b1010_1010`.
- Optional type suffix: `42i32`, `0xFFu64`. Default: `int`.

### Float literals

- Decimal: `3.14`, `1.0e-3`, `2.5E+10`.
- Optional type suffix: `3.14f32`, `2.5f64`. Default: `float`.

### Decimal literals

- Suffix `d`: `1.50d`, `0.05d`. Used for the built-in `decimal`
  type (fixed-precision, no float artifacts; same shape as
  shopspring/decimal in grease).

### Time / duration literals

- Duration suffixes: `ns`, `us`, `ms`, `s`, `m`, `h`, `d`.
  Examples: `100ms`, `5s`, `1h30m`. Compound forms permitted.
- Time literals: ISO-8601 between backticks: `` `2026-05-08T12:00:00Z` ``.

### String literals

- Double-quoted: `"hello"`. Standard escape sequences (`\n`,
  `\t`, `\\`, `\"`, `\u{NNNN}`).
- Raw strings: `r"..."` — no escape processing.
- Multi-line strings: `"""..."""`.

### Boolean literals

- `true`, `false`.

### Nil literal

- `nil`. Represents the absent value of an option type. Distinct
  from numeric zero or empty string.

### Bytes literals

- `b"..."` for byte-string literals; same escapes as strings.

## Built-in identifiers (not keywords)

These identifiers have semantic meaning in the standard library
and are conventionally reserved, but are not parser-reserved
keywords:

```
B               c               sigma           phi
k_max           span_max
sum             prod            min             max
length          empty
print           println
```

`print` and `println` are built-in functions, always in scope
without an `import`. They write to stdout. `print` does not
emit a trailing newline; `println` does. They accept any number
of arguments of any displayable type and concatenate.

The framework names `(B, c, σ, φ)` use ASCII spellings in source:
`B`, `c`, `sigma`, `phi`. The framework's `k_max` is `k_max` or
its named alias `span_max`.

## Tokenization rules

- Longest match: `==` is one token, not `=` followed by `=`.
- Keywords win over identifiers: `run` is the lifecycle keyword,
  not an identifier.
- Numeric literals are recognized greedily up to the first
  non-numeric character (after handling `0x`, `0o`, `0b`
  prefixes, decimal point, exponent, and type suffix).
- Comments are stripped before parsing (except doc comments,
  which are attached to the following declaration as metadata).

## Reserved character classes

The following characters are not part of any token in v0 and are
lexer errors if encountered outside a string or comment:

```
` (outside time literal)    \   (vertical bar mid-token, not || or |=)
```

(Backticks are reserved for time literals only; bare backslash
outside a string literal is illegal.)
