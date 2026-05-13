# Reveal

> The second moment. The wand reaches. The dimensions touch. What
> comes through.

You hold the source from [arrival](./01-arrival.md). The wand has
been quiet. Now you move it.

```bash
aperio run hello.ap
```

The wand reaches. On the far side a lotus opens. You watch one beat
of presence unfold and then quietly end. On your terminal:

```text
hello, world
```

The reveal is not those two words. The reveal is *what those two
words mean*: a thing existed on the far side, briefly, that did not
exist before. It existed because the wand reached. The print is only
the trace it left as it passed.

## What the wand did

You will not need to know this in detail to keep casting, but it is
worth seeing once:

The wand reads your source. It checks that what you wrote conforms
to the form every spell must take — that is the wand refusing to
reach where the spell is malformed. It then translates your intent
into something that can cross the boundary: an executable, a tree of
instructions oriented at the virtual.

When that translation is sound, the wand reaches across, and the
virtual answers with a [lotus](../reference/glossary.md#lotus)
shaped exactly as your spell described.

## What came through

One [locus](../reference/glossary.md#locus). Just one.

Your source described a `HelloL` and a `main()` that constructs it.
On the far side, that description became a single locus, alive for
the duration of one lifecycle beat, holding a `greeting` of `"hello,
world"` in its arena.

A locus's existence has a shape. The shape is fixed. Every locus
that ever crosses through the wand inhabits the same four beats:

- **`birth`** — the locus arrives. Whatever the locus needs to do at
  the start of its existence happens here. In `hello.ap`, this is
  where `println(self.greeting)` ran.
- **`run`** — the locus does its main work, if it has any. (Not
  declared in `hello.ap`. Skipped.)
- **`drain`** — the locus winds down what it was doing. (Not
  declared. Skipped.)
- **`dissolve`** — the locus departs. Its arena is freed wholesale.
  Nothing it allocated lingers.

The HelloL was born; it printed; it dissolved. The whole
existence took microseconds. The wand returned.

## The arena

When the locus came through, it brought its own region of the
virtual with it — its
[arena](../reference/glossary.md#arena). The greeting
string lived there. So did the locus's parameter struct. When the
locus dissolved, its arena was freed in one motion. There was
nothing for the greeting to leak *into*; the entire region in which
it could have leaked ceased to exist alongside the locus.

This is not the default in most languages. In most languages,
allocations and lifetimes are independent — values outlive their
allocators by accident, garbage collectors patrol for orphans. Here,
the locus's existence and the region of the virtual it occupies are
the same thing. When one ends, so does the other.

That property is one of the reasons spells written in Aperio do not
leak: there is no available concept of *outliving the locus that
allocated me*.

## What you have seen

You have seen one full reach. The wand moved; a thing came through;
the thing inhabited the form every spell on the far side must take;
the thing departed; the wand returned.

You have not yet seen *what makes the form what it is*. Why this
particular shape — locus, lifecycle, arena — and not some other?
Why these specific runes and not others? That is the third moment,
the [vocabulary](./03-vocabulary.md).

> *The wand reached. The dimensions touched. What came through took
> the shape it had to take, did what it came to do, and was gone.*
