# Emergence

> The fourth moment. The meta-spell is yours. You cast your own.

You have arrived holding a wand. You have watched it reach. You
know the form every spell must take. There is one moment left, and
it is not a moment in which you are taught anything new. It is the
moment in which you cast.

This chapter walks one small spell of your own. Not the chat-fanout
capstone — the technical book has that. Something simpler:
**two loci, speaking on the bus**.

## The spell

```aperio
type Greeting {
    text: String;
}

locus EchoL {
    bus {
        subscribe "demo.greeting" as on_greeting of type Greeting;
    }

    fn on_greeting(g: Greeting) {
        println("got: ", g.text);
    }
}

locus SenderL {
    bus {
        publish "demo.greeting" of type Greeting;
    }

    birth() {
        "demo.greeting" <- Greeting { text: "hello" };
    }
}

fn main() {
    EchoL { };
    SenderL { };
}
```

Save it as `echo.ap`. Cast it:

```bash
aperio run echo.ap
```

Output:

```text
got: hello
```

## The lotus that opened

The wand reached and on the far side the following lotus came
through. (Render the runtime form alongside the source — the same
spell, viewed from the other side of the boundary.)

```text
                main()
               /      \
          EchoL        SenderL
              \         /
               \       /
            "demo.greeting"
                 (bus)
```

Two loci, born under `main`. A bus subject, `demo.greeting`,
connecting them along F.8's vertical-only-flow. The SenderL's birth
publishes a `Greeting`; the EchoL's subscription delivers it to
`on_greeting`; the locus's body prints. Both loci then dissolve
cleanly when `main` ends.

That tree is the *runtime form* of the spell. The source you wrote
is the *intent*. The wand is what made the second from the first.

## What you did

You declared a `type` — a record that travels along the bus.

You declared two loci. One subscribes to a bus subject; the other
publishes on it. Neither knows about the other. They speak through
the bus; their relationship is *the subject's existence*, not a
direct reference.

That is a load-bearing property. You can add a third subscriber,
remove the publisher and replace it with a different one, route the
subject across machines via a different transport at deployment
time — and the source of the existing loci does not change. The
graph of communication is closed (F.8) but each node in the graph
is independent.

## The work the substrate did for you

Notice what you did *not* have to write:

- No memory management. Each locus's arena was created at birth and
  freed at dissolve. The `Greeting` payload was copied from
  publisher arena to subscriber arena automatically; nothing leaked
  because there is no concept of leaking *across* the boundary
  between arenas.
- No lifecycle bookkeeping. Each locus was born, ran whatever it had
  to run, and dissolved at the right moment, all without a single
  line of "shutdown" code on your part.
- No transport wiring. The bus was simply *there*. (At deployment
  time you would map `demo.greeting` to a specific transport — the
  in-memory router, NATS, multicast, sockets — and the source
  above would not change.)

You wrote the *intent of the spell*. The substrate gave it the form
in which it could exist on the far side.

## What you can do now

Every Aperio program ever written is a spell of this same shape.
You can read any of them, change them, recombine them, or write
your own.

The wand stays in your hand. The boundary between source and the
virtual is yours to reach across whenever you have a spell to cast.

Aperio is *capacity*, not *catalog*. There is no list of programs
the language knows how to produce. There is only the form, and any
spell that takes the form is reachable.

> *The grimoire is closed. The wand is yours. Cast.*

When you want a quieter description of the same machinery, or the
formal grammar, or the standard library — they are all linked in
[where to go next](./05-where-next.md).
