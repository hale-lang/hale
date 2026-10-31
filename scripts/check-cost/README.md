# Check cost

`hale check` on a program that uses nothing should cost what checking that
program costs: what a program does not use costs nothing. These two scripts
are how a change to the check's cost is measured and held to its outputs.
Every PR that changes the check's cost runs both and puts the numbers in its
body.

## Measure

```sh
cargo build --release
scripts/check-cost/measure.sh target/release/hale cg.out
callgrind_annotate --inclusive=yes cg.out | grep hale_ | head -40
```

`measure.sh` prints the instructions (callgrind's `Ir`) `hale check` spends
on a trivial seed (`fn main() { println("hi"); }`). Instructions, not time:
they are the same from run to run and from machine to machine. Measure the
base (`origin/main`) and the change with the same script, and state both.

## Hold the outputs alike

```sh
scripts/check-cost/compare.py <base hale> <candidate hale>
```

Build the base from `origin/main` and keep its binary aside (one checkout's
`target/` holds one build at a time). `compare.py` runs both over every
corpus example, `dna/host`, `dna/core`, `dna/api`, the API contract program,
every `tests/hale` program and the trivial seed, each plain and under
`--json`, `--dump-alloc-summary`, `--dump-effects-manifest`,
`--dump-topology`, `--dump-resource-budget`, `--warn-resource-leak
--report-unbounded-use` and `--api`. It compares stdout, stderr and the
exit status, with the build-identity digests masked, and prints each run
that differs (a cost cut prints none: `compared 2056 runs, 0 differ`).
