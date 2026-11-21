//! GH #2134 (F.62): the check half of `@vectorize`.
//!
//! A fn marked `@vectorize` asserts that every innermost loop written in
//! its body is vectorized by the build's optimizer on the build's target
//! (`hale-codegen`'s `vectorize` module judges that from the vectorizer's
//! own remarks). `hale check` cannot run LLVM, so it refuses what it can
//! see that no target's vectorizer takes, each at the place that stops
//! it:
//!
//! - in the walk (`Checker::check_stmt`): a loop that leaves at a
//!   `break` or a `return` besides its own condition (LLVM 18 vectorizes
//!   a loop with one exit only), and a bus send, a call into the runtime;
//! - over the allocation summary and the effect rows ([`call_laws`]): an
//!   allocation, a publish or a locus built in the loop, a call to a fn
//!   with effects (each a call into the runtime the vectorizer cannot
//!   widen or move), and a call to a fn with a loop of its own (inlined,
//!   it makes the loop an outer one, which the vectorizer never takes,
//!   and which would otherwise pass unseen: an outer loop gets no remark);
//! - and a `@vectorize` fn with no loop at all, which asserts nothing.
//!
//! An element chain's loop (`while true { … let it = src.get(i) or {
//! break; }; … }`) has that one exit, and vectorizes. Not refused,
//! because some of them do vectorize: a fallible step (`v.get(j) or
//! raise` loses its failure branch to bounds-check elimination in `for j
//! in 0..v.len()`), a text value read in the loop (hoisted out of it when
//! it does not change), and an indirect call (devirtualized once the fn
//! is inlined where the fn value is known). What the build's vectorizer
//! says of those is the build's to report.
//! What is left — a reduction the target keeps in order, the cost model,
//! the vector width — is the build's target-dependent assertion.

use std::collections::BTreeSet;

use hale_syntax::ast::{Block, Expr, FnDecl, LocusMember, Program, Stmt, TopDecl};
use hale_syntax::{Diag, Span};

use crate::alloc_summary::{AllocSummary, Callee, DeclId, EffectSiteKind, FnSummary};
use crate::effect_rows::EffectRows;
use crate::listener_window::{visit_block, Visited};
use crate::stdlib_surface::EffectSet;

/// Whether `b` holds a `for` or `while`, at any depth (an `if` arm, a
/// `match` arm, a block expression, an element chain's loop).
pub fn block_has_loop(b: &Block) -> bool {
    let mut found = false;
    visit_block(b, &mut |v| {
        if let Visited::Stmt(Stmt::For { .. } | Stmt::While { .. }) = v {
            found = true;
        }
    });
    found
}

/// The innermost loops of `b`: each `for` and `while` that holds none.
pub fn innermost_loops(b: &Block) -> Vec<Span> {
    let mut out = Vec::new();
    visit_block(b, &mut |v| match v {
        Visited::Stmt(Stmt::For { body, span, .. } | Stmt::While { body, span, .. }) if !block_has_loop(body) => {
            out.push(*span)
        }
        _ => {}
    });
    out
}

/// What stops a loop from vectorizing on every target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Cannot {
    /// A `break` or `return` in a loop that has another exit.
    SecondExit,
    Send,
}

impl Cannot {
    fn says(self) -> &'static str {
        match self {
            Cannot::SecondExit => {
                "a second way out (a `break` or `return` besides the loop's own condition): the \
                 vectorizer takes a loop with one exit"
            }
            Cannot::Send => "a bus send, which is a call into the runtime",
        }
    }
}

/// The walk's findings in one innermost loop: the first place of each
/// kind.
#[derive(Debug)]
pub struct LoopFindings {
    /// How many ways the loop has out: its condition (unless `while
    /// true`) and each `break` and `return` in its body.
    exits: usize,
    found: Vec<(Cannot, Span)>,
}

impl LoopFindings {
    /// The findings frame for `s` when it is an innermost loop.
    pub fn of(s: &Stmt) -> Option<Self> {
        let (cond, body) = match s {
            Stmt::While { cond, body, .. } => (!matches!(cond, Expr::Literal(hale_syntax::ast::Literal::Bool(true), _)), body),
            Stmt::For { body, .. } => (true, body),
            _ => return None,
        };
        if block_has_loop(body) {
            return None;
        }
        let mut exits = usize::from(cond);
        visit_block(body, &mut |v| {
            if let Visited::Stmt(Stmt::Break(_) | Stmt::Return(..)) = v {
                exits += 1;
            }
        });
        Some(LoopFindings { exits, found: Vec::new() })
    }

    pub fn exits_more_than_once(&self) -> bool {
        self.exits > 1
    }

    pub fn note(&mut self, at: Span, what: Cannot) {
        if !self.found.iter().any(|(k, _)| *k == what) {
            self.found.push((what, at));
        }
    }

    pub fn diags(self, fn_name: &str) -> Vec<Diag> {
        self.found.into_iter().map(|(what, at)| refusal(at, fn_name, what.says())).collect()
    }
}

fn refusal(at: Span, fn_name: &str, what: &str) -> Diag {
    Diag::ty(
        at,
        format!(
            "`@vectorize` on `{fn_name}`: this loop cannot vectorize on any target: it holds {what}. \
             Move it out of the loop, or drop `@vectorize` (spec/semantics.md § Vectorized loops)"
        ),
    )
}

/// Every `@vectorize` fn of `program`: the fn, and its name as written
/// (`f`, `Locus.m`).
fn vectorized_fns(program: &Program) -> Vec<(&FnDecl, String)> {
    let mut out = Vec::new();
    for item in &program.items {
        match item {
            TopDecl::Fn(f) if f.vectorize.is_some() => out.push((f, f.name.name.clone())),
            TopDecl::Locus(l) => {
                for m in &l.members {
                    if let LocusMember::Fn(f) = m {
                        if f.vectorize.is_some() {
                            out.push((f, format!("{}.{}", l.name.name, f.name.name)));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Whether any of `programs` declares a `@vectorize` fn.
pub fn declares_any(programs: &[&Program]) -> bool {
    programs.iter().any(|p| !vectorized_fns(p).is_empty())
}

fn within(inner: Span, outer: Span) -> bool {
    outer.start.0 <= inner.start.0 && inner.end.0 <= outer.end.0
}

/// The rules the allocation summary and the effect rows answer, over
/// every `@vectorize` fn of `programs`: a fn with no loop; and in an
/// innermost loop, an allocation, a publish or a locus built, and a call
/// that has effects or loops itself. `effects` is `None`
/// where the rows were not derived: the effect rule is then left to the
/// build.
pub fn call_laws(programs: &[&Program], summary: &AllocSummary, effects: Option<&EffectRows>) -> Vec<Diag> {
    let mut out = Vec::new();
    for program in programs {
        for (f, name) in vectorized_fns(program) {
            let loops = innermost_loops(&f.body);
            if loops.is_empty() {
                let at = f.vectorize.as_ref().map(|v| v.span).unwrap_or(f.name.span);
                out.push(Diag::ty(
                    at,
                    format!(
                        "`@vectorize` on `{name}`: it has no loop, so there is nothing to vectorize. \
                         Drop `@vectorize`, or put it on the fn whose loop must vectorize"
                    ),
                ));
                continue;
            }
            let Some(row) = row_of(summary, f) else { continue };
            let in_loop = |s: Span| loops.iter().any(|l| within(s, *l));
            let mut said: BTreeSet<(usize, &'static str)> = BTreeSet::new();
            let mut say = |at: Span, kind: &'static str, what: String, out: &mut Vec<Diag>| {
                let Some(i) = loops.iter().position(|l| within(at, *l)) else { return };
                if said.insert((i, kind)) {
                    out.push(refusal(at, &name, &what));
                }
            };
            for site in &row.sites {
                if in_loop(site.span) {
                    say(site.span, "alloc", crate::effects::describe_alloc(&site.kind), &mut out);
                }
            }
            for site in &row.effect_sites {
                if in_loop(site.span) {
                    let what = match site.kind {
                        EffectSiteKind::Publish(_) => "a publish, which is a call into the runtime",
                        EffectSiteKind::Spawn(_) => "a locus built, which allocates its region",
                    };
                    say(site.span, "site", what.to_string(), &mut out);
                }
            }
            for call in &row.calls {
                if !in_loop(call.span) {
                    continue;
                }
                let (callee, set, loops_itself) = match &call.callee {
                    Callee::Resolved(k) => {
                        let set = effects.and_then(|e| e.row(k)).map(|r| r.effects).unwrap_or(EffectSet::PURE);
                        let loops = summary.fns.get(k).is_some_and(|s| !s.loops.is_empty());
                        (k.display(), set, loops)
                    }
                    Callee::Unresolved(n) => {
                        let segs: Vec<&str> = n.split("::").collect();
                        let set = crate::stdlib_surface::effects_for(&segs).unwrap_or(EffectSet::PURE);
                        (n.clone(), set, false)
                    }
                };
                if !set.is_unclassified() && set != EffectSet::PURE {
                    let classes = crate::frontier::render_effects(set).join(", ");
                    say(
                        call.span,
                        "effects",
                        format!("a call to `{callee}`, which has effects ({classes})"),
                        &mut out,
                    );
                } else if loops_itself {
                    say(
                        call.span,
                        "loops",
                        format!(
                            "a call to `{callee}`, which has a loop of its own: inlined, it makes this loop \
                             an outer one, which the vectorizer does not vectorize"
                        ),
                        &mut out,
                    );
                }
            }
        }
    }
    out
}

/// The summary's row for `f`, by its declaration's site.
fn row_of<'s>(summary: &'s AllocSummary, f: &FnDecl) -> Option<&'s FnSummary> {
    let decl = DeclId::user(f.id)?;
    summary.fns.values().find(|r| r.key.decl == Some(decl))
}
