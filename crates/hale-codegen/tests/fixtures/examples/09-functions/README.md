# 09 — user-defined fns

Exercises the multi-fn surface of codegen milestone 6:

- typed parameters (`Int`, `Float`, `Bool`, `String`, `Duration`)
- typed return values (and void fns with no `->` clause)
- calls in expression position (`let n = square(...)`) and
  statement position (`announce(...);`)
- recursion (each call site resolves because the codegen pass
  declares every user fn before lowering any body)

```
$ hale run   examples/09-functions/main.hl
square(7)=49
fib(12)=144
square(square(3))=81

$ hale build examples/09-functions/main.hl
built: examples/09-functions/main
$ ./examples/09-functions/main
square(7)=49
fib(12)=144
square(square(3))=81
```

## What this example does NOT yet exercise

- Default parameter values (`fn f(x: Int = 0)`) — declared in
  the AST, not yet lowered
- Generic fns (`fn f<T>(x: T)`) — wait on the generic
  monomorphization milestone
- Returning from `main` — main stays special-cased to the C
  entry point's `i32 main(void)` ABI; explicit returns from
  main land alongside the `Int → exit code` plumbing
- Calling a fn from inside a locus's lifecycle methods — waits
  on the locus-as-struct runtime ABI
