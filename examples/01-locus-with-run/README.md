# 01-locus-with-run

A locus with ongoing work after birth. Bounded — terminates
naturally after `n` ticks, then drains and dissolves.

```
import "std/time";

locus TickerL {
    params {
        n: int = 5;
        interval: duration = 1s;
    }

    run() {
        let mut i: int = 0;
        while i < self.n {
            println("tick ", i);
            time::sleep(self.interval);
            i = i + 1;
        }
    }
}

fn main() {
    TickerL { n: 3, interval: 500ms };
}
```

## What runs

1. `main()` invoked by runtime.
2. `TickerL { n: 3, interval: 500ms }` instantiates the locus
   with overridden params.
3. The locus's `birth()` runs (no-op default; not declared).
4. The locus's `run()` begins executing on its scheduler.
   - Initialize `i = 0`.
   - Loop while `i < 3`: println, sleep, increment.
5. Each `time::sleep` is a cooperative yield point. The scheduler
   may run other loci during the sleep; this scheduler stays on
   the ticker (no other loci on it in this trivial program).
6. After 3 iterations, the loop exits and `run()` returns.
7. `drain()` runs (default: no-op) and `dissolve()` runs
   (default: free region).
8. With no remaining child loci, `main()`'s implicit scope ends
   and the program exits.

## Primitives this exercises (new vs. hello-world)

- **`run()` lifecycle method** — called after birth completes;
  the locus's steady-state work happens here.
- **`while` loop** with mutable iteration variable.
- **`let mut x: T = ...`** — mutable binding.
- **`time::sleep(duration)`** — stdlib function; cooperative
  yield point.
- **Duration literals**: `1s`, `500ms` — type `duration`,
  language-native.
- **`import "std/time"`** — bring stdlib module into scope.
- **Param overrides at instantiation** — `TickerL { n: 3,
  interval: 500ms }` overrides the declared defaults.
- **Multiple-arg `println`** — `println("tick ", i)` concatenates
  string + int.

## Primitives this still does *not* exercise

- contract block, accept lifecycle, multi-locus coordination — 02
- closure tests — 03
- mode declarations — 04
- bus interface — 05
- failure handling, perspectives — later

## What writing this surfaced (for the spec)

Three issues, all resolved in this commit:

1. **`mut` keyword for mutable bindings.** v0.1 had `let
   IDENTIFIER : T = expr` with no mutability marker. Added
   `mut` as a reserved word and updated the `let_stmt`
   production to allow it.

   Decision: bindings are immutable by default; `let mut x`
   for mutable. Aligned with Rust; framework discipline
   prefers explicit mutation marking; agent-first prefers
   explicit over implicit.

2. **Unbound locus handle with `run()` semantics.** Original
   §A in design-rationale committed to "unbound handles
   dissolve at statement boundary." That worked for hello-
   world (only `birth`). For a locus with `run()`, the work
   is async; statement-boundary dissolution would kill the
   loop before it ran.

   Resolution: unbound + has-run() = anonymous child of the
   enclosing function scope. The function scope blocks on the
   child's dissolution before returning. Updated §A.

3. **Function scope as implicit locus.** Implicit in §A
   resolution above: every function (including `main`) has its
   own region; child loci instantiated in the function body
   are children of that region; the function returns when all
   its child loci have dissolved. Documented in §A and as a
   new §D in design-rationale.

## What writing this nearly surfaced but didn't need to

- **Signal handling.** This program terminates naturally;
  no SIGINT needed. A subsequent example with an infinite
  `run` will exercise SIGINT → drain → dissolve.
- **`loop` keyword.** `while true` works; no need for an
  infinite-loop keyword in v0.
- **Range expressions.** `for i in 0..n` would be cleaner,
  but ranges are reserved in precedence.md and we punt.
  `while i < n` works.

## Next on the ladder

`02-parent-child` — a coordinator locus that accepts a child
locus and reads its state through a contract. Adds `accept`,
`contract { expose ... ; consume ... ; }`, contract-graded
visibility, the parent-child memory hierarchy.
