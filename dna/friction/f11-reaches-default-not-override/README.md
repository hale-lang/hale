Two single-file programs (check each on its own with `hale check <file>`):

- `fail_open.hl` — default `Noop`, constructor override `Real` (the
  carrier). `hale check` passes. It should refuse: `Real::apply` runs.
- `false_positive.hl` — default `Real`, override `Noop`. `hale check`
  refuses with a witness through `Real::apply`, which never runs.

The claims engine resolves a call through an interface-typed field
against the field's declaration-site default impl, not the impl the
constructor actually stored (nor the set of all satisfying impls).
