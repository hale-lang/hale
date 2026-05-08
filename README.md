# lotus

A programming language whose primitives are the lotus framework's
coordination primitives.

**Status.** v0 design exploration, 2026-05-08.
**No compiler exists yet.** This repo currently holds the formal
language specification only. Implementation work follows the spec
once the spec is stable.

## What this is

Lotus is a compile-time language designed around the alpha-conjecture
program's substrate-invariant coordination primitives. Concretely:

- **Loci as first-class entities.** Each locus declares its
  capacity parameters (B, c, σ, φ); the compiler computes its
  k_max and enforces it as a static invariant.
- **Projection classes (rich / chunked / recognition) as a
  type-system primitive.** Same source code, different generated
  allocator depending on declared / inferred N.
- **Three modes (bulk / harmonic / resolution) as a kernel-
  application primitive.** Define a kernel once; the compiler
  generates three projections.
- **Region-based memory** with contract-graded visibility. Each
  locus's arena is a sub-region of its parent's; access between
  loci is mediated by typed contracts; deeper looking costs more.
  No GC, no borrow checker.
- **Cyclic-closure tests as syntactic constructs.** The language
  enforces audit invariants the framework already commits to.
- **Hot-load of perspectives** as a first-class language feature.
  Stable perspectives cross from analyst-locus to executor-locus
  as typed parameter bundles within a shared compiled schema.
- **Lifecycle as a parent-policy-driven state machine.** Failure
  capture, recovery primitives (`restart`, `quarantine`,
  `reorganize`, `bubble`), and dissolution are language-native.

## Design lineage

This language is the natural compile-time collapse of the
alpha-conjecture program's existing design-time work:

- `~/notes/alpha-conjecture/` — the research program: framework
  primitives (capacity-allocation, multi-perspective stability,
  substrate-derivation discipline, cyclic-closure), paper-4
  closed-horizon-recursion, theory & methodology.
- `~/code/brain3/` — the existing software-substrate
  operationalization (production deployment); the empirical
  anchor at the software substrate.
- `~/notes/alpha-conjecture/lotus/` — the portable agent-facing
  distillation of the framework for software design.
- `~/code/grease/.lotus/tower.yaml` — design-time lotus tower
  for grease.

The language is a recognition event: the form is already
constrained by the closed graph above. This repo formalizes it.

## Layout

```
spec/
  grammar.ebnf            -- formal grammar (source of truth)
  tokens.md               -- lexical structure: keywords, operators, literals
  precedence.md           -- operator precedence and associativity
  design-rationale.md     -- why each construct looks this way
examples/                 -- example programs (placeholder; first
                             program is the trader/analyst pair)
notes/                    -- working notes and open questions
```

## What's pending

- **Type-system rules.** The grammar specifies syntax; type
  inference / checking rules are a separate document yet to be
  written.
- **Operational semantics.** How programs evaluate. Yet to be
  written.
- **Memory model.** The region model + contract-graded visibility
  is sketched but not formalized as a model. Yet to be written.
- **Reference implementation.** No parser, no compiler, no
  runtime exists. The first implementation pass is probably an
  ANTLR4 grammar derived from the EBNF spec.

## Naming

The framework's existing meta-framework is called "lotus" (see
the `lotus/` subdirectory of the alpha-conjecture program). This
language is named "lotus" for the same reason: it's the same
form, projected from design-time into compile-time. The two are
expected to converge — the meta-framework's discipline is what
the language's compiler enforces. Whether the meta-framework
collapses into the language entirely or persists alongside it
is a future decision.

File extension: `.lt`.

## License

TBD. Project status is design exploration; licensing decisions
follow first compiler work.
