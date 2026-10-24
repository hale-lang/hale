import Std

/-
  Certificate mathematics for a finite directed graph. This module makes no
  assumption that a supplied graph faithfully represents a Hale program.
  Extraction, completeness, source identity and lowering are separate
  obligations at the compiler's certificate boundary.
-/
namespace Hale.Claims

abbrev Edges := List (Nat × Nat)

inductive Reachable (edges : Edges) : Nat → Nat → Prop where
  | refl (vertex : Nat) : Reachable edges vertex vertex
  | step {source next target : Nat} :
      (source, next) ∈ edges → Reachable edges next target →
      Reachable edges source target

def ClosedCertificate (edges : Edges) (sources targets closed : List Nat) : Prop :=
  (∀ source ∈ sources, source ∈ closed) ∧
  (∀ target ∈ targets, target ∉ closed) ∧
  (∀ edge ∈ edges, edge.1 ∈ closed → edge.2 ∈ closed)

instance (edges : Edges) (sources targets closed : List Nat) :
    Decidable (ClosedCertificate edges sources targets closed) := by
  unfold ClosedCertificate
  infer_instance

def checkClosed (edges : Edges) (sources targets closed : List Nat) : Bool :=
  decide (ClosedCertificate edges sources targets closed)

theorem reachable_stays_closed (edges : Edges) (closed : List Nat)
    (edge_closed : ∀ edge ∈ edges, edge.1 ∈ closed → edge.2 ∈ closed)
    {source target : Nat} (path : Reachable edges source target) :
    source ∈ closed → target ∈ closed := by
  induction path with
  | refl => exact id
  | @step source next target edge _ tail =>
      intro source_closed
      exact tail (edge_closed (source, next) edge source_closed)

theorem checkClosed_sound (edges : Edges) (sources targets closed : List Nat)
    (accepted : checkClosed edges sources targets closed = true) :
    ∀ source ∈ sources, ∀ target ∈ targets, ¬ Reachable edges source target := by
  have valid : ClosedCertificate edges sources targets closed := of_decide_eq_true accepted
  intro source source_selected target target_selected path
  exact valid.2.1 target target_selected
    (reachable_stays_closed edges closed valid.2.2 path (valid.1 source source_selected))

def walk (edges : Edges) (source : Nat) : List Nat → Option Nat
  | [] => some source
  | next :: rest => if (source, next) ∈ edges then walk edges next rest else none

theorem walk_sound (edges : Edges) (path : List Nat) (source target : Nat)
    (accepted : walk edges source path = some target) :
    Reachable edges source target := by
  induction path generalizing source with
  | nil =>
      simp only [walk, Option.some.injEq] at accepted
      subst target
      exact Reachable.refl source
  | cons next rest tail =>
      simp only [walk] at accepted
      split at accepted
      next edge => exact Reachable.step edge (tail next accepted)
      next => contradiction

def checkViolation (edges : Edges) (sources targets : List Nat)
    (source : Nat) (path : List Nat) : Bool :=
  sources.contains source &&
    match walk edges source path with
    | some target => targets.contains target
    | none => false

theorem checkViolation_sound (edges : Edges) (sources targets : List Nat)
    (source : Nat) (path : List Nat)
    (accepted : checkViolation edges sources targets source path = true) :
    ∃ target, source ∈ sources ∧ target ∈ targets ∧ Reachable edges source target := by
  simp only [checkViolation, Bool.and_eq_true, List.contains_iff_mem] at accepted
  cases result : walk edges source path with
  | none => simp [result] at accepted
  | some target =>
      refine ⟨target, accepted.1, ?_, walk_sound edges path source target result⟩
      simpa [result] using accepted.2

-- Small calculations exercise the actual executable checkers. They use kernel
-- reduction, with no native evaluation axiom and no external solver.
example : checkClosed [(0, 1), (1, 1), (2, 3)] [0] [3] [0, 1] = true := by decide
example : checkClosed [(0, 1), (1, 2)] [0] [2] [0, 1] = false := by decide
example : checkClosed [] [0] [2] [] = false := by decide
example : checkViolation [(0, 1), (1, 2)] [0] [2] 0 [1, 2] = true := by decide
example : checkViolation [(0, 1)] [0] [2] 0 [1, 2] = false := by decide
example : checkViolation [] [1] [1] 1 [] = true := by decide

#print axioms checkClosed_sound
#print axioms checkViolation_sound

end Hale.Claims
