# Lists & maps

> **Coming from Python / Node?** Hale has no built-in `list` /
> `[]` that grows, no `dict` / `{}`, no `Vec<T>` or `Map<K,V>`.
> Instead you declare a small locus and annotate it with a
> **form** — `@form(vec)` for a growable list, `@form(hashmap)`
> for a keyed map. You get the same operations (`push`, `get`,
> `len`, `set`, …); they're just methods on a locus you named.

## A growable list — `@form(vec)`

```hale
@form(vec)
locus Names {
    capacity { heap items of String; }
}

fn main() {
    let names = Names { };
    names.push("Ada");
    names.push("Grace");
    println(names.len());            // 2
    let first = names.get(0) or "";  // "Ada"
}
```

Three things are happening:

- **`@form(vec)`** tells the compiler "this locus is a growable
  list." It synthesizes the methods for you: `push`, `get`,
  `set`, `pop`, `len`, `is_empty`, and sorting.
- **`capacity { heap items of String; }`** is where the list's
  storage lives. Read it as "this list holds `String`s." The
  element type comes from here.
- **`get` and `pop` are fallible** — an index might be out of
  bounds — so you address them with `or`, just like any fallible
  call:
  ```hale
  let x = names.get(99) or "(missing)";
  ```

Iterate with `for` over the indices:

```hale
let mut i = 0;
while i < names.len() {
    println(names.get(i) or "");
    i = i + 1;
}
```

The element type can be anything — a primitive, or one of your
own `type` records:

```hale
type Player { id: String; score: Int; }

@form(vec)
locus Roster {
    capacity { heap players of Player; }
}
```

## A keyed map — `@form(hashmap)`

A map keys entries by a field *on the value itself* — the key is
one of the record's fields, named with `indexed_by`:

```hale
type Account { user: String; balance: Int; }

@form(hashmap)
locus Accounts {
    capacity { pool entries of Account indexed_by user; }
}

fn main() {
    let accts = Accounts { };
    accts.set(Account { user: "ada",   balance: 100 });
    accts.set(Account { user: "grace", balance: 250 });

    let a = accts.get("ada") or Account { user: "", balance: 0 };
    println(a.balance);                       // 100
    println(accts.has("grace"));              // true
}
```

- **`set(value)`** takes the whole record and reads the key out
  of its `indexed_by` field — there's no separate key argument.
- **`get(key)`** and **`remove(key)`** are fallible (the key
  might be absent); `has(key)` returns a plain `Bool`.
- Keys are `Int` or `String`.

This "the key is a field of the value" shape matches how keyed
stores almost always look in practice — you rarely have a key
that *isn't* already part of the thing you're storing.

## A bounded queue — `@form(ring_buffer)`

When you want a fixed-size FIFO that drops the oldest entry once
it's full (recent-events buffers, sliding windows):

```hale
@form(ring_buffer, cap = 64)
locus Recent {
    capacity { pool events of String; }
}
```

`push` returns a `Bool` — `false` when the buffer is full — so
you decide whether to drop or apply backpressure. `pop` is
fallible on empty.

## Why a form instead of a generic type

A list isn't just "a type parameterized by its element" — it's a
bundle of decisions: contiguous memory, dynamic length, who owns
the storage, what happens to it when the owner goes away. A form
makes those decisions at the declaration, and picks an
implementation tuned for the element type. The upshot for you at
this level is simple: **`@form(vec)` is your list, `@form(hashmap)`
is your map.** The reasoning behind forms — and how to choose
between them on performance grounds — is in [Forms under the
hood](../systems/forms.md) at the systems level.

One form per locus: a locus is a list *or* a map, not both. If
you need both, that's two loci — which is usually what the data
wanted anyway.

Next: [Records & data](./records.md).
