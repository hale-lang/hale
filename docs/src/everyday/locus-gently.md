# The locus, gently

> **Coming from Python / Node?** A `locus` is the closest thing
> Hale has to a class or a module. It bundles state (fields) with
> behavior (methods) and you make instances of it. There's no
> separate "module" and "class" — one construct plays both roles.
> This chapter only uses the object-like 80%; the lifecycle and
> messaging parts wait until you need them.

In *the basics*, a program was functions and a `main`. That's
fine until you have **state that lives over time** — a counter, a
cache, a configuration, a connection — or until a pile of free
functions wants a name to live under. That's what a locus is for.

## A locus with state

```hale
locus Counter {
    params {
        count: Int = 0;
    }
    fn bump() {
        self.count = self.count + 1;
    }
    fn value() -> Int {
        return self.count;
    }
}
```

`params` is the locus's state — typed fields, each with a default.
Inside any method, `self.field` reads and writes that state.
Methods are `fn`s, called with `.`:

```hale
fn main() {
    let c = Counter { };          // make one; count defaults to 0
    c.bump();
    c.bump();
    println(c.value());           // 2
}
```

You construct a locus with `Name { ... }`, overriding any field
you like:

```hale,fragment
let c = Counter { count: 10 };
```

If you've used objects before, this is familiar: `params` are
the instance variables, methods are the methods, `Counter { }`
is the constructor. Hale collapses "constructor parameters" and
"instance fields" into one `params` block — the same way Ruby's
`@foo` or Python's `self.foo` are just attributes.

One habit from those languages doesn't carry over: there are no
class constants. A `const` belongs at the top level, beside the
locus, where it is in scope inside every locus of the program —
writing one in a locus body is an error at the `const` keyword.
A constant that each instance should carry is a `params` field
with a default.

## `type` vs `locus`

You met `type` for plain records earlier. The line between them:

- **`type`** is pure data — a record you construct, pass around
  by value, and read. No methods, no state that changes itself,
  no lifecycle.
- **`locus`** is data *with behavior and identity* — it has
  methods, it mutates its own state, and (at the next level) it
  can run over time and send messages.

```hale
type Point { x: Int; y: Int; }        // just data

locus Tally {                          // data + behavior
    params { total: Int = 0; }
    fn add(n: Int) { self.total = self.total + n; }
}
```

These aren't rival categories — they're points on a gradient. A
`type` is a locus that hasn't grown behavior yet. When a record
starts accumulating methods, you promote it from `type` to
`locus`. There is no third thing to reach for.

Note the two declarations above are **siblings**. A `type` used
by one locus still goes beside it, not inside its body: a
top-level `type` is in scope everywhere in the program, including
inside every locus, and there are no nested types. A `type`
written inside a locus body is an error at the `type` keyword.

## Two everyday shapes

Almost every locus you write at this level is one of two shapes.

**The app locus** — the outer wrapper for a whole program. Your
`main` reads arguments and hands off to it:

```hale
locus App {
    params { name: String = "world"; }
    run() {
        println("hello, ", self.name);
    }
}

fn main() {
    App { name: std::env::arg_or(1, "world") };
}
```

This replaces the bare-`main`-with-helpers shape from the basics:
the app's top-level state and entry point now have a home. (`run`
is a reserved lifecycle name — the runtime drives it; you never
declare it with `fn` or call it yourself.)

**The namespace locus** — a home for a coherent vocabulary of
helpers, with little or no state. Hale's stand-in for a "module
of functions" or a static class:

```hale
locus Temps {
    fn c_to_f(c: Float) -> Float { return c * 9.0 / 5.0 + 32.0; }
    fn f_to_c(f: Float) -> Float { return (f - 32.0) * 5.0 / 9.0; }
}

fn main() {
    let t = Temps { };
    println(t.c_to_f(100.0));     // 212
}
```

You instantiate it once and dispatch through it. When three or
more related free functions show up, this is usually the tidier
home for them.

### What about `module`?

There is a `module NAME { ... }` block, and it does less than the
word suggests. It groups declarations **for the reader** and
introduces no namespace: whatever you declare inside it is
registered under its plain name, so you spell it the same way
from anywhere in the file.

```hale
module geo {
    type Point { x: Int = 0; y: Int = 0; }

    fn manhattan(p: Point) -> Int { return p.x + p.y; }
}

fn main() {
    let p = Point { x: 4, y: 5 };   // `Point`, never `geo::Point`
    println(manhattan(p));          // 9
}
```

Everything else follows from that. A declaration one brace
deeper is an ordinary declaration: it typechecks, compiles, and
is reported on identically — the hot-path lint reaches into it,
a bus topic declared in it routes, a library's module-nested type
is reachable across an `import` as `lib::Point`, and two modules
declaring the same name is the same duplicate-name error as two
top-level ones.

Two things a module does *not* hold:

- The program's **entry point**: `fn main` has to be at the top
  level.
- A **`target wasm { }`** block, which is a build directive for the
  whole program rather than a declaration. One inside a module is
  an error — *`target` is a program-level declaration; move it to
  the top level* — rather than a line that quietly does nothing.

If you want a namespace, the namespace locus above is the tool.

## A rule worth meeting early

Hale has one structural commitment that shapes everything above:

> Every named piece of state belongs to exactly one locus.

No globals, no shared mutable buffer that nobody owns, no
"floating" value passed around by side channel. If you're not
sure where some state should live, the productive question is
"which locus *owns* this?" — and there's almost always a clean
answer. This is what lets Hale clean up memory and coordinate
failure without a garbage collector; you'll see the payoff at the
systems level. For now it's just good hygiene: put state where it
belongs.

Next: the collections you'll reach for constantly — [Lists &
maps](./collections.md).
