# Repositories

A **repository** is a locus annotated with `@repo(T)` that manages authoritative domain records of type `T` (backed by in-memory storage, SQL, or files). Marking a storage locus with `@repo(T)` enforces a standard CRUD signature contract, requires explicit idempotency declarations on every write operation, and propagates typed state effects (`reads(T)`, `writes(T)`, `creates(T)`, `deletes(T)`, `mutates(T)`) through the call graph so architectural `claims { }` can govern data access.

```hale,fragment
type OrderId = Int;
type Order {
    id: OrderId `db:"id,key"`;
    customer: Int;
    total: Int;
}
type OrderError = enum { @not_found Missing, @conflict Exists };

@repo(Order)
locus Orders {
    @effects(is: { reads(Order) })
    fn get(id: OrderId) -> Order fallible(OrderError) { … }

    @effects(is: { creates(Order) }) @idempotent
    fn create(o: Order) -> Order fallible(OrderError) { … }

    @effects(is: { writes(Order) }) @idempotent
    fn put(id: OrderId, o: Order) -> Order fallible(OrderError) { … }

    @effects(is: { deletes(Order) }) @idempotent
    fn remove(id: OrderId) fallible(OrderError) { … }
}
```

> [!NOTE]
> **Program-Local Declaration Rule**
> An `@repo(T)` locus must be declared in the program seed that uses it. Declaring `@repo(T)` inside an imported library module is rejected by `hale check`.

## Record Key Tags (`db:"...,key"`)

The record type `T` passed to `@repo(T)` must declare **exactly one** field tagged with `key` inside its `db:"..."` field tag:

```hale,fragment
type OrderId = Int;
type Order {
    id: OrderId `db:"id,key"`;
    customer: Int;
    total: Int;
}
```

- The type of the `key`-tagged field (`OrderId`) determines the key parameter type required by `get` and `remove` and the first parameter of `put`.
- A record type with zero `key` tags or multiple `key` tags is rejected at compile time at the `@repo(T)` annotation.

## Required CRUD Methods & State Effect Classes

Every `@repo(T)` locus must implement the four standard CRUD methods (`get`, `create`, `put`, `remove`), all sharing a single `fallible(E)` error type. You may also define additional query or mutation methods on the repository (such as `total_of(customer)` or `find_by_customer`), provided each method declares its state effect class.

### State Effect Classes & Idempotency Rules

| Effect Class | Typical Method | Meaning | Idempotency Annotation Required? |
|---|---|---|---|
| `reads(T)` | `get(id: K) -> T fallible(E)` | Reads `T` without modifying repository state. | No |
| `creates(T)` | `create(v: T) -> T fallible(E)` | Inserts a new record `T`. | **Yes** (`@idempotent` or `@not_idempotent`) |
| `writes(T)` | `put(id: K, v: T) -> T fallible(E)` | Writes or replaces record `T` at `id`. | **Yes** (`@idempotent` or `@not_idempotent`) |
| `deletes(T)` | `remove(id: K) fallible(E)` | Deletes record `T` at `id`. | **Yes** (`@idempotent` or `@not_idempotent`) |
| `mutates(T)` | Custom transition (e.g., `increment`) | Modifies `T` where the resulting state depends on prior state (such as a counter increment or guarded state transition). | **Yes** (`@idempotent` or `@not_idempotent`) |

Every method with a write-family effect (`creates(T)`, `writes(T)`, `deletes(T)`, or `mutates(T)`) **must** carry either `@idempotent` or `@not_idempotent`. The compiler never infers idempotency automatically; omitting the annotation on a write method is a compile error.

## Effect Propagation, `claims { }`, and Atomicity

Repository effect classes participate in Hale's transitive effect system:

- **Automatic propagation**: Any handler or helper method that calls `self.orders.put(...)` transitively acquires the `writes(Order)` effect without needing a manual annotation.
- **Architectural claims**: You can restrict which components or API surfaces are permitted to modify domain records using `claims { }`:
  ```hale,fragment
  claims {
      forbid reaches(Public, effects(writes(Order)));
  }
  ```
- **Read-only classification**: A method that reaches no write-family effect (`writes`, `creates`, `deletes`, `mutates`) on any `@repo` locus is classified as read-only with respect to authoritative domain state, even if it mutates local scratch buffers or caches.
- **Per-operation atomicity**: Atomicity is scoped to a single repository method call. A handler that calls `get` followed by `put` performs two separate operations, and no multi-repository transactions are implied.
- **Separating validation from storage**: Keep `@repo(T)` loci focused on storage driver mechanics. Put domain validation rules in a wrapper locus that exposes the same method signatures and delegates inward to the `@repo(T)` driver.

### Further Reading

- Complete in-memory repository example: `tests/hale/repo_memory_test.hl`
- Full specification: [`spec/api.md`](https://github.com/hale-lang/hale/blob/main/spec/api.md) § Repositories
