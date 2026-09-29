# Introduction

**A general-purpose language with a GC-free native runtime — typed
message-bus concurrency, data-race-free by design.**

*Write the system, not the lore.*

A complete Hale program can be three lines — a `fn main()` that
reads an argument and prints. You can write useful Hale — CLI
tools, file and JSON work, HTTP clients and servers — without
declaring a lifecycle, a topic, a placement, or a claim. Those
constructs arrive when the program's growth earns them, and
nothing you wrote earlier changes when they do.

Most languages pick a level and live there. Python and
JavaScript sit high — fast to write, far from the metal. Go
sits in the middle — concurrency in the language, a runtime
underneath. Rust and C++ sit low — you own memory and layout,
and you pay attention to both.

Hale is a single language you can write at any of those levels,
and move between them without switching tools. The same file
can read like a script at the top and like a systems program at
the bottom. There is one primitive — the **locus** — and the
only thing that changes as you descend is how much of it you
choose to see.

> **Try it now:** the [playground](https://play.hale-lang.org/) compiles
> and runs your Hale right in the browser — no install, same compiler
> you'd install locally. For a guided start, the
> [example gallery](https://hale-lang.org/play/) walks curated programs
> chapter by chapter.

This guide is built around that idea. It comes in six parts,
each one opening with a page that says what the layer is and
what the next one adds:

- **The language** — values, math, functions, control flow,
  failure as a value, and everyday files, JSON, HTTP and tests.
  Hale at the altitude you'd reach for Python or Node.
- **The locus model** — loci that run over time: a lifecycle, a
  typed message bus, placement, supervision, programs split
  across binaries. Hale where you'd reach for Go.
- **Saying what must hold** — effect contracts, claims over the
  whole program graph, and constitutions: promises the build
  checks before anything runs.
- **Systems control** — memory, layout, lifetime, zero-copy I/O,
  C, WebAssembly, and watching a program run. Hale where you'd
  reach for Rust or C++.
- **The organism** — an application that is governed: it
  proposes its own changes, proves them, asks the people with
  authority, and applies exactly what they approve.
- **The habitat** — the design for what several organisms and
  their people share.

Each part expands on the one before it without contradicting
it. The function you wrote in *the language* still works in
*systems control* — you've just learned to see more of what was
always there.

## A taste

Here's a small service. Don't worry about every keyword yet;
notice that each phrase you'd say out loud has a place to live.

```hale
type Player    { id: String; name: String; }
type MatchInfo { match_id: String; size: Int; }

topic JoinQueue  { payload: Player; }
topic MatchReady { payload: MatchInfo; }

locus Matchmaker {
    params {
        target_size: Int = 4;
        queued:      Int = 0;
    }
    bus {
        subscribe JoinQueue as on_join;
        publish   MatchReady;
    }

    fn on_join(p: Player) {
        self.queued = self.queued + 1;
        if self.queued >= self.target_size {
            MatchReady <- MatchInfo { match_id: p.id, size: self.queued };
            self.queued = 0;
        }
    }
}
```

*"A matchmaker"* → `locus Matchmaker`. *"That receives players"*
→ `subscribe JoinQueue`. *"And announces matches"* → `publish
MatchReady`. *"When enough are queued"* → the `if`. The code
keeps the shape of the sentence.

That's the bet behind Hale: the gap between *how you describe a
system* and *what you type* doesn't have to be there. The
[design](./the-design.md) chapter explains why one shape works
across the whole range — and across human, LLM, and machine.

## How to read this

If you're new to programming or to systems languages, start at
**The language** and go in order. If you already program, skim
it for the parts that differ from what you know (the failure
model and the money/time types are worth a look), then jump to
the part that matches the program you want to write. Many
chapters open with a short *"Coming from X?"* box to orient
you.

When you want the exact rules rather than the tour, the
[reference](./reference.md) points into `spec/` — the canonical
contract the compiler enforces.

Head to [Install](./getting-started/install.md) to set up the
toolchain, then [Your first run](./getting-started/first-run.md)
to put a program on screen.
