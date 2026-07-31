//! Aliasing stage 1, indexed accessors (#322, 2026-07-31).
//!
//! `lotus_str_len` / `lotus_bytes_len` / `lotus_bytes_data` have
//! carried `memory(read) nounwind willreturn` since 2026-07-01 so LICM
//! can hoist a length read out of a loop. Their INDEXED siblings —
//! `lotus_bytes_at`, `lotus_str_byte_at` — were missed, so a
//! loop-invariant `std::bytes::at(b, 0)` stayed inside the loop body
//! across every iteration while the identically-shaped `len` call in
//! the same program was hoisted to `entry:` and its loop folded away.
//! The only difference was the attribute.
//!
//! The exclusion tests below are the important half. The bar is an
//! accessor over IMMUTABLE data with no store, no out-parameter, and
//! no lock — audited against the runtime C. Marking a container
//! accessor would be a miscompilation, not a slow path: hoisting a
//! poll loop's `len` read out of the loop turns a spin into a hang.

use hale_codegen::build_executable;
use hale_syntax::parse_source;

#[path = "support/harness.rs"]
mod harness;

/// Builtins are declared unconditionally and `LOTUS_DUMP_IR` dumps
/// PRE-optimization, so a trivial program's IR carries every runtime
/// declaration — no fixture needs to call them.
fn builtin_ir(name: &str) -> String {
    let program = parse_source("fn main() { println(\"hi\"); }").expect("parse");
    let bin = harness::unique_bin(name);
    std::env::set_var("LOTUS_DUMP_IR", "1");
    build_executable(&program, &bin).expect("build");
    std::env::remove_var("LOTUS_DUMP_IR");
    let ll = bin.with_extension("ll");
    let ir = std::fs::read_to_string(&ll).expect("IR dumped");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&ll);
    ir
}

/// The attribute-group body a declaration resolves to, e.g.
/// `{ nounwind willreturn memory(read) }`. `None` when the symbol is
/// declared with no group at all.
fn attrs_of(ir: &str, sym: &str) -> Option<String> {
    let decl = ir
        .lines()
        .find(|l| l.starts_with("declare") && l.contains(&format!("@{}(", sym)))?;
    let group = decl
        .split_whitespace()
        .last()
        .filter(|t| t.starts_with('#'))?;
    ir.lines()
        .find(|l| l.starts_with(&format!("attributes {} =", group)))
        .map(|l| l.to_string())
}

#[test]
fn indexed_immutable_accessors_are_pure_reads() {
    let ir = builtin_ir("pure_read_idx");
    for sym in ["lotus_bytes_at", "lotus_str_byte_at"] {
        let body = attrs_of(&ir, sym)
            .unwrap_or_else(|| panic!("{} should carry an attribute group", sym));
        assert!(
            body.contains("memory(read)"),
            "{} must be memory(read) so LICM can hoist it: {}",
            sym,
            body
        );
        assert!(
            body.contains("willreturn") && body.contains("nounwind"),
            "{} must also carry willreturn + nounwind: {}",
            sym,
            body
        );
    }
}

/// The new symbols must land in the SAME group as the 2026-07-01
/// precedent — one shared "pure read" classification, not a parallel
/// one that could drift.
#[test]
fn indexed_accessors_match_the_length_accessor_precedent() {
    let ir = builtin_ir("pure_read_precedent");
    let precedent = attrs_of(&ir, "lotus_str_len").expect("str_len annotated");
    for sym in ["lotus_bytes_at", "lotus_str_byte_at", "lotus_bytes_len"] {
        assert_eq!(
            attrs_of(&ir, sym).as_deref(),
            Some(precedent.as_str()),
            "{} should share the pure-read attribute group with lotus_str_len",
            sym
        );
    }
}

/// Concurrently-mutable container lengths must NOT be pure reads.
///
/// These read `__atomic_load_n` on a live container under
/// striped/lockfree modes. `memory(read)` would license LICM to hoist a
/// poll loop's read out of the loop, so a worker waiting on another
/// worker's push would never observe it — a hang, not a slowdown.
#[test]
fn mutable_container_lengths_are_not_pure_reads() {
    let ir = builtin_ir("pure_read_excl_len");
    for sym in ["lotus_vec_len", "lotus_hashmap_len", "lotus_ring_buffer_len"] {
        if let Some(body) = attrs_of(&ir, sym) {
            assert!(
                !body.contains("memory(read)") && !body.contains("memory(none)"),
                "{} reads concurrently-mutable state and must NOT be a pure \
                 read — hoisting a poll loop's length read is a hang: {}",
                sym,
                body
            );
        }
    }
}

/// Out-parameter writers and read-mutating accessors are excluded too:
/// `lotus_vec_get` / `lotus_hashmap_get` `memcpy` into an out pointer
/// (and hashmap_get takes a lock), and `lotus_lru_get` writes a
/// recency tick ON READ.
#[test]
fn out_param_and_recency_writing_accessors_are_not_pure_reads() {
    let ir = builtin_ir("pure_read_excl_out");
    for sym in ["lotus_vec_get", "lotus_hashmap_get", "lotus_lru_get"] {
        if let Some(body) = attrs_of(&ir, sym) {
            assert!(
                !body.contains("memory(read)") && !body.contains("memory(none)"),
                "{} writes memory (out-param or recency tick) and must NOT \
                 be a pure read: {}",
                sym,
                body
            );
        }
    }
}
