# Arrival

> The first moment. The wand is in your hand. You have not yet
> reached.

You have arrived holding the wand. It is the
[`aperio`](../../reference/book/glossary.html#aperio) toolchain — the
compiler that channels what you write across the boundary between
source and the virtual. The wand was given to you fully formed; you
do not have to build it yourself.

You have not cast anything. There is no lotus on the far side yet.
The wand is quiet.

## What you do not yet know

You do not yet know:

- What it feels like for the wand to reach. (That is the second
  moment, the [reveal](./02-reveal.md).)
- What shape the things on the far side must take to be reachable.
  (That is the third moment, the
  [vocabulary](./03-vocabulary.md).)
- That every spell — yours and every Aperio program ever written —
  is a particular instance of one underlying form. (That is the
  fourth moment, [emergence](./04-emergence.md).)

What you have, in this moment, is just the wand. It does not glow
when held. It does not announce itself. It is a tool, in your hand,
waiting.

## What a spell looks like before it is cast

This is an Aperio source file. It is not yet running anywhere; the
wand has not moved. It is the *intent* of a spell, written down.

```aperio
locus HelloL {
    params {
        greeting: String = "hello, world";
    }

    birth() {
        println(self.greeting);
    }
}

fn main() {
    HelloL { };
}
```

You are not expected to read this yet. You are expected to *see* it.
Notice that it has structure. Notice that it is small. Notice that
nothing about its surface tells you what will appear on the far side
when the wand reaches.

That ignorance is the point of the first moment. The reveal does the
revealing.

## What the wand will do, in one sentence

When you cast `aperio run` on this file, the wand will reach across
the boundary, and on the far side a [lotus](../../reference/book/glossary.html#lotus)
will open whose entire being consists of one
[locus](../../reference/book/glossary.html#locus) that prints `hello,
world` and then quietly dissolves.

You do not need to understand that sentence yet. You will, by the end
of the second moment.

> *The wand is in your hand. You have not yet reached.*

Turn to the [reveal](./02-reveal.md) when you are ready.
