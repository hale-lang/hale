# 12-user-types

User-defined `type` declarations as plain data records, with
struct literal instantiation and field reads.

## What it shows

A `type T { f1: T1; ... }` decl lowers to an LLVM struct type;
a literal `T { f1: v1, ... }` lowers to alloca + per-field
stores returning a pointer; `value.f1` lowers to GEP+load. The
field reads thread through `let` bindings, mixed-type println
arguments, and as operands of further struct literals.

```
$ lotus run examples/12-user-types/main.lt
p.x=3 p.y=4
q.x=13 q.y=8
alice says hello (priority 7)

$ lotus build examples/12-user-types/main.lt
$ ./examples/12-user-types/main
[same output]
```

## Why this is interesting

This is the substrate for the bus router (m12). Bus payloads
are always user types, and handlers receive them as
`TypeRef`-typed pointer params. Once `<-` dispatch lands, this
same lowering machinery handles every bus payload — the only
new piece is the subject → handler registry + the publish-side
call sequence.

This milestone also paves the way for fn params and locus
fields of user-type kind, both of which compile cleanly today
with no further changes — all that's needed is at-call-site
type-checking, which already happens through the unified
`type_expr_to_codegen_ty` path.

## What's not lowered yet

- Type aliases (`type Foo = Bar;`) — rejected at declare time.
- Enum types (`type Result { Ok(T); Err(String) }`) — rejected
  at declare time.
- Generic types — rejected at declare time.
- Struct literals with default field values — every field must
  be supplied at the call site (locus params do support
  defaults; user types don't, by spec).

These all wait on later milestones; the bus router doesn't
need them.
