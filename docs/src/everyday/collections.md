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
  ```hale,fragment
  let x = names.get(99) or "(missing)";
  ```

Iterate with `for` over the items:

```hale,fragment
for name in names.items {
    println(name);
}
```

(The indexed `while i < names.len()` + `.get(i)` walk also works,
and is what you want when you need the index — but prefer `.items`
as the default: it reads better and, on hashmaps especially, it's
dramatically faster. A hashmap walk via `key_at(i)`/`entry_at(i)`
rescans from slot 0 on every call — O(cap×len) for the whole walk —
while `for e in m.entries` visits each occupied slot once.)

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

## A list inside a type — `bounded[T; N]`

The forms above are *loci* — whole entities with their own
lifecycle. A `type` is pure data, so it can't hold one. What it CAN
hold is a **bounded** collection — a
fixed-capacity list laid out inline in the value:

```hale
type Message {
    id:   String;
    tags: bounded[String; 32];
}

fn main() {
    let msg = Message { id: "msg1" };   // tags starts empty —
                                        // bounded fields can't be
                                        // spelled in a literal
    push(msg.tags, "urgent") or raise;
    push(msg.tags, "billing") or raise;

    for tag in msg.tags {
        println(tag);
    }
    println(count(msg.tags));           // 2
}
```

Six operations, all compiler intrinsics (types stay method-free,
like `len(s)`):

- `push(f, x)` — append; **fallible** with
  `CapacityError { cap, count }` when full. What to do at capacity
  is *your* policy, written in the `or` arm.
- `at(f, i)` — read slot `i`; fallible `IndexError` out of range.
- `set(f, i, x)` — overwrite a live slot; fallible `IndexError`.
- `count(f)` — the live count (the capacity lives in the type).
- `clear(f)` — reset to empty.
- `truncate(f, n)` — shrink the count (never grows); with `set`,
  this is the drop-front idiom for FIFO windows.

Use `bounded` when the maximum is known and the list is a *field of
a value* — per-message tags, route parameters, a chat window. The
old workaround (a tab-separated string you re-parse on every read)
is retired. Whole-struct copies carry the elements automatically, and
scalar-element bounded values even cross the zero-copy bus as flat
bytes.

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

## Asking questions about a collection

Once you have a collection, the common thing you want is not to *build
a new one* — it's to answer a question about the one you have. How
many are active? Put the active ones over there.

```hale,fragment
let n = users.filter(it.active).count();
users.filter(it.active).into(actives);
```

`it` is the current element. Chain as many stages as you like; they
run in a single pass:

```hale,fragment
let n = readings.filter(it > 10).filter(it < 100).count();
let total = users.filter(it.active).map(it.age).sum();
```

`map` rebinds the element; `sum` adds it up. Bare `sum()` is for Int
elements; give it a seed and the seed's type drives the accumulator —
`sum(0.0)` sums Floats, `sum(100)` starts an Int sum at 100.

Position matters too, and there are stages for it:

```hale,fragment
let podium = scores.take(3).sum();          // stop after three
let rest = scores.skip(3).sum();            // drop the first three
let page = items.skip(20).take(10).count(); // pagination in one pass
```

`take` and `skip` count elements arriving *at their own spot in the
chain*, so `filter(p).skip(2)` skips the first two matches, not the
first two elements. When you need the position itself, `enumerate()`
binds `idx` for everything after it:

```hale,fragment
let evens = xs.enumerate().filter(idx % 2 == 0).sum();
xs.filter(it.active).enumerate().each {
    println(idx, ": ", it.name);   // idx counts the matches
}
```

Yes-or-no questions have their own terminals:

```hale,fragment
if users.any(it.age < 18) { restrict(); }
if users.all(len(it.name) > 0) { proceed(); }
```

On an empty selection `any` is `false` and `all` is `true` — the
usual vacuous-truth convention.

Looking *for* an element is fallible — an empty result is handled
with the same `or` you already use everywhere:

```hale,fragment
let bob = users.find(it.id == want) or User { id: 0, age: 0, name: "?" };
let oldest = users.max(it.age) or raise;
let youngest = users.min(it.age) or raise;
```

`find` returns the element itself (project after, not before —
`map` doesn't compose with `find`), and `min`/`max` take a key and
return the *element* that had it, not the key.

And when the point is the side effect, `each` takes a block:

```hale,fragment
users.filter(it.age >= 18).each {
    total = total + it.age;
    "user.greet" <- Greeting { id: it.id };
}
```

The block *is* the loop body — `it` is in scope, `break` and
`continue` do what they do in any loop, and nothing is captured
because there is no closure.

The whole-set operations — the ones that need every element before
they can produce anything — write into a collection *you* own:

```hale,fragment
let ranked = Scores { };
scores.filter(it > 0).sort_into(ranked);        // ascending
scores.sort_into(ranked, harder_first);         // your comparator fn
recent.take(10).reverse_into(latest_first);
let by_desk = DeskCounts { };
orders.group_count_into(by_desk, it.desk);      // tally per key
```

`sort_into` and `reverse_into` fill a `@form(vec)` and reorder it in
place; `group_count_into` bumps a per-key Int counter in a
`@form(hashmap)` (its cell type is the key plus one Int field). The
chain itself still allocates nothing — the storage is yours, declared
where you can see it.

This looks like an iterator chain from another language, and it is
deliberately not one. There is no intermediate collection between the
stages, and no lazy object holding state — the whole chain is rewritten
to one loop before your program is typechecked. What you write is
exactly what runs.

That has a practical consequence worth knowing: **a chain allocates
nothing**, so it is legal in a handler with a strict budget.

```hale,fragment
@budget(alloc_per_call = 0)
fn urgent(rs: Readings) -> Int {
    return rs.filter(it > 90).count();
}
```

In most languages the composable style is the one you give up on the
hot path. Here it is the one that survives, because nothing is being
built.

Two things follow from the same fact. Because the chain is not a
value, you can't store one in a variable or return it — it's a
question you ask, not a thing you hold. And because there's no
closure, `it` can't capture anything or outlive the call, so none of
the usual questions about what a lambda captures arise.

Operations that need to see every element before producing any — sorting,
grouping — can't be part of a fused pass. Those write into storage you
supply, and the allocation shows up where you can see it.

## Membership

If the question is only *is this here*, reach for a set rather than a
map with a value you don't use:

```hale,fragment
type Item { key: String; }

@form(set)
locus Seen { capacity { pool items of Item indexed_by key; } }

seen.insert(Item { key: "a" });
if seen.contains("a") { ... }
```

`contains` answers `Bool` directly. Through a map you would be writing
`get(k) or false` at every call site, which is the value you didn't
want leaking back into your code.
