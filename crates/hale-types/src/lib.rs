//! Hale type checker. Phase 1 milestone 2.
//!
//! Public surface:
//! - [`check_program`] — check a single program in isolation.
//! - [`check_bundle`] — check a multi-file bundle (e.g., a
//!   project that imports across files).
//! - [`Bundle`] — the compilation-unit shape the bundle checker
//!   takes.
//! - [`ty::Ty`] — resolved-type representation.
//!
//! Milestone-2 cut: literal typing, binary/unary op type
//! compatibility, struct-literal field typing, bus-send
//! subject + payload type matching, closure-assertion type
//! compatibility, `self.field` resolution against enclosing
//! locus's params. Externally-imported names (stdlib paths
//! like `time::sleep`, `println` builtins) resolve to
//! `Ty::Unknown` and pass through.
//!
//! Deferred to milestone 3: contract compatibility (F.8),
//! generic instantiation, k_max compile-time computation,
//! closure cycle existence, full call-site signature checking
//! against built-ins.

pub mod alloc_summary;
pub mod check;
pub mod purity;
pub mod resolve;
pub mod resource_budget;
pub mod symbol;
pub mod sync_inference;
pub mod ty;
pub mod working_set;

/// m94: subject wildcard matching used by the type checker
/// (publish-side authorization for computed subjects) and
/// mirrored at runtime by the C runtime's `lotus_subject_match`.
/// v0 supports a
/// trailing `**` that matches *zero or more* remaining
/// dot-separated segments — `log.app.**` matches the root
/// `log.app` AND any descendant. Both implementations
/// must agree.
pub fn wildcard_match(pattern: &str, subject: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix("**") {
        if prefix.is_empty() {
            return true;
        }
        if !prefix.ends_with('.') {
            return false;
        }
        let root = &prefix[..prefix.len() - 1];
        if subject == root {
            return true;
        }
        subject.starts_with(prefix) && subject.len() > prefix.len()
    } else if pattern.contains("**") {
        false
    } else {
        pattern == subject
    }
}

use std::collections::BTreeMap;

use hale_syntax::ast::Program;
use hale_syntax::Diag;

pub use crate::symbol::Bundle;
pub use crate::ty::Ty;

/// Check a single program. Returns all diagnostics from
/// resolution + type checking.
pub fn check_program(program: &Program) -> Vec<Diag> {
    let mut programs: BTreeMap<String, &Program> = BTreeMap::new();
    programs.insert(String::new(), program);
    check_bundle(&Bundle { programs })
}

/// Check a bundle of programs (one logical compilation unit
/// spread across multiple `.hl` files, linked by `import`).
pub fn check_bundle(bundle: &Bundle<'_>) -> Vec<Diag> {
    check_bundle_opts(bundle, false)
}

/// Like `check_bundle`, but `allow_unowned_subscriber` downgrades
/// the "bus-subscribing locus instantiated unowned in a method
/// body" hard error to allowed — the `--allow-unowned-subscriber`
/// escape hatch for code that manages the subscriber's lifetime
/// some other way.
/// Render the per-method allocation summary + call graph for a bundle
/// (GH #18 item 1). Drives `--dump-alloc-summary`.
pub fn dump_alloc_summary(bundle: &Bundle<'_>) -> String {
    let progs: Vec<&hale_syntax::ast::Program> = bundle.programs.values().copied().collect();
    alloc_summary::summarize_programs(&progs).render()
}

/// Render the per-program resource budget — pinned threads, cooperative
/// pools, bus subjects (GH #18 item 5, count slice). Drives
/// `--dump-resource-budget`.
pub fn dump_resource_budget(bundle: &Bundle<'_>) -> String {
    let progs: Vec<&hale_syntax::ast::Program> = bundle.programs.values().copied().collect();
    resource_budget::budget_for_programs(&progs).render()
}

/// Bound-solver warnings: one per unbounded-accumulation allocation site
/// (GH #18 item 1). `include_all = false` reports only sites inside a
/// `@bounded` locus (the always-on in-source opt-in); `true` is the
/// whole-program survey behind `--warn-unbounded-alloc`. `@unbounded`-fn
/// sites are suppressed in both modes.
pub fn unbounded_alloc_warnings(bundle: &Bundle<'_>, include_all: bool) -> Vec<Diag> {
    let progs: Vec<&hale_syntax::ast::Program> = bundle.programs.values().copied().collect();
    alloc_summary::unbounded_alloc_diags(&progs, include_all)
}

/// Resource-leak warnings: an fd-acquiring call whose result is stored
/// resident in an unbounded context (GH #18 item 5, leak stage). Opt-in
/// via `--warn-resource-leak`.
pub fn resource_leak_warnings(bundle: &Bundle<'_>) -> Vec<Diag> {
    let progs: Vec<&hale_syntax::ast::Program> = bundle.programs.values().copied().collect();
    resource_budget::resource_leak_diags(&progs)
}

/// Check a bundle's resource counts against declared ceilings (GH #18 item
/// 5, the CI gate). Returns one violation message per over-budget resource
/// (empty = within budget). Drives `--check-resource-budget`.
pub fn check_resource_ceiling(
    bundle: &Bundle<'_>,
    ceiling: &resource_budget::ResourceCeiling,
) -> Vec<String> {
    let progs: Vec<&hale_syntax::ast::Program> = bundle.programs.values().copied().collect();
    let budget = resource_budget::budget_for_programs(&progs);
    resource_budget::check_ceiling(&budget, ceiling)
}

pub fn check_bundle_opts(
    bundle: &Bundle<'_>,
    allow_unowned_subscriber: bool,
) -> Vec<Diag> {
    let (top, mut diags) = resolve::build_top_scope(bundle);
    diags.extend(check::check_bundle(bundle, &top, allow_unowned_subscriber));
    diags
}

/// FUv0.8.2 #4 (2026-05-25): auto-apply sync inference.
///
/// Walks the program for every `@form(hashmap)` locus that
/// carries no explicit `sync = ` kwarg, runs the F.32-1∞
/// inference, and injects the picked discipline as a
/// synthetic `FormArg` on the locus's annotation when the
/// rule produces a non-None pick.
///
/// Designed to run BEFORE [`check_bundle`]:
///
///   apply_sync_inference(&mut program);
///   let diags = check_bundle(&bundle);  // sees the synced AST
///
/// Net behavior: a user who writes
///
///   @form(hashmap)
///   locus Registry { capacity { pool entries of E indexed_by k; } }
///
/// and accesses it cross-pool gets the inference's
/// discipline applied automatically — `sync = serialized` /
/// `sync = striped` per the rule. The cross-pool diagnostic
/// (F.32-0) sees an explicit sync and stays quiet.
///
/// Loci with an existing `sync = X` arg are left alone.
/// Inference returning `SyncDiscipline::None` (single-pool
/// use) also leaves the locus alone — no annotation injected
/// because none is needed.
///
/// Returns any diagnostics raised during the resolver pass
/// (typically empty). Codegen errors surface later in
/// `check_bundle` / `build_executable` as usual.
pub fn apply_sync_inference(
    program: &mut hale_syntax::ast::Program,
) -> Vec<Diag> {
    use hale_syntax::ast::{Expr, FormArg, Ident, TopDecl};

    // Build a temporary single-program bundle for the
    // resolver. The bundle borrows the program immutably; we
    // drop it before mutating.
    let inferred = {
        let mut programs = BTreeMap::new();
        programs.insert(String::new(), &*program);
        let bundle = Bundle { programs };
        let (top, diags) = resolve::build_top_scope(&bundle);
        if !diags.is_empty() {
            // Resolver errors will be re-raised by
            // `check_bundle`; auto-apply skips work in that
            // case (the program won't compile anyway).
            return diags;
        }
        let pool_map = check::compute_pool_of_locus_type(&bundle, &top);
        sync_inference::infer_sync_for_bundle(&bundle, &top, &pool_map)
    };

    // Mutate: inject `sync = <picked>` for each candidate
    // that has a non-None inferred discipline. The candidate
    // set (no existing sync kwarg) was already filtered by
    // `infer_sync_for_bundle`; here we re-check defensively
    // and pick the rendering shape.
    for item in &mut program.items {
        if let TopDecl::Locus(l) = item {
            let Some(form) = &mut l.form else { continue };
            if form.name.name != "hashmap" {
                continue;
            }
            if form
                .args
                .iter()
                .any(|a| a.name.name == "sync")
            {
                continue;
            }
            let Some(inf) = inferred.get(&l.name.name) else {
                continue;
            };
            let label = match inf.discipline {
                sync_inference::SyncDiscipline::None => continue,
                sync_inference::SyncDiscipline::Serialized => "serialized",
                sync_inference::SyncDiscipline::Striped => "striped",
            };
            // Build a synthetic FormArg `sync = <label>`. Span
            // pinned to the form's own span so a downstream
            // error (e.g. a future "striped requires cap = N"
            // would point at the form decl rather than a
            // location-less zero-span). Ident has no canonical
            // synthetic span, so reuse the form's.
            let span = form.span;
            let arg_name =
                Ident { name: "sync".to_string(), span };
            let arg_value = Expr::Ident(Ident {
                name: label.to_string(),
                span,
            });
            form.args.push(FormArg {
                name: arg_name,
                value: arg_value,
                span,
            });
        }
    }

    Vec::new()
}

#[cfg(test)]
mod flat_shapeable_tests {
    //! Form K (2026-05-20): `is_flat_shapeable` predicate
    //! drives the route-selection matrix for the bus-decl
    //! constraint substrate. These tests pin the predicate's
    //! behavior on the cases the route matrix consults.

    use super::*;
    use hale_syntax::ast::PrimType;
    use hale_syntax::parse_source;

    use crate::resolve::{build_top_scope, TopScope};
    use crate::symbol::Bundle;
    use crate::ty::{is_flat_shapeable, Ty};

    fn with_scope(src: &str, f: impl FnOnce(&TopScope)) {
        let p = parse_source(src).expect("parses");
        let mut programs = BTreeMap::new();
        programs.insert(String::new(), &p);
        let bundle = Bundle { programs };
        let (scope, _) = build_top_scope(&bundle);
        f(&scope);
    }

    #[test]
    fn flat_for_pure_primitives() {
        with_scope("fn main() {}", |s| {
            for p in [
                PrimType::Int,
                PrimType::Uint,
                PrimType::Float,
                PrimType::Decimal,
                PrimType::Bool,
                PrimType::Time,
                PrimType::Duration,
            ] {
                assert!(
                    is_flat_shapeable(&Ty::Prim(p), s),
                    "primitive {:?} should be flat",
                    p
                );
            }
        });
    }

    #[test]
    fn not_flat_for_string_and_bytes() {
        with_scope("fn main() {}", |s| {
            for p in [
                PrimType::String,
                PrimType::Bytes,
                PrimType::BytesView,
                PrimType::StringView,
            ] {
                assert!(
                    !is_flat_shapeable(&Ty::Prim(p), s),
                    "variadic primitive {:?} should NOT be flat",
                    p
                );
            }
        });
    }

    #[test]
    fn not_flat_for_fixed_size_array() {
        // A fixed-size array is stored out-of-line (the field is a pointer,
        // not inline bytes), so it is NOT memcpy-flat — a raw copy across a
        // zero-copy / shm boundary would share a dangling pointer.
        with_scope("fn main() {}", |s| {
            let ty = Ty::Array(Box::new(Ty::Prim(PrimType::Int)), Some(8));
            assert!(!is_flat_shapeable(&ty, s));
        });
    }

    #[test]
    fn not_flat_for_unbounded_array() {
        with_scope("fn main() {}", |s| {
            let ty = Ty::Array(Box::new(Ty::Prim(PrimType::Int)), None);
            assert!(!is_flat_shapeable(&ty, s));
        });
    }

    #[test]
    fn not_flat_for_array_of_string() {
        with_scope("fn main() {}", |s| {
            let ty = Ty::Array(Box::new(Ty::Prim(PrimType::String)), Some(4));
            assert!(!is_flat_shapeable(&ty, s));
        });
    }

    #[test]
    fn flat_for_struct_of_primitives() {
        with_scope(
            "type Quote { bid: Decimal; ask: Decimal; venue: Int; } fn main() {}",
            |s| {
                let ty = Ty::Named("Quote".to_string());
                assert!(is_flat_shapeable(&ty, s));
            },
        );
    }

    #[test]
    fn not_flat_for_struct_with_string_field() {
        with_scope(
            "type Note { code: Int; text: String; } fn main() {}",
            |s| {
                let ty = Ty::Named("Note".to_string());
                assert!(!is_flat_shapeable(&ty, s));
            },
        );
    }

    #[test]
    fn flat_for_nested_struct_when_all_flat() {
        with_scope(
            "type Inner { v: Int; } type Outer { a: Inner; b: Decimal; } fn main() {}",
            |s| {
                let ty = Ty::Named("Outer".to_string());
                assert!(is_flat_shapeable(&ty, s));
            },
        );
    }

    #[test]
    fn not_flat_for_unknown_named() {
        with_scope("fn main() {}", |s| {
            let ty = Ty::Named("NoSuchType".to_string());
            // Conservative: predicate cannot assert flatness
            // for a type it cannot see.
            assert!(!is_flat_shapeable(&ty, s));
        });
    }

    #[test]
    fn not_flat_for_fallible() {
        with_scope("fn main() {}", |s| {
            let ty = Ty::Fallible {
                success: Box::new(Ty::Prim(PrimType::Int)),
                payload: Box::new(Ty::Prim(PrimType::Int)),
            };
            assert!(!is_flat_shapeable(&ty, s));
        });
    }

    #[test]
    fn flat_for_unit() {
        with_scope("fn main() {}", |s| {
            assert!(is_flat_shapeable(&Ty::Unit, s));
        });
    }
}

#[cfg(test)]
mod binding_constraint_tests {
    //! Form K4a (2026-05-20): typecheck-time validity matrix
    //! for the `where ...` clause on binding entries.

    use super::*;
    use hale_syntax::parse_source;

    fn check(src: &str) -> Vec<Diag> {
        let p = parse_source(src).expect("parses");
        check_program(&p)
    }

    #[test]
    fn unix_with_intra_machine_is_clean() {
        let src = r#"
            type Ping { n: Int; }
            topic Evt { payload: Ping; }
            locus Pub { bus { publish Evt; } }
            main locus App {
                accept(p: Pub) { }
                bindings {
                    Evt: unix("/tmp/evt.sock") where intra_machine;
                }
            }
        "#;
        let diags = check(src);
        assert!(
            !diags.iter().any(|d| d.message.contains("`where`")
                || d.message.contains("intra_machine")
                || d.message.contains("zero_copy")),
            "unix + intra_machine should be clean, got: {:?}",
            diags
        );
    }

    #[test]
    fn unix_with_zero_copy_rejected() {
        let src = r#"
            type Ping { n: Int; }
            topic Evt { payload: Ping; }
            locus Pub { bus { publish Evt; } }
            main locus App {
                accept(p: Pub) { }
                bindings {
                    Evt: unix("/tmp/evt.sock") where zero_copy;
                }
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| d.message.contains("`unix` transport memcpys")
                && d.message.contains("zero_copy")),
            "expected unix + zero_copy rejection, got: {:?}",
            diags
        );
    }

    #[test]
    fn unix_with_cross_machine_rejected() {
        let src = r#"
            type Ping { n: Int; }
            topic Evt { payload: Ping; }
            locus Pub { bus { publish Evt; } }
            main locus App {
                accept(p: Pub) { }
                bindings {
                    Evt: unix("/tmp/evt.sock") where cross_machine;
                }
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| d.message.contains("host-local")
                && d.message.contains("cross_machine")),
            "expected unix + cross_machine rejection, got: {:?}",
            diags
        );
    }

    #[test]
    fn unix_with_intra_process_rejected() {
        let src = r#"
            type Ping { n: Int; }
            topic Evt { payload: Ping; }
            locus Pub { bus { publish Evt; } }
            main locus App {
                accept(p: Pub) { }
                bindings {
                    Evt: unix("/tmp/evt.sock") where intra_process;
                }
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| d.message.contains("crosses OS process")
                && d.message.contains("intra_process")),
            "expected unix + intra_process rejection, got: {:?}",
            diags
        );
    }

    #[test]
    fn zero_copy_plus_cross_machine_rejected() {
        // Internal contradiction, fires before transport-
        // specific checks.
        let src = r#"
            type Ping { n: Int; }
            topic Evt { payload: Ping; }
            locus Pub { bus { publish Evt; } }
            main locus App {
                accept(p: Pub) { }
                bindings {
                    Evt: unix("/tmp/evt.sock") where zero_copy, cross_machine;
                }
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| d.message.contains("contradict")
                && d.message.contains("zero_copy")
                && d.message.contains("cross_machine")),
            "expected zero_copy + cross_machine contradiction diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn multiple_scope_constraints_rejected() {
        let src = r#"
            type Ping { n: Int; }
            topic Evt { payload: Ping; }
            locus Pub { bus { publish Evt; } }
            main locus App {
                accept(p: Pub) { }
                bindings {
                    Evt: unix("/tmp/evt.sock") where intra_machine, intra_process;
                }
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| d.message.contains("multiple scope constraints")),
            "expected multiple-scope diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn zero_copy_with_non_flat_payload_rejected() {
        // Payload contains a String field — variadic, not flat.
        // Even without considering the transport, the constraint
        // is unsatisfiable.
        let src = r#"
            type Note { code: Int; text: String; }
            topic Evt { payload: Note; }
            locus Pub { bus { publish Evt; } }
            main locus App {
                accept(p: Pub) { }
                bindings {
                    Evt: unix("/tmp/evt.sock") where zero_copy;
                }
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| d.message.contains("not flat-shapeable")
                && d.message.contains("Note")),
            "expected non-flat-payload diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn adapter_with_zero_copy_rejected() {
        let src = r#"
            type Ping { n: Int; }
            topic Evt { payload: Ping; }
            locus Pub { bus { publish Evt; } }
            locus MyAdapter {
                params { }
                fn send(subject: String, bytes: Bytes) { }
            }
            main locus App {
                accept(p: Pub) { }
                bindings {
                    Evt: MyAdapter { } where zero_copy;
                }
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| d.message.contains("Adapter")
                && d.message.contains("zero_copy")
                && d.message.contains("serialization")),
            "expected adapter + zero_copy rejection, got: {:?}",
            diags
        );
    }

    #[test]
    fn adapter_with_scope_constraint_is_trusted() {
        // Adapter's actual scope can't be known from the type
        // alone; trust the user's assertion.
        let src = r#"
            type Ping { n: Int; }
            topic Evt { payload: Ping; }
            locus Pub { bus { publish Evt; } }
            locus MyAdapter {
                params { }
                fn send(subject: String, bytes: Bytes) { }
            }
            main locus App {
                accept(p: Pub) { }
                bindings {
                    Evt: MyAdapter { } where cross_machine;
                }
            }
        "#;
        let diags = check(src);
        assert!(
            !diags.iter().any(|d| d.message.contains("cross_machine")),
            "adapter + cross_machine should be trusted, got: {:?}",
            diags
        );
    }

    #[test]
    fn shm_ring_with_zero_copy_is_clean() {
        // Form K4b: shm_ring is the substrate that satisfies
        // zero_copy on a flat payload — should typecheck clean.
        let src = r#"
            type Ping { n: Int; v: Int; }
            topic Evt { payload: Ping; }
            locus Pub { bus { publish Evt; } }
            main locus App {
                accept(p: Pub) { }
                bindings {
                    Evt: shm_ring("/hale_evt", on_overflow: drop) where zero_copy, intra_machine;
                }
            }
        "#;
        let diags = check(src);
        assert!(
            !diags.iter().any(|d| d.message.contains("`shm_ring`")
                || d.message.contains("zero_copy")
                || d.message.contains("intra_machine")),
            "shm_ring + zero_copy + intra_machine should be clean, got: {:?}",
            diags
        );
    }

    #[test]
    fn shm_ring_with_cross_machine_rejected() {
        let src = r#"
            type Ping { n: Int; v: Int; }
            topic Evt { payload: Ping; }
            locus Pub { bus { publish Evt; } }
            main locus App {
                accept(p: Pub) { }
                bindings {
                    Evt: shm_ring("/hale_evt", on_overflow: drop) where cross_machine;
                }
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| d.message.contains("host-local")
                && d.message.contains("cross_machine")),
            "expected shm_ring + cross_machine rejection, got: {:?}",
            diags
        );
    }

    #[test]
    fn shm_ring_with_intra_process_rejected() {
        let src = r#"
            type Ping { n: Int; v: Int; }
            topic Evt { payload: Ping; }
            locus Pub { bus { publish Evt; } }
            main locus App {
                accept(p: Pub) { }
                bindings {
                    Evt: shm_ring("/hale_evt", on_overflow: drop) where intra_process;
                }
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| d.message.contains("cross-process")
                && d.message.contains("intra_process")),
            "expected shm_ring + intra_process rejection, got: {:?}",
            diags
        );
    }

    #[test]
    fn shm_ring_with_non_flat_payload_rejected() {
        // Even with the right transport, a non-flat payload
        // can't ride zero_copy.
        let src = r#"
            type Note { code: Int; text: String; }
            topic Evt { payload: Note; }
            locus Pub { bus { publish Evt; } }
            main locus App {
                accept(p: Pub) { }
                bindings {
                    Evt: shm_ring("/hale_evt", on_overflow: drop) where zero_copy;
                }
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| d.message.contains("not flat-shapeable")),
            "expected non-flat-payload diag on shm_ring binding, got: {:?}",
            diags
        );
    }

    #[test]
    fn shm_ring_with_hale_subscriber_is_clean_post_k6b() {
        // Form K6b: same-bundle Hale subscribers on
        // shm_ring-bound topics are typecheck-clean. The
        // codegen wires a reader thread that dispatches to
        // the handler.
        let src = r#"
            type TickPayload { px: Int; sz: Int; }
            topic Tick { payload: TickPayload; }
            locus Pub { bus { publish Tick; } }
            locus Sub {
                bus { subscribe Tick as on_tick of type TickPayload; }
                fn on_tick(t: TickPayload) { }
            }
            main locus App {
                bindings {
                    Tick: shm_ring("/x", on_overflow: drop) where zero_copy;
                }
            }
        "#;
        let diags = check(src);
        assert!(
            !diags.iter().any(|d| d.message.contains("shm_ring")
                && d.message.contains("Hale-side subscribers")),
            "K6a rejection should be gone after K6b, got: {:?}",
            diags
        );
    }

    #[test]
    fn shm_ring_publish_only_is_clean() {
        // Publish-only on a shm_ring binding (no subscriber
        // declared in the bundle) is the supported v1 shape.
        let src = r#"
            type TickPayload { px: Int; sz: Int; }
            topic Tick { payload: TickPayload; }
            locus Pub { bus { publish Tick; } }
            main locus App {
                bindings {
                    Tick: shm_ring("/x", on_overflow: drop) where zero_copy;
                }
            }
        "#;
        let diags = check(src);
        assert!(
            !diags.iter().any(|d| d.message.contains("shm_ring")),
            "publish-only shm_ring should be clean, got: {:?}",
            diags
        );
    }

    #[test]
    fn binding_without_where_clause_unaffected() {
        // Regression guard: bindings without a `where` clause
        // continue to typecheck cleanly.
        let src = r#"
            type Ping { n: Int; }
            topic Evt { payload: Ping; }
            locus Pub { bus { publish Evt; } }
            main locus App {
                accept(p: Pub) { }
                bindings {
                    Evt: unix("/tmp/evt.sock");
                }
            }
        "#;
        let diags = check(src);
        assert!(
            !diags.iter().any(|d| d.message.contains("where")
                || d.message.contains("constraint")),
            "no-constraint binding should be unaffected, got: {:?}",
            diags
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hale_syntax::parse_source;

    fn check(src: &str) -> Vec<Diag> {
        let p = parse_source(src).expect("parses");
        check_program(&p)
    }

    #[test]
    fn ok_simple_locus() {
        let src = r#"
            locus L {
                params { x: Int = 5; }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(diags.is_empty(), "expected no diags, got: {:?}", diags);
    }

    #[test]
    fn err_struct_field_type_mismatch() {
        let src = r#"
            type Point { x: Int; y: Int; }
            fn main() {
                let p = Point { x: "hi", y: 2 };
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| d.message.contains("field `x`")),
            "expected field-type error, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_send_subject_not_declared() {
        let src = r#"
            type Msg { text: String; }
            locus L {
                bus { publish "ok" of type Msg; }
                run() { "wrong" <- Msg { text: "x" }; }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("not declared in locus")),
            "expected undeclared-subject error, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_send_payload_type_mismatch() {
        let src = r#"
            type Msg { text: String; }
            type Other { v: Int; }
            locus L {
                bus { publish "s" of type Msg; }
                run() { "s" <- Other { v: 1 }; }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| d.message.contains("not assignable")),
            "expected payload-type error, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_contract_consume_missing_on_child() {
        let src = r#"
            locus ChildL {
                params { v: Int = 0; }
                contract { expose v: Int; }
            }
            locus ParentL {
                contract { consume value: Int; }
                accept(c: ChildL) { }
            }
            fn main() { ParentL { }; }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("does not expose")),
            "expected contract-missing error; got: {:?}",
            diags
        );
    }

    #[test]
    fn err_contract_type_mismatch() {
        let src = r#"
            locus ChildL {
                params { value: String = "hi"; }
                contract { expose value: String; }
            }
            locus ParentL {
                contract { consume value: Int; }
                accept(c: ChildL) { }
            }
            fn main() { ParentL { }; }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| d.message.contains("exposes it as")),
            "expected type-mismatch error; got: {:?}",
            diags
        );
    }

    #[test]
    fn err_consume_without_accept() {
        let src = r#"
            locus ParentL {
                contract { consume thing: Int; }
            }
            fn main() { ParentL { }; }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("declares no `accept")),
            "expected accept-missing error; got: {:?}",
            diags
        );
    }

    #[test]
    fn err_typo_in_self_field() {
        let src = r#"
            locus L {
                params { x: Int = 5; }
                closure typo_check {
                    self.greting ~~ self.x within 0;
                }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("no field `greting`")),
            "expected typo detection; got: {:?}",
            diags
        );
    }

    #[test]
    fn err_closure_pure_literal_assertion() {
        let src = r#"
            locus L {
                params { x: Int = 5; }
                closure dud {
                    5 ~~ 5 within 0;
                }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("pure literals")),
            "expected pure-literal closure error; got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_closure_one_side_literal() {
        // One literal side is fine — `self.x ~~ 0 within 5`
        // is a meaningful "x stays near zero" invariant.
        let src = r#"
            locus L {
                params { count: Int = 0; }
                closure stays_low {
                    self.count ~~ 0 within 100;
                }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(diags.is_empty(), "expected no diags; got: {:?}", diags);
    }

    #[test]
    fn ok_approx_within_as_idents_outside_closure() {
        // F.10-style contextual narrowing (2026-05-11): `approx`
        // and `within` are not reserved at the lexer level, so
        // they can appear as free-fn / let-binding identifiers
        // outside closure bodies. Resolves
        // notes/hale-friction.md 2026-05-10
        // closure-keyword-shadows-helper-ident.
        let src = r#"
            fn approx(actual: Float, expected: Float, eps: Float) -> Bool {
                let diff = actual - expected;
                let within = -eps;
                return diff > within;
            }
            fn main() {
                let ok = approx(3.14, 3.14159, 0.01);
                println("ok=", ok);
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected `approx` / `within` to parse as idents; got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_approx_keyword_inside_closure_still_works() {
        // The contextual narrowing must still admit the
        // long-form `approx` spelling inside closure assertions
        // (alongside the `~~` operator). `approx` is the infix
        // operator-keyword: `LEFT approx RIGHT within TOL`.
        let src = r#"
            locus L {
                params { x: Int = 0; }
                closure stays_low {
                    self.x approx 0 within 100;
                }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected long-form `approx` inside closure to parse; got: {:?}",
            diags
        );
    }

    #[test]
    fn err_match_not_exhaustive() {
        let src = r#"
            fn main() {
                let x = 7;
                match x {
                    1 -> println("one"),
                    2 -> println("two"),
                }
            }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("not exhaustive")),
            "expected exhaustiveness error; got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_generic_enum_match_with_monomorph_arms_no_wildcard() {
        // m68: matching a generic-enum-typed scrutinee with
        // arms that use the synthesized monomorph name
        // (Result_Int_String::Ok / ::Err) should be exhaustive
        // without a wildcard. The typechecker only sees the
        // template `Result` (with variants Ok, Err); the user's
        // arms use the mangled names codegen recognizes. The
        // exhaustiveness check accepts the mangle prefix as
        // covering the template's variants.
        let src = r#"
            type Result<T, E> = enum {
                Ok(T),
                Err(E),
            };

            fn main() {
                let r: Result<Int, String> = Result_Int_String::Ok(7);
                match r {
                    Result_Int_String::Ok(n)  -> println("ok: ", n),
                    Result_Int_String::Err(s) -> println("err: ", s),
                }
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected no diags; got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_match_with_wildcard() {
        let src = r#"
            fn main() {
                let x = 7;
                match x {
                    1 -> println("one"),
                    _ -> println("other"),
                }
            }
        "#;
        let diags = check(src);
        assert!(diags.is_empty(), "expected no diags; got: {:?}", diags);
    }

    #[test]
    fn ok_bool_match_covers_both_cases() {
        let src = r#"
            fn main() {
                let x = true;
                match x {
                    true -> println("yes"),
                    false -> println("no"),
                }
            }
        "#;
        let diags = check(src);
        assert!(diags.is_empty(), "expected no diags; got: {:?}", diags);
    }

    #[test]
    fn err_bool_match_only_true() {
        let src = r#"
            fn main() {
                let x = true;
                match x {
                    true -> println("yes"),
                }
            }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("not exhaustive")),
            "expected exhaustiveness error; got: {:?}",
            diags
        );
    }

    #[test]
    fn err_typo_on_struct_value() {
        let src = r#"
            type Point { x: Int; y: Int; }
            fn main() {
                let p = Point { x: 1, y: 2 };
                let _q = p.zee;
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| d.message.contains("no field `zee`")),
            "expected typo detection; got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_contract_compatible() {
        let src = r#"
            locus ChildL {
                params { value: Int = 0; }
                contract { expose value: Int; }
            }
            locus ParentL {
                contract { consume value: Int; }
                accept(c: ChildL) { }
            }
            fn main() { ParentL { }; }
        "#;
        let diags = check(src);
        assert!(diags.is_empty(), "expected clean check; got: {:?}", diags);
    }

    #[test]
    fn err_let_type_mismatch() {
        let src = r#"
            fn main() {
                let x: Int = "hello";
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| d.message.contains("let `x`")),
            "expected let-type error, got: {:?}",
            diags
        );
    }

    // m50: immutable-binding enforcement.
    #[test]
    fn err_assign_to_immutable_let() {
        let src = r#"
            fn main() {
                let x: Int = 0;
                x = 1;
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| {
                d.message.contains("cannot assign to `x`")
                    && d.message.contains("immutable")
            }),
            "expected immutable-binding error on `x = 1;`, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_assign_to_let_mut() {
        let src = r#"
            fn main() {
                let mut n: Int = 0;
                n = 1;
                n = n + 2;
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean check on let mut + reassignment; got: {:?}",
            diags
        );
    }

    #[test]
    fn err_assign_to_fn_param() {
        let src = r#"
            fn bump(n: Int) {
                n = n + 1;
            }
            fn main() { bump(0); }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| {
                d.message.contains("cannot assign to `n`")
                    && d.message.contains("immutable")
            }),
            "expected immutable-binding error on fn-param reassignment, \
             got: {:?}",
            diags
        );
    }

    #[test]
    fn err_assign_to_for_loop_var() {
        let src = r#"
            fn main() {
                for i in 0..3 {
                    i = 99;
                }
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| {
                d.message.contains("cannot assign to `i`")
                    && d.message.contains("immutable")
            }),
            "expected immutable-binding error on for-loop-var \
             reassignment, got: {:?}",
            diags
        );
    }

    // Field/index reassignment THROUGH an immutable head still
    // allowed — `x.field = ...` mutates state, doesn't rebind x.
    #[test]
    fn ok_field_assign_through_immutable_self() {
        let src = r#"
            locus L {
                params { count: Int = 0; }
                run() {
                    self.count = 7;
                }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean check on `self.field = ...` in lifecycle; \
             got: {:?}",
            diags
        );
    }

    // F.20 structural interfaces — typechecker recognizes the
    // declaration and enforces the structural-impl rule at every
    // call site where a fn declares an interface-typed param.

    #[test]
    fn ok_locus_satisfies_interface() {
        let src = r#"
            interface Sink {
                fn write(s: String);
                fn line(s: String);
            }
            locus StdoutSinkL {
                params { }
                fn write(s: String) { print(s); }
                fn line(s: String) { println(s); }
            }
            fn render(sink: Sink) { }
            fn main() {
                let s = StdoutSinkL { };
                render(s);
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean check on satisfying locus; got: {:?}",
            diags
        );
    }

    #[test]
    fn err_locus_missing_interface_method() {
        let src = r#"
            interface Sink {
                fn write(s: String);
                fn line(s: String);
            }
            locus BrokenL {
                params { }
                fn write(s: String) { print(s); }
            }
            fn render(sink: Sink) { }
            fn main() {
                let s = BrokenL { };
                render(s);
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| {
                d.message.contains("does not satisfy interface")
                    && d.message.contains("missing method `line`")
            }),
            "expected missing-method diagnostic, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_string_plus_int_auto_coerces() {
        let src = r#"
            fn main() {
                let port = 8080;
                let msg = "port=" + port;
                println(msg);
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean check on String + Int auto-coerce; got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_int_plus_string_auto_coerces() {
        let src = r#"
            fn main() {
                let n = 42;
                let msg = n + " items";
                println(msg);
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean check on Int + String (symmetric); got: {:?}",
            diags
        );
    }

    #[test]
    fn err_locus_interface_arity_mismatch() {
        let src = r#"
            interface Greet {
                fn hello(name: String);
            }
            locus BadArityL {
                params { }
                fn hello(name: String, extra: Int) { }
            }
            fn welcome(g: Greet) { }
            fn main() {
                let g = BadArityL { };
                welcome(g);
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| {
                d.message.contains("arity does not match interface")
            }),
            "expected arity-mismatch diagnostic, got: {:?}",
            diags
        );
    }

    // === v1.x-FORM-1 PR2 fallible typecheck =============

    #[test]
    fn err_fallible_call_not_addressed_in_let() {
        let src = r#"
            type E { msg: String; }
            fn parse(s: String) -> Int fallible(E) { return 0; }
            fn main() {
                let v = parse("42");
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| d.message.contains("error not addressed")),
            "expected error-not-addressed diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_fallible_call_not_addressed_in_expr_stmt() {
        let src = r#"
            type E { }
            fn doit() -> Int fallible(E) { return 0; }
            fn main() {
                doit();
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| d.message.contains("error not addressed")),
            "expected error-not-addressed diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_fallible_addressed_via_or_raise() {
        let src = r#"
            type E { }
            fn parse(s: String) -> Int fallible(E) { return 0; }
            fn main() {
                let v = parse("42") or raise;
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean check on `or raise`, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_fallible_addressed_via_or_substitute() {
        let src = r#"
            type E { }
            fn parse(s: String) -> Int fallible(E) { return 0; }
            fn main() {
                let v = parse("42") or 99;
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean check on `or 99`, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_fallible_substitute_type_mismatch() {
        let src = r#"
            type E { }
            fn parse(s: String) -> Int fallible(E) { return 0; }
            fn main() {
                let v = parse("42") or "not an int";
            }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("does not match success type")),
            "expected substitute-type-mismatch diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_or_substitute_coerces_locus_to_interface() {
        // 2026-05-18 — substitute RHS may be a concrete locus
        // when the fallible's success type is an interface the
        // locus structurally satisfies. Mirrors the same
        // coercion the call-site and struct-literal init use.
        let src = r#"
            interface Greeter { fn greet() -> String; }
            locus Hello {
                fn greet() -> String { return "hi"; }
            }
            fn maybe_greeter() -> Greeter fallible(Int) { fail 1; }
            fn main() {
                let fallback = Hello { };
                let g = maybe_greeter() or fallback;
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean check on locus→interface `or <substitute>`, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_or_substitute_locus_missing_interface_method() {
        // Negative case: substitute locus that doesn't
        // structurally satisfy the interface still reports the
        // missing-method diagnostic.
        let src = r#"
            interface Greeter {
                fn greet() -> String;
                fn shout() -> String;
            }
            locus PartialHello {
                fn greet() -> String { return "hi"; }
            }
            fn maybe_greeter() -> Greeter fallible(Int) { fail 1; }
            fn main() {
                let fallback = PartialHello { };
                let g = maybe_greeter() or fallback;
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| {
                d.message.contains("does not satisfy interface")
                    && d.message.contains("missing method `shout`")
            }),
            "expected missing-method diag on substitute locus, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_err_binding_in_or_substitute_rhs() {
        let src = r#"
            type E { code: Int; }
            fn parse(s: String) -> Int fallible(E) { return 0; }
            fn handle(e: E) -> Int { return e.code; }
            fn main() {
                let v = parse("42") or handle(err);
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean check on `or handle(err)`, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_or_on_non_fallible_expression() {
        let src = r#"
            fn main() {
                let v = 1 + 1 or raise;
            }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("expects a fallible-typed")),
            "expected non-fallible-or diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_fail_with_matching_payload_type() {
        let src = r#"
            type E { code: Int; }
            fn parse(s: String) -> Int fallible(E) {
                fail E { code: 1 };
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean check on matching-payload fail, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_fail_payload_type_mismatch() {
        let src = r#"
            type E { code: Int; }
            type Other { msg: String; }
            fn parse(s: String) -> Int fallible(E) {
                fail Other { msg: "wrong type" };
            }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("fail: expected payload")),
            "expected fail-payload-type-mismatch diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_right_associative_chain_typechecks() {
        let src = r#"
            type E { }
            fn a() -> Int fallible(E) { return 0; }
            fn b() -> Int fallible(E) { return 0; }
            fn main() {
                let v = a() or b() or raise;
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean check on chain, got: {:?}",
            diags
        );
    }

    // === v1.x-FORM-1 PR3 form-shape verification ========

    #[test]
    fn ok_form_vec_with_correct_shape() {
        let src = r#"
            @form(vec)
            locus ItemListL {
                capacity { heap items of Int; }
            }
            fn main() { ItemListL { }; }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean check on @form(vec) with heap slot, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_form_vec_with_pool_slot() {
        let src = r#"
            @form(vec)
            locus ItemListL {
                capacity { pool items of Int; }
            }
            fn main() { ItemListL { }; }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("@form(vec) requires a `heap` slot")),
            "expected pool-rejected diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_form_vec_with_no_capacity() {
        let src = r#"
            @form(vec)
            locus ItemListL {
                params { x: Int = 0; }
            }
            fn main() { ItemListL { }; }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("found no `capacity")),
            "expected missing-capacity diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_form_vec_with_multiple_slots() {
        let src = r#"
            @form(vec)
            locus L {
                capacity {
                    heap a of Int;
                    heap b of Int;
                }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("exactly one `heap`")),
            "expected multiple-slots diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_form_vec_with_args() {
        let src = r#"
            @form(vec, cap = 64)
            locus L {
                capacity { heap items of Int; }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("@form(vec) takes no arguments")),
            "expected vec-no-args diag, got: {:?}",
            diags
        );
    }

    // === v1.x-FORM-4 PR2 tests ===========================
    //
    // @form(hashmap) shape contract: exactly one `pool` slot
    // with `indexed_by <fieldname>` on a struct cell type whose
    // field exists.

    #[test]
    fn ok_form_hashmap_with_correct_shape() {
        let src = r#"
            type Entry { name: String; v: Int; }
            @form(hashmap)
            locus Registry {
                capacity { pool entries of Entry indexed_by name; }
            }
            fn main() { Registry { }; }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean check on @form(hashmap) with pool + indexed_by, \
             got: {:?}",
            diags
        );
    }

    #[test]
    fn err_form_hashmap_with_heap_slot() {
        let src = r#"
            type Entry { name: String; v: Int; }
            @form(hashmap)
            locus Registry {
                capacity { heap entries of Entry indexed_by name; }
            }
            fn main() { Registry { }; }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("@form(hashmap) requires a `pool` slot")),
            "expected heap-rejected diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_form_hashmap_missing_indexed_by() {
        let src = r#"
            type Entry { name: String; v: Int; }
            @form(hashmap)
            locus Registry {
                capacity { pool entries of Entry; }
            }
            fn main() { Registry { }; }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("must declare `indexed_by")),
            "expected missing-indexed_by diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_form_hashmap_field_does_not_exist() {
        let src = r#"
            type Entry { name: String; v: Int; }
            @form(hashmap)
            locus Registry {
                capacity { pool entries of Entry indexed_by nope; }
            }
            fn main() { Registry { }; }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("has no field `nope`")),
            "expected field-not-found diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_form_hashmap_cell_is_primitive() {
        let src = r#"
            @form(hashmap)
            locus Registry {
                capacity { pool entries of Int indexed_by name; }
            }
            fn main() { Registry { }; }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("must be a user-declared struct")),
            "expected primitive-cell-rejected diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_form_hashmap_with_multiple_slots() {
        let src = r#"
            type Entry { name: String; v: Int; }
            @form(hashmap)
            locus L {
                capacity {
                    pool entries of Entry indexed_by name;
                    heap log of Int;
                }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("exactly one capacity slot")),
            "expected multiple-slots diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_form_hashmap_with_unknown_arg() {
        // F.32-1α (2026-05-24): @form(hashmap) accepts `sync = X`
        // (X ∈ {serialized, striped, lockfree}) and γ-v1 added
        // `cap = N` (valid with sync = lockfree). Other kwargs
        // remain rejected with a "unknown arg" diagnostic naming
        // the valid surface.
        let src = r#"
            type Entry { name: String; v: Int; }
            @form(hashmap, bogus = 64)
            locus L {
                capacity { pool entries of Entry indexed_by name; }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("unknown arg `bogus`")
                      && d.message.contains("sync = X")),
            "expected hashmap unknown-arg diag naming sync = X, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_form_hashmap_with_sync_serialized() {
        // F.32-1α (2026-05-24): the canonical opt-in path for
        // cross-pool @form(hashmap). Standalone test that the
        // typecheck accepts the kwarg.
        let src = r#"
            type Entry { name: String; v: Int; }
            @form(hashmap, sync = serialized)
            locus L {
                capacity { pool entries of Entry indexed_by name; }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean typecheck on sync = serialized; got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_form_hashmap_with_sync_striped() {
        // F.32-1β2 (2026-05-25): striped accepted — cell-level
        // CAS + rwlock-on-grow + cache-padded cells. Was
        // rejected as "not yet shipped" pre-β2.
        let src = r#"
            type Entry { name: String; v: Int; }
            @form(hashmap, sync = striped)
            locus L {
                capacity { pool entries of Entry indexed_by name; }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean typecheck on sync = striped; got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_form_hashmap_with_sync_lockfree_and_cap() {
        // F.32-1γ-v1: lockfree requires cap = N.
        let src = r#"
            type Entry { name: String; v: Int; }
            @form(hashmap, sync = lockfree, cap = 1024)
            locus L {
                capacity { pool entries of Entry indexed_by name; }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean typecheck on sync = lockfree with cap; got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_form_hashmap_lockfree_without_cap() {
        // F.32-1γ-v2 (2026-05-26): lockfree no longer requires
        // `cap = N`. Once grow shipped, cap became an
        // initial-size hint; omitting it starts at
        // LOTUS_HASHMAP_INITIAL_CAP and grows on demand.
        let src = r#"
            type Entry { name: String; v: Int; }
            @form(hashmap, sync = lockfree)
            locus L {
                capacity { pool entries of Entry indexed_by name; }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        let lockfree_diags: Vec<&_> = diags
            .iter()
            .filter(|d| d.message.contains("lockfree"))
            .collect();
        assert!(
            lockfree_diags.is_empty(),
            "expected clean typecheck on cap-less lockfree; got: {:?}",
            lockfree_diags
        );
    }

    #[test]
    fn err_form_hashmap_cap_without_lockfree() {
        // cap = N is only valid with sync = lockfree (other
        // sync modes grow dynamically).
        let src = r#"
            type Entry { name: String; v: Int; }
            @form(hashmap, sync = serialized, cap = 1024)
            locus L {
                capacity { pool entries of Entry indexed_by name; }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("`cap = N` is only valid with")
                      && d.message.contains("sync = lockfree")),
            "expected cap-only-with-lockfree diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_form_hashmap_with_unknown_sync_value() {
        let src = r#"
            type Entry { name: String; v: Int; }
            @form(hashmap, sync = potato)
            locus L {
                capacity { pool entries of Entry indexed_by name; }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("unknown sync discipline")),
            "expected unknown-sync-value diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_form_ring_buffer_missing_cap() {
        // v1.x-FORM-5: ring_buffer shipped but requires `cap = N`.
        // Bare `@form(ring_buffer)` without the cap arg is a hard
        // error — the backing buffer is fixed-capacity and the
        // substrate needs the size at locus-birth time.
        let src = r#"
            @form(ring_buffer)
            locus L {
                capacity { pool history of Int; }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("ring_buffer") && d.message.contains("cap")),
            "expected ring_buffer-needs-cap diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_form_ring_buffer_with_cap() {
        // v1.x-FORM-5: ring_buffer with `cap = N` and one pool
        // slot is the canonical shape — no diags expected.
        let src = r#"
            @form(ring_buffer, cap = 8)
            locus L {
                capacity { pool history of Int; }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(diags.is_empty(), "expected no diags, got: {:?}", diags);
    }

    #[test]
    fn err_form_ring_buffer_heap_slot() {
        // ring_buffer recycles fixed-capacity cells (pool); a heap
        // slot belongs to @form(vec) instead.
        let src = r#"
            @form(ring_buffer, cap = 4)
            locus L {
                capacity { heap history of Int; }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("requires a `pool`")),
            "expected pool-required diag, got: {:?}",
            diags
        );
    }

    // === v1.x-FORM-4 PR3 tests: method synthesis ============

    #[test]
    fn ok_form_hashmap_set_and_has_resolve() {
        // `set(value: S) -> ()` and `has(key: K) -> Bool` are
        // synthesized and resolve at call sites. K is String
        // (Entry.name's type); S is Entry.
        let src = r#"
            type Entry { name: String; v: Int; }
            @form(hashmap)
            locus Registry {
                capacity { pool entries of Entry indexed_by name; }
            }
            fn main() {
                let r = Registry { };
                r.set(Entry { name: "k", v: 1 });
                let h = r.has("k");
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean check on set + has, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_form_hashmap_get_fallible_addressed() {
        let src = r#"
            type Entry { name: String; v: Int; }
            @form(hashmap)
            locus Registry {
                capacity { pool entries of Entry indexed_by name; }
            }
            fn main() {
                let r = Registry { };
                let v = r.get("missing") or raise;
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean check on get + or raise, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_form_hashmap_get_not_addressed() {
        // `get` returns `S fallible(KeyError)`; calling it as
        // an expression statement without addressing the
        // error channel must error.
        let src = r#"
            type Entry { name: String; v: Int; }
            @form(hashmap)
            locus Registry {
                capacity { pool entries of Entry indexed_by name; }
            }
            fn main() {
                let r = Registry { };
                let v = r.get("missing");
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| d.message.contains("error not addressed")
                || d.message.contains("fallible")),
            "expected error-not-addressed diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_form_hashmap_remove_substitute_with_err_binding() {
        // `remove` is fallible(KeyError) with Unit success; the
        // substitute RHS (`or <expr>`) sees `err: KeyError` in
        // scope. No explicit `substitute` keyword — `or <expr>`
        // IS the substitute form; `or raise` is the diverge form.
        let src = r#"
            type Entry { name: String; v: Int; }
            @form(hashmap)
            locus Registry {
                capacity { pool entries of Entry indexed_by name; }
            }
            fn report_err(kind: String) { }
            fn main() {
                let r = Registry { };
                r.remove("k") or report_err(err.kind);
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean check on remove + or <fallback>, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_form_hashmap_len_and_is_empty_synthesized() {
        let src = r#"
            type Entry { name: String; v: Int; }
            @form(hashmap)
            locus Registry {
                capacity { pool entries of Entry indexed_by name; }
            }
            fn main() {
                let r = Registry { };
                let n: Int = r.len();
                let e: Bool = r.is_empty();
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean check on len + is_empty, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_form_hashmap_key_error_in_scope() {
        // `KeyError` is injected into the bundle scope whenever
        // any form-locus exists; it's usable as a type in user
        // code (e.g., to declare a fallible-handler param).
        let src = r#"
            type Entry { name: String; v: Int; }
            @form(hashmap)
            locus Registry {
                capacity { pool entries of Entry indexed_by name; }
            }
            fn describe(e: KeyError) -> String { return e.kind; }
            fn main() { Registry { }; }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean check using KeyError, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_form_hashmap_int_key() {
        // K = Int when the indexed-by field's type is Int.
        let src = r#"
            type Entry { id: Int; payload: String; }
            @form(hashmap)
            locus ById {
                capacity { pool entries of Entry indexed_by id; }
            }
            fn main() {
                let r = ById { };
                r.set(Entry { id: 7, payload: "p" });
                let v = r.get(7) or raise;
                let h = r.has(42);
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean check on Int-keyed hashmap, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_form_unknown_name() {
        let src = r#"
            @form(banana)
            locus L { }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| d.message.contains("unknown form")),
            "expected unknown-form diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_locus_without_form_unaffected() {
        // Regression guard: locus declarations without @form
        // are completely unaffected by the form-shape checks.
        let src = r#"
            locus L {
                capacity { pool entries of Int; }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "non-form locus regressed, got: {:?}",
            diags
        );
    }

    // === v1.x-FORM-1 PR3b form-method-synthesis ===========

    #[test]
    fn ok_form_vec_push_resolves() {
        let src = r#"
            @form(vec)
            locus ItemListL {
                capacity { heap items of Int; }
            }
            fn main() {
                let l = ItemListL { };
                l.push(42);
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "synthesized push should resolve, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_form_vec_get_fallible_addressed() {
        let src = r#"
            @form(vec)
            locus ItemListL {
                capacity { heap items of Int; }
            }
            fn main() {
                let l = ItemListL { };
                let v = l.get(0) or raise;
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "get + or raise should typecheck, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_form_vec_get_not_addressed() {
        let src = r#"
            @form(vec)
            locus ItemListL {
                capacity { heap items of Int; }
            }
            fn main() {
                let l = ItemListL { };
                let v = l.get(0);
            }
        "#;
        let diags = check(src);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("error not addressed")),
            "expected error-not-addressed on bare get(), got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_form_vec_pop_substitute_with_typed_err_handler() {
        let src = r#"
            @form(vec)
            locus ItemListL {
                capacity { heap items of Int; }
            }
            fn fallback(e: IndexError) -> Int { return -1; }
            fn main() {
                let l = ItemListL { };
                let v = l.pop() or fallback(err);
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "pop + or handler(err) should typecheck (err typed as IndexError), \
             got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_form_vec_len_and_is_empty_synthesized() {
        let src = r#"
            @form(vec)
            locus ItemListL {
                capacity { heap items of Int; }
            }
            fn main() {
                let l = ItemListL { };
                let n = l.len();
                let e = l.is_empty();
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "synthesized len/is_empty should resolve, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_form_vec_with_struct_cell_type() {
        // Cell type can be a user-defined struct; synthesized
        // methods carry that T through.
        let src = r#"
            type Pair { x: Int; y: Int; }
            @form(vec)
            locus PairsL {
                capacity { heap items of Pair; }
            }
            fn main() {
                let l = PairsL { };
                l.push(Pair { x: 1, y: 2 });
                let p = l.get(0) or raise;
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "@form(vec) over a struct cell should typecheck, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_index_error_type_in_scope_when_form_used() {
        // The synthesized IndexError type is callable as an
        // ordinary type in user code when any form is used.
        let src = r#"
            @form(vec)
            locus L {
                capacity { heap items of Int; }
            }
            fn inspect(e: IndexError) -> Int { return e.index; }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "IndexError should be in scope when form is used, got: {:?}",
            diags
        );
    }

    // === Open-question #24 (2026-05-25) =================
    // The blanket "no fallible on locus methods" rule is
    // narrowed: user-declared `fn` members may carry
    // `fallible(E)`. Substrate-facing surfaces still can't —
    // lifecycle methods (physically by AST shape), mode
    // methods (same), and bus-subscribed handlers (by an
    // explicit rejection at the subscribe site).
    // Spec narrowing: open-questions.md #24.

    #[test]
    fn ok_locus_method_declared_fallible_post_24() {
        let src = r#"
            type E { code: Int; }
            locus L {
                fn check() -> Int fallible(E) {
                    return 1;
                }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected clean check on user-declared fallible locus method \
             post-#24; got: {:?}",
            diags
        );
    }

    #[test]
    fn err_bus_subscribed_handler_cant_be_fallible() {
        // Bus dispatch has no caller frame to address a value
        // error; subscribing a fallible fn is the kind of
        // construct #24 explicitly preserves a rejection for.
        let src = r#"
            type Tick { n: Int; }
            type E { code: Int; }
            locus L {
                bus { subscribe "tick" as on_tick of type Tick; }
                fn on_tick(t: Tick) -> Int fallible(E) {
                    return 1;
                }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| {
                d.message.contains("bus-subscribed")
                    && d.message.contains("can't be fallible")
            }),
            "expected bus-subscribed-fallible rejection, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_locus_method_calls_fallible_free_fn() {
        // The escape hatch: a locus method can call a fallible
        // free fn and address the error at the call site.
        let src = r#"
            type E { msg: String; }
            fn parse_int(s: String) -> Int fallible(E) { return 0; }
            locus L {
                fn handle() -> Int {
                    let v = parse_int("42") or 0;
                    return v;
                }
            }
            fn main() { L { }; }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "locus method calling fallible free fn with `or` should typecheck, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_form_vec_method_fallible_unaffected() {
        // Stdlib-synthesized `@form(vec)` methods (get / pop)
        // are application-layer storage substrate, not locus-
        // structural surface. They remain fallible.
        let src = r#"
            @form(vec)
            locus L { capacity { heap items of Int; } }
            fn main() {
                let l = L { };
                l.push(1);
                let v = l.get(0) or -1;
                let _ = v;
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "@form(vec) synthesized get should still be fallible, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_send_via_cross_seed_qualified_topic() {
        // Bug 1 regression. The parser admits `alias::Topic` as a
        // bus subject in `subscribe`, `publish`, and `<-` positions.
        // The codegen-side pre-pass resolves the qualified path
        // through the import path-rename table. The typechecker
        // doesn't see the merged + mangled program, so it must
        // accept `Expr::Path(alias::Topic) <- payload;` directly
        // using the leaf segment as the subject — mirroring how
        // resolve_bus_subject treats QualifiedTopic in subscribe /
        // publish declarations (leaf name + Ty::Unknown payload).
        //
        // Before the fix, the send-statement LHS fell through to
        // "computed subject" and errored with "wildcard publish
        // required".
        let src = r#"
            locus Pub {
                bus { publish src::Heartbeat; }
                run() {
                    src::Heartbeat <- src::Beat { n: 42 };
                }
            }
            fn main() { Pub { }; }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "expected cross-seed send to typecheck cleanly; got: {:?}",
            diags
        );
    }

    // === FUv0.8.2 #1 stdlib error-type collision tests ====

    #[test]
    fn err_user_parse_error_shadows_stdlib_with_wrong_shape() {
        // User declares `type ParseError` with fields the
        // stdlib's parse_int / parse_float don't expect. The
        // collision check fires at the user's decl span — we
        // need to USE a stdlib parse_* fn so `bundle_uses_form_machinery`
        // routes through inject_form_stdlib_types.
        let src = r#"
            type ParseError { code: Int; }
            fn main() {
                let v = std::str::parse_int("42") or raise;
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| {
                d.message.contains("shadows the stdlib's `ParseError`")
                    && d.message.contains("kind: String")
                    && d.message.contains("std::str::ParseError")
            }),
            "expected ParseError-shadow diagnostic, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_user_parse_error_matches_stdlib_shape() {
        // User declares a type with the same name but the right
        // shape; the collision check is silent (typecheck clean
        // on shape grounds; codegen uses the user's matching
        // version).
        let src = r#"
            type ParseError { kind: String; input: String; }
            fn main() {
                let v = std::str::parse_int("42") or raise;
            }
        "#;
        let diags = check(src);
        assert!(
            !diags.iter().any(|d| d.message.contains("shadows")),
            "expected no shadow diag when shape matches; got: {:?}",
            diags
        );
    }

    #[test]
    fn err_user_io_error_shadows_stdlib_with_wrong_shape() {
        // Same pattern for IoError, which the stdlib uses for
        // std::io::fs::* path-calls.
        let src = r#"
            type IoError { reason: String; }
            fn main() {
                let s = std::io::fs::read_file("/tmp/x") or raise;
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| {
                d.message.contains("shadows the stdlib's `IoError`")
                    && d.message.contains("std::io::IoError")
            }),
            "expected IoError-shadow diagnostic, got: {:?}",
            diags
        );
    }

    #[test]
    fn err_user_crypto_error_shadows_stdlib_with_wrong_shape() {
        // CryptoError is reached via `std::crypto::ecdsa_p256_sign`
        // in `or` context (fallible form). A user type of the same
        // name with the wrong shape must surface the shadow diag.
        let src = r#"
            type CryptoError { code: Int; }
            fn main() {
                let k = std::bytes::from_string("k");
                let m = std::bytes::from_string("m");
                let sig = std::crypto::ecdsa_p256_sign(k, m) or raise;
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| {
                d.message.contains("shadows the stdlib's `CryptoError`")
                    && d.message.contains("std::crypto::CryptoError")
            }),
            "expected CryptoError-shadow diagnostic, got: {:?}",
            diags
        );
    }

    #[test]
    fn ok_user_parse_error_in_form_free_program() {
        // Without any form machinery in use, inject_form_stdlib_types
        // doesn't run, so the collision check is skipped. The
        // user is free to declare any type name.
        let src = r#"
            type ParseError { code: Int; }
            fn main() {
                let _ = ParseError { code: 1 };
            }
        "#;
        let diags = check(src);
        assert!(
            diags.is_empty(),
            "user-declared ParseError in a form-free program \
             should typecheck clean; got: {:?}",
            diags
        );
    }

    // === FUv0.8.2 #4 auto-applied sync inference =========

    #[test]
    fn apply_sync_inference_injects_striped_on_two_writer_pools() {
        // Two pools (io + compute) each fire `self.reg.set` in
        // on_tick handlers — inference picks striped. The
        // pre-pass should inject `sync = striped` into the
        // @form(hashmap) annotation.
        let src = r#"
            type Entry { k: Int; v: Int; }
            type Tick { n: Int; }

            @form(hashmap)
            locus Registry {
                capacity { pool entries of Entry indexed_by k; }
            }

            locus IoWorker {
                params { reg: Registry = Registry { }; }
                bus { subscribe "tick" as on_tick of type Tick; }
                fn on_tick(t: Tick) {
                    self.reg.set(Entry { k: t.n, v: 1 });
                }
            }

            locus CompWorker {
                params { reg: Registry = Registry { }; }
                bus { subscribe "tick" as on_tick of type Tick; }
                fn on_tick(t: Tick) {
                    self.reg.set(Entry { k: t.n, v: 2 });
                }
            }

            main locus App {
                params {
                    io: IoWorker = IoWorker { };
                    cpu: CompWorker = CompWorker { };
                }
                placement {
                    io: cooperative(pool = io);
                    cpu: cooperative(pool = compute);
                }
                bus { publish "tick" of type Tick; }
                run() { }
            }

            fn main() { App { }; }
        "#;
        let mut prog = parse_source(src).expect("parse");
        let diags = apply_sync_inference(&mut prog);
        assert!(diags.is_empty(), "got: {:?}", diags);

        // Find Registry locus + inspect its form args.
        let registry = prog
            .items
            .iter()
            .find_map(|item| match item {
                hale_syntax::ast::TopDecl::Locus(l)
                    if l.name.name == "Registry" =>
                {
                    Some(l)
                }
                _ => None,
            })
            .expect("Registry not found");
        let form = registry
            .form
            .as_ref()
            .expect("Registry has @form annotation");
        let sync_arg = form
            .args
            .iter()
            .find(|a| a.name.name == "sync")
            .expect("auto-apply should have injected sync arg");
        match &sync_arg.value {
            hale_syntax::ast::Expr::Ident(i) => {
                assert_eq!(
                    i.name, "striped",
                    "expected striped, got: {}",
                    i.name
                );
            }
            other => panic!("expected Ident value, got: {:?}", other),
        }
    }

    #[test]
    fn apply_sync_inference_leaves_existing_sync_alone() {
        // Locus already has `sync = serialized` written by
        // hand. Auto-apply must not touch it (even if the
        // inference would have picked something else).
        let src = r#"
            type Entry { k: Int; v: Int; }
            @form(hashmap, sync = serialized)
            locus Registry {
                capacity { pool entries of Entry indexed_by k; }
            }
            fn main() { Registry { }; }
        "#;
        let mut prog = parse_source(src).expect("parse");
        apply_sync_inference(&mut prog);

        let registry = prog
            .items
            .iter()
            .find_map(|item| match item {
                hale_syntax::ast::TopDecl::Locus(l)
                    if l.name.name == "Registry" =>
                {
                    Some(l)
                }
                _ => None,
            })
            .expect("Registry not found");
        let form = registry.form.as_ref().unwrap();
        let sync_args: Vec<_> = form
            .args
            .iter()
            .filter(|a| a.name.name == "sync")
            .collect();
        assert_eq!(
            sync_args.len(),
            1,
            "should still have exactly 1 sync arg"
        );
        match &sync_args[0].value {
            hale_syntax::ast::Expr::Ident(i) => {
                assert_eq!(i.name, "serialized");
            }
            other => panic!("expected serialized, got: {:?}", other),
        }
    }

    #[test]
    fn apply_sync_inference_skips_single_pool_use() {
        // Registry used only from one pool — inference returns
        // None; no annotation injected.
        let src = r#"
            type Entry { k: Int; v: Int; }
            @form(hashmap)
            locus Registry {
                capacity { pool entries of Entry indexed_by k; }
            }
            main locus App {
                params { reg: Registry = Registry { }; }
                run() {
                    self.reg.set(Entry { k: 1, v: 1 });
                }
            }
            fn main() { App { }; }
        "#;
        let mut prog = parse_source(src).expect("parse");
        apply_sync_inference(&mut prog);

        let registry = prog
            .items
            .iter()
            .find_map(|item| match item {
                hale_syntax::ast::TopDecl::Locus(l)
                    if l.name.name == "Registry" =>
                {
                    Some(l)
                }
                _ => None,
            })
            .unwrap();
        let form = registry.form.as_ref().unwrap();
        assert!(
            !form.args.iter().any(|a| a.name.name == "sync"),
            "single-pool use should not inject sync; got args: {:?}",
            form.args
        );
    }

    // === Phase 3 routing-key static checks ===============

    #[test]
    fn keyed_by_field_must_exist_on_payload() {
        let src = r#"
            type T { n: Int; }
            topic K { payload: T; subject: "k"; keyed_by missing; }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| {
                d.message.contains("`keyed_by`")
                    && d.message.contains("missing")
                    && d.message.contains("does not exist")
            }),
            "expected missing-field diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn keyed_by_field_must_be_key_eligible() {
        let src = r#"
            type T { name: String; }
            topic K { payload: T; subject: "k"; keyed_by name; }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| {
                d.message.contains("int-shaped")
                    || d.message.contains("routing-key fields")
            }),
            "expected key-eligibility diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn keyed_by_accepts_int_decimal_time_duration_bool() {
        let src = r#"
            type T {
                a: Int;
                b: Decimal;
                c: Time;
                d: Duration;
                e: Bool;
            }
            topic A { payload: T; subject: "a"; keyed_by a; }
            topic B { payload: T; subject: "b"; keyed_by b; }
            topic C { payload: T; subject: "c"; keyed_by c; }
            topic D { payload: T; subject: "d"; keyed_by d; }
            topic E { payload: T; subject: "e"; keyed_by e; }
        "#;
        let diags = check(src);
        assert!(
            !diags.iter().any(|d| {
                d.message.contains("routing-key fields")
                    || d.message.contains("does not exist")
            }),
            "expected no key-eligibility diag for valid types; got: {:?}",
            diags
        );
    }

    // (`on_unmatched_fail_pending_impl_diag` and
    // `on_unmatched_fallback_pending_impl_diag` were authored in
    // typecheck slice 2 when both policies still emitted "v0.1
    // ships swallow first" diags. fail + fallback shipped in
    // later slices; the diags are gone. Positive coverage for
    // both policies now lives in
    // crates/hale-codegen/tests/bus_routing_keys.rs end-to-end.)

    #[test]
    fn on_unmatched_without_keyed_by_rejected() {
        let src = r#"
            type T { n: Int; }
            topic K {
                payload: T; subject: "k";
                on_unmatched: swallow;
            }
        "#;
        let diags = check(src);
        assert!(
            diags.iter().any(|d| {
                d.message.contains("on_unmatched")
                    && d.message.contains("no `keyed_by`")
            }),
            "expected on_unmatched-without-keyed_by diag, got: {:?}",
            diags
        );
    }

    #[test]
    fn keyed_by_with_swallow_is_clean() {
        let src = r#"
            type T { id: Int; }
            topic K {
                payload: T; subject: "k"; keyed_by id;
                on_unmatched: swallow;
            }
        "#;
        let diags = check(src);
        assert!(
            !diags.iter().any(|d| {
                d.message.contains("not yet implemented")
                    || d.message.contains("does not exist")
                    || d.message.contains("routing-key fields")
                    || d.message.contains("no `keyed_by`")
            }),
            "expected no Phase-3 diags for valid swallow topic; got: {:?}",
            diags
        );
    }
}

#[cfg(test)]
mod unowned_subscriber_tests {
    //! 2026-05-29: a `bus subscribe`-declaring locus instantiated
    //! unowned inside a bus HANDLER body dissolves at handler
    //! return, so its subscription can never fire — hard error,
    //! softenable via `--allow-unowned-subscriber`
    //! (`check_bundle_opts(_, true)`). Owned (accept'd) and
    //! `run()`-spawned subscribers are NOT flagged.

    use super::*;
    use hale_syntax::parse_source;
    use std::collections::BTreeMap;

    fn flagged(src: &str, allow: bool) -> bool {
        let p = parse_source(src).expect("parses");
        let mut programs: BTreeMap<String, &Program> = BTreeMap::new();
        programs.insert(String::new(), &p);
        let diags = check_bundle_opts(&Bundle { programs }, allow);
        diags.iter().any(|d| d.message.contains("instantiated unowned"))
    }

    const UNOWNED_IN_HANDLER: &str = r#"
        type Trigger { n: Int; }
        type Data { v: Int; }
        locus Child {
            params { id: Int = 0; }
            bus { subscribe "data" as on_data of type Data; }
            fn on_data(d: Data) { println("got"); }
        }
        locus Disp {
            bus { subscribe "trig" as on_trig of type Trigger; }
            fn on_trig(t: Trigger) { let c = Child { id: 1 }; }
        }
        fn main() { Disp { }; }
    "#;

    #[test]
    fn unowned_subscriber_in_handler_rejected() {
        assert!(flagged(UNOWNED_IN_HANDLER, false));
    }

    #[test]
    fn allow_flag_silences_it() {
        assert!(!flagged(UNOWNED_IN_HANDLER, true));
    }

    #[test]
    fn accepted_subscriber_in_handler_clean() {
        // Disp accepts Child -> owned -> not flagged.
        let src = r#"
            type Trigger { n: Int; }
            type Data { v: Int; }
            locus Child {
                params { id: Int = 0; }
                bus { subscribe "data" as on_data of type Data; }
                fn on_data(d: Data) { println("got"); }
            }
            locus Disp {
                accept(c: Child) { }
                bus { subscribe "trig" as on_trig of type Trigger; }
                fn on_trig(t: Trigger) { Child { id: 1 }; }
            }
            fn main() { Disp { }; }
        "#;
        assert!(!flagged(src, false));
    }

    #[test]
    fn subscriber_spawned_in_run_clean() {
        // Spawned in run() (not a handler) -> lives for run()'s
        // scope, receives messages published during it -> fine.
        let src = r#"
            type Data { v: Int; }
            locus Watcher {
                params { id: Int = 0; }
                bus { subscribe "data" as on_data of type Data; }
                fn on_data(d: Data) { println("got"); }
            }
            locus Producer {
                bus { publish "data" of type Data; }
                run() {
                    Watcher { id: 1 };
                    "data" <- Data { v: 10 };
                }
            }
            fn main() { Producer { }; }
        "#;
        assert!(!flagged(src, false));
    }

    #[test]
    fn non_subscriber_in_handler_clean() {
        // Child has no bus subscribe -> not flagged even unowned.
        let src = r#"
            type Trigger { n: Int; }
            locus Child {
                params { id: Int = 0; }
                fn read() -> Int { return self.id; }
            }
            locus Disp {
                bus { subscribe "trig" as on_trig of type Trigger; }
                fn on_trig(t: Trigger) { let c = Child { id: 1 }; }
            }
            fn main() { Disp { }; }
        "#;
        assert!(!flagged(src, false));
    }
}
