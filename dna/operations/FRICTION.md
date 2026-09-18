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

## JSON nullable fields are not optional strings

The stdlib's `find_string_field` returns the raw scalar spelling when a field
is not a JSON string. For a native topology root's `owner: null`, that is the
string `"null"`, not an empty value. The Organization reader first inspects the
raw field, validates `null | string`, and explicitly maps null to an absent
parent. Otherwise it would invent a parent instance named `null`. The native
compiler fixture covers both the root and real containment edges.

## Cache the String byte length in parser scans

Native `len(String)` calls `strlen`; checked one-byte string slices also inspect
the source length. Repeating them while walking a large artifact made strict
JSON validation quadratic. A generated DNA organization's 1 MiB artifact took
10.6 seconds to validate even though source acquisition took under one second.
The parser now passes one cached length through recursive helpers and uses
`byte_at_unchecked` only after explicit bounds checks. Keys are copied from their
already validated ranges into a small byte builder. The same artifact validates
in about 17 ms; the one MiB native regression retains a generous three-second
bound. Parent indices are also resolved once before containment traversal, so
following a deep chain does not perform another linear identity search per edge.
