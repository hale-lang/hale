# 10 — stateful locus

Exercises the locus runtime ABI introduced in codegen milestone 7:
the lifecycle methods (`birth`, `run`) are LLVM functions taking the
locus's struct pointer as their first arg; `self.X` reads and writes
lower to `getelementptr` against that struct.

State flows through the alloca'd struct: what `birth` writes is
what `run` reads.

```
$ hale run   examples/10-stateful-locus/main.hl
total=160
step=30

$ hale build examples/10-stateful-locus/main.hl
built: examples/10-stateful-locus/main
$ ./examples/10-stateful-locus/main
total=160
step=30
```

## Trace

```
birth:  total += step  →  total = 10
        step  *= 3     →  step  = 30
run:    loop 5 times { total += step }  →  total = 160
```

## What this demonstrates

- **Locus → LLVM struct.** `Accumulator` lowers to
  `{ i64, i64, i64 }` (total / step / limit).
- **Lifecycle methods → LLVM fns.** `birth` and `run` each become
  `void method(ptr self_ptr)`.
- **`self.X` reads via GEP+load** in both `while self.total < ...`
  and the implicit reads inside the assignment RHS.
- **`self.X = ...` and `self.X += ...`** via GEP+store, with the
  compound op going through the same `lower_binop` machinery used
  for local mutation.
- **birth → run state handoff** through the same struct alloca:
  values written by `birth` survive to `run` because both methods
  receive the same `self_ptr` from the instantiation site.

## What this example does NOT yet exercise

- `accept()`, `dissolve()`, `drain()` lifecycle methods (parent-child
  + recovery work)
- Modes (`bulk` / `harmonic` / `resolution`)
- Closure-test cycles
- Bus subscriptions
- Long-lived loci surviving past the surrounding fn — requires
  the cooperative scheduler + region allocator
