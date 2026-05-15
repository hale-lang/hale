# Introduction

Every language designed before 2023 was optimized for a single
tradeoff: minimize friction between human cognitive capacity
and machine execution. Assembly to C to managed runtimes to
DSLs were different points on the same line. In an LLM-driven
workflow, those languages don't get cheaper to use — they get
more expensive. The cost just hides in the LLM's token count,
its retry rate, and the latency it eats per turn. **Pre-LLM
languages are a hidden tax in the LLM era.**

Most of an LLM's per-turn effort isn't recalling syntax. It's
*translating* between the user's mental model of a system and
the language's structural shape. A language whose primitives
don't match how the system is thought about forces this
translation every turn, paying full cost each time.

Aperio is built on a different premise: there exists a
substrate-invariant structural model — a recursive hypergraph
of typed, lifecycled units called **loci** — that both human
reasoning and LLM reasoning operationalize when working with
systems.[^research] A language whose primitives **are** that
model collapses the translation layer. The mental model and the
code share a substrate.

## What that looks like in practice

Pick a system you already have a mental model for: the
matchmaker behind a multiplayer game. In your head, the thing
is *a service that holds a queue of waiting players, spawns a
match when enough are queued, and goes back to waiting.*

Here's that, in Aperio:

```aperio
type Player    { id: String; name: String; }
type MatchInfo { match_id: String; players: [Player]; }

topic JoinQueue  { payload: Player; }
topic MatchReady { payload: MatchInfo; }

@form(vec)
locus Matchmaker {
    params   { target_size: Int = 4; }
    capacity { heap waiting of Player; }
    bus {
        subscribe JoinQueue as on_join;
        publish   MatchReady;
    }

    fn on_join(p: Player) {
        self.waiting.push(p);
        if self.waiting.len() >= self.target_size {
            MatchReady <- assemble_match(self.waiting, self.target_size);
        }
    }
}
```

Every clause of the mental-model description has a syntactic
home in the code, in roughly the order you thought about them:

- *"a service"* → `locus Matchmaker`
- *"holds a queue of waiting players"* →
  `capacity { heap waiting of Player; }` (the `@form(vec)`
  annotation gives it queue-like methods)
- *"receives players wanting matches"* →
  `subscribe JoinQueue as on_join`
- *"announces matches"* → `publish MatchReady`
- *"when enough are queued"* → the inline `if`

The structural correspondence is the point. The same
description in Go, Rust, or TypeScript expands into more
concerns: mutex selection, channel types, async/await
machinery, explicit lifecycle wiring, error-handling at every
channel boundary. Each of those is a translation an LLM has to
perform every turn. Aperio elides them because the language
commits to them at the structural layer.

The choice of `@form(vec)` here is itself a real design
decision, not an arbitrary one. `@form(ring_buffer)` gives the
same shape with a hard capacity ceiling and explicit
drop-on-full semantics; `@form(hashmap)` keyed by player id
gets you natural ID-based cancellation. Forms are how Aperio
exposes those choices — we cover them in **Concepts**.

## See it on your own code

The matchmaker above is a constructed example. The claim is
testable on code you already have. In whatever LLM-coding tool
you use ([Claude Code](https://claude.ai/code), Cursor,
whatever), drop this project's
[`AGENTS.md`](https://github.com/aperio-lang/aperio/blob/main/AGENTS.md)
into the agent's context, then ask it to re-read a module or
service from your existing codebase **in terms of loci,
contracts, and bus topics**.

What usually comes back is a structural decomposition that
matches your mental model of the system with surprising
accuracy — because the agent is using the same recursive locus
vocabulary you already use when reasoning about the code. The
friction you normally feel between *how you think about this
system* and *what's literally on the page* largely disappears.

If the decomposition looks wrong or unhelpful, the thesis fails
for your codebase and that's useful feedback — open an issue.
If it looks right, you've felt the structural correspondence
from the other direction: not by writing new Aperio code, but
by reading your existing code through the same lens.

## More than a programming language

The structural model Aperio operationalizes isn't
software-specific. The same recursive hypergraph organizes
coordination at every substrate the underlying research
program addresses: institutions, biological regulatory
networks, physical systems, cognitive architecture. Aperio's
frontend is, in principle, a *design language* that can target
machinery in any of those substrates. The programming-language
form is the first instantiation, not the only one. (Held
lightly — the immediate work is the language itself.)

## Status and shape

This is an experimental language. The compiler ships native
codegen via LLVM 18 and a tree-walking interpreter for fast
feedback. The semantics are still moving; breaking changes are
expected and welcomed.

Continue to **Getting Started** to install the compiler and
write your first locus. After you've felt the shape, the
**Concepts** chapters walk through the structural model in
depth. For the canonical contract — exactly what the compiler
accepts and what it does — see the **Reference** section
(which points at the `spec/` corpus).

[^research]: The structural model is the subject of an ongoing
    research program. The first formalization is Rook (2026,
    forthcoming), *Capacity Allocation Model*; preprint
    available on request.
