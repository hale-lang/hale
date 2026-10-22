# Perspectives

A **perspective** is a program-wide, live-rebindable contract slot. Loci program against the contract's method signatures and bus subscriptions through `perspective(Name)`, while the deployment designates a concrete locus that `serves` the contract—and can hot-swap the active implementation at runtime via `reperspective` while preserving live state.

```hale
perspective Router {
    fn route(code: Int) -> Int;
    fn health() -> Int;
}

locus RouterV1 : serves Router {
    fn route(code: Int) -> Int { return code + 100; }
    fn health() -> Int { return 1; }
}

locus Gateway {
    params {
        router: perspective(Router) = RouterV1 { };
    }
    fn handle(code: Int) -> Int {
        return self.router.route(code);
    }
}
```

## Interface or Perspective

Hale provides three distinct boundary mechanisms depending on whether you are decoupling method callers, swapping a singleton subsystem, or exposing child state to a parent supervisor:

| Construct | Scope & Cardinality | Dispatch & Representation | Primary Use Case |
|---|---|---|---|
| **`perspective`** | Single program-global slot shared by all holders of `perspective(P)` | `{ data, vtable }` slot; 1 pointer load + 1 indirect call; supports live vtable hot-swap via `reperspective` | Subsystem seams (routers, storage engines, policies) configured at assembly or hot-swapped in place. |
| **`interface`** | Per-value fat pointer; many implementations can coexist simultaneously | Structural satisfaction (any locus with a superset of methods satisfies `I`, no `serves` clause needed) | Heterogeneous collections, per-instance plugins, polymorphic `accept(j: Job)` parent hooks. |
| **`contract { expose; consume; }`** | Parent-child depth edge (`H4`/`H5`) | Direct static field access checked between a parent's `accept(c: Child)` and `Child` | Declaring which internal fields a child locus exposes to its supervising parent. See [Parents & Children](./parents-children.md). |

## Declaring & Serving a Perspective

### The Contract (`perspective`)

A `perspective` declaration lists bodyless method signatures (and optional bus subscriptions) that form the contract's ABI:

```hale
perspective Router {
    fn route(code: Int) -> Int;
    fn health() -> Int;
}
```

### The Implementation (`serves`)

A locus implements a perspective by adding `: serves <Perspective>` to its header and providing every method and bus subscription declared by the perspective:

```hale
locus RouterV1 : serves Router {
    fn route(code: Int) -> Int { return code + 100; }
    fn health() -> Int { return 1; }
}
```

- **Structural signature verification**: Parameter types, return types, and `fallible` annotations must match the perspective declaration exactly. Missing methods or mismatched signatures fail at compile time at the `serves` clause.
- **Combining with annotations**: `serves` shares the post-colon header list with locus annotations (e.g., `locus RouterV1 : serves Router, tier 2`).
- **Cross-seed perspectives**: A locus can serve a perspective imported from another library seed using a qualified path:
  - Header: `locus RouterV1 : serves lib::Router`
  - Parameter type: `perspective(lib::Router)`
  - Live swap: `reperspective self.router as lib::RouterV2;`

## Holding, Designating, and Calling a Slot

Holders declare a parameter of type `perspective(Router)` (using a bare `Router` as a type is a compile error):

- **Designating the implementation**: Providing a default or override initializer (`router: perspective(Router) = RouterV1 { };` or `Gateway { router: RouterV2 { } }`) constructs the implementation and binds the program-global `Router` slot to it.
- **Overriding at assembly**: A parent or `main locus` can substitute any locus that `serves Router` when constructing the holder:

```hale
locus RouterV2 : serves Router {
    fn route(code: Int) -> Int { return code + 200; }
    fn health() -> Int { return 1; }
}

main locus App {
    params {
        gw: Gateway = Gateway { router: RouterV2 { } };
    }
}

fn main() { App { }; }
```

- **Holding without designating**: Omitting the default initializer (`params { router: perspective(Router); }`) binds the field to the program's existing `Router` slot without constructing a new instance or re-pointing the slot. This allows dynamically spawned children to call through the implementation designated by `main`:

```hale
locus Attempt {
    params { router: perspective(Router); }   // holds the global slot without re-designating
    run() { println(self.router.route(1)); }
}

locus Work {
    accept(a: Attempt) { }
    run() { Attempt { }; Attempt { }; }       // both calls route through RouterV2
}

main locus App {
    params {
        router: perspective(Router) = RouterV2 { };   // designates RouterV2 for the program
        w: Work = Work { };
    }
}
```

### Slot Constraints & Diagnostics

1. **Program-global singleton slot**: Every `perspective(Router)` handle in the binary references the same underlying slot. If multiple holders designate different implementations at construction, the most recently constructed designation wins program-wide.
2. **Must be designated before use**: `hale check` rejects any program where a locus holds `perspective(P)` with no initializer unless at least one constructed locus designates `P`. At runtime, calling through an undesignated slot (for example, from an earlier sibling parameter born before the designating parameter) aborts the process naming the perspective.
3. **Methods-only encapsulation**: Callers can only invoke methods declared on the `perspective`. Direct field reads or writes through the slot (`self.router.hits = 0;`) are rejected at compile time.
4. **No `fallible` calls through slots**: While a `perspective` signature and `serves` implementation must agree on `fallible`, calling a `fallible` method through a `perspective(P)` slot is currently rejected at compile time because perspective dispatch does not propagate a value-error channel.

## Live Redeployment (`reperspective`)

The `reperspective` statement hot-swaps the active implementation behind a perspective slot at runtime with a single vtable pointer store:

```hale
perspective Router {
    fn route(code: Int) -> Int;
    fn health() -> Int;
}
locus RouterV1 : serves Router {
    fn route(code: Int) -> Int { return code + 100; }
    fn health() -> Int { return 1; }
}
locus RouterV2 : serves Router {
    fn route(code: Int) -> Int { return code + 200; }
    fn health() -> Int { return 1; }
}

locus Gateway {
    params { router: perspective(Router) = RouterV1 { }; }
    run() {
        println(self.router.route(1));        // prints 101 (RouterV1)
        reperspective self.router as RouterV2;
        println(self.router.route(1));        // prints 201 (RouterV2)
    }
}

fn main() { Gateway { }; }
```

### `reperspective` Rules

- **Ownership authority**: `reperspective self.<field> as NewImpl;` can only be executed by a locus that holds the perspective slot in `self.<field>`, and `NewImpl` must declare `: serves <Perspective>`.
- **Live state preservation & identical footprint**: A perspective slot stores `{ data, vtable }`. Executing `reperspective` swaps only the `vtable` pointer, leaving `data` untouched so the new implementation continues running on the live state accumulated by the previous implementation:
  ```hale,fragment
  self.counter.bump();
  self.counter.bump();                          // count == 2 under CounterV1
  reperspective self.counter as CounterV2;      // swaps vtable only
  println(self.counter.get());                  // still 2 under CounterV2
  ```
  To guarantee memory layout compatibility across swaps, **every locus that `serves` a given perspective must declare an identical `params` footprint** (same field names and types). Adding, removing, or retyping a parameter field across versions is a compile error.
- **Zero-allocation swap**: Nothing is re-allocated, `birth()` is not re-run, and `dissolve()` is not called during `reperspective`.

## Bus Subscriptions in Perspectives

A `perspective` can include a `bus { }` block alongside its synchronous methods so that both synchronous calls and asynchronous bus deliveries are governed by the contract:

```hale
type Order { id: Int; }

perspective OrderRouter {
    fn health() -> Int;
    bus { subscribe "orders" as on_order of type Order; }
}
```

- **Structural bus checking**: Any locus that `: serves OrderRouter` must declare the matching `bus { subscribe "orders" as on_order of type Order; }` entry and handler method.
- **Live bus handler re-pointing**: Executing `reperspective self.router as OrderV2;` tombstones the previous implementation's bus subscriptions on the shared slot state and registers `OrderV2`'s handlers over the same state:
  ```hale,fragment
  "orders" <- Order { id: 1 };            // dispatched to OrderV1::on_order
  reperspective self.router as OrderV2;   // hot-swaps sync vtable and bus handlers
  "orders" <- Order { id: 2 };            // dispatched to OrderV2::on_order
  ```
- **Deterministic swap boundary**: Because cooperative bus publishing captures the target handler at the moment `<-` executes, messages published before `reperspective` are processed by the old handler and messages published after `reperspective` are processed by the new handler.

Next: [Parents & Children](./parents-children.md).
