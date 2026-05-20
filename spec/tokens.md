# Lexical structure

This document specifies the lexical layer of Aperio: the set of
tokens the lexer produces and feeds to the parser. The formal
grammar in `grammar.ebnf` is defined over these tokens.

## Source encoding

Source files are UTF-8 encoded. File extension: `.ap`.

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
interface
```

`interface` (F.20) declares a structural interface — a named set
of method signatures. Any locus whose method set is a superset
structurally satisfies the interface (no `impl I for L`
declaration). Phase A (typecheck) and Phase B (codegen vtable
dispatch) both shipped 2026-05-11; interface values are usable as
fn params, fn returns, locus param/field values, and
`@form(vec)` cell elements. Interface elements in tuples / fixed
arrays remain gated on the broader composite-construction
coercion design (same gap as tuple-of-`LocusRef` escape).

### Locus member keywords

```
params          contract        bus             capacity
```

`capacity` introduces an F.22 `capacity { ... }` block carrying
zero or more `pool X of T;` / `heap Y of T;` slot declarations.
The slot-kind words `pool` and `heap` are **contextual idents** —
they lex as ordinary Idents and the parser recognizes them only
in slot-decl head position inside a capacity block. So
`fn pool_alloc(...)`, `let heap = ...`, and `type Heap { ... }`
all stay admissible outside capacity blocks. Same F.10-style
narrowing the closure-keyword family uses for `approx` / `within`.

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

### Schedule-class keywords

```
schedule        cooperative     pinned
```

### Closure keywords

```
closure         epoch           persists_through    resets_on
```

`approx` and `within` are **contextual keywords**, recognized
only inside a `closure { ... }` block body (and only in the
specific positions the closure-assertion grammar admits them).
They lex as ordinary Idents elsewhere, so `fn approx(...)` and
`let within = ...` are admissible outside closure bodies. Same
F.10-style narrowing the mode-keyword family uses
post-dot. Shipped 2026-05-11; resolves
`notes/aperio-friction.md` 2026-05-10
`closure-keyword-shadows-helper-ident`.

`captures` and `inline` (v1.x-VIOLATE, F.27) are
**contextual keywords**. `captures` is recognized only as a
clause keyword inside `closure { ... }` body, in the
`captures: f1, f2 ... ;` production. `inline` is recognized
only as an `epoch_spec` variant (immediately after `epoch` in
a closure clause). Both lex as ordinary Idents elsewhere — so
`let captures = ...` and `fn inline(...)` stay admissible
outside closure bodies. Same F.10-style narrowing.

### Recovery primitives

```
restart         restart_in_place    quarantine      reorganize
bubble
```

`violate` (v1.x-VIOLATE, F.27) is a **contextual keyword**:
recognized only as the leading token of a statement inside a
locus method body (the `violate_stmt` production in
`grammar.ebnf` § 14). Outside that position it lexes as an
ordinary Ident, so `let violate = ...` and `fn violate(...)`
stay admissible. The optional trailing `with` in
`violate NAME with EXPR;` is the previously-reserved keyword
`with` (no longer reserved-for-future-use); recognized only as
the separator before the payload expression in this production.

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
let             mut             if              else
match           for             in              while
return          break           continue        true
false           nil             tier            self
```

Bindings are immutable by default. `let mut x = ...` declares a
mutable binding; reassignment via `x = ...` is permitted. Without
`mut`, reassignment is a compile-time error.

`self` is meaningful only inside a lifecycle block, mode block,
or closure block. It refers to the enclosing locus's own params
and contract-exposed state. Outside such a block, `self` is a
parse error.

### Predefined type names (NOT keywords)

```
Int             Uint            Float           Decimal
String          Bool            Time            Duration
Bytes
```

PascalCase per the type-name convention. The lexer emits these
as `Ident` tokens; the parser recognizes them by name in **type
position only**. In expression / namespace position, these names
are unreserved — `time::sleep` is a regular path because `time`
(lowercase) is an ordinary identifier. This eliminates the
lexical collision between primitive type names and stdlib
namespace names that would otherwise occur.

Shadowing a predefined type name with a user-defined type
(`type Int = ...`) is permitted by the grammar but produces a
compiler warning.

### Reserved for future use (not yet legal)

```
trait           impl            async           await
macro
```

(`with` is no longer in this list — v1.x-VIOLATE recognizes it
as a contextual keyword inside the `violate_stmt` production.
`where` is no longer in this list either — Form K (2026-05-20)
recognizes it as the suffix keyword on `binding_entry` carrying
operational constraints.)

`yield` is a real statement keyword (m26b) — explicit
cooperative yield point; lowers to a bus-queue drain in
codegen. Listed under cooperative-scheduler keywords.

### Cooperative-scheduler keywords

```
yield
```

### Fallible / error-addressing keywords (v1.x-FORM-1)

```
fallible        fail            or              raise
```

All four are **contextual keywords**, recognized only in the
positions described below. Outside those positions they lex as
ordinary `Ident` tokens, so existing code that uses
`let fail = ...`, `fn or_else(...)`, or `let raise = ...` stays
admissible.

- **`fallible`** — recognized only immediately after a function
  declaration's return type and before its body, in the
  `fallible_marker = "fallible" "(" type_expr ")"` production.
  Marks the function as one whose call sites MUST address the
  error. *Permitted at declaration sites: free fns + stdlib-
  synthesized methods over `@form(...)` containers; rejected at
  typecheck on user-declared locus methods per the two-channel
  rule (see `spec/semantics.md` § "Fallible call semantics"
  § "Where each channel lives").*
- **`fail`** — recognized only as the leading token of a
  statement inside a fallible-fn body. Exits via the error
  path, attaching the expression as the typed payload.
  Symmetric to `return`.
- **`or`** — recognized as a postfix on any expression of
  fallible type (the `or_clause` production). Three RHS forms:
  `or raise` (propagate one frame up the call stack),
  `or <expression>` (substitute), `or <handler-call>` (hand
  off).
- **`raise`** — recognized only as the immediate RHS of `or`.
  Diverges the expression by re-entering the fallible-return
  shape of the enclosing `fallible(E)` fn, with the payload
  carried into the enclosing fn's error sret slot. Past every
  enclosing fallible frame — at the implicit main locus's
  root boundary — the runtime panics via `lotus_root_panic`.
  The value-error channel is orthogonal to the closure-
  violation channel; `or raise` does **not** enter the
  `bubble` / `on_failure` machinery by default. See
  `spec/semantics.md` § "Fallible call semantics" for the
  full two-channel semantics.

The implicit binding `err` is in scope on the RHS of an `or`
clause and resolves to the fallible call's payload value
(typed as the fn's `fallible(T)` marker). This is a typecheck
rule, not a lexer rule — `err` lexes as an ordinary identifier
everywhere; the typechecker just introduces it as a binding in
the `or` RHS scope.

Rule of thumb for use:
- A stdlib fn whose failure carries useful diagnostic context
  returns `fallible(T)` with a named payload type T.
- A stdlib fn whose "failure" is a benign not-found case (and
  whose successful value has a natural default like 0 or "")
  uses the sentinel-with-discriminator idiom instead
  (`parse_int` / `can_parse_int`). Not every stdlib fn is
  fallible — the marker is reserved for true error paths.

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

### Bus send

```
<-          Send a typed message on a declared bus subject:
            `"subject" <- value;`. The left side names a subject
            declared in the locus's `bus { publish ... }`; the
            right side is the typed payload. Same Erlang-shape
            as `Pid ! Msg`; one-direction (subscribe is
            declarative, not an operator).
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

### Annotation prefix (v1.x-FORM-1)

```
@
```

`@` introduces a decorator-shaped annotation that decorates the
declaration immediately following it. v1 recognizes one such
annotation: `@form(<name>, <args>...)`, which sits above a
`locus` declaration and picks an efficient lowering (see
`spec/forms.md`).

Lexically, `@` is a single-character punctuation token. The
parser routes the following `form` ident as a contextual
keyword in this position only.

Reserved for future annotation surfaces; user-defined
annotations are not in v1.

### Reserved (no v0 meaning)

```
#   $   ?   ??  ?:
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
  type (fixed-precision, no float artifacts; same semantics as
  the `shopspring/decimal` Go library).

### Time / duration literals

- Duration suffixes: `ns`, `us`, `ms`, `s`, `m`, `h`, `d`.
  Examples: `100ms`, `5s`, `1h30m`. Compound forms permitted.
- Time literals: ISO-8601 between backticks: `` `2026-05-08T12:00:00Z` ``.

### String literals

- Double-quoted: `"hello"`. Standard escape sequences:
  `\n`, `\t`, `\r`, `\\`, `\"`, `\'`, `\0`, and `\xNN` for
  ASCII bytes 0x00..=0x7f (added 2026-05-16; useful for
  separators like `"a\x01b"`). High bytes (`\x80`+) are
  rejected with a message pointing at `std::bytes::*` — the
  Rust String invariant would UTF-8-encode them as two bytes
  and surprise the caller. No `\u{NNNN}` Unicode escape at
  v1; agents needing non-ASCII drop to byte literals.
- Raw strings: `r"..."` — no escape processing.
- Multi-line strings: `"""..."""`.
- F-strings (v1.x-10): `f"hello {name}"` — interpolates Aperio
  expressions inside `{...}`, each rendered via the same
  formatter `println` uses. `{{` and `}}` are literal braces.
  Plain `"..."` strings keep `{` and `}` as ordinary characters
  for back-compat (no breaking change). Interpolation accepts
  any expression; types are converted with `to_string`.

### Boolean literals

- `true`, `false`.

### Nil literal

- `nil`. Represents the absent value of an option type. Distinct
  from numeric zero or empty string.

### Bytes literals

- `b"..."` — byte-string literal. Same escape table as `STRING_LIT`
  (`\n`, `\t`, `\r`, `\\`, `\"`, `\0`, `\xNN`), but the body is
  raw bytes — **no UTF-8 promotion**, so `\xNN` accepts the full
  `0x00..0xFF` range (unlike the string literal which clamps at
  `\x7f`). Embedded NULs are preserved; the value's length is
  carried alongside the byte buffer, so `len(b)` and
  `std::bytes::at(b, i)` work over the entire literal. Codegen
  lowers via `lotus_bytes_from_buf(arena, src, len)`, which
  copies the source bytes into the caller's arena.

  Use cases: binary protocol headers / framing, magic bytes,
  embedded fixtures. Earlier code reached for
  `std::bytes::from_string("...")`, which only worked for ASCII;
  the `b"..."` form replaces that workaround end-to-end.

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
to_string       len             abs
Int             Float
```

`Int(x)` (v1.x-11) is a built-in cast — explicit Float → Int
narrowing via `fptosi` (truncate toward zero). Int arg is the
identity; other types reject. There is no implicit Float → Int
conversion; the user must commit via this constructor-shaped
call. `to_string(x)`, `len(x)`, `abs(x)`, `min(a, b)`, `max(a, b)`
are similarly bare-name builtins.

`print` and `println` are built-in functions, always in scope
without an `import`. They write to stdout. `print` does not
emit a trailing newline; `println` does. They accept any number
of arguments of any displayable type and concatenate.

`publish` is a built-in function in scope inside any locus
that declares matching `bus { publish SUBJECT of type T; }`.
The compiler verifies the subject and type at each call site.
Out of scope in loci with no publish declaration.

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
