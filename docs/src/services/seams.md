# Seams

Most programs that talk to the outside world have an edge with three
jobs: turn bytes into a message, act on the message, and turn the
result back into bytes. The three are easy to write and easy to tangle.
The decoder starts to know which handler wants what, the handler starts
to know how the wire spells things, and the dispatch between them grows
a default arm where a new kind of message quietly goes missing.

A **seam** keeps the three apart. You write the decoder, the handler and
the encoder as ordinary loci, and one line says how they connect. The
compiler writes the dispatch and checks that every kind of message has
exactly one place to go.

## The pieces

A seam works on [sets](../everyday/records.md#sets--one-of-several-types):
a type that is one of several record types, each member its own tag.
Start with the messages:

```hale,fragment
type NewOrder { id: Int; qty: Int; }
type Cancel { id: Int; }
type Ack { id: Int; }
type Reject { id: Int; why: String; }
type WireError { at: Int; }

type Inbound = NewOrder | Cancel;
type Outbound = Ack | Reject | WireError;
```

The decoder and the encoder know the bytes and nothing about the desk:

```hale,fragment
locus Wire {
    fn decode(frame: String) -> Inbound fallible(WireError) {
        if frame == "cancel" { return Cancel { id: 1 }; }
        if std::str::starts_with(frame, "new ") {
            let qty = std::str::parse_int(frame[4..len(frame)]) or { fail WireError { at: 4 }; };
            return NewOrder { id: qty * 10, qty: qty };
        }
        fail WireError { at: 0 };
    }
    fn encode(o: Outbound) -> String {
        return match o {
            Ack(a) -> "ack " + to_string(a.id),
            Reject(r) -> "reject " + to_string(r.id) + " " + r.why,
            WireError(e) -> "bad frame at " + to_string(e.at),
        };
    }
}
```

The handler knows its own types and nothing about the wire. It has one
method per kind of message it takes:

```hale,fragment
locus Desk {
    params { open: Int = 0; }
    fn new_order(n: NewOrder) -> Outbound {
        if n.qty > 10 { return Reject { id: n.id, why: "too big" }; }
        self.open = self.open + 1;
        return Ack { id: n.id };
    }
    fn cancel(c: Cancel) -> Outbound {
        self.open = self.open - 1;
        return Ack { id: c.id };
    }
}
```

## Stitching them

The seam lives in a locus that holds the pieces, and reads left to
right:

```hale,fragment
locus Counter {
    params { wire: Wire = Wire { }; desk: Desk = Desk { }; }
    seam answer(frame: String) -> String {
        frame -> self.wire.decode -> self.desk -> self.wire.encode;
    }
}

fn main() {
    let c = Counter { };
    println(c.answer("new 3"));
    println(c.answer("new 50"));
    println(c.answer("cancel"));
    println(c.answer("hello"));
    println("open: ", c.desk.open);
}
```

```text
ack 30
reject 500 too big
ack 1
bad frame at 0
open: 0
```

`answer` is a method like any other: it takes a frame and returns the
encoded reply. Each step after the arrow is a **hop**:

- `self.wire.decode` and `self.wire.encode` are **method hops**: the
  value goes to that one method.
- `self.desk` is a **locus hop**: the value is a set, and each member
  goes to the `Desk` method whose parameter is that member. A
  `NewOrder` reaches `new_order` and a `Cancel` reaches `cancel`. That
  is the dispatch you did not write.

Nothing in `Wire` names `Desk`, and nothing in `Desk` names `Wire`. The
seam is the only place they meet.

## Every message has one place to go

The compiler checks the dispatch at every boundary, both ways. Delete
`Desk::cancel` and the seam is refused where the `Cancel` would have
been dropped:

```text
type error: `self.desk` has no method taking `Cancel`, which reaches it from `self.wire.decode`: every member has one eater
            frame -> self.wire.decode -> self.desk -> self.wire.encode;
                                         ^^^^^^^^^
```

Two methods that both take `Cancel` are refused the same way, naming
both. At a method hop, every member arriving must be one the parameter
holds. So adding a member to `Inbound` fails the build until the desk
handles it, which is the point: the hand-written version of this
dispatch is a `match` whose `_` arm is where the new member goes
missing at run time. (A refused seam's method does not exist, so its
callers are refused too, with "no field `answer`". Fix the seam first.)

## Failures are answers

`decode` can fail with a `WireError`. Because `WireError` is a member of
`Outbound`, the failure enters the chain at the first later hop that
takes it, which is `encode`, and becomes an ordinary reply: `bad frame
at 0` above. A protocol error answered on the wire is usually exactly
what you want.

When no later hop takes a failure, the seam has to say what happens to
it, or it is refused:

```text
type error: `self.wire.decode` fails with `WireError`, and no later hop takes it: add `WireError` to a later hop's set, or route it with `or end;` or `or violate <closure>;`
```

`or end;` ends the flow: a callable seam such as `answer` then fails
with the error, so its caller writes `c.answer(frame) or …`. `or violate
NAME;` violates the locus's closure `NAME` instead, which hands the
failure to the parent's `on_failure` like any other [structural
failure](./failure.md).

## A seam that runs by itself

Give the chain a source and a sink and it needs no caller. The source
is a call that makes each frame, and the sink a method that takes the
last value and returns nothing:

```hale,fragment
type Ended { why: String; }

locus Feed {
    params { at: Int = 0; }
    fn next() -> String fallible(Ended) {
        self.at = self.at + 1;
        if self.at == 1 { return "new 2"; }
        if self.at == 2 { return "junk"; }
        if self.at == 3 { return "cancel"; }
        fail Ended { why: "no more frames" };
    }
}

locus Log {
    fn write(line: String) { println(line); }
}

locus Gateway {
    params { feed: Feed = Feed { }; wire: Wire = Wire { }; desk: Desk = Desk { }; log: Log = Log { }; }
    seam {
        self.feed.next() -> self.wire.decode -> self.desk -> self.wire.encode -> self.log.write;
    }
}
```

```text
ack 20
bad frame at 0
ack 1
```

This closed seam *is* the locus's `run`: while the locus is not
draining, it pulls a frame and runs the chain, and a failing source
(`Ended`) ends the flow by itself; `or end;` and `or violate` are for the
failures of the hops after it. A locus with a closed seam declares no
`run` of its own; writing both is refused.

Each frame is one turn of that `run`: the compiler writes the frame
(pull, decode, dispatch, encode, write) as a method of its own, which
`run` calls until the source ends. So what a frame makes is freed when
the frame is done, and a gateway that runs for hours holds flat.

## One tower

A seam is wiring inside one locus tower. Its hops are the locus's own
methods and its children's, called directly; each call is inlined, a
member keeps its own static type from the decoder to the encoder, and
no union is built that the program did not declare. Two shapes are
refused because they would break that:

- **A hop on an interface.** The method it would call is not known
  until run time, and a seam's chain is static.
- **A hop on a child placed on another pool.** Crossing pools is the
  bus's job: publish the decoded message to a topic, and let the other
  pool's locus subscribe.

## Your API already has one

When you serve a [surface](./api.md), every row is a seam the compiler
writes on its receiver: one decoder turns the request body into the
row's request type (JSON, or protobuf on gRPC), the dispatch hands it to
your method, and one encoder writes the answer, your error, or the
server error a violating handler gets. That is why an API handler is
just a method over its own types, and why none of that plumbing shows
up in what callers see.

## Where it is specified

- Seams: `spec/semantics.md` § Seams.
- Sets: `spec/types.md` § Set types.
- The style guide's rules for the shape: `spec/styleguide.md` 2.9 and
  2.10.
