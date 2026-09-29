# The language

This part is Hale as a small, ordinary language: values and variables, numbers that include money and time, functions, control flow, strings, lists and maps, records, and the everyday work of files, JSON, HTTP, sockets, hashing, command lines, logging, metrics and tests. A complete program is a `fn main()` that does something and ends:

```hale
fn main() {
    let total = 19.99d * 3.0d;
    println("three at 19.99 is " + total);
}
```

Two things set the language apart from the start, and both are introduced here rather than later.

- **Failure is a value.** A call that can fail says so in its type, and the caller decides on the spot, with `or`, what happens when it does ([When a call can fail](../basics/fallible.md)). Nothing is thrown past you.
- **The locus appears early, and gently.** A locus is a named thing with state and methods ([The locus, gently](../everyday/locus-gently.md)). In this part it behaves like an object you call. It is the one primitive the rest of the book grows from, and nothing you write with it here changes later.

You can stop at the end of this part and write real tools: scripts, CLIs, clients and servers, with their tests beside them (`hale test`).

**What the next part adds.** [The locus model](./locus-model.md) lets a locus run over time. It gets a lifecycle, talks to others over a typed bus, is placed on threads, supervises its children, spans binaries, and can be driven from outside. The code in this part keeps working unchanged: the next part shows more of what was already there.
