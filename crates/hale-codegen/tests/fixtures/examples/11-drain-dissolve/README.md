# 11-drain-dissolve

`drain()` and `dissolve()` lifecycle methods firing at scope-exit
in F.4 depth-first cascade order.

## What it shows

Each locus's lifecycle runs in order birth → run → drain →
dissolve. When a parent's `run()` body instantiates children,
each child synchronously completes its own full sequence (birth
→ run → drain → dissolve) before returning. By the time the
parent's `drain()` fires, all descendants are already gone — the
cascade is implicit in v0's synchronous-instantiation model.

```
$ hale run examples/11-drain-dissolve/main.hl
parent: birth
child-a: birth
child-a: drain
child-a: dissolve
child-b: birth
child-b: drain
child-b: dissolve
parent: drain
parent: dissolve

$ hale build examples/11-drain-dissolve/main.hl
$ ./examples/11-drain-dissolve/main
[same output]
```

## Why this is interesting

This is the codegen-arc milestone that closes the lifecycle
quartet (m10). Previously `drain` / `dissolve` declarations were
rejected by `hale build`; now they're lowered to LLVM functions
and dispatched at the end of `lower_locus_instantiation`,
mirroring what the interpreter does inside `dissolve_locus`.

Per F.4, drain "always cascades depth-first." In v0 that
cascade is structural, not explicit: children are instantiated
inside their parent's lifecycle bodies, and each child dissolves
before the body returns. When the cooperative scheduler lands
and loci can be long-lived, the cascade walks explicitly; the
lifecycle-method ABI doesn't change.

## Notable interpreter parity work

The interpreter previously never invoked user-declared `drain()`
bodies — it treated drain as the cascade only, with `dissolve()`
being the only invocable cleanup hook. As of this milestone both
paths invoke `drain()` between the child cascade and the closure
evaluation, so the two paths produce identical stdout.

A second pre-existing bug surfaced: ephemeral children dissolved
twice — once at end-of-instantiation, then again via the
parent's child cascade because they were still in
`parent.children`. The fix pops the just-dissolved ephemeral
from the parent's child list so its drain/dissolve only fires
once.
