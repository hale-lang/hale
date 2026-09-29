# Saying what must hold

By the end of the last part the compiler holds your whole program as one graph: every locus and who owns it, every topic and who reaches it, every call and what it touches. This part lets you write down what must stay true about that graph, and makes the build refuse a program that stops being so.

There are three sizes of promise.

- **A function's** ([Effects & contracts](../effects.md)). `@no_syscall`, `@deterministic` and their kin say what a function never does. The compiler proves each over the whole call graph beneath it, through helpers, methods, imported seeds and the standard library. A violation is a build error that names the path to it.
- **The program's** ([Claims & the law](../claims.md)). A claim is a named sentence over the assembled graph, such as "the storefront never reaches the books except through payments" or "one locus settles an order". It sits in the main locus's `claims { }` block, `hale check` evaluates it as an error, and it costs nothing at run time.
- **Every program's that is deployed somewhere** ([Constitutions](../constitutions.md)). A constitution is a named set of claims written once and adopted by each entrypoint. `hale.toml` binds one to each environment. This is how an application states its own law, its dna, so that the next change, by a person or a model, cannot quietly take it apart.

None of this runs. Every promise here is decided before the program starts, from the source alone, and the witness the compiler prints is a path through your code, not a stack trace from production.

**What the next part adds.** [Systems control](./systems.md) goes the other way, from what the program must be to how it runs. It covers memory and lifetime, performance, the forms under a locus's collections, zero-copy messaging, C, WebAssembly, and the tools for watching and replaying a running program.
