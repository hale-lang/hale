# Your first program

The smallest Aperio program is one locus with one lifecycle method:

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

Save it as `hello.ap` (Aperio source files use the `.ap` extension)
and run it:

```bash
aperio run hello.ap
```

You should see:

```text
hello, world
```

The same example lives at `examples/hello-world/main.ap` in the
repository.

## What just happened

- `locus HelloL { ... }` declares a *locus* — the unit of structure
  inside a [lotus](../reference/glossary.md#lotus). At
  runtime your program *is* a tree of these.
- `params { greeting: String = "hello, world"; }` declares a parameter
  with a default. Like a struct field, but checked at the locus
  boundary.
- `birth()` is the first method in the lifecycle quartet (`birth` →
  `run` → `drain` → `dissolve`). It runs once, when the locus is
  instantiated.
- `fn main() { HelloL { }; }` is the entry point. Constructing the
  locus runs its lifecycle.

To compile to a native ELF binary instead of running the interpreter,
use `aperio build hello.ap` — the binary lands next to the source.

When your program outgrows one file, decompose into multiple `.ap`
files in a directory and build the directory: `aperio build apps/myapp/`.
Every `.ap` file in the directory contributes to one bundle (one
binary); top-level decls in any file are visible to every other
file. Same shape Go gets from per-package visibility. See
`examples/multi-file-seed/` for the smallest demo;
`spec/design-rationale.md` F.19 for the rationale.
