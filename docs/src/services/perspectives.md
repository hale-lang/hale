# Perspectives

A **perspective** is a live-rebindable handle to a *contract*. You
program against the contract — a set of method signatures — and
reach the real implementation only through an indirection the
system can re-point underneath you. It's the seam that lets you
swap a component's implementation without the code that calls it
changing.

> This chapter covers what ships today: declaring a contract, an
> implementation that `serves` it, calling through the slot, and
> the live swap — re-pointing the slot at a new implementation
> while the program runs, via `reperspective` (last section).

## The contract

A `perspective` declares bodyless method signatures — the stable
ABI a holder programs against:

```hale
perspective Router {
    fn route(code: Int) -> Int;
    fn health() -> Int;
}
```

## Serving a contract

A locus `serves` a perspective by providing every method it
declares — matching argument and return types. The compiler checks
this structurally, the way it checks interfaces; there's no
separate registration:

```hale
locus RouterV1 : serves Router {
    fn route(code: Int) -> Int { return code + 100; }
    fn health() -> Int { return 1; }
}
```

Miss a method, or get a signature wrong, and it's a compile error
pointing at the `serves` clause. `serves` shares the locus header's
post-`:` list with annotations, so `locus RouterV1 : serves Router,
tier 2` is fine.

## Holding and calling through a perspective

A holder programs against `perspective(Router)` — never a concrete
implementation. It designates the slot with a conforming impl, then
calls through it:

```hale
locus Gateway {
    params {
        router: perspective(Router) = RouterV1 { };
    }
    fn handle(code: Int) -> Int {
        return self.router.route(code);   // dispatched through the slot
    }
}
```

`self.router.route(...)` doesn't call `RouterV1` directly — it goes
through the perspective's **slot**. That indirection is the whole
point: it's the seam a future redeploy re-points.

## One slot, not many handles

Unlike an interface value (which is a fat pointer copied into each
holder), a perspective has exactly **one** program-global slot.
Every holder of `perspective(Router)` funnels through it. That's a
deliberate design choice: because there's a single indirection and
the compiler sees every call site, re-pointing that one slot will
redirect the entire program at once — a single pointer flip, no
matter how many holders. That live re-point is the `reperspective`
statement — see [Live redeploy](#live-redeploy-reperspective) below.

The cost is one load plus one predicted indirect call per call into
a perspective — near-direct. A program that declares no
perspectives pays nothing.

## Interface or perspective?

Both let you program against a set of method signatures. Reach for
an **interface** when you want many implementations coexisting, each
value carrying its own — a heterogeneous collection, a plugin list.
Reach for a **perspective** when there's one current implementation
behind a stable seam that the *system* owns and may redeploy — a
router, a storage engine, a policy. Interface is a value; a
perspective is a deployment slot.

## Live redeploy: `reperspective`

The whole point of the slot is that it can be re-pointed while the
program runs. `reperspective` does exactly that:

```hale
locus Gateway {
    params { router: perspective(Router) = RouterV1 { }; }
    run() {
        println(self.router.route(1));        // RouterV1
        reperspective self.router as RouterV2;
        println(self.router.route(1));        // RouterV2 — same call site
    }
}
```

The `self.router.route(...)` call didn't change. What changed is
what's behind the slot: `reperspective` instantiated a fresh
`RouterV2` and flipped the slot to it. Because every holder shares
the one slot, that single flip redirects the entire program at
once — no matter how many places call through the perspective.

A few rules:

- **You swap what you own.** `reperspective self.<field>` runs on
  the locus holding the slot; the new impl must `serve` the same
  perspective. A caller that merely *uses* a perspective can't
  redeploy it — redeployment authority is ownership.
- **State carries over.** The slot holds `{ data, vtable }` — the
  data *is* the running state, the vtable is the code. A swap
  replaces only the vtable, so the new impl picks up right where the
  old one left off, on the same live state:

  ```hale,fragment
  self.counter.bump();  self.counter.bump();   // count = 2 (V1)
  reperspective self.counter as CounterV2;      // redeploy
  println(self.counter.get());                  // still 2 — carried
  ```

  This is sound because every impl of a perspective must share the
  same **footprint** (same params, same types). A version that
  *changes* the footprint — adds a field, changes a type — can't
  reinterpret the old state, so it's a compile error today: that's
  the `migrate` case, a later slice.
- **Cost.** A swap is a single pointer store (the vtable). Nothing
  is re-instantiated and nothing is torn down — the state was never
  the code.

## Contracts can declare a bus surface

A perspective reached over the bus dispatches to *the current
impl's mailbox* — so the contract, the thing a swap is checked
against, can name its bus edges too, not just its sync methods:

```hale
type Order { id: Int; }

perspective OrderRouter {
    fn health() -> Int;                            // sync ABI
    bus { subscribe "orders" as on_order of type Order; }
}
```

A `serves` impl must provide every declared edge — the methods
*and* the bus subscriptions — checked structurally, the same way
it must provide the `fn`s. This keeps a perspective's full ABI in
one place.

And `reperspective` re-points the bus edges too. The swap
tombstones the current impl's subscriptions on the shared slot
state and re-registers the new impl's handlers on that same state,
so a message published *after* the swap lands on the new handler
while state the old handler accumulated carries across — the async
counterpart of the sync vtable flip:

```hale,fragment
"orders" <- Order { id: 1 };            // → the old impl's handler
reperspective self.router as OrderV2;   // redeploy, live
"orders" <- Order { id: 2 };            // → the new impl's handler
```

Cooperative bus dispatch is deferred (a publish enqueues a cell
capturing the handler current at that moment; handlers run when
the queue drains), so each message runs on whichever impl was live
when it was published — the swap boundary is respected.
