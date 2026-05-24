; Tree-sitter highlight queries for Hale.
;
; Consumed by:
;   - iris/source_pane.hl (block spotlighting + syntax coloring)
;   - Editor extensions (VSCode, Helix, Neovim, Zed, Emacs)
;
; Highlight group names follow tree-sitter community
; conventions (@keyword, @function, @type, etc.) so editor
; themes pick them up without per-theme configuration.

; ============================================================
; Comments
; ============================================================

(line_comment) @comment
(block_comment) @comment
(doc_comment) @comment.documentation

; ============================================================
; Declaration keywords
; ============================================================

[
  "locus"
  "perspective"
  "interface"
  "module"
  "topic"
  "type"
  "const"
  "fn"
  "import"
  "as"
  "main"
] @keyword

; Locus annotation keywords
[
  "tier"
  "projection"
  "schedule"
  "rich"
  "chunked"
  "recognition"
  "fixed_cell"
  "shared_slab"
  "spillover"
  "summary_only"
  "cooperative"
  "pinned"
  "cap"
  "core"
] @keyword.modifier

; Locus member keywords
[
  "params"
  "contract"
  "bus"
  "capacity"
  "bindings"
] @keyword

; Lifecycle + mode keywords
[
  "birth"
  "accept"
  "run"
  "drain"
  "dissolve"
  "on_failure"
  "bulk"
  "harmonic"
  "resolution"
  "mode"
  "birth_check"
] @keyword.function

; Contract keywords
[
  "expose"
  "consume"
  "inferred"
] @keyword

; Bus keywords
[
  "subscribe"
  "publish"
  "of"
  "payload"
  "subject"
] @keyword

; Capacity keywords
[
  "pool"
  "heap"
  "indexed_by"
  "as_parent_for"
] @keyword

; Closure keywords
[
  "closure"
  "epoch"
  "persists_through"
  "resets_on"
  "captures"
  "inline"
  "tick"
  "duration"
  "explicit"
  "approx"
  "within"
] @keyword

; Perspective keywords
[
  "stable_when"
  "serialize_as"
] @keyword

; Transport / binding keywords
[
  "unix"
  "shm_ring"
  "where"
  "role"
  "listen"
  "connect"
  "slot_count"
  "on_overflow"
  "block"
  "drop"
  "intra_process"
  "intra_machine"
  "cross_machine"
  "zero_copy"
] @keyword

; Statement / expression keywords
[
  "let"
  "mut"
  "if"
  "else"
  "match"
  "for"
  "in"
  "while"
  "return"
  "break"
  "continue"
  "true"
  "false"
  "nil"
  "self"
  "yield"
  "sum"
  "prod"
] @keyword

; Recovery primitives
[
  "restart"
  "restart_in_place"
  "quarantine"
  "reorganize"
  "bubble"
  "violate"
  "with"
  "until"
] @keyword.exception

; Fallible / error keywords. `raise` and `discard` are wrapped
; in named rules (raise_disposition / discard_disposition) so
; we query the rule rather than the bare string token.
[
  "fallible"
  "fail"
  "or"
] @keyword.exception

(raise_disposition)   @keyword.exception
(discard_disposition) @keyword.exception
(fail_disposition "fail" @keyword.exception)

; Reserved-for-future keywords (trait, impl, async, await,
; macro) aren't queryable because the grammar doesn't
; reference them — they exist only as lexer-level reservations
; in the real compiler. If they show up in source they'll
; parse as identifiers; consumers can highlight separately
; if desired.

; ============================================================
; Annotation decorators (@form, @ffi)
; ============================================================

(form_annotation
  "@" @attribute
  "form" @attribute)

(ffi_annotation
  "@" @attribute
  "ffi" @attribute)

; ============================================================
; Types
; ============================================================

(primitive_type) @type.builtin

; Declaration introduces this type's name.
(type_decl name: (identifier) @type)
(locus_decl name: (identifier) @type)
(interface_decl name: (identifier) @type)
(perspective_decl name: (identifier) @type)
(topic_decl name: (identifier) @type)

; Generic params
(generic_param name: (identifier) @type)

; Projection-class wrappers
[
  "Rich"
  "Chunked"
  "Recognition"
] @type.builtin

; Named types in type-expression positions look like identifiers.
; We cover this via parameter / field type fields:
(parameter type: (named_type (qualified_name (identifier) @type)))
(struct_field type: (named_type (qualified_name (identifier) @type)))
(param_decl type: (named_type (qualified_name (identifier) @type)))

; ============================================================
; Functions + methods
; ============================================================

(function_decl name: (identifier) @function)
(interface_method_sig name: (identifier) @function.method)
(ffi_function_decl name: (identifier) @function)

; Call sites
(call_expr callee: (identifier) @function.call)
(call_expr callee: (field_expr member: (identifier) @function.method.call))
(call_expr callee: (path_expr member: (identifier) @function.call))

; ============================================================
; Identifiers (lvalues, variables, field accesses)
; ============================================================

; Parameter names
(parameter name: (identifier) @variable.parameter)
(param_decl name: (identifier) @variable.member)

; Struct field names
(struct_field name: (identifier) @variable.member)

; Let-binding names
(let_stmt name: (identifier) @variable)

; Field access — member position
(field_expr member: (identifier) @variable.member)

; Struct literal field positions
(struct_init field: (identifier) @variable.member)

; ============================================================
; Literals
; ============================================================

(integer_literal) @number
(float_literal) @number.float
(decimal_literal) @number
(duration_literal) @number
(time_literal) @string.special
(boolean_literal) @constant.builtin
(nil_literal) @constant.builtin

(string_literal) @string
(bytes_literal) @string.special
(fstring_literal) @string.special

; ============================================================
; Operators
; ============================================================

[
  "+"
  "-"
  "*"
  "/"
  "%"
  "<<"
  ">>"
  "&"
  "|"
  "^"
  "~"
  "!"
  "&&"
  "||"
  "=="
  "!="
  "<"
  ">"
  "<="
  ">="
  "~~"
  "="
  "+="
  "-="
  "*="
  "/="
  "%="
  "&="
  "|="
  "^="
  "->"
] @operator

; Bus send — distinctive enough to call out.
"<-" @operator.special

; ============================================================
; Punctuation
; ============================================================

[
  ";"
  ","
  "::"
  "."
  ":"
] @punctuation.delimiter

[
  "("
  ")"
  "{"
  "}"
  "["
  "]"
] @punctuation.bracket

; ============================================================
; Subject literals (bus addresses) — distinguish from other strings
; ============================================================

(bus_subscribe subject: (string_literal) @string.special.symbol)
(bus_publish subject: (string_literal) @string.special.symbol)

; ============================================================
; Special expressions
; ============================================================

; sum / prod are language-native reductions
(sum_expr "sum" @function.builtin)
(prod_expr "prod" @function.builtin)

; Self
(self_expr) @variable.builtin
