# Hello, locus

The smallest runnable Aperio program is one
[locus](../reference/glossary.md#locus) with one lifecycle
method. This chapter walks it end-to-end: the source, what each
line means, how to run it, how to build it, and what happens on
the far side when it runs.

## The program

```aperio
locus HelloL {
    params {
        greeting: String = "hello, world";
    }

    birth() {
        println(self.greeting);
    }
}

fn main() {
    HelloL { };
}
```

This is `examples/hello-world/main.ap` in the repository. Save it
as `hello.ap` if you are following along.

## Line by line

```aperio
locus HelloL {
```

`locus` is a keyword that declares a new kind of locus called
`HelloL`. By convention, locus names are PascalCase and end in `L`
— the suffix is a writer's discipline, not a compiler rule, and
helps loci stand out at a glance from `type` declarations and
plain functions.

A locus is the unit of presence at runtime. Every Aperio program's
runtime tree is built out of loci.

```aperio
    params {
        greeting: String = "hello, world";
    }
```

The `params` block declares the locus's *parameter struct* — the
configurable values it carries throughout its existence. Each
parameter has a type and an optional default. Here, `greeting` is
a `String` that defaults to `"hello, world"`; a caller
constructing a `HelloL` may override it.

Parameters are immutable bindings inside the locus body. Read with
`self.greeting`; reassigning is a compile-time error.

```aperio
    birth() {
        println(self.greeting);
    }
```

`birth()` is the first method in the locus's lifecycle. It runs
exactly once, when the locus is instantiated. Whatever a locus
needs to do at the start of its existence happens here — for
`HelloL`, that is one call to `println`.

`println` is a built-in. It accepts any number of arguments,
formats each by type, joins them with no separator, and writes the
result to stdout followed by a newline. Here it prints the value
of `self.greeting`.

```aperio
fn main() {
    HelloL { };
}
```

`fn main()` is the program's entry point, like in most systems
languages. The body constructs a `HelloL` with default parameters
(no overrides between the braces), which causes the locus to be
born; its `birth()` method runs; the locus then dissolves at the
end of `main`. Process exits with status `0`.

There is no `return` here. Programs that want a non-zero exit code
return an `Int` from `main`; `0` is the default when `main`'s
return type is unit.

## Running it

The fastest way to see the program run is the interpreter:

```bash
aperio run hello.ap
```

Output:

```text
hello, world
```

`aperio run` parses the source, type-checks it against the full
F.1–F.18 rule set, and then executes it via the tree-walking
interpreter. The interpreter is the v0 reference runtime; it is
fast enough for development loops and supports the full language.

## Building it

To compile to a native ELF binary instead:

```bash
aperio build hello.ap
```

This produces an executable next to the source — `hello` in this
case. Run it directly:

```bash
./hello
hello, world
```

`aperio build` parses, type-checks, and emits LLVM IR which is
compiled and linked against the bundled
[lotus](../reference/glossary.md#lotus) C runtime. The
resulting binary statically embeds the runtime's region allocator,
bus router, and lifecycle scaffolding; it has no external Aperio
dependency at runtime.

The interpreter and the codegen path are observably equivalent —
the same source produces the same output under either. (This is
not an accident; the codegen tests assert it on every build.)

## What happened on the far side

When `main` ran, the following happened under the runtime:

1. **A locus was instantiated.** A `HelloL` came into existence.
   Its arena — its private region of memory — was allocated. The
   parameter struct was placed in the arena with `greeting` set to
   the default `"hello, world"`.
2. **`birth()` ran.** The body executed once, calling `println`,
   which read `self.greeting` from the parameter struct and wrote
   the bytes to stdout.
3. **The locus dissolved.** When `main` returned, the `HelloL`'s
   lifecycle completed. Because no `run()` or `drain()` body was
   declared, those phases were skipped. `dissolve()` was likewise
   not declared, so dissolution had no body to run; the runtime
   simply freed the locus's arena wholesale.

The whole sequence took microseconds. No allocation outlived the
locus that made it; nothing leaked, because no allocation could
escape the arena that was freed when the locus departed.

## What you have not yet seen

This chapter showed one locus with one lifecycle method.
Everything that makes Aperio interesting — multiple loci,
parent-child relationships, the bus, closures, recovery — comes
in later chapters. For now you have the smallest piece, end-to-end:

- the source you can write,
- the toolchain that turns it into a running program,
- one locus's worth of runtime behavior.

The next chapter, **[Types and values](./03-types-and-values.md)**,
introduces the value model — the built-in types (`Int`, `String`,
`Decimal`, `Bool`, etc.), user-defined records via `type`
declarations, and how values flow through expressions.
