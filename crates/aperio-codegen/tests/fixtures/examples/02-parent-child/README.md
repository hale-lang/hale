# 02-parent-child

A coordinator that accepts greeter children and reads their
state through the contract surface.

```
locus GreeterL {
    params {
        greeting: string = "hello";
    }

    contract {
        expose greeting: string;
    }
}

locus CoordinatorL {
    params {
        B: int = 100;
        c: int = 1;
        sigma: int = 1;
        phi: float = 1.0;
    }

    contract {
        consume greeting: string;
    }

    accept(g: GreeterL) {
        println("greeting from child: ", g.greeting);
    }

    run() {
        GreeterL { greeting: "hello" };
        GreeterL { greeting: "hi" };
        GreeterL { greeting: "yo" };
    }
}

fn main() {
    CoordinatorL { };
}
```

## What runs

1. `main()` invoked. Coordinator instantiates as anonymous
   child of `main`'s implicit locus.
2. Coordinator's `birth()` runs (default, no-op).
3. Coordinator's `run()` begins.
4. First `GreeterL { greeting: "hello" }` expression in `run()`:
   - Child handle created (no region yet).
   - Coordinator's `accept(g)` is invoked synchronously with
     the child's declared params; reads `g.greeting` via the
     contract; prints "greeting from child: hello".
   - Child region allocated as sub-region of coordinator's
     region.
   - Child's `birth()` runs (default, no-op).
   - Expression returns. Child is unbound and has no `run`,
     so it dissolves at statement boundary.
5. Same pattern for "hi" and "yo".
6. `run()` returns. Coordinator drains (depth-first; any
   remaining children dissolve first), then dissolves its own
   region.
7. `main()` returns. Program exits.

## Primitives this exercises (new vs. 01)

- **`contract { expose ... }`** — Greeter declares that its
  `greeting` field is part of the contract surface visible to
  coordinators above.
- **`contract { consume ... }`** — Coordinator declares it
  reads `greeting` from coordinatees. The compiler verifies
  the consume-surface is a subset of the child's expose-surface
  (contract compatibility).
- **`accept()` lifecycle method** — invoked synchronously when
  a child is added; receives the child handle; can read the
  contract-exposed state; can reject (not demonstrated here —
  future example).
- **Child instantiation inside a lifecycle method** —
  `GreeterL { ... }` inside `run()` attaches to the enclosing
  locus (the coordinator), not to `run()`'s implicit scope.
  Lifecycle methods run *as the locus*; children created in
  them are children *of* the locus.
- **Contract-graded access** — `g.greeting` in the parent's
  `accept` block; this is the parent reading the child's
  exposed state via the contract. The compiler enforces that
  only contract-exposed fields are accessible.
- **Parent-child memory hierarchy** — child's region is a
  sub-region of parent's region. Per the framework's
  recursion property: child arena nested in parent arena;
  contract-mediated access; deeper-looking-costs-more (one
  contract hop here; multi-level would compound).

## What writing this surfaced (for the spec)

Two issues, resolved in this commit:

1. **Lifecycle methods vs. free functions w.r.t. implicit
   locus.** §D in design-rationale committed every function
   scope to having its own implicit locus. But lifecycle
   methods (`birth`, `accept`, `run`, `drain`, `dissolve`)
   are special: they run *as the locus*, not in their own
   scope. Children instantiated inside a lifecycle method
   attach to the enclosing locus, not to a fresh implicit
   scope. Updated §D to clarify; added §F.6.

2. **`accept()` invocation timing.** Open-questions previously
   left this ambiguous. Resolved here: `accept(child_handle)`
   runs **before** the child's region is allocated and birth
   runs. This lets `accept()` validate and potentially reject
   the child without committing resources. The accept's
   parameter is the child's declared params + contract surface,
   not its running state. Documented in §F.7 (accept timing).

3. **Contract compatibility checking.** When CoordinatorL's
   `consume greeting: string` and GreeterL's `expose greeting:
   string` types must match. The compiler verifies at compile
   time. Documented as a typing rule (will live in `spec/types.md`
   when that's written; sketched in §F.8).

## What this still does *not* exercise

- closure tests — 03
- mode declarations — 04
- bus interface — 05
- recovery primitives (`accept` rejection, `bubble`,
  `quarantine`) — later
- inferred contracts — later
- multi-level nesting (grandchildren) — later
- mutable/long-lived children with own `run` lifecycle —
  later

## Next on the ladder

`03-closure-test` — a parent locus with two children whose
outputs must satisfy a cyclic-closure test. Adds `closure
name { ... ~~ ... within ... ; }`, epoch boundaries, and the
runtime accumulator. This is the first example that exercises
the framework's discipline-as-language-feature.
