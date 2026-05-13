# Grammar

## Synopsis

The full Aperio grammar lives in `spec/grammar.ebnf` in the
source tree as the canonical EBNF definition. This page
provides a curated overview; for the source-of-truth grammar,
read `spec/grammar.ebnf` directly.

## Top-level

```text
program ::= (top-level-decl)*

top-level-decl ::= type-decl
                | locus-decl
                | perspective-decl
                | fn-decl
                | import-decl

import-decl ::= "import" string-literal ";"
```

## Types

```text
type-decl   ::= struct-decl | enum-decl
struct-decl ::= "type" PascalCase-Ident generic-params? "{"
                  field-decl (";" field-decl)* ";"?
                "}"
enum-decl   ::= "type" PascalCase-Ident generic-params? "=" "enum" "{"
                  variant-decl ("," variant-decl)* ","?
                "}"

field-decl    ::= snake_case-Ident ":" type-expr
variant-decl  ::= PascalCase-Ident ("(" type-expr ("," type-expr)* ")")?

generic-params ::= "<" generic-param ("," generic-param)* ">"
generic-param  ::= PascalCase-Ident (":" constraint)?
constraint     ::= "Numeric" | "ProjectionClass" | projection-class
projection-class ::= "Rich" | "Chunked" | "Recognition"
```

## Type expressions

```text
type-expr ::= primitive-type-name
           | PascalCase-Ident generic-args?
           | tuple-type
           | array-type
           | function-type

primitive-type-name ::= "Int" | "Uint" | "Float" | "Decimal"
                     | "Bool" | "String" | "Time" | "Duration" | "Bytes"

tuple-type    ::= "(" type-expr ("," type-expr)+ ")"
array-type    ::= "[" type-expr ";" integer-literal "]"
function-type ::= "fn" "(" type-expr ("," type-expr)* ")" ("->" type-expr)?
generic-args  ::= "<" type-expr ("," type-expr)* ">"
```

## Loci

```text
locus-decl ::= "locus" PascalCase-Ident generic-params? annotations? "{"
                 locus-member*
               "}"

annotations ::= ":" annotation ("," annotation)*
annotation  ::= tier-ann | projection-ann | schedule-ann

schedule-ann ::= "schedule" schedule-class
schedule-class ::= "cooperative"
                | "pinned"
                | "pinned" "(" "core" "=" Int ")"

locus-member ::= params-block
              | contract-block
              | bus-block
              | closure-decl
              | lifecycle-method
              | fn-member
              | mode-decl
              | on-failure-decl

params-block   ::= "params" "{" param-decl* "}"
param-decl     ::= snake_case-Ident ":" type-expr param-init? ";"
param-init     ::= "=" expr | ":" "inferred"

contract-block ::= "contract" "{" contract-decl* "}"
contract-decl  ::= ("expose" | "consume") snake_case-Ident ":" type-expr ";"

bus-block      ::= "bus" "{" bus-decl* "}"
bus-decl       ::= subscribe-decl | publish-decl
subscribe-decl ::= "subscribe" string-literal "as" snake_case-Ident
                       "of" "type" type-expr ";"
publish-decl   ::= "publish" string-literal "of" "type" type-expr ";"

closure-decl   ::= "closure" snake_case-Ident "{" closure-clause+ "}"
closure-clause ::= assertion | epoch-clause
assertion      ::= expr ("~~" | "approx") expr "within" expr ";"
                ; The long-form spellings `approx` / `within` are
                ; CONTEXTUAL — they lex as ordinary idents outside
                ; closure bodies; the parser recognizes them as
                ; assertion keywords only inside `closure { ... }`
                ; bodies (F.10-style narrowing, 2026-05-11).
epoch-clause   ::= "epoch" epoch-name ";"
epoch-name     ::= "birth" | "dissolve" | "tick" | "duration" | "explicit"

lifecycle-method ::= birth-method | accept-method | run-method
                  | drain-method | dissolve-method
birth-method     ::= "birth"    "(" ")" block
accept-method    ::= "accept"   "(" Ident ":" type-expr ")" block
run-method       ::= "run"      "(" ")" block
drain-method     ::= "drain"    "(" ")" block
dissolve-method  ::= "dissolve" "(" ")" block

fn-member        ::= "fn" snake_case-Ident generic-params?
                       "(" param-list? ")" return-type? block

on-failure-decl  ::= "on_failure" "(" Ident ":" type-expr ","
                                       Ident ":" "ClosureViolation" ")"
                       block
```

## Perspectives

```text
perspective-decl ::= "perspective" PascalCase-Ident generic-params? "{"
                       params-block
                       stable-when-block?
                       serialize-as?
                     "}"
stable-when-block ::= "stable_when" block
serialize-as      ::= "serialize_as" type-expr ";"
```

## Functions

```text
fn-decl    ::= "fn" snake_case-Ident generic-params?
                 "(" param-list? ")" return-type? block

param-list  ::= param ("," param)*
param       ::= snake_case-Ident ":" type-expr
return-type ::= "->" type-expr
```

## Statements and expressions

```text
statement ::= let-stmt
           | assign-stmt
           | expr-stmt
           | if-stmt
           | while-stmt
           | for-stmt
           | break-stmt
           | continue-stmt
           | return-stmt
           | yield-stmt
           | bus-send-stmt
           | match-stmt

let-stmt       ::= "let" "mut"? pattern (":" type-expr)? "=" expr ";"
assign-stmt    ::= lvalue assign-op expr ";"
assign-op      ::= "=" | "+=" | "-=" | "*=" | "/=" | "%="
                | "&=" | "|=" | "^="
expr-stmt      ::= expr ";"
if-stmt        ::= "if" expr block ("else" (if-stmt | block))?
while-stmt     ::= "while" expr block
for-stmt       ::= "for" pattern "in" expr block
break-stmt     ::= "break" ";"
continue-stmt  ::= "continue" ";"
return-stmt    ::= "return" expr? ";"
yield-stmt     ::= "yield" ";"
bus-send-stmt  ::= string-literal "<-" expr ";"
match-stmt     ::= "match" expr "{" match-arm ("," match-arm)* ","? "}"
match-arm      ::= pattern ("if" expr)? "->" arm-body
arm-body       ::= expr | block

; Phase 2b (2026-05-11): a block's last item may be an expression
; without a trailing `;`. That trailing expression is the block's
; *value* when the block is used in expression position (Expr::If
; arm body, Expr::Block at let-RHS, fn-call argument). In
; statement position (function body, loop body, Stmt::If/Match
; block) the trailing expression is evaluated for side effects
; and the value is discarded.
block          ::= "{" statement* expr? "}"

; Phase 2b: `if` is dual-position. As a statement (`if-stmt`
; above) it carries no value; as an expression it requires an
; else branch with a trailing value, and the then- and else-
; branches' tail values are phi-merged at the join. The same
; block-as-expression form lets `let x = { let t = 1; t + 1 };`
; work directly.
if-expr        ::= "if" expr block "else" (if-expr | block)

; Phase 2d (2026-05-11): `[val; N]` repeats a single expression
; N times. `val` is evaluated exactly once; the result is
; broadcast to all N slots. N is a non-negative Int literal
; at v0 (const-eval is a future addition).
array-repeat   ::= "[" expr ";" integer-literal "]"

pattern ::= "_"
         | literal
         | snake_case-Ident
         | path-pattern
         | tuple-pattern
         | range-pattern

path-pattern  ::= type-expr "::" PascalCase-Ident ("(" pattern ("," pattern)* ")")?
tuple-pattern ::= "(" pattern ("," pattern)+ ")"
range-pattern ::= literal ("..=" | "..") literal
```

## Operator precedence

Refer to `spec/precedence.md` in the source tree for the
authoritative precedence and associativity table. Headlines
in [lexical structure](./lexical.md) and
[expressions](./expressions/index.md).

## See Also

- [Lexical structure](./lexical.md)
- [Expressions](./expressions/index.md)
- [Statements](./statements/index.md)
- [Locus declarations](./loci/index.md)
- The canonical EBNF: `spec/grammar.ebnf` in the source tree
