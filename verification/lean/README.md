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

`lean-toolchain` pins the toolchain. The compiled library module is
`GraphCertificates`; `haleGraphCertificates` is the Lake package name, not a
module to pass to `leanchecker`. CI audits declarations defined in the
`GraphCertificates` module, including the `Hale.Claims` namespace, with the
allowlist `propext,Quot.sound`. The audit root names a defining module;
a declaration namespace is not a substitute. A proof that adds an arbitrary
axiom or a native-evaluation assumption fails that audit even if the compiler
accepted its term.

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
It contains no replacement graph interpreter. The positive cases cover:

- A closed cyclic cone with a masked gate, a stdlib interior effect path,
  reflexive violation, the empty-target theorem, phase restriction and bus
  composition.
- Imported library/constitution laws with shadowed group names. This case
  checks both origins rather than treating them as one selected law.
- Locus parameter, data-field and function-argument defaults, each with an
  effect path and a masked helper barrier.
- Explicit closure checkpoints, automatic assertion phase roots and actual
  birth-check predicates.
- Authored self hooks in birth, run, drain and dissolve, plus parent accept
  and release callbacks reached from construction contexts.

Every generated module is checked by Lean and replayed by `leanchecker`.
Negative controls require Lean to reject an omitted-root certificate and the
compiler to refuse theorem export for invalid laws, unresolved automatic
epochs, phase-local failure coverage, conditional birth rejection, unresolved
parent invocation and unresolved invocation at each of the four self stages.
Compiler refusals must leave neither the requested JSON nor Lean output.

Generated JSON, Lean modules and oleans live under
`target/verification/lean/`. `--offline` uses cached Cargo dependencies;
`--lake /absolute/path/to/lake` selects an installed pinned toolchain. The
runner fails on nonzero positive checks, unexpected negative results and
timeouts. CI uploads available evidence as an artifact even when a later
check fails; an artifact or successful export alone is not a passing proof gate.

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
