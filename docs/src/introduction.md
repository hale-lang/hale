# Introduction

Hale is a compiled, general-purpose language. Every piece of state has one
owner, and the parts of a program talk only over declared, typed message
channels. That shape lets the compiler check a program's architecture the
way it checks its types. It runs natively with no garbage collector and no
borrow checker, and its concurrency has no locks and no data races.

A complete Hale program can be three lines: a `fn main()` that reads an
argument and prints. You can write useful Hale, such as CLI tools, file and
JSON work, and HTTP clients and servers, without declaring a lifecycle, a
topic, a placement or a claim. Those arrive when the program's growth earns
them, and nothing you wrote earlier changes when they do.

Most languages pick a level and live there. Python and JavaScript sit high:
fast to write, far from the metal. Go sits in the middle, with concurrency
in the language and a runtime underneath. Rust and C++ sit low: you own
memory and layout, and you pay attention to both.

Hale is one language you can write at any of those levels, moving between
them without switching tools. The same file can read like a script at the
top and like a systems program at the bottom. There is one construct, the
**locus**, and the only thing that changes as you go deeper is how much of
it you choose to see.

> **Try it now:** the [playground](https://play.hale-lang.org/) compiles and
> runs your Hale in the browser, with the same compiler you would install.
> The [example gallery](https://hale-lang.org/play/) walks curated programs
> chapter by chapter.

## Hale's words, in familiar terms

Hale uses a small vocabulary of its own. Each word names something you
already know, with one difference worth knowing:

| Hale says | You may know it as | The difference |
| --- | --- | --- |
| **locus** | an actor, a small service, an object nobody else can reach into | it is the only kind of component, so the compiler sees who owns what and who talks to whom |
| **topic** | a pub/sub topic, as in Kafka or NATS | it is declared in source with its payload type, so the compiler knows every sender and receiver |
| **main locus** | `main()` plus a deployment manifest | where each part runs and how each channel travels is one block in `main` |
| **claim** | an architecture test | a sentence about the whole program, checked over every path and compiled to no code |
| **witness** | a counterexample | the concrete path that breaks a rule, in your own names |
| **organism** | an application with its own pipeline, operations and audit log | all of it is one Hale program next to your code, and it changes only under review |

The [glossary](https://hale-lang.org/glossary) has every term.

## How this guide is laid out

The guide comes in six parts. Each opens with a page that says what the
layer is and what the next one adds:

- **The language:** values, math, functions, control flow, failure as a
  value, and everyday files, JSON, HTTP and tests. Hale at the level you
  would use Python or Node for.
- **The locus model:** loci that run over time, with a lifecycle, a typed
  message bus, placement, supervision, and programs split across binaries.
  Hale where you would use Go.
- **Saying what must hold:** effect contracts, claims over the whole
  program graph, and constitutions, which are promises the build checks
  before anything runs.
- **Systems control:** memory, layout, lifetime, zero-copy I/O, C,
  WebAssembly, and watching a program run. Hale where you would use Rust or
  C++.
- **The organism:** an application that is governed. It proposes its own
  changes, proves them, asks the people with authority, and applies
  exactly what they approve.
- **The habitat:** the design for what several organisms and their people
  share.

Each part expands on the one before it and contradicts none of it. The
function you wrote in *the language* still works in *systems control*; you
have only learned to see more of what was always there.

## A taste

Here is a small service. Each phrase you would say out loud has a place to
live, so the keywords can wait.

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

*"A matchmaker"* is `locus Matchmaker`. *"That receives players"* is
`subscribe JoinQueue`. *"And announces matches"* is `publish
MatchReady`. *"When enough are queued"* is the `if`. The code keeps the
shape of the sentence.

The gap between how you describe a system and what you type does not have
to be there; Hale is built on that bet. The [design](./the-design.md)
chapter explains why one shape works across the whole range, and across
people, language models and the machine.

## How to read this

Readers new to programming, or to systems languages, can start at **The
language** and go in order. Readers who already program can skim it for
the parts that differ from what they know (the failure model and the money,
time and unit types are worth a look), then jump to the part that matches
the program they want to write. Many chapters open with a short *"Coming
from X?"* box to orient you.

The [reference](./reference.md) points into `spec/`, the canonical
contract the compiler enforces, for the exact rules behind the tour.

Head to [Install](./getting-started/install.md) to set up the toolchain,
then [Your first run](./getting-started/first-run.md) to put a program on
screen.
