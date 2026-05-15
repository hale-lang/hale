# 06 — mutable counter

Exercises the mutable-binding subset of codegen milestone 3:

- `let mut x = ...;`
- plain assignment `x = ...;`
- compound assignments `+=` / `-=` / `*=` / `/=` / `%=`

No control flow yet, so the counter steps are hand-unrolled. Compiles
to a native ELF via `lotus build` and also runs under the tree-walking
interpreter via `lotus run`. Both paths produce `n=2`.

```
$ lotus run   examples/06-mutable-counter/main.lt
n=2

$ lotus build examples/06-mutable-counter/main.lt
built: examples/06-mutable-counter/main
$ ./examples/06-mutable-counter/main
n=2
```

Once `if` / `while` land in codegen, this example will get a folded-up
loop variant; the unrolled form here is what the alloca-only subset
can express.
