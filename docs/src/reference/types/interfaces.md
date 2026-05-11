# Interfaces

> **Status (v0.1).** Phase A — typecheck-only. Codegen vtable
> dispatch (Phase B) is the next milestone; until then, calling
> a fn that takes an interface-typed param errors at codegen
> with a friendly Phase-B-pending message. Library design
> proceeds against the locked syntax today; binaries that
> actually pass loci across interface boundaries wait for Phase
> B.

A **structural interface** is a named set of method signatures.
Any locus whose declared methods are a superset of the
interface's required methods *implicitly* satisfies it — there
is no `impl I for L` declaration. Same shape Go gets from its
interface mechanism.

Interfaces are the v0 answer to the recurring "Sink-shape
polymorphism" friction (`std::text::Sink`, `std::log::*`):
StdoutSink / StringSink / FileSink can coexist as separate loci
behind one shared `Sink` interface.

## Declaration

```aperio
interface Sink {
    fn write(s: String);
    fn line(s: String);
    fn newline();
}
```

The body is signature-only (no method bodies — no default
methods at v0). Method order is significant: it determines the
vtable layout once Phase B lands. Don't reorder once you've
shipped.

## Satisfaction (structural rule)

A locus L satisfies an interface I iff:

- For every method `fn name(args...) -> ret;` in I, L has a
  method declared `fn name(args...) -> ret { ... }` with:
  - the same name,
  - the same arity,
  - compatible param types (positional, by `Ty` equality),
  - a compatible return type.

Satisfaction is implicit. There is no declaration to write.

```aperio
locus StdoutSinkL {
    params { }
    fn write(s: String)   { print(s); }
    fn line(s: String)    { println(s); }
    fn newline()          { println(""); }
}
// StdoutSinkL satisfies Sink — no impl declaration needed.
```

## Use sites

Interface types appear in fn parameter positions:

```aperio
fn render(sink: Sink) {
    sink.line("hello");
    sink.line("world");
}

fn main() {
    let s = StdoutSinkL { };
    render(s);   // implicit conversion: locus → interface
}
```

The structural-impl check fires at every call site where the
expected param is an interface and the arg is a concrete locus.
Mismatches produce typed diagnostics:

```
locus `BrokenL` does not satisfy interface `Sink`: missing method `line`
locus `X` method `write` arity does not match interface `Sink`: expected 1 arg(s), locus has 2
locus `Y` method `write` arg #0 type mismatch: interface `Sink` requires `String`, locus has `Int`
```

## v0.1 scope

What works today:

- Declaration parses and registers as a top-level type.
- Typechecker enforces the structural-impl rule at every call
  site.
- Friendly error if you try to `aperio build` code that
  actually passes a locus to an interface-typed fn (Phase B
  pending).

Not yet supported:

- Vtable dispatch — passing a locus where an interface is
  expected currently errors at codegen.
- `let s: Sink = StdoutSinkL { }` — interface-typed
  let-bindings.
- Heterogeneous storage — `[Sink]`-style mixed-impl arrays.
- Interface-typed return values from fns.
- Default methods.
- Interface inheritance / extension.
- Multi-interface bounds on generics.

These all unblock together when Phase B (vtable codegen) ships.

## See also

- `spec/design-rationale.md` F.20 — full rationale and Phase B
  sketch.
- `spec/design-rationale.md` F.21 — paired sketch for the
  cascading-dimension interface form (the F.14-style specialty
  for substrate-aware n-dim arena cascade per the
  `std::lotus::Grow` arc).
- `notes/aperio-friction.md` 2026-05-10 sink-as-tagged-locus —
  the friction that drove the design.
