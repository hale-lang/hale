# Seams

A **seam** is a declarative pipeline inside a locus that connects a wire decoder, a domain handler locus, and an encoder over [set types](../everyday/records.md#set-types-a--b--c) (`type Inbound = A | B;`). Instead of writing manual `match` dispatch glue with catch-all `_` arms, you write a single `->` chain and the compiler generates statically inlined dispatch while verifying that every message variant and error type is handled by exactly one target.

```hale,fragment
type NewOrder  { id: Int; qty: Int; }
type Cancel    { id: Int; }
type Ack       { id: Int; }
type Reject    { id: Int; why: String; }
type WireError { at: Int; }

type Inbound  = NewOrder | Cancel;
type Outbound = Ack | Reject | WireError;

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

Running this program prints:

```text
ack 30
reject 500 too big
ack 1
bad frame at 0
open: 0
```

## Seam Forms & Hop Types

| Construct | Syntax | Behavior |
|---|---|---|
| **Callable seam** | `seam name(arg: T) -> R { arg -> ...; }` | Generates a callable method `self.name(arg)` on the enclosing locus. |
| **Self-running (closed) seam** | `seam { self.src.next() -> ... -> self.sink.write; }` | Generates the locus's `run()` loop, pulling frames until `!self.draining` is false or the source fails. |
| **Method hop** | `-> self.wire.decode` | Passes the current value directly to the named method. |
| **Locus hop** | `-> self.desk` | Dispatches each variant of an incoming set type (`NewOrder \| Cancel`) to the unique method on `self.desk` whose parameter matches that variant's type. |

## Bidirectional Exhaustiveness Checking

In the example above, `Wire` knows nothing about `Desk`, and `Desk` knows nothing about wire framing. The `seam` connects them and verifies exhaustiveness in both directions at compile time:

1. **Locus hop coverage**: Every member in the set arriving at `self.desk` must match the parameter type of **exactly one** method on `Desk`. If `Desk::cancel` is omitted, compilation fails at `self.desk`:
   ```text
   type error: `self.desk` has no method taking `Cancel`, which reaches it from `self.wire.decode`: every member has one eater
               frame -> self.wire.decode -> self.desk -> self.wire.encode;
                                            ^^^^^^^^^
   ```
   Similarly, if two methods on `Desk` both accept `Cancel`, the compiler rejects the ambiguity naming both methods.
2. **Method hop coverage**: At a method hop (`self.wire.encode`), every member produced by the preceding hop must be a member of the method parameter's set type (`Outbound`).

Adding a new message type to `Inbound` immediately fails compilation at `self.desk` until a corresponding handler method is added. (Note: if a seam has a type error, its generated method is omitted, which will also produce a secondary `"no field answer"` diagnostic at call sites until the seam is fixed.)

## Error Routing in Seams

When a hop in a seam is `fallible(E)` (such as `self.wire.decode` failing with `WireError`), the compiler routes `E` using one of three mechanisms:

| Error Route | Syntax | Behavior |
|---|---|---|
| **Forward into a later hop's set** | Include `E` in a downstream hop's input set (`type Outbound = Ack \| Reject \| WireError;`) | `E` bypasses intermediate hops and enters the first downstream hop that accepts `E` (e.g., `self.wire.encode`), turning the error into an ordinary encoded reply. |
| **End the seam (`or end;`)** | Append `or end;` to the seam block | On a callable seam, makes the seam method `fallible(E)` so callers handle it with `c.answer(frame) or ...`. On a closed seam, terminates the `run()` loop. |
| **Escalate structurally (`or violate`)** | Append `or violate <closure>;` to the seam block | Fires the enclosing locus's `epoch inline` closure `<closure>`, routing a `ClosureViolation` to the parent's `on_failure` hook. See [Failure & Supervision](./failure.md). |

If a hop can fail with `E` and neither a downstream hop's set nor an `or` clause handles `E`, compilation fails:

```text
type error: `self.wire.decode` fails with `WireError`, and no later hop takes it: add `WireError` to a later hop's set, or route it with `or end;` or `or violate <closure>;`
```

## Self-Running Closed Seams

When a seam begins with a source call expression (`self.feed.next()`) and ends with a void sink method (`self.log.write`), it forms a **closed seam** that serves as the locus's `run()` method:

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

- **Loop lifecycle**: While `!self.draining` is true, the generated `run()` pulls one item from `self.feed.next()` and executes the chain. When the source fails (`fail Ended`), the loop exits cleanly. (`or end;` and `or violate` apply to failures in hops *after* the source.)
- **Mutually exclusive with `run()`**: Because a closed seam *is* the locus's `run()` phase, declaring both a closed `seam { ... }` and an explicit `run()` on the same locus is a compile error.
- **Flat per-frame memory**: The compiler extracts a single turn of the closed seam (source pull, decode, dispatch, encode, sink write) into a helper method called inside the `run()` loop. Any scratch memory allocated while processing a frame is reclaimed at the end of that frame, keeping memory usage flat across long-running streams.

## Single-Tower Constraints & API Integration

Every hop in a seam is compiled as a direct, statically inlined call within a single locus tower: each variant retains its concrete static type from the decoder through to the encoder without boxing into an unrequested runtime union.

To preserve static single-thread lowering, `hale check` rejects two hop targets:

1. **Interface-typed fields**: The concrete target method of an `interface` field is not known at compile time.
2. **Children placed on a different pool**: Cross-pool communication must go through the [bus](./bus.md) (`Topic <- msg;`) rather than a synchronous seam hop.

> [!NOTE]
> **How `api::serve` Uses Seams**
> When you expose an [API surface](./api.md), the compiler lowers each route into a seam on the receiver locus: a generated decoder parses the HTTP/JSON or gRPC/Protobuf request body into your request type, a hop dispatches to your handler method, and an encoder serializes the response, error, or handler violation status code.

### Specification References

- Seam semantics: `spec/semantics.md` § Seams
- Set types: `spec/types.md` § Set types
- Style guide rules: `spec/styleguide.md` §§ 2.9–2.10

Next: [Repositories](./repositories.md).
