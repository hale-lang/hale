# The locus model

In the last part a locus was state plus methods: something you construct and call. This part lets it **run over time**, and that is where Hale stops looking like a scripting language.

A running locus has a lifecycle the runtime drives: `birth()`, `run()`, `drain()` and `dissolve()`, in that order, with shutdown cascading through the tree. Loci talk over a typed **bus**. A `topic` names a payload type, one locus publishes and another subscribes, and the compiler checks every edge. Each locus is **placed** on a pool (`cooperative(…)`, `pinned(…)` and the rest), and handlers never share a mutable value across threads, so there is no data race to write. A locus that constructs another is its **parent**: it owns the child's memory and decides what happens when the child fails. A **perspective** is a seam you can re-point while the program runs.

None of these needs a new program when the system grows. A topic that was an in-process queue becomes a Unix socket or a broker with one line in `main`'s `bindings { }` ([Across binaries](../services/multi-binary.md)). Everything a program already declares on its bus is its API once `bindings { api: … }` hands it out ([The API binding](../services/api.md)).

The part closes with [The model](../the-model.md): the typed description of what the program *is*, derived once from the source. It lists every locus and who owns it, every topic and who reaches it, and what the compiler could not determine, recorded as data. Every checker and tool reads that one description rather than working it out again.

**What the next part adds.** Because the compiler holds that whole graph, you can state what must hold over it, and have the build refuse a program that breaks it. [Saying what must hold](./what-must-hold.md) covers:
- effect contracts on functions;
- claims over the assembled graph;
- constitutions that carry one set of claims to every program deployed to a place.
