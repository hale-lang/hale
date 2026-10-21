# Repositories

Some state matters: an order, a ledger row. Most does not: a cache, a
counter, a buffer. A **repository** is a locus you mark `@repo(T)` so the
compiler knows where the state that matters lives, and can tell a read
from a write there without proving anything about purity.

You write the driver by hand (in memory, SQL, a file). The compiler checks
its shape and tracks what its methods do to `T`.

A repository is declared in the program that uses it. A library or imported
module that declares an `@repo` is refused at the check; imported
repositories wait for generated drivers.

## The type and its key

`T` is a record with one field tagged as the key:

```hale,fragment
type OrderId = Int;
type Order {
    id: OrderId `db:"id,key"`;
    customer: Int;
    total: Int;
}
type OrderError = enum { @not_found Missing, @conflict Exists };
```

The key's type is what `get` and `remove` take. A record with no key tag,
or two, is refused at the `@repo`.

## The method set

A repo carries `get`, `create`, `put` and `remove`, each `fallible` with
the one error type, and each declares what kind of change it makes:

```hale,fragment
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

The class is one of `reads`, `writes`, `creates`, `deletes`, `mutates`
(a result that depends on the prior state: an increment, a guarded
transition). Every class except `reads` also says `@idempotent` or
`@not_idempotent`. There is no default: the compiler never answers "is it
safe to repeat" for you, so you answer it at every write.

A method beyond the four, `total_of(customer)` or `find_by_customer`, is
allowed, and carries a class like the rest.

## What you get

The classes are an ordinary effect family, so they travel through calls. A
handler that holds the repo has the repo's classes without saying so, and
a claim can name them:

```hale,fragment
claims {
    forbid reaches(Public, effects(writes(Order)));
}
```

A method with no write class over any repo is read-only, whatever it did
to its own fields.

Atomicity is per repo operation. A handler that calls `get` and then `put`
has made two operations; nothing across two repos is promised. Validation
belongs in a wrapper locus that satisfies the same method set and calls
inward, not in the driver.

A complete in-memory driver and a handler that uses it are in
`tests/hale/repo_memory_test.hl`. The full rules are in
[`spec/api.md`](https://github.com/hale-lang/hale/blob/main/spec/api.md)
§ Repositories.
