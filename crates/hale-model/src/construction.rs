//! Source construction identities and lifecycle context inventory.
//!
//! Templates, source occurrences, arranged instances and runtime incarnations
//! are different grains. None of these rows assert actual admission, runtime
//! liveness, exact ancestry or execution completeness. This inventory is not
//! consumed as a lifecycle safety certificate.

use crate::{LifecycleStage, LocusDeclId, ProvenanceId};
use hale_graph::ids::SiteId;

/// Which store minted a site. The two stores number independently, each
/// from seed 0 and index 0, so a `SiteId` names a site only with its
/// universe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SiteUniverse {
    /// The snapshot's identities: the seed's files and the imports linked
    /// in.
    User,
    /// The stdlib analysis copy's (minted independently of user sources).
    StdlibAnalysis,
}

/// A site the table names: the universe that minted it and its id there.
/// Every site in the table is one. Equality, ordering and hashing compare
/// the universe first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SiteRef {
    /// The independent identity store that minted this site.
    pub universe: SiteUniverse,
    /// The site within that universe; meaningful only with its universe.
    pub id: SiteId,
}

impl SiteRef {
    pub fn user(id: SiteId) -> SiteRef {
        SiteRef {
            universe: SiteUniverse::User,
            id,
        }
    }

    pub fn stdlib(id: SiteId) -> SiteRef {
        SiteRef {
            universe: SiteUniverse::StdlibAnalysis,
            id,
        }
    }
}

/// Static instance identity: the scope that constructs the tree, the
/// field path from its top, the replica. A key names a template, never a
/// runtime occurrence.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InstanceKey {
    /// The original construction, binding or entry that constructs this tower.
    pub origin: Origin,
    /// The fields from the origin's top; empty is the top itself.
    pub path: Vec<Step>,
    /// `Some(i)` on a `replicas = K > 1` field and on every row nested
    /// under it.
    pub replica: Option<u32>,
}

/// The scope that constructs a static tower.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Origin {
    /// A literal: one of the root's (`App { … }`), one template per
    /// literal site with the root at `[]`, or one the entry `fn main`
    /// builds directly (the placement table's entry literals), its locus at
    /// `[]`.
    Construction(SiteRef),
    /// An adapter literal in the root's `bindings { }`, by its binding
    /// entry's site (the adapter literal has no site of its own): built
    /// once, in the bindings prelude, however often the root is
    /// constructed.
    Binding(SiteRef),
    /// The entry itself, constructing a root no literal builds: the
    /// root's implicit template, by the entry's site (the top-level user
    /// `fn main`'s when there is one, else the root declaration's). Its
    /// tower is enumerated exactly as under a literal, from the
    /// declaration's defaults, and its bound is `Once`: the entry runs
    /// once per process.
    Entry(SiteRef),
}

/// One field of a key's path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Step {
    /// The canonical params field, including its element position when present.
    pub field: String,
    /// The literal taken, when the field's initializer chooses among
    /// literals; `None` when it has one.
    pub alternative: Option<SiteRef>,
}

/// Where an instance comes from: a template of the static tower, or a
/// literal outside it (a method or fn body, a let-bound literal, an
/// accepted child), every occurrence of which shares the literal's rows.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Template {
    Static(InstanceKey),
    Dynamic { literal: SiteRef },
}

/// Source construction bound. This does not establish a runtime population ceiling.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Bound {
    /// At most one.
    Once,
    /// At most `n`.
    AtMost(u32),
    /// No static bound; the reason, which a count over the table renders
    /// as its uncertainty.
    Unbounded(String),
}

/// The reason a literal written inside a loop is unbounded.
pub const BUILT_IN_A_LOOP: &str = "built in a loop";

impl Bound {
    /// The literal is written inside a loop, so one site builds an
    /// occurrence per iteration. Distinct from a literal in a scope that
    /// runs many times (a factory called in a loop, a locus body), each
    /// run of which builds one occurrence at its own site.
    pub fn built_in_a_loop(&self) -> bool {
        matches!(self, Bound::Unbounded(why) if why == BUILT_IN_A_LOOP)
    }
}

/// A domain's index in the source domain table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DomainId(pub u32);

/// A resolved set of logical CPUs, sorted and deduplicated: a `cores`
/// spec expanded, or a `node` / `l3` affinity resolved against the
/// root's `topology { }`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CoreSet(pub Vec<i64>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainKind {
    /// The program's main thread; adds no thread.
    Main,
    /// One worker per name. A pool carries at most one affinity (rule 16).
    Pool {
        name: String,
        async_io: bool,
        affinity: Option<CoreSet>,
    },
    /// One thread per anchor (per replica); the anchor's origin is the
    /// scope that creates it.
    Pinned {
        anchor: InstanceKey,
        affinity: Option<CoreSet>,
        numa_node: Option<i64>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Domain {
    /// Index in this inventory's source-domain table.
    pub id: DomainId,
    /// Resolved source execution-domain kind.
    pub kind: DomainKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiteralPosition {
    /// `Kid { };` — torn down where the statement ends (the eager spine).
    Statement,
    /// `let k = Kid { };` — torn down at the scope's exit (a frame entry).
    Let,
    /// Inside another expression (a receiver, an argument, a field
    /// default): torn down with the frame, as a let-bound one is.
    Nested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Construction {
    /// A template's top: a root or entry literal, or the entry's implicit
    /// construction of the root; `in_fn_main` where `fn main`'s frame
    /// holds it (a root built by another fn is torn down at that fn's
    /// exit).
    Top {
        built: LiteralPosition,
        main_locus: bool,
        in_fn_main: bool,
    },
    /// A params field of its owner.
    Field,
    /// An adapter of the root's `bindings { }`.
    Adapter,
    /// A literal its enclosing locus accepts.
    Accepted { flow: bool },
    /// A literal its scope tears down.
    Body {
        built: LiteralPosition,
        in_fn_main: bool,
    },
}

/// One parent context of a template: the owner it is built under there,
/// and the domains it is built and runs on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contribution {
    /// The instance whose `on_failure` it fails to and whose arena holds
    /// it: a field's owner, an accepted child's acceptor, a body
    /// literal's enclosing locus. `None` for a template's top, a literal
    /// in a free fn, and the occurrences of a cycle of owners.
    pub owner: Option<usize>,
    /// A representative of the owner's execution context: for a field,
    /// its owner's context with the field's own placement; for a body or
    /// accepted literal, the enclosing occurrence that runs it. `None`
    /// pairs it with every contribution of the owner.
    pub under: Option<usize>,
    /// The instantiating thread: the domain running the code that holds
    /// the literal. `None` when the scope's domains are unknown or many.
    pub it: Option<DomainId>,
    /// The queue owner: where its `run()` runs and its cells land.
    pub own: Option<DomainId>,
    /// Every represented construction contribution runs in a failure handler.
    pub in_handler: bool,
}

/// The source identity that defines a concrete specialization. Substitution
/// identity is the compiler's canonical lowered key, never a display spelling.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DeclarationKey {
    /// Declaration identity and the independently minted source universe.
    pub site: SiteRef,
    /// Canonical specialization key; never an author-facing display spelling.
    pub specialization: String,
}

/// A known declaration binding or an explicit boundary of the entity horizon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclarationBinding {
    Declared(LocusDeclId),
    Foreign,
    Unrepresented,
}

/// A declaration reference, preserving source identity even outside entities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextDeclaration {
    /// Original declaration and specialization identity.
    pub key: DeclarationKey,
    /// Binding into user model entities, or a typed entity-horizon boundary.
    pub binding: DeclarationBinding,
    /// Original declaration or construction witness, in its own source universe.
    pub provenance: ProvenanceId,
}

/// An ownership coverage gap; unknown ownership never proves an ownerless object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnerGap {
    Recursive { candidate: usize },
    UnrepresentedEnclosing { declaration: ContextDeclaration },
    MissingPlacedOwner { key: InstanceKey },
}

/// Canonical projection of one source template, in source construction order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleContext {
    /// The constructed declaration, including foreign identity and coverage.
    pub declaration: ContextDeclaration,
    /// Source template identity; never a runtime instance or incarnation.
    pub template: Template,
    /// Original constructor literal, when known. Implicit entries and bindings
    /// have no authored constructor site; unknown field literals stay absent.
    pub construction_site: Option<SiteRef>,
    /// Construction and retention shape from the shared source family.
    pub how: Construction,
    /// All possible owner/domain contributions, addressing this same inventory.
    pub contributions: Vec<Contribution>,
    /// Unresolved ownership coverage, never evidence of runtime ownerlessness.
    pub owner_gaps: Vec<OwnerGap>,
    /// Source construction bound; not a certified live population or byte ceiling.
    pub bound: Bound,
    /// Every represented construction contribution runs in a failure handler.
    pub in_handler: bool,
    /// Original lifecycle stage enclosing a statement construction, if known.
    pub statement_of: Option<LifecycleStage>,
    /// The canonical params field, including its element position when present.
    pub field: Option<String>,
    /// Whether placement decided this template's domain independently of its owner.
    pub placed: bool,
    /// Original declaration or construction witness, in its own source universe.
    pub provenance: ProvenanceId,
}

/// One source execution domain with the rule or declaration introducing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextDomain {
    /// Original placement-domain identity and resolved kind.
    pub source: Domain,
    /// Source witness or named placement derivation rule.
    pub provenance: ProvenanceId,
}

/// Source facts, not an execution relation. Missing inventory is represented by
/// `None` in `Analyses`; an empty inventory is an available source world.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LifecycleInventory {
    /// Original placement domains, in dense source ID order.
    pub domains: Vec<ContextDomain>,
    /// Every source template, in producer order; owner indices retain this grain.
    pub contexts: Vec<LifecycleContext>,
}

impl LifecycleInventory {
    pub fn validate(&self, model: &crate::ApplicationModel) -> Result<(), crate::ModelError> {
        let invalid = |index| crate::ModelError::InvalidLifecycleContext { index };
        let valid_prov = |p: ProvenanceId| p.index() < model.provenance.records.len();
        let valid_decl = |d: &ContextDeclaration| {
            valid_prov(d.provenance)
                && !d.key.specialization.is_empty()
                && match d.binding {
                    DeclarationBinding::Declared(id) => {
                        d.key.site.universe == SiteUniverse::User
                            && model
                                .entities
                                .loci
                                .get(id.index())
                                .is_some_and(|l| l.decl == Some(d.key.site.id))
                    }
                    DeclarationBinding::Foreign => {
                        d.key.site.universe == SiteUniverse::StdlibAnalysis
                            && matches!(
                                model.provenance.records.get(d.provenance.index()),
                                Some(
                                    crate::Provenance::ForeignSpan { .. }
                                        | crate::Provenance::Synthetic { .. }
                                )
                            )
                    }
                    DeclarationBinding::Unrepresented => d.key.site.universe == SiteUniverse::User,
                }
        };
        if !matches!(
            self.domains.first(),
            Some(ContextDomain {
                source: Domain {
                    id: DomainId(0),
                    kind: DomainKind::Main
                },
                ..
            })
        ) {
            return Err(invalid(0));
        }
        let mut declarations = std::collections::BTreeSet::new();
        for (i, d) in model.entities.loci.iter().enumerate() {
            if d.decl.is_some_and(|site| !declarations.insert(site)) {
                return Err(invalid(i));
            }
        }
        let mut domains = std::collections::BTreeSet::new();
        for (i, d) in self.domains.iter().enumerate() {
            if d.source.id.0 as usize != i
                || !domains.insert(d.source.id)
                || !valid_prov(d.provenance)
            {
                return Err(invalid(i));
            }
        }
        let mut templates = std::collections::BTreeSet::new();
        for (i, c) in self.contexts.iter().enumerate() {
            if !valid_decl(&c.declaration)
                || !valid_prov(c.provenance)
                || !templates.insert((c.declaration.key.clone(), c.template.clone()))
                || c.in_handler
                    != (!c.contributions.is_empty() && c.contributions.iter().all(|p| p.in_handler))
                || c.statement_of.is_some()
                    && !matches!(
                        c.how,
                        Construction::Body {
                            built: LiteralPosition::Statement,
                            ..
                        }
                    )
            {
                return Err(invalid(i));
            }
            let root_site = match &c.template {
                Template::Dynamic { literal } => Some(Some(*literal)),
                Template::Static(key) if key.path.is_empty() => Some(match key.origin {
                    Origin::Construction(site) => Some(site),
                    Origin::Entry(_) | Origin::Binding(_) => None,
                }),
                Template::Static(_) => None,
            };
            if root_site.is_some_and(|site| c.construction_site != site) {
                return Err(invalid(i));
            }
            for p in &c.contributions {
                if [p.it, p.own]
                    .into_iter()
                    .flatten()
                    .any(|d| d.0 as usize >= self.domains.len())
                {
                    return Err(invalid(i));
                }
                if let Some(owner) = p.owner {
                    let Some(parent) = self.contexts.get(owner) else {
                        return Err(invalid(i));
                    };
                    if owner == i || p.under.is_some_and(|n| n >= parent.contributions.len()) {
                        return Err(invalid(i));
                    }
                } else if p.under.is_some() {
                    return Err(invalid(i));
                }
            }
            for gap in &c.owner_gaps {
                match gap {
                    OwnerGap::Recursive { candidate } if *candidate >= self.contexts.len() => {
                        return Err(invalid(i))
                    }
                    OwnerGap::UnrepresentedEnclosing { declaration }
                        if !valid_decl(declaration) =>
                    {
                        return Err(invalid(i))
                    }
                    _ => {}
                }
            }
        }
        let mut indegree = vec![0usize; self.contexts.len()];
        let mut children = vec![Vec::new(); self.contexts.len()];
        for (i, c) in self.contexts.iter().enumerate() {
            let parents: std::collections::BTreeSet<_> =
                c.contributions.iter().filter_map(|p| p.owner).collect();
            indegree[i] = parents.len();
            for owner in parents {
                children[owner].push(i);
            }
        }
        let mut ready: Vec<_> = indegree
            .iter()
            .enumerate()
            .filter_map(|(i, n)| (*n == 0).then_some(i))
            .collect();
        while let Some(i) = ready.pop() {
            for &child in &children[i] {
                indegree[child] -= 1;
                if indegree[child] == 0 {
                    ready.push(child);
                }
            }
        }
        if let Some(i) = indegree.iter().position(|n| *n != 0) {
            return Err(invalid(i));
        }
        Ok(())
    }
}
