# Interfaces & Contracts

An **interface** is a named set of method signatures. Any locus whose methods cover it satisfies it, with no declaration on the locus's side: satisfaction is structural, as in Go. A holder programs against the interface and never names the implementation, so the locus behind it is chosen where the holder is constructed.

```hale
interface Router {
    fn route(code: Int) -> Int;
    fn health() -> Int;
}

locus RouterV1 {
    fn route(code: Int) -> Int { return code + 100; }
    fn health() -> Int { return 1; }
}

locus RouterV2 {
    fn route(code: Int) -> Int { return code + 200; }
    fn health() -> Int { return 1; }
}

locus Gateway {
    params { router: Router = RouterV1 { }; }
    fn handle(code: Int) -> Int { return self.router.route(code); }
}

main locus App {
    params {
        a: Gateway = Gateway { };                      // RouterV1, the default
        b: Gateway = Gateway { router: RouterV2 { } }; // RouterV2, named at construction
    }
    run() {
        println(self.a.handle(1));   // 101
        println(self.b.handle(1));   // 201
    }
}

fn main() { App { }; }
```

`Gateway` calls `self.router.route` without knowing which locus is behind it. The param's default names `RouterV1`; a construction that names another locus that satisfies `Router` (`Gateway { router: RouterV2 { } }`) substitutes it. The router is `Gateway`'s owned child either way, torn down with it ([Locus Lifecycle](./lifecycle.md)), and each `Gateway` holds its own.

## Choosing a Boundary

Hale has two boundary mechanisms, depending on whether you are decoupling callers from an implementation or exposing a child's state to its supervising parent:

| Construct | Scope & Cardinality | Dispatch & Representation | Primary Use Case |
|---|---|---|---|
| **`interface`** | Per value; many implementations coexist, each holder holds its own | Structural satisfaction (any locus whose methods cover `I` satisfies it); a fat pointer, one indirect call | Pluggable subsystems (routers, storage engines, policies) chosen at construction, heterogeneous collections, polymorphic `accept(j: Job)` parent hooks. |
| **`contract { expose; consume; }`** | Parent-child depth edge (`H4`/`H5`) | Direct static field access checked between a parent's `accept(c: Child)` and `Child` | Declaring which internal fields a child locus exposes to its supervising parent. See [Ownership, Children & Flow Reclamation](./parents-children.md). |

A locus converts into an interface where the place is declared: an argument, a return, a field's initializer, a `let` that declares the interface, an element of an array declared as one. A value read back from any of those is that interface's value and offers only its methods. It does not convert into a *different* interface, even one declaring the same methods, so passing it where another interface is expected is refused at the argument; declare the parameter as the value's own interface, or pass the locus. Assigning a locus to an interface place after its declaration (`g = Hey { };` for `let mut g: Greeter`) is refused too: bind a new `let`.

## When the Implementation's Type Matters

An interface-typed value offers only the interface's methods. When the holder needs the implementation's own type (its fields, a method the interface does not declare, or a direct call instead of an indirect one), make the holder generic over it ([Generics](../everyday/generics.md)):

```hale
locus RouterV2 {
    params { hits: Int = 0; }
    fn route(code: Int) -> Int {
        self.hits = self.hits + 1;
        return code + 200;
    }
}

// Generic over the implementation: `Gateway<RouterV2>` holds a RouterV2.
locus Gateway<R> {
    params { router: R; }
    fn handle(code: Int) -> Int { return self.router.route(code); }
}

fn main() {
    let gw: Gateway<RouterV2> = Gateway { router: RouterV2 { } };
    println(gw.handle(1));      // 201
    println(gw.router.hits);    // 1: a RouterV2 field, read through the type
}
```

`Gateway<RouterV2>` is a separate copy of `Gateway` built for `RouterV2`, and `self.router.route` is checked against `RouterV2`'s methods in that copy. Choose the implementation at the holder's type, not at construction.

Hale had a `perspective` declaration for program-global, live-swappable contracts; it was removed (`spec/decisions.md` F.55). A program that still writes `perspective`, `serves` or `reperspective` gets a parse error naming this replacement. A live swap of a running implementation has no replacement.

Next: [Seams & Typed Pipelines](./seams.md).
