# Formal Design Model (`%DESIGN v1`)

Hale operationalizes a formal recursive hypergraph model (**`%DESIGN v1`**) at the programming language substrate. Every syntactic construct and compiler invariant in Hale maps to a node, hyperedge, or invariant in this specification.

---

## 1. The Recursive Hypergraph (`%DESIGN v1`)

```text
%DESIGN v1

nodes:
  α   axiom                ; declared, irreducible
  Σ   system               ; α with 1→N decomposition
  Δ   dimension            ; projection of Σ's 1→N
  Π   perspective          ; choice of Δ
  K   capacity             ; bound on Σ
  D   displacement         ; K-full + new → drop-least
  ↑   failure-up           ; D insufficient → Σ.parent
  ⊥   root-as-boundary     ; recursion stops at horizon
  ∥   vertical-only        ; edges ⊆ {parent↔child}
  ⋈   multi-DAG-projection ; substrate-DAGs join via form

hyperedges:                              ; arity ≥ 2
  H1  unfold     (α, Σ, [α₁..αₙ])        ; α-as-Σ has children
  H2  recurse    (Σ, αᵢ)                 ; each child read-as-α
  H3  compose-Δ  (Δ₁, Δ₂) → Δ₃           ; ⊗-closed
  H4  observe    (Π, Δ)                  ; perspective picks Δ
  H5  depth-Δ    (Π, Δd)                 ; cross-depth Π IS a Δ
  H6  bound      (Σ, K)
  H7  displace   (Σ, K-full, new) → kept ; drop reveals priority
  H8  bubble     (Σ, ↑) → Σ.parent.on_fail
  H9  vertical   ∀ edge ∈ Σ-tree : edge ∈ ∥
  H10 lateral-Δ  compose-Δ is licit-lateral-at-Δ-layer only
  H11 root       (Σ.root, ⊥) = current-observable-horizon
  H12 DAG-join   (⋈, DAGᵢ) via form

invariants:
  I1 form         : portable across deployment substrates
  I2 params       : substrate-local
  I3 pyramid      : ∀ d ∈ depths(Σ-tree), ∃ stability-tuple_d
  I4 MS2          : ∀ q ∈ model, q ∈ exactly-one-Σ-tower
                    floating-q ⟹ modeling-error
  I5 form-content : form claims are perspective-invariant;
                    content reduction claims are perspective-conditioned
  I7 lower-reads  : lowering consumes rows, never derives them
  I8 dialect      : every dialect's graph is closed at a named horizon;
                    past it is a hole, never an assumption
```

---

## 2. Four Structural Commitments

All of Hale's runtime and compile-time guarantees derive from four structural rules on a `locus` ($\Sigma$):

1. **Bounded Attachment (`H6`, `K`):** Every locus bounds the memory and child slots attached to it (`capacity { }`), enabling static resource accounting and pre-allocated arenas.
2. **Vertical-Only Ownership (`H9`, `∥`):** Direct references exist only between parent and child loci—never sideways between siblings. Because no pointers cross sideways, a locus's memory arena frees wholesale on `dissolve` with no garbage collector and no borrow checker. Siblings coordinate exclusively through their shared parent or the typed message bus (`H10`, $\Delta$).
3. **Upward Failure Propagation (`H8`, `↑`):** When a runtime invariant (`closure`) is violated, failure routes strictly upward to the parent's `on_failure` policy (`restart`, `restart_in_place`, or `bubble`).
4. **Explicit Horizon Closure (`H11`, `I8`, `⊥`):** Every static graph derived by the compiler closes at a named boundary (locus, contract edge, seed, or `main`). Anything beyond that boundary is recorded explicitly as a `Hole` in the `ApplicationModel`, never silently assumed safe.

---

## 3. Language Construct to Graph Mapping

Every declaration in a `.hl` file populates rows in a specific compiler graph closed at a defined horizon (`I8`):

| Language Construct | Formal Counterpart | Graph Declared | Graph Shape | Closure Horizon ($\perp$) |
|---|---|---|---|---|
| `locus`, `params`, `accept`, `on_failure`, `birth`, `run` | $\Sigma$, `H1`, `H8`, `H9` | Component instances, ownership, supervision | Rooted tree (vertical-only) | Entry's root (`main`) |
| `type` | $\Sigma\text{-proto}$ | Value record layouts (no lifecycle/flow) | Directed acyclic record graph | Its declaration |
| `interface`, `contract`, `expose`, `consume`, `perspective` | $\Pi$, `H4`, `H5` | Surface visibility across depth/process edges | Bipartite graph over depth edges | The contract / process edge |
| `topic`, `bus { publish; subscribe; }`, `<-` | $\Delta$, `H3`, `H10` | Pub/sub channels, publishers, subscribers | Directed multigraph (cycles allowed) | The seed |
| `bindings { }` | $\bowtie$, `H12` | Topic-to-transport mappings and codecs | Edge labeling of the bus graph | `main` / `fleet` |
| `placement { }` | `I1`, `I2` | Instance-to-thread/pool assignment | Partition of locus instances | `main` / `fleet` |
| `capacity { }`, `@form(...)`, `projection` | $K$, `H6`, `H7` | Arena slots and specialized collection ops | Labeled slot set with closed ops | The locus |
| `@effects`, `@no_*`, `@budget`, `@phase_effects` | Capability lattice | Effect classes per function/phase | Lattice + DAG over the call graph | The seed / locus |
| `closure { }` | `I3` | Runtime state invariants over lifecycle epochs | Clause set per locus | The locus |
| `@export`, `@sealed`, `api`, `@rpc` | Surface projection | Externally served operations & role guards | Role-labeled subset of tower surface | `main` |
| `claims { }`, `constitution`, `adopt` | Law hypergraph | Architectural invariants over `ApplicationModel` | Quantified predicates over model relations | Adopting `main` / `fleet` |

---

## 4. Further Reading in the Repository

- **[`AGENTS.md`](https://github.com/hale-lang/hale/blob/main/AGENTS.md):** One-page operational summary of `%DESIGN v1`, the seven idiomatic `.hl` patterns, and compiler workflow rules.
- **[`spec/registry.md`](https://github.com/hale-lang/hale/blob/main/spec/registry.md):** The compiler graph registry mapping every derived fact in `ApplicationModel` to its single producer, its consumers, and its verification tests (`I7`).
- **[`spec/design-rationale.md`](https://github.com/hale-lang/hale/blob/main/spec/design-rationale.md) & [`spec/decisions.md`](https://github.com/hale-lang/hale/blob/main/spec/decisions.md):** Detailed architectural rationale and the numbered `F.1`–`F.40` design decision log.
