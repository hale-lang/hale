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
| **`[T; N]`** (Fixed array) | `let xs: [Int; 4] = [0; 4];` | `xs[i]`, `len(xs)` | `for x in xs { ... }`; fixed size `N` |

*\*Operations marked with `*` are `fallible` (out-of-bounds, missing key, empty, or at capacity) and require an `or` clause.*

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

`@form(lru_cache, cap = N)` is a fixed-capacity keyed table that automatically evicts the least-recently-used entry when `put` exceeds `cap`:

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
    let msg = Message { id: "msg1" };   // `tags` starts empty (cannot be set in literal)
    push(msg.tags, "urgent") or raise;
    push(msg.tags, "billing") or raise;

    for tag in msg.tags {
        println(tag);
    }
    println(count(msg.tags));           // 2
}
```

### `bounded[T; N]` Intrinsics and Rules

| Intrinsic | Signature | Behavior |
|---|---|---|
| `push(b, x)` | `(bounded[T; N], T) -> () fallible(CapacityError)` | Appends `x`; fails with `CapacityError { cap, count }` when full |
| `at(b, i)` | `(bounded[T; N], Int) -> T fallible(IndexError)` | Reads element at index `i` (`0 <= i < count(b)`) |
| `set(b, i, x)` | `(bounded[T; N], Int, T) -> () fallible(IndexError)` | Overwrites live slot `i` (`0 <= i < count(b)`) |
| `count(b)` | `(bounded[T; N]) -> Int` | Returns live element count (`len(b)` is rejected and points to `count(b)`) |
| `clear(b)` | `(bounded[T; N]) -> ()` | Resets live count to `0` |
| `truncate(b, n)` | `(bounded[T; N], Int) -> ()` | Shrinks live count to `n` (never grows) |

- **Supported element types:** Scalars (`Int`, `Float`, `Bool`, `Decimal`, `Duration`, `Time`, unit types), payload-free enums, `String`, `Bytes`, and structs. Wrap tuples, fixed arrays, or payload-carrying enums in a struct to store them in a `bounded`.
- **Copying and returning:** Assigning `row.tags = msg.tags;` between two `bounded[T; N]` fields of the exact same type copies all live elements and the count. A function cannot return a bare `bounded[T; N]` directly; wrap it in a `type` record and return the record.
- **Populating via helper functions:** A helper function `fn fill(r: Row)` that pushes `String`, `Bytes`, or structs into `r` allocates those elements in the caller's frame. Passing a locus field directly (`fill(self.row)`) is rejected because those frame-allocated elements would be freed when the caller returns; instead, populate a local record and assign it whole (`let mut mine = Row { }; fill(mine); self.row = mine;`), which copies the elements into the locus's arena.
- **Printing:** `bounded` of scalar elements (`Int`, `Float`, `Bool`, `Decimal`, `Duration`) prints its live slice (`[7, 8]`). `bounded[String; N]` (and structs containing one) cannot be printed directly—iterate over it or print `count(b)`.
- **Custom function overrides:** The six intrinsic names (`count`, `push`, `at`, `set`, `clear`, `truncate`) are dispatched by the exact `bounded[T; N]` receiver type and are not reserved keywords. Declaring `fn count(xs: bounded[Int; 8]) -> Int` overrides the intrinsic for `bounded[Int; 8]` without affecting `bounded[Int; 4]`.

## Collection Query Chains

You can query, transform, and aggregate `@form(vec)`, `@form(hashmap)` (`.entries`), `bounded[T; N]`, and `[T; N]` collections using single-pass fused chains:

```hale,fragment
let active_count = users.filter(it.active).count();
let total_age    = users.filter(it.active).map(it.age).sum();
users.filter(it.active).into(active_users_vec);
```

Every chain is lowered by the compiler into a single inline loop with **zero heap allocations**, making chains legal inside `@budget(alloc_per_call = 0)` and `@hot` functions. Within stage expressions, `it` refers to the current element (and `idx` refers to the 0-based position after `.enumerate()`).

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
