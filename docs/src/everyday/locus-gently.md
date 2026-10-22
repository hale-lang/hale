# The Locus, Gently

A `locus` is Hale's primary structural building block, combining state (`params`), behavior (`fn` methods), and lifecycle management (`birth`, `run`, `dissolve`) into a single construct. Where other languages separate classes, modules, actors, and services, Hale uses loci for all stateful or behavioral components and `type` declarations for pure data records.

```hale
locus Counter {
    params {
        count: Int = 0;
    }

    fn bump() {
        self.count = self.count + 1;
    }

    fn value() -> Int {
        return self.count;
    }
}

fn main() {
    let c = Counter { count: 10 };
    c.bump();
    c.bump();
    println(c.value());           // 12
}
```

## `type` vs. `locus`

Every named construct in Hale is either a pure data `type` or a `locus`:

| Property | `type` (Record / Enum) | `locus` (Component) |
|---|---|---|
| **Purpose** | Pure data values, messages, configuration records | Stateful objects, namespaces, collections, services |
| **State** | Value fields (`x: Int;`) | Instance state in `params { count: Int = 0; }` |
| **Behavior** | None (no methods) | `fn` methods with `self` access |
| **Lifecycle** | None (copied by value) | `birth() -> run() -> dissolve()` hooks |
| **Ownership** | Can be returned, copied, and sent over the bus | Owns its own arena; cannot be stored inside a `type` |

```hale
type Point { x: Int; y: Int; }        // pure data record

locus Tally {                          // state + behavior
    params { total: Int = 0; }
    fn add(n: Int) { self.total = self.total + n; }
}
```

> [!IMPORTANT]
> **Single Ownership of State (`I4`):** Every piece of mutable state in a Hale program belongs to a `locus` (or a local `let mut` variable inside a function). There are no mutable global variables or unowned shared buffers. This invariant allows Hale to manage memory with per-locus arenas and deterministic lifecycle hooks instead of a garbage collector.

## State (`params`), Methods (`self`), and Instantiation

### `params` and `self`

A locus declares its fields and constructor parameters together in a `params { ... }` block. Callers construct a locus with `LocusName { field: value }`, omitting any field that has a default value. Inside a locus's methods and lifecycle hooks, `self.field` reads and mutates `params` fields.

### Lifecycle Hooks (`birth`, `run`, `dissolve`) and Lifetime

A locus may optionally define lifecycle hooks (written **without** the `fn` keyword) that the runtime executes automatically:

- **`birth() { ... }`** runs when the locus is constructed to initialize resources.
- **`run() { ... }`** runs the locus's main body after `birth()` completes.
- **`dissolve() { ... }`** runs when the locus is torn down to release descriptors or external resources before its memory arena is freed.

How you instantiate a locus determines when `dissolve()` runs:

| Instantiation Form | Example | Lifecycle & Lifetime |
|---|---|---|
| **`let`-bound** | `let c = Counter { };` | Runs `birth` and `run` immediately; keeps `c` alive for method calls (`c.bump()`) and runs `dissolve` when the enclosing function scope exits. |
| **Statement-position** | `App { name: "world" };` | Runs `birth`, `run`, and `dissolve` immediately, reclaiming the locus at the semicolon `;`. |

### Declaration Rules for Loci

- **`const` and `type` are top-level only:** Do not place `const` or `type` declarations inside a `locus` body (`hale check` rejects them at the keyword). Declare `const` and `type` at the top level alongside the locus; they are automatically in scope inside every locus in the seed. For per-instance defaults, use a `params` field.
- **No recursive `params` defaults:** A `params` field default cannot construct the same locus type directly (`params { next: Node = Node { }; }`), via a helper function (`next: Node = make_node();`), or through a cycle of loci (`Alpha` -> `Beta` -> `Alpha`), because construction would recurse infinitely. Holding a *different* locus in `params` with a default is valid; to hold a locus of the same type, omit the default (`next: Node;`) so the caller supplies it.

## The Seven Idiomatic Hale Shapes

Every idiomatic Hale program is composed from seven structural patterns (`spec/styleguide.md`):

| # | Pattern | Shape & When to Use |
|---|---|---|
| 1 | **App locus** | Outer wrapper for a binary (`params` populated from CLI args, `run()` entry point, instantiated as `App { ... };` in `fn main()`). |
| 2 | **Namespace locus** | Empty (or config-only) `params`, methods only. Groups 3+ related functions into a single callable vocabulary. |
| 3 | **Service locus** | Long-lived component using `birth`, `run`, and `dissolve` (and bus subscriptions). Uses sentinel defaults (such as `fd: Int = -1`) so `dissolve` safely no-ops if `birth` stops early. |
| 4 | **Spawned child** | `let s = Child { ... };` scoped to a function, or a dynamic per-connection child attached via `accept(c: Conn)` and reclaimed via `release(c: Conn)`. |
| 5 | **Data collection** | A `@form(vec)`, `@form(hashmap)`, `@form(ring_buffer)`, `@form(lru_cache)`, `@form(set)`, or `@form(heap)` locus managing typed storage ([Collections](./collections.md)). |
| 6 | **Shape type** | `type Foo { a: Int; b: String; }` for pure value records ([Records & Data](./records.md)). |
| 7 | **Free `fn`** | Top-level function for stateless calculations; promote to a Namespace locus once 3+ functions form a domain vocabulary. |

### Example: App Locus and Namespace Locus

```hale
locus Temps {
    fn c_to_f(c: Float) -> Float { return c * 9.0 / 5.0 + 32.0; }
    fn f_to_c(f: Float) -> Float { return (f - 32.0) * 5.0 / 9.0; }
}

locus App {
    params {
        celsius: Float = 100.0;
    }

    run() {
        let conv = Temps { };
        println(self.celsius, "C = ", conv.c_to_f(self.celsius), "F");
    }
}

fn main() {
    App { celsius: 100.0 };
}
```

## Grouping Code with `module` Blocks

A `module NAME { ... }` block visually groups related top-level declarations inside a file without introducing a namespace prefix:

```hale
module geo {
    type Point { x: Int = 0; y: Int = 0; }

    fn manhattan(p: Point) -> Int { return p.x + p.y; }
}

fn main() {
    let p = Point { x: 4, y: 5 };   // referenced as `Point`, never `geo::Point`
    println(manhattan(p));          // 9
}
```

Declarations inside a `module` block behave identically to top-level declarations:
- They are registered under their plain names (`Point`, `manhattan`) across the seed, and exported across `import "lib" as lib;` as `lib::Point`.
- Duplicate declarations across two `module` blocks in the same seed fail with the standard duplicate-name error.
- Hot-path lints, `@effects` / `causes:` checks, and bus topic routing inspect declarations inside `module` blocks identically to top-level items.

> [!WARNING]
> Two program-level constructs must remain at the **top level** and are rejected inside a `module` block:
> 1. **Program entry points:** `fn main()` and the entry `main locus App` must be declared at the top level outside any `module`.
> 2. **Target directives:** `target wasm { ... }` applies to the whole build and must appear at the top level.

Next: [Lists & Maps](./collections.md).
