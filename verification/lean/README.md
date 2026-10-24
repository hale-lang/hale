# Graph-certificate kernel

`GraphCertificates.lean` defines two executable finite-graph certificate
checkers and proves their soundness in Lean 4.35.0:

- `checkClosed` verifies a set containing all sources, disjoint from targets,
  and closed under every outgoing edge. `checkClosed_sound` proves that no
  selected source can reach a selected target when the certificate is accepted.
- `checkViolation` verifies a source and an explicit edge path ending at a
  selected target. `checkViolation_sound` proves that acceptance establishes
  such a path, including a zero-length path when a source is also a target.

The graph may contain cycles, disconnected vertices, repeated edges and
multiple sources or targets. The executable examples include accepted and
rejected certificates. Proofs use kernel reduction and the standard `propext`
and `Quot.sound` axioms; they use no placeholders, native evaluation axiom,
external solver or Mathlib dependency.

```sh
cd verification/lean
lake build
lake env leanchecker GraphCertificates
```

`lean-toolchain` pins the toolchain. CI also audits the axioms reachable from
the `Hale.Claims` namespace with the allowlist `propext,Quot.sound`. A proof
that adds an arbitrary axiom or a native-evaluation assumption fails that
audit even if the compiler accepted its term.

The proposition concerns the **supplied graph and source/target lists**. It
does not establish that an extracted graph represents every relevant Hale
execution, that a selected source is an authentic declaration, or that lowering
implements it. Model completeness, relevant holes, the claim's projection and
identity, and the binding between source/model/evidence remain obligations of
the compiler certificate interface. The compiler's reachability producer now exports closed-set and path
certificates from the existing judgment, retaining its projection and demand
gates. Use the four `hale check --{dump,check}-reachability-{proofs,lean}`
options documented in `spec/verification.md`. The JSON report and actual Lean
source can both be replayed against current source; replay alone does not run
the kernel. These are separate from topology artifacts and certify only the
selected `forbid reaches` rows.

## Compiler-to-kernel gate

```sh
# From the repository root (Rust and the pinned Lean toolchain required):
python3 verification/lean/check_generated.py
```

The runner builds `hale-types`' LLVM-free `reachability_proofs` example. That
harness loads each real Hale seed through the shared frontend, demands its
canonical model and law selection, and calls the same exporter as the CLI.
It contains no replacement graph interpreter. The fixtures exercise a closed
cyclic cone with a masked gate, a stdlib interior effect witness, a reflexive
violation, the empty-target theorem, phase restriction, bus composition,
imported library/constitution bindings with shadowed group names, and a locus
parameter default, a data-field default and a function-argument default, each with an effect path and a
masked helper barrier, plus an explicit closure-checkpoint path and a closed
barrier excluding its assertion owner. A sixteenth fixture starts a birth-phase path at an automatic assertion
without an authored hook. The compiler fixtures preserve
the explicit-checkpoint body and its scoped failure gap. Every
generated module is checked by Lean and replayed by `leanchecker`. An omitted-root mutation must be
rejected by Lean, and an invalid law must be refused by the compiler. A third
negative control requires unresolved automatic-epoch construction to refuse
theorem export without leaving a JSON or Lean output behind. A fourth
negative control requires a phase-local assertion with unresolved failure
coverage to refuse theorem export too.

Generated JSON, Lean modules and oleans live under
`target/verification/lean/`. `--offline` uses cached Cargo dependencies;
`--lake /absolute/path/to/lake` selects an installed pinned toolchain. The
runner fails on nonzero positive checks, unexpected negative results and
timeouts. CI retains the evidence as an artifact.

The finite certificate checker verifies closure or path arithmetic
independently of the compiler's search bookkeeping. Compiler replay checks
exact canonical documents, so it also binds the propositions, named vertex
mapping, source contents, law selection, analysis version and coverage. Its
FNV identities are stale-evidence checks, not cryptographic signatures. The
remaining trust boundary above still applies.

The birth-check fixtures retain the real predicate clause as a birth-phase
root. A known printing helper produces a path certificate; unresolved
conditional rejection refuses an absence theorem. This does not establish
constructor scheduling, failure-handler admission or runtime refinement.

The lifecycle gate varies the authored self hook across birth, run, drain and
dissolve using the same source fixtures. Every printing body supplies a path
certificate through its guarded may-entry; each pure nonempty body retains
unresolved invocation context and refuses an absence theorem. The kernel
proves a path in the supplied graph. It does not prove that the construction,
run-admission or teardown guard occurs in a feasible runtime trace. No caller
stack charge, admission or retention fact is inferred from that edge.
