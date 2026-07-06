# Perspectives

A **perspective** is a live-rebindable handle to a *contract*. You
program against the contract — a set of method signatures — and
reach the real implementation only through an indirection the
system can re-point underneath you. It's the seam that lets you
swap a component's implementation without the code that calls it
changing.

> This chapter covers what ships today: declaring a contract, an
> implementation that `serves` it, and calling through the slot.
> The live swap (re-pointing the slot at a new implementation
> while the program runs) is the next slice.

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
matter how many holders. (That live re-point is the next slice;
today the slot is set once, at designation.)

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
