# Perspectives and projection classes

## Synopsis

Two related but distinct mechanisms:

- **Projection classes** (`Rich<T>`, `Chunked<T>`,
  `Recognition<T>`) are language-native generic constructors
  that select per-locus allocator strategies.
- **Perspectives** are serializable parameter bundles with a
  commit predicate (`stable_when`) and an optional schema
  evolution annotation (`serialize_as`).

## Projection classes

Per **F.2**, three projection classes:

| Class | Allocator behavior |
|---|---|
| `Rich<T>` | Each child gets an independent arena (`lotus_arena_create`) |
| `Chunked<T>` | Parent carves a sub-region per child with free-list slot reuse (`lotus_arena_create_subregion`) |
| `Recognition<T>` | Same path as Chunked in v0; documented stub for a future bitmap-pool optimization |

### Annotation on locus declarations

```aperio
locus RichCoord : projection rich {
    accept(c: Leaf) { }
    run() { /* ... */ }
}

locus ChunkedCoord : projection chunked { /* ... */ }
locus RecognitionCoord : projection recognition { /* ... */ }
```

The annotation drives arena allocation, not surface behavior.
All three classes are observably equivalent at the language
level; they differ only in *how* the population's storage is
laid out.

### Default

If a locus that declares `accept` does not annotate its
projection class, the default is `chunked` if the compiler
cannot statically determine the child population size N. For
loci without parent-child relationships the projection class is
unused.

### `ProjectionClass` constraint

Per **F.2**, `<T: ProjectionClass>` is a built-in any-of-three
constraint: the type variable resolves to one of `Rich`,
`Chunked`, or `Recognition`. The compiler emits one
specialization per resolution. See
[generics](./generics.md#projectionclass).

```aperio
fn process<P: ProjectionClass, T>(input: P<T>) -> P<T> {
    // ... operates on P<T> regardless of which class P is
}
```

There is no trait system underneath. `ProjectionClass` is a
recognized name in the constraint position only.

## Perspectives

A `perspective` declaration introduces a serializable parameter
bundle.

### Grammar

```text
perspective-decl ::= "perspective" PascalCase-Ident generic-params? "{"
                       params-block
                       stable-when-block?
                       serialize-as?
                     "}"

stable-when-block ::= "stable_when" "{" expr-statement* "}"
serialize-as      ::= "serialize_as" type-expr ";"
```

### Example

```aperio
perspective KernelPerspective {
    params {
        kernel: Kernel;
        validation_count: Int = 0;
    }

    stable_when {
        return self.validation_count >= 3;
    }

    serialize_as Kernel;
}
```

### Three parts

- **A `params` block** — the values that travel together.
- **A `stable_when { ... }` block** — a commit predicate. The
  fitting locus may hold multiple candidate perspectives in
  flight; only those that satisfy `stable_when` are eligible
  to ship. Invoked via `p.is_stable()` on a perspective value.
- **An optional `serialize_as TypeV1` annotation** — the
  schema-evolution mechanism (open-question #13). Names the
  type the perspective serializes *as* on the wire; future
  versions of the same perspective may declare different
  internal fields but serialize as the same wire type.

### `is_stable()`

A perspective value `p` exposes `p.is_stable()`, which runs
the `stable_when` block and returns its `Bool` result. v0 has
no other perspective-level methods; broader perspective
methods are post-v1.

## Status of v0

- **Projection-class allocator strategies**: implemented.
- **`ProjectionClass` constraint**: implemented.
- **Multi-implementation contract fields** (per **F.14**): the
  typing rule is enforced; the dispatch syntax (e.g.
  `@projection rich fn ...`) is deferred to post-v1.
- **`serialize_as TypeV1` rolling deployments**: declared in
  the spec; runtime support is roadmap.

## See Also

- [Generics](./generics.md)
- [Loci — projection class annotation](../loci/index.md)
- [Memory model](../memory.md)
