# Native API test harness friction

The port was checked and built with this worktree's `target/release/hale`.

- `hale test` expects a successful assertion-based program to be silent. Printing
  routine case names made the runner reject an otherwise successful exit. The
  single native test program runs all 25 named cases without progress output.
- A returned `std::process::Child` cannot be assigned directly into a locus field.
  The fixture constructs the field with the returned OS handles, then disarms the
  temporary wrapper. Stopping a child explicitly closes and clears its pipe
  descriptors before a subsequent spawn can reuse them.
- A failed `std::test::assert` exits without dissolving fixture loci. The suite
  therefore installs an owned-scratch exit watcher before starting children. A
  disposable native smoke program deliberately failed after starting the real
  API; its child was gone and its scratch directory removed afterward.
- Immediately after the first native journal append, cached journal genesis was
  empty. Fixtures read authoritative identity through `GitRecord.identity()`.
- Direct `hale check dna/api/tests` with the September 18 local compiler rejects
  `eprint` as an unknown free function. The cleanup re-exec prints nonempty child
  stderr with `eprintln` instead; success remains silent for the test runner.
