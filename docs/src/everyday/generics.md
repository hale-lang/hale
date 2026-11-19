# Generics

A `type`, a `locus` or a `fn` can take type parameters in angle brackets, so one declaration serves several types. Each use names its types (`Envelope<Order>`), and the compiler builds a separate copy of the declaration for each, so a generic costs nothing at run time.

Hale has no generic container types such as `Vec<T>` or `HashMap<K, V>`. A list or a map is a `@form` locus, or a `bounded[T; N]` inside a record ([Collections](./collections.md)).

## Generic Records

An API that wraps every response in the same envelope declares the envelope once:

```hale
type Order {
    id: String;
    total: Decimal<2>;
}

type Customer {
    id: String;
    name: String;
}

// What every endpoint answers with: the payload, and the request id
// a client quotes when it reports a problem.
type Envelope<T> {
    data: T;
    request_id: String = "";
}

fn order_reply(o: Order, req: String) -> Envelope<Order> {
    return Envelope { data: o, request_id: req };
}

fn main() {
    let r = order_reply(Order { id: "o-1", total: 19.99d }, "req-81");
    println(r.data.id, " ", r.data.total, " (", r.request_id, ")");   // o-1 19.99 (req-81)

    let c: Envelope<Customer> = Envelope { data: Customer { id: "c-1", name: "Ada" } };
    println(c.data.name);                                              // Ada
}
```

In `Envelope<Order>`, `data` is an `Order`, so `r.data.total` is checked as a `Decimal<2>`; in `Envelope<Customer>` it is a `Customer`. Putting a `Customer` into an `Envelope<Order>` is a type error at the value.

A parameter can be the element of a list the record holds inline. A paged API reply carries up to a page of whatever the endpoint lists:

```hale
type Order {
    id: String;
    total: Decimal<2>;
}

type Page<T> {
    items: bounded[T; 50];
    next_cursor: String = "";
}

fn main() {
    let p: Page<Order> = Page { items: [Order { id: "o-1", total: 19.99d }], next_cursor: "c-2" };
    push(p.items, Order { id: "o-2", total: 5.25d }) or raise;
    for o in p.items {
        println(o.id, " ", o.total);                       // o-1 19.99, then o-2 5.25
    }
    let ids: Page<Int> = Page { items: [7, 8, 9] };
    println(count(p.items), " ", count(ids.items), " ", p.next_cursor);   // 2 3 c-2
}
```

`Page<Order>` holds a `bounded[Order; 50]` and `Page<Int>` a `bounded[Int; 50]`; a locus's `params` take `bounded[T; N]` the same way.

An alias can name an instantiation you use often: `type OrderReply = Envelope<Order>;`, then `let a: OrderReply = Envelope { data: o };`. The alias itself cannot take parameters (`type Reply<T> = Envelope<T>;` is refused).

## Generic Loci

A locus takes type parameters the same way, and its `params` and methods use them:

```hale
type Quote {
    symbol: String;
    px: Decimal<2>;
}

type Reading {
    sensor: String;
    celsius: Float;
}

// Keeps the most recent value of one kind, and how many it has seen.
locus Latest<T> {
    params {
        current: T;
        seen: Int = 0;
    }

    fn put(v: T) {
        self.current = v;
        self.seen = self.seen + 1;
    }
}

fn main() {
    let q: Latest<Quote> = Latest { current: Quote { symbol: "ACME", px: 10.00d } };
    q.put(Quote { symbol: "ACME", px: 10.25d });
    println(q.current.px, " after ", q.seen, " update");   // 10.25 after 1 update

    let r: Latest<Reading> = Latest { current: Reading { sensor: "attic", celsius: 21.5 } };
    println(r.current.sensor, " ", r.current.celsius);    // attic 21.5
}
```

`q.put` takes a `Quote` and `r.put` a `Reading`. A method inside a locus cannot declare type parameters of its own (`fn put<U>(…)` is refused at its name): make the locus the template, as here, or write a free generic `fn`.

## Generic Functions

A generic `fn` takes each type argument from the argument in the position that names it:

```hale
fn clamp<T: Numeric>(x: T, lo: T, hi: T) -> T {
    if x < lo { return lo; }
    if x > hi { return hi; }
    return x;
}

type Envelope<T> {
    data: T;
    request_id: String = "";
}

fn reply<T>(data: T, req: String) -> Envelope<T> {
    return Envelope { data: data, request_id: req };
}

fn main() {
    println(clamp(150, 0, 100), " ", clamp(-0.5, 0.0, 1.0));   // 100 0
    let r = reply("pong", "req-9");                             // an Envelope<String>
    println(r.data, " ", r.request_id);                         // pong req-9
}
```

- **`Numeric` is the only bound.** `<T: Numeric>` admits `Int`, `Float`, `Decimal` and `Duration`, so the body may compare and do arithmetic on `T`. Hale has no traits, so any other bound (`<T: Eq>`) is refused.
- **Each use is checked.** The body is checked as written and again for each set of types its calls use, so `fn scale<T>(x: T) -> T { return x * 1.5; }` is refused when a call passes an `Int`, since the result would be a `Float`.
- **Call it; don't pass it.** A generic `fn` has no single type, so `let f = clamp;` is refused. Pass a `fn` written over the concrete types that calls it.
- **Pass every argument.** A call fills no default on a generic `fn`'s parameter, so pass the argument.
- **Imported and fallible ones work as usual.** A generic `fn` from another seed is called through its alias (`lib::id(3)`), and a generic `fallible(E)` `fn` takes `or` like any fallible call (`parse(s) or 0`).

See [Functions](../basics/functions.md) for everything else about functions.

## Rules You Will Hit

- **A literal needs its type arguments from the place it is written.** `Envelope { data: 42 }` does not say what `T` is, and Hale does not work it out from the fields. Write the literal where a declared type supplies the arguments: a `let` with a type (`let e: Envelope<Int> = Envelope { data: 42 };`), a `return` from a `fn` declared `-> Envelope<Int>`, a record field or `params` default whose type is written out, or an assignment to such a place. Anywhere else (an untyped `let`, a call's argument, a method receiver, an operand) `hale check` refuses the literal and shows the `let` to write. Bind it there first, then use the name.
- **Imported types work as arguments.** A type from another seed is an argument like a local one (`Envelope<billing::Invoice>`), and an imported template is spelled through its alias in both places: `let e: api::Envelope<billing::Invoice> = api::Envelope { ... };`.
- **A locus cannot be a generic record's argument.** A `type` is plain data copied by value, so it cannot hold a locus: `Envelope<Worker>` is refused. Make the template a generic locus (`locus Slot<T> { params { held: T; } }` holds a `Worker`), or pass the locus as a parameter where it is used. A generic locus's instantiation is a locus too, so `Envelope<Cache<Int, String>>` is refused the same way. A generic `fn` and a generic locus do take a locus as an argument.
- **An array, tuple, `bounded` or `fn` value cannot be a generic `fn`'s argument yet.** `first([1, 2])` is refused at the call. Wrap the value in a record (`type Pair { v: [Int; 2]; }`), or write the `fn` over the concrete type.

The full rules, including every place that supplies type arguments and which types may be arguments, are in [`spec/types.md` § Generics](https://github.com/hale-lang/hale/blob/main/spec/types.md#generics).

Next: [Introduction to Loci](./locus-gently.md).
