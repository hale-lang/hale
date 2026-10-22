# Part III: Static Verification & Architecture Laws

Because `hale check` derives a complete, canonical graph of your program—every locus, ownership edge, bus publish/subscribe edge, function call, and side effect—you can state architectural and capability invariants in source code and enforce them at compile time with zero runtime overhead.

## Three Levels of Static Contracts

| Level | Construct | What It Enforces |
|---|---|---|
| **1. Function / Locus** | `@effects(...)`, `@no_syscall`, `@no_block`, `@deterministic`, `@budget(...)`, `@phase_effects(...)` | Proves transitively across the call graph (including imported libraries and interface conformers) which side effects or allocations a function or lifecycle phase can reach. |
| **2. Application (`main locus`)** | `group` & `claims { ... }` | Enforces whole-program reachability, isolation (`forbid reaches`), mandatory interposition (`avoiding gate`), secret confinement (`require sealed`), effect attribution (`require attributed`), and topic cardinality (`count publishers`). |
| **3. Organization / Environment** | `constitution` & `adopt` | Packages a reusable set of architectural claims (such as `std::law::Standard`) adopted across entrypoints or bound per deployment environment (`dev`, `prod`) in `hale.toml`. |

When any contract or claim fails, `hale check` rejects the build and emits a concrete **witness path** naming every function call and topic hop that violated the rule.

## Chapters in Part III

- **[Effect Contracts & Resource Budgets](../effects.md)** — Built-in and user-defined effect classes, domain-indexed effects, allocation ceilings (`@budget`), and lifecycle phase restrictions (`@phase_effects`).
- **[Architectural Claims & Topology Proofs](../claims.md)** — Group declarations, reachability/gating/bound/coverage claims, secret confinement (`@sealed` + `std::secret`), and topology snapshot diffing (`--dump-topology`, `--check-topology`).
- **[Constitutions & Multi-Environment Laws](../constitutions.md)** — Writing and extending `constitution` blocks, adopting `std::law::Standard`, and verifying multi-environment matrices (`hale check --matrix`).
