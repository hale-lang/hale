//! Self lifecycle declarations shared by execution analysis and native lowering.
//! Admission (`accept`/`release`) belongs to the owner's separate invocation
//! account. These rows describe the instance's birth/run/drain/dissolve bodies.
use std::collections::BTreeMap;
use hale_syntax::{ast::{LifecycleKind, NodeId}, Span};
use crate::{alloc_summary::{DeclId, FnKey}, placement::SiteUniverse};
pub use hale_model::LifecycleStage;

/// The real source body of one self lifecycle stage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LifecycleHook {
    pub key: FnKey,
    pub stage: LifecycleStage,
    pub empty: bool,
    pub synthesized: bool,
    pub span: Span,
}
/// Source owner, retaining the analysis API's unminted name-only case.
/// Native lowering requires `declaration`; no identity is fabricated here.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct LifecycleOwner {
    pub universe: SiteUniverse,
    pub declaration: Option<u32>,
    pub name: String,
}
impl LifecycleOwner {
    pub fn new(universe: SiteUniverse, node: NodeId, name: String) -> Self {
        Self { universe, declaration: (!node.is_none()).then_some(node.0), name }
    }
    pub fn declaration_id(&self) -> Option<DeclId> {
        self.declaration.map(|index| DeclId { universe: self.universe, index })
    }
    pub fn matches(&self, universe: SiteUniverse, node: NodeId, name: &str) -> bool {
        self.universe == universe && match self.declaration {
            Some(index) => !node.is_none() && index == node.0,
            None => node.is_none() && self.name == name,
        }
    }
}
/// Includes an empty row for a locus with no self lifecycle declarations.
pub type LifecycleHooks = BTreeMap<LifecycleOwner, Vec<LifecycleHook>>;

/// The source stage, without inferring it from a callable name.
pub fn stage(kind: LifecycleKind) -> Option<LifecycleStage> {
    Some(match kind {
        LifecycleKind::Birth => LifecycleStage::Birth,
        LifecycleKind::Run => LifecycleStage::Run,
        LifecycleKind::Drain => LifecycleStage::Drain,
        LifecycleKind::Dissolve => LifecycleStage::Dissolve,
        LifecycleKind::Accept | LifecycleKind::Release => return None,
    })
}
/// One body projected through the existing source correspondence.
#[derive(Clone, Debug)]
pub struct LoweredHook {
    pub declaration: NodeId,
    pub stage: LifecycleStage,
    pub empty: bool,
    pub synthesized: bool,
}
pub type LoweredHooks = BTreeMap<u32, Vec<LoweredHook>>;

/// User identities are retained by the merge; stdlib identities join through
/// its correspondence. This projects facts and surveys no syntax or plans.
pub fn lower(hooks: &LifecycleHooks, stdlib: impl Fn(NodeId) -> Option<NodeId>) -> Result<LoweredHooks, &'static str> {
    let node = |d: DeclId| match d.universe {
        SiteUniverse::User => Some(NodeId(d.index)),
        SiteUniverse::StdlibAnalysis => stdlib(NodeId(d.index)),
    };
    let mut out = LoweredHooks::new();
    for (owner, hooks) in hooks {
        let declaration = owner.declaration.ok_or("a lifecycle owner has no source identity")?;
        let Some(owner) = node(DeclId { universe: owner.universe, index: declaration }) else { continue };
        let mut rows = Vec::new();
        for hook in hooks {
            let declaration = node(hook.key.decl.ok_or("a lifecycle hook has no source identity")?)
                .ok_or("a merged locus lost its lifecycle declaration correspondence")?;
            rows.push(LoweredHook { declaration,stage:hook.stage,empty:hook.empty,synthesized:hook.synthesized });
        }
        if out.insert(owner.0,rows).is_some() { return Err("lifecycle owner correspondence is ambiguous"); }
    }
    Ok(out)
}
