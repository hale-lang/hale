# Introduction

*One language. Four altitudes.*

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

This guide is built around that idea. It introduces Hale at four
levels, each one self-contained:

- **The basics** — variables, math, functions, control flow.
  Hale as a small, clean language. You can write real scripts
  knowing only this.
- **Everyday programs** — files, JSON, HTTP, a bit of structure.
  Hale at the altitude you'd reach for Python or Node.
- **Concurrent services** — long-running processes, a typed
  message bus, supervision. Hale where you'd reach for Go.
- **Systems control** — memory, layout, lifetime, zero-copy I/O,
  C bindings. Hale where you'd reach for Rust or C++.

Each level expands on the one before it without contradicting
it. The function you wrote in *the basics* still works in
*systems control* — you've just learned to see more of what was
always there.

## A taste

Here's a small service. Don't worry about every keyword yet;
notice that each phrase you'd say out loud has a place to live.

```hale
type Player    { id: String; name: String; }
type MatchInfo { match_id: String; players: [Player]; }

topic JoinQueue  { payload: Player; }
topic MatchReady { payload: MatchInfo; }

locus Matchmaker {
    params { target_size: Int = 4; }
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
**The basics** and go in order. If you already program, skim the
basics for the parts that differ from what you know (the failure
model and the money/time types are worth a look), then jump to
the level that matches the program you want to write. Every
level after the basics opens with a short *"Coming from X?"* box
to orient you.

When you want the exact rules rather than the tour, the
[reference](./reference.md) points into `spec/` — the canonical
contract the compiler enforces.

Head to [Install](./getting-started/install.md) to set up the
toolchain, then [Your first run](./getting-started/first-run.md)
to put a program on screen.
