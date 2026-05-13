# Types vs loci

## Synopsis

> **Every named structural thing is a locus.** Types are
> loci-in-waiting — the smallest growth stage on the locus
> gradient.

If a value has lifecycle, contracts, bus participation, or
projection, it is a fully-grown
[locus](../glossary.md#locus). If it is pure data — record-
shaped, returnable by value, no flow — it is a
[type](../glossary.md#type), which is a locus whose growth
stopped at shape. The two are points on one gradient, not
separate categories.

For the full framing — the gradient stages, the three axes
of a locus (capacity, projection, form), and the locked-in
form behavior — see [Design philosophy](./design-philosophy.md).
This page is the entry point.

## The locus gradient

Each stage strictly extends the previous; you can't have a
higher stage's mechanic without the lower stages' shape.

| Stage | Surface | Adds |
|---|---|---|
| Pure shape | `type T { fields }` | name + fields |
| Parametric shape | `type T<G> { fields }` | + generics |
| Tagged shape | `type T { Variants }` | + enum variants |
| Identity | `locus L { params {} }` | + arena, tower position |
| Substrate | `locus L { capacity { ... } }` | + F.22 storage discipline |
| Behavior | `locus L { params {} run() {} }` | + lifecycle bodies |
| Audit | `locus L { closure {} }` | + closure assertions |
| Recovery | `locus L { on_failure(c, err) {} }` | + F.9 failure routing |
| Cross-process | `locus L { bus { ... } }` | + bus participation |
| Interop | `perspective P of L { ... }` | + parametric reflection |

The keyword distinction (`type` vs `locus`) is ergonomic sugar
— the parser treats them as one construct with different
default starting points.

## The recursive principle

Loci are the fundamental building block at every layer:

- An app is a locus. (Outer encapsulation; one per
  `apps/<name>/main.ap`.)
- A namespace of pure helpers is a locus. (Empty `params { }`,
  only methods.)
- A long-running service is a locus. (`birth` / `run` /
  `dissolve`; often with `bus subscribe`.)
- A spawned async worker is a locus. (Child of its parent;
  cooperative schedule by default.)
- A bus subscriber is a locus. (HTTP route, message handler,
  event listener — all the same shape.)
- A configured cache, pool, pipeline, or queue is a locus.

Inside any locus, behavior is itself a locus tower one layer
down. A cache's lookup flow has its own birth (acquire lock),
run (probe + return), dissolve (release lock). The recursion
bottoms at primitive operations — arithmetic, single field
reads, primitive calls. Everything above the floor is loci
nested in loci.

## What's NOT a locus

Four exclusions, exhaustively:

- **Primitives.** `Int`, `Float`, `Bool`, `Decimal`, `Time`,
  `Duration`, `String`, `Bytes`. The atomic value layer
  beneath locus.
- **Functions.** Pure mappings from inputs to outputs. No
  identity, no lifecycle, no position in the tower.
- **Generic parameters.** Placeholders bound at
  monomorphization, not loci.
- **Seeds.** The directory-level unit of source organization;
  a grouping over loci, not a super-locus.

Every other named structural construct in source code is a
locus.

## Semantics

The axiom is not tooling-enforced at v0 — the compiler does
not refuse to declare a `type` with bus participation, because
at v0 `type` declarations cannot carry methods or bus blocks
at all. The boundary is enforced by what each construct
*can* hold:

| Construct | Can hold |
|---|---|
| `type` | Fields. Nothing else. (Smallest growth stage.) |
| `locus` | Fields (in `params`), lifecycle methods, regular `fn` methods, `capacity` blocks, `bus` blocks, `closure` blocks, `on_failure`, perspectives. |

If a thing wants methods, it is a locus. If a thing wants a
field set that another part of the program reads or copies but
never *runs*, it is a type. The distinction is what the value
*does*, not how complex it looks.

## Examples

```aperio
// Pure shape — proto-locus, returnable by value.
type Message {
    sender: String;
    body: String;
    ts: Time;
}

// Flow — a namespace of pure helpers. Empty params; only methods.
locus MorphemeL {
    params {
        flavor: String = "go";
    }
    fn lookup(m: String) -> String {
        // ...
    }
}

// Flow — a long-lived service. Full lifecycle plus bus.
locus ChatServerL {
    params {
        host: String = "127.0.0.1";
        port: Int = 8080;
    }
    bus {
        subscribe "chat.message" as on_message of type Message;
    }
    birth() { /* bind listener */ }
    run() { /* accept loop */ }
    dissolve() { /* close fd */ }
    fn on_message(m: Message) { /* ... */ }
}
```

A reader can classify each of the three on sight: `Message`
has fields and no flow → proto-locus (type); `MorphemeL` has
methods → locus; `ChatServerL` has lifecycle + bus → locus.

## See Also

- [Design philosophy](./design-philosophy.md) — the full
  everything-is-a-locus framing, the three axes (capacity /
  projection / form), and the locked-in v1 form behavior.
- [Pattern catalog](./patterns.md) — the six shapes loci take.
- [Anti-patterns](./anti-patterns.md) — what reaching for
  `type` when you wanted `locus` (or vice versa) looks like.
- [Seeds and exports](./seeds.md) — only types and loci
  cross seed boundaries; free fns do not.
