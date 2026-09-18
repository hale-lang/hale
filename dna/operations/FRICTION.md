# Operations friction

## Temporary query literal followed by a method call

While extending the native Git fixture, this expression typechecked and built but
the test process terminated by signal:

```hale
let count = ops::Queries { j: dna::GitJournal { repo: repo } }.practice_count();
```

The supported let-bound child shape passes the same fixture:

```hale
let queries = ops::Queries { j: dna::GitJournal { repo: repo } };
let count = queries.practice_count();
```

This is an observed test failure, not a diagnosed compiler root cause. Keep query
loci explicitly owned for the query lifetime; do not rely on a temporary literal
receiver. No compiler changes were made for this slice.
