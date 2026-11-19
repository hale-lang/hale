# Lists & Maps

Hale does not use garbage-collected generic container values like `Vec<T>` or `HashMap<K, V>`. Instead, dynamic collections are loci annotated with `@form(...)` whose storage and methods are specialized at compile time for their element type, while fixed-capacity inline lists inside `type` records use `bounded[T; N]` or `[T; N]`.

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

## Collection Forms and Inline Sequences Overview

Each `@form` locus manages a single collection (one `@form` per locus). Use `@form` loci for component state and `bounded[T; N]` or `[T; N]` for pure data fields inside `type` records.

| Collection | Declaration Syntax | Primary Operations | Iteration & Notes |
|---|---|---|---|
| **`@form(vec)`** (Growable list) | `capacity { heap items of T; }` | `push(x)`, `get(i)`*, `set(i, x)`*, `pop()`*, `len()`, `is_empty()`, `clear()`, `truncate(n)`, `sort*` | `for x in v.items { ... }` |
| **`@form(hashmap)`** (Keyed table) | `capacity { pool entries of T indexed_by k; }` | `set(v)`, `get(k)`*, `has(k) -> Bool`, `remove(k)`*, `len()`, `is_empty()`, `bump(k)`, `key_at(i)`, `entry_at(i)` | `for e in m.entries { ... }`; keys are `Int` or `String` |
| **`@form(set)`** (Keyed membership) | `capacity { pool items of T indexed_by k; }` | `insert(v)`, `contains(k) -> Bool`, `remove(k)`*, `len()`, `is_empty()` | Keyed existence set without unused value payloads |
| **`@form(ring_buffer, cap = N)`** (Bounded FIFO) | `capacity { pool events of T; }` | `push(x) -> Bool` (`false` if full), `pop()`*, `len()`, `is_full()` | Fixed capacity `N`; caller controls backpressure on `false` |
| **`@form(lru_cache, cap = N)`** (Bounded LRU cache) | `capacity { pool entries of T indexed_by k; }` | `put(v)`, `get(k)`*, `contains(k) -> Bool`, `remove(k)`*, `len()`, optional `fn on_evict(k, v)` | Evicts least-recently-used entry when exceeding `N` |
| **`@form(heap)`** (Priority queue) | `capacity { heap jobs of T ordered_by min\|max f; }` | `push(v)`, `pop()`*, `peek()`*, `len()`, `is_empty()`, `clear()` | Drain with `while !q.is_empty() { let x = q.pop() or ...; }` |
| **`bounded[T; N]`** (Inline bounded list) | `tags: bounded[String; 32];` (field on `type`) | `push(f, x)`*, `at(f, i)`*, `set(f, i, x)`*, `count(f)`, `clear(f)`, `truncate(f, n)` | `for x in f { ... }`; starts empty (`count = 0`) |
| **`[T; N]`** (Fixed array) | `let xs: [Int; 4] = [0; 4];` | `xs[i]`, `len(xs)` | `for x in xs { ... }`; fixed size `N`, an integer literal (`0` included: `[x; 0]`, or `let xs: [Int; 0] = [];`) |

*\*Operations marked with `*` are `fallible` (out-of-bounds, missing key, empty, or at capacity) and require an `or` clause.*

A vec's `sort()` orders `Int`, `Float` and `String` cells by themselves; a vec of any other cell (a `Bool`, a record) has no default ordering, and `hale check` refuses `v.sort()` there, pointing at `v.sort_by(less_than)` with a comparator `fn less_than(a: T, b: T) -> Bool`.

A `bounded` is not indexed with `[i]`: read an element with `at(f, i) or …` and write one with `set(f, i, x) or …`, each fallible past the live count (`hale check` refuses `f[i]` and names both).

The free builtin `len(x)` measures a `String`, a `Bytes` or a fixed array only. A form's length is its own `len()` method and a `bounded`'s live count is `count(f)`; `hale check` refuses `len(v)` on either and names the spelling to use (`v.len()` / `count(f)`).

## Growable Lists (`@form(vec)`)

Annotate a locus with `@form(vec)` and declare a `heap` slot in `capacity { ... }` to create a dynamically resized contiguous list:

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
    for name in names.items {
        println(name);
    }
}
```

- **Element types:** Can be any scalar (`Int`, `String`, etc.) or user-defined `type` record (`type Player { id: String; score: Int; }`).
- **Iteration via `.items`:** Always iterate a `@form(vec)` with `for x in v.items` regardless of the slot name written in `capacity { heap my_slot of T; }` (writing `v.my_slot` produces a diagnostic pointing to `v.items`). Use indexed `while i < v.len()` + `v.get(i)` only when you need the index.
- **Value ownership and memory reuse:** `get` and `pop` return an independent copy of elements containing `String` or `Bytes`. `pop`, `set`, `clear()`, and `truncate(n)` immediately reclaim the memory of dropped or overwritten elements so a long-lived vec used as a queue or per-message scratch buffer (cleared via `clear()`) maintains a flat memory footprint.

## Keyed Maps (`@form(hashmap)`), Sets (`@form(set)`), and Caches (`@form(lru_cache)`)

### `@form(hashmap)`

A `@form(hashmap)` keys records by one of the record's own `Int` or `String` fields declared via `indexed_by`:

```hale,fragment
type Account { user: String; balance: Int; }

@form(hashmap)
locus Accounts {
    capacity { pool entries of Account indexed_by user; }
}
```

- **`set(record)`** inserts or updates the record using its `indexed_by` field as the key (there is no separate key argument).
- **`get(key)` and `remove(key)`** are `fallible(KeyError)`; **`has(key)`** returns `Bool`.
- **Copy semantics:** Modifying a record returned by `get(key)` does not mutate the map entry unless you pass it back to `set(record)`. When you mutate the local and immediately `set` it back under the same key, the compiler optimizes the update in place with zero allocations.
- **Iteration via `.entries`:** Always use `for e in m.entries { ... }` to iterate over a hashmap (`O(capacity)` single pass). Avoid looping with `key_at(i)` or `entry_at(i)`, which rescan from slot `0` on each call (`O(capacity × len)`).

### `@form(set)`

When you only need key membership without storing extra value fields, use `@form(set)`:

```hale
type Item { key: String; }

@form(set)
locus Seen {
    capacity { pool items of Item indexed_by key; }
}

fn main() {
    let seen = Seen { };
    seen.insert(Item { key: "req-1" });
    if seen.contains("req-1") {
        println("already processed");
    }
}
```

### `@form(lru_cache, cap = N)`

`@form(lru_cache, cap = N)` is a fixed-capacity keyed table that automatically evicts the least-recently-used entry when `put` exceeds `cap`. Its key, like a hashmap's, is an `Int` or a `String` field (an identity, range or quantity counts as its `Int`); `hale check` refuses any other key at the `indexed_by` name:

```hale
type Entry { key: String; fd: Int; }

@form(lru_cache, cap = 128)
locus ConnCache {
    capacity { pool entries of Entry indexed_by key; }

    fn on_evict(k: String, v: Entry) {
        println("evicted ", k, " fd=", v.fd);
    }
}

fn main() {
    let c = ConnCache { };
    c.put(Entry { key: "a", fd: 3 });
    let e = c.get("a") or Entry { key: "", fd: -1 };
    println(e.fd);
}
```

- **`put(record)`** is infallible and evicts the oldest entry if full.
- **`get(key)`** (`fallible(KeyError)`) marks the entry as recently used; **`contains(key)`** (`Bool`) checks existence without updating recency; **`remove(key)`** (`fallible(KeyError)`) removes and returns the entry.
- **`fn on_evict(k: K, v: T)`:** Optional hook on the locus called whenever an entry is evicted—and for each remaining entry (least-recently-used first) when the cache locus dissolves. `on_evict` may not call `put` on its own cache.

## Bounded Queues (`@form(ring_buffer)`) and Priority Queues (`@form(heap)`)

### `@form(ring_buffer, cap = N)`

`@form(ring_buffer, cap = N)` is a fixed-capacity FIFO queue:

```hale
@form(ring_buffer, cap = 64)
locus Recent {
    capacity { pool events of String; }
}
```

`push(x)` returns `Bool`: `true` if stored, or `false` if the buffer is at capacity (leaving the buffer unchanged so the caller can drop the item, apply backpressure, or `pop()` the oldest entry first). `pop()` removes and returns the oldest entry and is fallible when empty.

### `@form(heap)`

`@form(heap)` is a binary priority queue ordered by a `min` or `max` record field (`Int`, `Float`, `String`, `Time`, or `Duration`):

```hale
type Job { name: String; priority: Int; }

@form(heap)
locus Jobs {
    capacity { heap jobs of Job ordered_by min priority; }
}

fn main() {
    let q = Jobs { };
    q.push(Job { name: "deploy", priority: 2 });
    q.push(Job { name: "page",   priority: 0 });
    q.push(Job { name: "report", priority: 5 });
    while !q.is_empty() {
        let j = q.pop() or raise;
        println(j.name);            // page, deploy, report
    }
}
```

`pop()` and `peek()` are fallible on an empty heap. Heaps do not support indexed `get(i)` or `for` loops; drain them with `pop()`. See [Forms Under the Hood](../systems/forms.md) for layout and cross-pool synchronization (`sync = serialized | striped | lockfree`).

## Inline Bounded Lists (`bounded[T; N]`)

Because a `type` record is pure data copied by value, it cannot contain a `@form` locus. To embed a variable-length list up to a known maximum `N` directly inside a `type`, use `bounded[T; N]`:

```hale
type Message {
    id:   String;
    tags: bounded[String; 32];
}

fn main() {
    let msg = Message { id: "msg1" };   // `tags` left out: it starts empty
    push(msg.tags, "urgent") or raise;
    push(msg.tags, "billing") or raise;

    for tag in msg.tags {
        println(tag);
    }
    println(count(msg.tags));           // 2
}
```

A `bounded` is a value. An array literal written where a `bounded[T; N]` is declared (a `let` with that type, a `return` from a fn returning one, an argument, an assignment, a record literal's field) is that list, its count the literal's length; `[]` is empty. A literal longer than `N` is a check error where you wrote it. A function returns one by value, and the caller owns what it gets back:

```hale
type Doc { title: String; tags: bounded[String; 8]; }

fn hashtags(d: Doc) -> bounded[String; 8] {
    let mut out: bounded[String; 8] = [];
    for t in d.tags {
        push(out, "#" + t) or raise;
    }
    return out;
}

fn main() {
    let ids: bounded[Int; 4] = [1, 2, 3];          // count 3
    let d = Doc { title: "notes", tags: ["a", "b"] };
    let tags = hashtags(d);
    println(count(ids), " ", count(tags), " ", count(hashtags(d)));   // 3 2 2
}
```

A local bound from a place is a copy, as a record or a string is: `let mut mine = self.row.tags; push(mine, "x") or raise;` leaves `self.row.tags` as it was, and a later write to `self.row.tags` does not reach `mine`. To change the place, name it (`push(self.row.tags, "x")`), or store the local back whole (`self.row.tags = mine;`). Writing a local with an intrinsic needs `let mut`.

### `bounded[T; N]` Intrinsics and Rules

| Intrinsic | Signature | Behavior |
|---|---|---|
| `push(b, x)` | `(bounded[T; N], T) -> () fallible(CapacityError)` | Appends `x`; fails with `CapacityError { cap, count }` when full |
| `at(b, i)` | `(bounded[T; N], Int) -> T fallible(IndexError)` | Reads element at index `i` (`0 <= i < count(b)`) |
| `set(b, i, x)` | `(bounded[T; N], Int, T) -> () fallible(IndexError)` | Overwrites live slot `i` (`0 <= i < count(b)`) |
| `count(b)` | `(bounded[T; N]) -> Int` | Returns live element count (`len(b)` is rejected and points to `count(b)`) |
| `clear(b)` | `(bounded[T; N]) -> ()` | Resets live count to `0` |
| `truncate(b, n)` | `(bounded[T; N], Int) -> ()` | Shrinks live count to `n` (never grows) |

- **Supported element types:** Scalars (`Int`, `Float`, `Bool`, `Decimal`, `Duration`, `Time`, unit types), enums (payload-free or carrying values: `bounded[Status; 8]` with `type Status = enum { Ok, Other(String) };`), `String`, `Bytes`, and structs. Wrap tuples or fixed arrays in a struct to store them in a `bounded`.
- **Copying and returning:** Assigning `row.tags = msg.tags;` between two `bounded[T; N]` of the exact same type, fields or locals, copies all live elements and the count; so does binding a local from a field. A function, a locus method and an interface method return a `bounded[T; N]` by value, its elements copied into the caller's arena. A call's result is a temporary: `count(make())` reads it, and `push(make(), x)` is refused (bind it with `let mut` first). A field takes no default: it starts empty, and a record literal that wants elements spells them (`Doc { tags: ["a"] }`).
- **Memory:** A `bounded` a locus holds owns its `String`, `Bytes`, struct and payload-carrying enum elements, with everything they carry (an enum element's `String`, `Bytes`, record or nested enum). Whatever stops being an element is reclaimed: the element `set` overwrites, the ones `clear` and `truncate` drop, and every element of a record you replace whole (`self.row = fresh;`). So a locus that keeps rewriting such a field stays the same size. `at(b, i)` gives you the element itself, and a local you keep across a call that rewrites the field gets its own copy automatically.
- **Value semantics:** Every `bounded`, a local record's too, holds its own elements. After `push(l.items, item)`, writing `item.k = 2` does not change the element, and writing a copy read with `at` and `set` back does not change `item`. `push` and `set` keep the value without copying it when nothing else names it: a literal, a call's result that holds nothing of its arguments, a local you do not use after the store, or an element read with `at` and set back to its own slot (`let mut x = at(l.items, i) or …; x.k = 2; set(l.items, i, x) or …;`). Otherwise they store a copy, which `hale verify` reports on a hot path. A call's result whose record took a parameter's `bounded` field (`v.tags = r.tags; return v;`) still holds that parameter's elements, so `push` copies it, and `hale verify` says so ("a call's result holding another name's block"); build that field from values of the callee's own to store the result as it is. A field that is an inline scalar (a number, a `Bool`, a `Time`, a payload-free enum) holds nothing of the parameter's: `S { n: r.n, st: r.st }` with `st` a payload-free enum is stored as it is.
- **Populating via helper functions:** A helper function `fn fill(r: Row)` that pushes `String`, `Bytes`, or structs into `r` allocates those elements in the caller's frame. Passing a locus field directly (`fill(self.row)`) is rejected because those frame-allocated elements would be freed when the caller returns; instead, populate a local record and assign it whole (`let mut mine = Row { }; fill(mine); self.row = mine;`), which copies the elements into the locus's arena.
- **Printing:** `bounded` of scalar elements (`Int`, `Float`, `Bool`, `Decimal`, `Duration`) prints its live slice (`[7, 8]`). `bounded[String; N]` (and structs containing one) cannot be printed directly—iterate over it or print `count(b)`.
- **Only on a `bounded`:** the intrinsics apply to a `bounded[T; N]` argument. `count(b)` on the record that holds one is refused by `hale check`, which names the field to count instead (`count(b.items)`); the same goes for the other five names, unless your program declares a `fn` of that name for the argument.
- **Custom function overrides:** The six intrinsic names (`count`, `push`, `at`, `set`, `clear`, `truncate`) are dispatched by the exact `bounded[T; N]` receiver type and are not reserved keywords. Declaring `fn count(xs: bounded[Int; 8]) -> Int` overrides the intrinsic for `bounded[Int; 8]` without affecting `bounded[Int; 4]`.

## Collection Query Chains

You can query, transform, and aggregate `@form(vec)`, `@form(hashmap)` (`.entries`), `bounded[T; N]`, and `[T; N]` collections, and the parts of a text, using single-pass fused chains:

```hale,fragment
let active_count = users.filter(it.active).count();
let total_age    = users.filter(it.active).map(it.age).sum();
users.filter(it.active).into(active_users_vec);
```

A chain over a `bounded` or array field walks it by its type, whatever the field is called and however you reach it: `b.tags.filter(it > 1).count()`, `o.inner.tags.map(it * 2).sum()`, or a field named `items` or `entries`. Only on a `@form` locus are `.items` and `.entries` the form's own iteration surface.

Every chain is lowered by the compiler into a single inline loop with **zero heap allocations**, making chains legal inside `@budget(alloc_per_call = 0)` and `@hot` functions. Within stage expressions, `it` refers to the current element (and `idx` refers to the 0-based position after `.enumerate()`).

The text sources `std::str::split(s, sep)`, `std::str::lines(s)` and `std::str::words(s)` start a chain too, with each part of the text as `it`: `std::str::words(s).filter(len(it) > 2).count()`. Such a chain is one pass over the text with no list of the parts, but each part is a new `String`, so unlike a chain over a collection it allocates, one part at a time (see [Strings](../basics/strings.md)). Any terminal works on a text source without a stage (`std::str::lines(s).count()`).

### Chain Stages and Terminals

| Stage / Terminal | Example | Result & Behavior |
|---|---|---|
| `.filter(pred)` | `xs.filter(it > 10)` | Keeps elements where `pred` is `true` |
| `.map(expr)` | `users.map(it.age)` | Projects each element to `expr` (use `.map(it)` as an identity stage on `[T; N]`) |
| `.take(n)` / `.skip(n)` | `items.skip(20).take(10)` | Takes or skips `n` elements arriving at that position in the chain |
| `.enumerate()` | `xs.enumerate().filter(idx % 2 == 0)` | Binds `idx: Int` (counting elements reaching `.enumerate()`) for subsequent stages |
| `.count()` | `users.filter(it.active).count()` | Returns `Int` count of matching elements |
| `.sum()` / `.sum(seed)` | `prices.map(it).sum(0.0d)` | Sums elements (`sum()` defaults to `Int` `0`; pass `seed` for `Float`, `Decimal`, or non-zero start) |
| `.any(pred)` / `.all(pred)` | `users.any(it.age < 18)` | Short-circuiting `Bool` test (`false` / `true` on empty collections) |
| `.find(pred)` | `users.find(it.id == want) or def` | `fallible`: returns first matching element (does not compose after `.map`) |
| `.min(key)` / `.max(key)` | `users.max(it.age) or raise` | `fallible`: returns the whole element with the minimum or maximum `key` |
| `.into(target_vec)` | `users.filter(it.active).into(out)` | Appends matching elements into caller-owned `@form(vec)` `out` |
| `.each { ... }` | `users.each { println(it.name); }` | Executes block for each element (`break` and `continue` supported) |
| `.sort_into(vec[, cmp])` | `scores.sort_into(ranked, cmp_fn)` | Copies into `@form(vec)` `ranked` and sorts ascending (or via `cmp_fn`) |
| `.reverse_into(vec)` | `recent.take(10).reverse_into(out)` | Copies into `@form(vec)` `out` in reverse order |
| `.group_count_into(map, key)` | `orders.group_count_into(by_desk, it.desk)` | Increments per-key `Int` counter in `@form(hashmap)` `by_desk` |

> [!NOTE]
> Chain stages and terminals (`map`, `filter`, `count`, `find`, `first`) are recognized only after `.`, so you can freely define top-level functions with those names. Only bare compiler builtins (`sum`, `prod`, `min`, `max`, `abs`, `len`, `to_string`, `print`, `println`) are reserved at free-function position—and even those remain valid as locus method names (`fn sum()`).

Next: [Records & Data](./records.md).
