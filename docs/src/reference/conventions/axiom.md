# Types vs loci

## Synopsis

> **Types are for shapes. Loci are for flow.**

If a value has lifecycle, contracts, bus participation, or
projection, it is a [locus](../glossary.md#locus). If it is pure
data — record-shaped, returnable by value, no flow — it is a
[type](../glossary.md#type). There is no third category.

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

## Semantics

The axiom is not a tooling-enforced rule at v0 — the compiler
does not refuse to declare a `type` with bus participation,
because at v0 `type` declarations cannot carry methods or bus
blocks at all. The boundary is enforced by what each construct
*can* hold:

| Construct | Can hold |
|---|---|
| `type` | Fields. Nothing else. |
| `locus` | Fields (in `params`), lifecycle methods, regular `fn` methods, `bus` blocks, `closure` blocks, `on_failure`. |

If a thing wants methods, it is a locus. If a thing wants a
field set that another part of the program reads or copies but
never *runs*, it is a type. The distinction is what the value
*does*, not how complex it looks.

## Examples

```aperio
// Shape — pure data, returnable by value.
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
has fields and no flow → type; `MorphemeL` has methods → locus;
`ChatServerL` has lifecycle + bus → locus.

## See Also

- [Pattern catalog](./patterns.md) — the six shapes loci take.
- [Anti-patterns](./anti-patterns.md) — what reaching for
  `type` when you wanted `locus` (or vice versa) looks like.
- [Seeds and exports](./seeds.md) — only types and loci
  cross seed boundaries; free fns do not.
