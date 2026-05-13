# Bus blocks

## Synopsis

A locus's `bus { ... }` block declares its bus interface — the
set of subjects it subscribes to and publishes on. The compiler
verifies the locus's source is consistent with the declared
interface; any `<-` to a subject the locus did not declare
`publish` for is a compile-time error.

## Grammar

```text
bus-block ::= "bus" "{" bus-decl* "}"
bus-decl  ::= subscribe-decl | publish-decl

subscribe-decl ::= "subscribe" string-literal "as" snake_case-Ident
                       "of" "type" type-expr ";"
publish-decl   ::= "publish" string-literal "of" "type" type-expr ";"
```

## Example

```aperio
type Greeting {
    text: String;
    sender: String;
}

type Acknowledgment {
    received: String;
}

locus EchoL {
    bus {
        subscribe "demo.greeting" as on_greeting of type Greeting;
        publish   "demo.ack"               of type Acknowledgment;
    }

    fn on_greeting(g: Greeting) {
        println("got: ", g.text, " from ", g.sender);
        "demo.ack" <- Acknowledgment { received: g.text };
    }
}
```

## Semantics

### Subscription

`subscribe SUBJECT as HANDLER of type T` binds the bus subject
named `SUBJECT` (a string literal) to a method named `HANDLER`
on the same locus. The method must be a `fn` member with the
signature:

```aperio
fn HANDLER(payload: T) { ... }
```

When the runtime delivers a message of type `T` on `SUBJECT`,
the handler runs with the payload as its argument. The handler
runs on the subscriber's arena (its argument is a copy that
lives in the subscriber's region of memory).

### Publication

`publish SUBJECT of type T` declares that the locus may emit
messages of type `T` on `SUBJECT`. The compiler verifies every
`<-` statement in the locus body uses a subject the locus
declared `publish` for, and that the right-hand side has the
declared type.

### Subject typing

Every subject carries exactly one type. The compiler verifies
that all `subscribe` and `publish` declarations naming the same
subject across the program use the same type. A subject named
`"demo.greeting"` cannot be `Greeting` in one locus and a
different type in another — that is a compile-time error.

### Subscription registration timing

Subscriptions register at the end of the locus's `birth()`
body. Until `birth` runs, a locus is not a subscriber on any
subject; messages published on the subject before then are not
delivered to it.

In v0, this means startup ordering matters: a publisher's
`birth` should not fire before the subscribers' have. The
typical pattern is to construct subscribers first, then
publishers, in `main`:

```aperio
fn main() {
    EchoL { };          // subscribes to "demo.greeting"
    AckLogL { };        // subscribes to "demo.ack"
    SenderL { };        // publishes "demo.greeting" in birth
}
```

### `<-` (bus send)

The send operator is statement-position only. The form is:

```text
"subject" <- expr ;
```

with a string-literal subject on the left and any expression
producing the subject's declared type on the right. The
runtime delivers a copy of the payload to each currently-active
subscriber on the subject.

`<-` produces no value and does not nest in expressions.

## F.8: vertical-only-flow

Per **F.8**, the bus is vertical-only-flow:

- The graph of communication is closed by the union of all
  declared subscribe / publish entries. There is no way for a
  message to reach a locus that did not declare a subscription.
- Failures do not flow along the bus. A `ClosureViolation`
  propagates *upward* through the parent's `on_failure`, not
  laterally through the bus.

## Cross-process / cross-thread

The bus is transport-agnostic at the source level. Subjects
may be bound at deployment time to in-memory dispatch,
cross-thread mailboxes (for pinned subscribers), Unix sockets,
NATS, UDP multicast, or TCP. The locus's source code does not
change.

See [bus dispatch](../bus/index.md) for the full transport
model and [runtime — scheduling](../runtime.md) for cross-thread
mailbox mechanics.

## See Also

- [Bus dispatch and routing](../bus/index.md)
- [Statements (the `<-` operator)](../statements/index.md)
- [Runtime](../runtime.md)
- [Deployment](../deployment.md)
