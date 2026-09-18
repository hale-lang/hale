# Iris cockpit delivery checklist

Scope: [product issue #690](https://github.com/hale-lang/hale/issues/690) and
[service development plan](../../dna/SERVICE-DEVELOPMENT-PLAN.md). Baseline
`a757226d` plus the Organization increment in the working tree, assessed
2026-09-18. Checked boxes describe implemented, tested
behavior; unchecked boxes remain required. A read-only workspace is an
increment toward the goal, not completion of its administration requirement.

## Delivered foundation

- [x] Independently served browser assets; authenticated, typed DNA API reads;
  stable Record identity, snapshot-bound pagination and explicit source errors.
- [x] Practices and Reviews browse/detail, cross-links, opaque identifiers,
  Unicode/multiline text, approval distinct from practice adoption, receipt
  visibility enforced by the backend, and no unsafe HTML rendering.
- [x] Session loss clears old content; obsolete responses cannot restore it;
  browser history and mobile detail/back navigation preserve context.
- [x] Runtime workspace connects to the existing independent observer without
  requiring a DNA session. This link is not the generic administration gate.
- [x] Native API/contract fixtures and Chromium tests use real temporary Git
  Records and native receipt writers; read-only browser suite has 18 passing
  cases. Real OIDC flow is covered by the native API suite, while browser
  session-loss cases use intercepted failures.

## Current slice: Organization, then Definitions and Knowledge

- [x] Organization read adapter from actual compiler declarations/static
  instances, pinned to source revision and artifact identity; the separate
  declared ownership map comes from the same source revision. No hand-maintained org
  registry or inference that a declaration is a running/vacant position.
- [x] Organization browser acceptance: nested/repeated instances remain
  distinct, declaration-only coverage is explicit, exact source provenance is
  visible, history and mobile inspection work, and unavailable/stale sources
  cannot masquerade as current or empty results.
  The 18-case Chromium suite passed in 36.7 seconds without retries against real
  native fixtures, with `HALE_BIN`
  explicitly selecting the supplied native compiler. This validates declaration
  inspection, not semantic positions or administration.
- [x] Normally generated projects preserve and inspect actual ignored vendor
  dependencies; dependency bytes have separate provenance and invalidate the
  cache at unchanged source HEAD. Missing/invalid dependencies clear old UI,
  and inspection never upgrades dependencies or changes the caller's files.
- [ ] Definitions catalog reads the application's real `WorkflowCatalog`,
  exact revisions, ordered Steps, leaf specifications and child references.
  Definition, leaf member specification and admitted Work remain distinct.
- [ ] Knowledge reads enumerate bounded nodes/edges/bindings, provenance,
  applicability and dependents from the authoritative store. Normative practice
  authority remains distinct from descriptive assertions. Protected information
  cannot leak through graph edges, search, labels or counts.

## Core administration: all four workspaces

- [ ] Organization: propose, validate, review and adopt supported position,
  responsibility, ownership, performer and grant changes through source/domain
  authority; show affected work and preserve/reassign obligations on retirement.
  Semantic position metadata and bindings must come from a source-declared
  domain catalog; compiler instance paths alone do not provide those meanings.
- [ ] Practices: author, scope, review, activate, supersede and retire with
  typed attribution, exact subject identity, impact and evidence. An approved
  Review may still have pending/refused adoption; show both facts.
- [ ] Knowledge: create/correct supported assertions and relationships, bind
  them to consumers, inspect impact, supersede/retire through domain rules;
  historical attempts retain what they actually read.
- [ ] Definitions: author and version native leaf fields and recursive Steps,
  reject cycles/bounds, inspect dependents, publish through governed source
  activation; earlier admissions retain revision N after N+1 is published.
- [ ] Source editing has a bounded, round-trippable model: preserve unsupported
  source, expose deterministic diff and verification, distinguish draft/candidate/
  accepted/deployed identities, reject stale concurrent edits. General-purpose
  drag-and-drop assembly is later; these four editors are not deferred to it.

## Shared authority, execution and operation

- [ ] Active viewing position consistently scopes Organization, Knowledge,
  Practices, Definitions, Attention and Work; URLs/history retain it. Principal,
  viewing context, acting position, owner and effective grants remain distinct.
  Changing context confers no permission and never impersonates an occupant.
- [ ] Browser and remote CLI share typed operations, authenticated authority,
  subject/version preconditions, stable request identity and durable receipts.
  Duplicate/retried requests recover one result; changed payload conflicts;
  reconnect finds the original command; unknown effects remain unresolved.
- [ ] Commands survive API/UI restart and interrupted domain progression;
  accepted/submitted is never shown as completed. Refusals and permitted
  reconciliation are inspectable with evidence and affected identities.
- [ ] Work joins native recipe/facts/projection and routed memories: non-code
  outcomes, human obligations, attempts, barriers, retries, cancellation and
  restart. Reuse the workflow implementation; Iris does not schedule work or
  infer completion from missing processes. Missing runtime capabilities stay
  explicit dependencies, not simulated success.
- [ ] Plain Hale has a common public API and one declared administrative
  operation with revision precondition and app-owned persisted receipt, tested
  with no DNA Record, body, database or fabricated DNA capabilities.
- [ ] Source/semantic/verification evidence for an exact candidate leads through
  authorized decision to deployment, observation and supported rollback.
- [ ] Attention, Changes and Activity connect obligations, causes and results;
  application/organization/runtime identity and observation coverage stay clear.
- [ ] Fresh service deployment, external state ownership/readiness, remote CLI,
  restart/retry, migration and matching Record/Ledger backup/restore are tested.
  Existing local development and observation remain usable.

## Completion gates from #690

- [ ] **Plain Hale:** attach, diagnose, invoke one app-declared administrative
  capability and verify its result without DNA.
- [ ] **Ordinary DNA work:** submit a non-code outcome, inspect human/software
  work, satisfy an obligation and verify evidence, including wait and refusal.
- [ ] **Application evolution:** inspect the exact candidate, decide with
  effective authority, follow deployment and observation or rollback.
- [ ] **Shape the organization:** revise a position, curate linked knowledge,
  establish a scoped practice, publish a definition, run it, navigate to exact
  bindings, then revise a dependency without rewriting an earlier run.
- [ ] Each gate includes two actors/positions, stale/concurrent decisions,
  session expiry, reconnect, protected evidence and an attributable outcome.
  Browser/CLI receipts agree; application work survives API/UI restarts.

Completion requires these operational demonstrations, not just navigation or
schema coverage. The application chooses its org shape, policies, performers
and business procedures; Iris supplies faithful inspection and authorized
administration of those choices.
