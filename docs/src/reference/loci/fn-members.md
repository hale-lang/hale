# Fn members

## Synopsis

A locus body may declare `fn` members — methods callable on
locus values, including bus handlers and helper functions. Fn
members run on the locus's arena and have access to `self`.

## Grammar

```text
fn-member ::= "fn" snake_case-Ident generic-params?
                "(" param-list? ")" return-type? block

param-list  ::= param ("," param)*
param       ::= snake_case-Ident ":" type-expr
return-type ::= "->" type-expr
```

The implicit `self` parameter is *not* listed in the parameter
list; it is in scope inside any fn member's body.

## Example

```aperio
locus EchoL {
    bus {
        subscribe "demo.greeting" as on_greeting of type Greeting;
    }

    fn on_greeting(g: Greeting) {
        // bus handler — name matches the subscribe binding
        println("got: ", g.text);
    }

    fn shout(s: String) -> String {
        // helper — callable from other methods on this locus
        return "!! " + s + " !!";
    }
}
```

## Categories

### Bus handlers

A fn whose name matches a `subscribe SUBJECT as NAME` declaration
is the handler for that subject. It must take exactly one
parameter — the typed payload — and return unit (`-> ()` or
omitted return type).

The runtime calls the handler each time a message arrives.

### Helper methods

A fn whose name does not match a bus subscription is a helper
method. It may take any number of parameters of any type and
return any type. Callable from other methods on the same locus
or, if the locus exposes the value (via `expose`), from a
parent.

### Translation implementations (F.14)

Per **F.14**, a fn member that satisfies a contract entry must
return the contract's declared type. Multi-implementation
syntax (e.g. `@projection rich fn foo() -> T`) is deferred to
post-v1; in v0, the param of the same name is the implicit
implementation.

## Semantics

### `self` access

Every fn member can read `self.field` and (for mutable
parameter fields, per **F.3**) write `self.field = expr`.

```aperio
locus Counter {
    params {
        n: Int = 0;
    }

    fn increment() {
        self.n = self.n + 1;
    }
}
```

### Memory

Allocations made inside a fn member's body live in the locus's
arena. Strings, struct values, and other heap-typed
allocations are freed wholesale when the locus dissolves.

### No first-class methods (v0)

Methods are not first-class values in v0; you cannot pass a
method as a function-typed argument. Function types
(`fn(A) -> B`) are accepted in type position and as values for
free fns, but locus methods are bound to their locus.

## Free fns vs locus fns

A `fn` declared at top-level (outside any locus body) is a
*free fn*. Free fns get their own per-call subregion of the
caller's arena (per the m49+m51+m53 free-fn-arena arc); they
have no `self`.

```aperio
fn shout(s: String) -> String {
    return "!! " + s + " !!";   // result deep-copied to caller's arena
}
```

A fn declared inside a locus body is a locus fn — it has
`self`, runs on the locus's arena, and is named by its locus's
type.

## See Also

- [Locus declarations](./index.md)
- [Bus blocks](./bus.md)
- [Memory model](../memory.md)
