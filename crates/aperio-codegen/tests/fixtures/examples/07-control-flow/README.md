# 07 — control flow

Exercises the basic-block-management subset of codegen milestone 4:

- `if` / `else` / `else if`
- `while`
- `break` / `continue`

Folds the hand-unrolled counter from `06-mutable-counter` into a
loop that walks 1..=10, skips `n == 7`, and exits at `n == 9`.

```
$ lotus run   examples/07-control-flow/main.lt
sum=29 stopped at n=9

$ lotus build examples/07-control-flow/main.lt
built: examples/07-control-flow/main
$ ./examples/07-control-flow/main
sum=29 stopped at n=9
```

`for` is not yet in the codegen subset (needs Array values); use
the interpreter for `for`-loop programs until the locus-as-struct
ABI lands.
