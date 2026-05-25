//! F.32-2 (2026-05-25): compile-time working-set estimator.
//!
//! Computes a coarse byte-size estimate for each user-declared
//! locus, suitable for an operator-facing "does my locus tower
//! fit in L2?" report. Wire-up consumer for v0.1 is the
//! `hale build --locality-report` flag.
//!
//! The estimator is pure: no LLVM TargetData, no codegen
//! cooperation. It works off the raw AST so it can run
//! post-typecheck without first lowering to codegen IR. That
//! costs some accuracy — alignment padding is ignored, the
//! synthetic header fields collapse into a flat 64-byte arena
//! overhead, and method-body scratch is heuristic-only — but
//! the operator-facing question ("am I in the L1 / L2 / L3
//! envelope") is rough enough that an order-of-magnitude
//! estimate is the whole game.
//!
//! Scope (v0.1):
//!  - Struct size: sum of user-declared `params { }` field type
//!    sizes, plus a flat [`ARENA_OVERHEAD`] for synthetic
//!    headers.
//!  - Capacity slots: `cap` (read from the locus's `@form` args
//!    when present, else assumed unbounded and surfaced via
//!    [`WorkingSetEstimate::unbounded_slots`]) × cell-stride
//!    estimate.
//!  - Child loci: recursive expansion through any param field
//!    typed as another locus name in the same program.
//!
//! Out of scope (v0.1), pursue when a real workload demands it:
//!  - `@locality(L1|L2|L3|any)` per-locus annotation surface +
//!    `--target-cache=lN --strict` gate. The v0.1 surface is
//!    operator-facing report only; no compile failure.
//!  - Auto-detection of cache-tier sizes from
//!    `/sys/devices/system/cpu/cpu0/cache/index{0,2,3}/size`.
//!    The static constants on [`CacheTier`] are conservative
//!    typical-x86_64 defaults.
//!  - Method-body scratch high-water mark (the formula in the
//!    F.32 plan calls this out as heuristic-only anyway).
//!  - Alignment-padding accounting between fields.
//!
//! See `notes/f32-cache-aware-delivery-plan.md` § F.32-2 for the
//! original design; this file ships the engine + report.

use std::collections::BTreeMap;

use hale_syntax::ast::{
    CapacitySlot, Expr, LocusDecl, LocusMember, Literal, PrimType,
    TopDecl, TypeDecl, TypeDeclBody, TypeExpr,
};

/// Approximate working-set estimate for one locus, in bytes.
///
/// Decomposed so the report can attribute bytes back to a
/// source. `total` is the sum of the three byte fields.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkingSetEstimate {
    /// `params { }` field sizes plus [`ARENA_OVERHEAD`]. Assumes
    /// packed layout (no alignment padding accounted for).
    pub struct_bytes: u64,
    /// `capacity { }` slot bytes. `cap × cell_stride` per slot;
    /// 0 for slots whose cap couldn't be resolved (those slot
    /// names land in `unbounded_slots`).
    pub capacity_bytes: u64,
    /// Bytes contributed by locus-typed children. Recursive
    /// `compute_locus_working_set(child).total()` per child
    /// field.
    pub child_bytes: u64,
    /// Slot or field names the estimator couldn't bound at
    /// compile time (unbounded arrays, capacity slots with no
    /// `cap = N`, named types that resolve to opaque). Surfaced
    /// in the report so the operator can decide whether to pin
    /// down a cap.
    pub unbounded_slots: Vec<String>,
}

impl WorkingSetEstimate {
    pub fn total(&self) -> u64 {
        self.struct_bytes
            .saturating_add(self.capacity_bytes)
            .saturating_add(self.child_bytes)
    }
}

/// Cache-tier budget guidance. Constants are conservative
/// approximations of typical x86_64 hardware as of 2026:
/// 32 KB L1, 512 KB L2-per-core, 8 MB shared L3. Auto-detection
/// from sysfs is a follow-up; static defaults are fine for the
/// v0.1 report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheTier {
    L1,
    L2,
    L3,
}

impl CacheTier {
    pub fn budget_bytes(self) -> u64 {
        match self {
            CacheTier::L1 => 32 * 1024,
            CacheTier::L2 => 512 * 1024,
            CacheTier::L3 => 8 * 1024 * 1024,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            CacheTier::L1 => "L1",
            CacheTier::L2 => "L2",
            CacheTier::L3 => "L3",
        }
    }
}

/// Per-locus arena overhead the estimator adds to every
/// non-elidable locus: rough byte budget for `lotus_arena_t` +
/// chunk header + the locus's synthetic fields (`__arena`,
/// `__quarantined`, accept bitmask, etc.). 64 bytes is one
/// cache line; the actual synthetic-field count grows with
/// feature additions but stays well under a line as of F.32.
const ARENA_OVERHEAD: u64 = 64;

/// Compute the working-set estimate for the locus named
/// `locus_name`. Returns `None` if the name doesn't resolve to a
/// locus declaration in `items`. Recursion guarded against
/// cycles (locus type referring to itself directly or
/// transitively) via the `visited` ledger.
pub fn compute_locus_working_set(
    locus_name: &str,
    items: &[TopDecl],
) -> Option<WorkingSetEstimate> {
    let by_name = build_index(items);
    let locus = by_name.loci.get(locus_name)?;
    let mut visited = Vec::new();
    Some(estimate_locus(locus, &by_name, &mut visited))
}

/// Convenience: estimate every user-declared locus in `items`.
/// Returned map is keyed by locus name in declaration order
/// (BTreeMap, so iteration is alphabetical — the report sorts
/// elsewhere if a different order is wanted).
pub fn compute_program_working_set(
    items: &[TopDecl],
) -> BTreeMap<String, WorkingSetEstimate> {
    let by_name = build_index(items);
    let mut out: BTreeMap<String, WorkingSetEstimate> = BTreeMap::new();
    for (name, locus) in &by_name.loci {
        let mut visited = Vec::new();
        out.insert(
            (*name).to_string(),
            estimate_locus(locus, &by_name, &mut visited),
        );
    }
    out
}

/// Pretty-print the per-locus report on stderr. Lines are
/// stable across builds (sorted alphabetically by locus name)
/// so diffs of `--locality-report` output between two builds
/// are reviewable as text. Each line shows the total, the
/// nearest cache tier the locus fits inside, and the byte
/// decomposition. Trailing summary names any unbounded slots
/// so the operator can decide whether to pin a `cap = N`.
pub fn render_locality_report(
    map: &BTreeMap<String, WorkingSetEstimate>,
) -> String {
    let mut out = String::new();
    out.push_str(
        "locality report (F.32-2 working-set estimator):\n",
    );
    if map.is_empty() {
        out.push_str("  (no user-declared loci)\n");
        return out;
    }
    let widest_name = map
        .keys()
        .map(|n| n.len())
        .max()
        .unwrap_or(0)
        .max("locus".len());
    for (name, est) in map {
        let total = est.total();
        let tier = nearest_tier(total);
        let tier_label = match tier {
            Some(t) => format!("fits {}", t.label()),
            None => "exceeds L3".to_string(),
        };
        out.push_str(&format!(
            "  {:<width$}  ~{:>8} B  ({})  struct={} capacity={} children={}\n",
            name,
            total,
            tier_label,
            est.struct_bytes,
            est.capacity_bytes,
            est.child_bytes,
            width = widest_name,
        ));
        if !est.unbounded_slots.is_empty() {
            out.push_str(&format!(
                "    unbounded: {}\n",
                est.unbounded_slots.join(", ")
            ));
        }
    }
    out
}

fn nearest_tier(total_bytes: u64) -> Option<CacheTier> {
    for tier in [CacheTier::L1, CacheTier::L2, CacheTier::L3] {
        if total_bytes <= tier.budget_bytes() {
            return Some(tier);
        }
    }
    None
}

struct Index<'a> {
    loci: BTreeMap<&'a str, &'a LocusDecl>,
    types: BTreeMap<&'a str, &'a TypeDecl>,
}

fn build_index(items: &[TopDecl]) -> Index<'_> {
    let mut loci: BTreeMap<&str, &LocusDecl> = BTreeMap::new();
    let mut types: BTreeMap<&str, &TypeDecl> = BTreeMap::new();
    for item in items {
        match item {
            TopDecl::Locus(l) => {
                loci.insert(l.name.name.as_str(), l);
            }
            TopDecl::Type(t) => {
                types.insert(t.name.name.as_str(), t);
            }
            _ => {}
        }
    }
    Index { loci, types }
}

fn estimate_locus(
    locus: &LocusDecl,
    idx: &Index<'_>,
    visited: &mut Vec<String>,
) -> WorkingSetEstimate {
    // Cycle guard: don't recurse through a locus we're already
    // unwinding higher in the call stack. A → B → A would
    // otherwise blow the recursion budget. Estimator returns
    // an empty contribution for the recursive leg; the
    // top-level locus still gets its own struct/capacity bytes.
    if visited.iter().any(|n| n == &locus.name.name) {
        return WorkingSetEstimate::default();
    }
    visited.push(locus.name.name.clone());

    let mut est = WorkingSetEstimate {
        struct_bytes: ARENA_OVERHEAD,
        ..Default::default()
    };

    // Walk params { } — every user field contributes either to
    // struct_bytes (primitive / type) or to child_bytes (locus-
    // typed field, recursive expansion).
    for member in &locus.members {
        if let LocusMember::Params(pb) = member {
            for p in &pb.params {
                let Some(ty) = &p.ty else { continue };
                if let Some(child) = locus_type_name(ty, idx) {
                    let mut child_est =
                        estimate_locus(child, idx, visited);
                    // Child's own arena overhead is paid by the
                    // child estimate; no double-charge.
                    est.child_bytes = est
                        .child_bytes
                        .saturating_add(child_est.total());
                    est.unbounded_slots
                        .append(&mut child_est.unbounded_slots);
                } else {
                    let (size, unbounded) = type_size(ty, idx);
                    est.struct_bytes =
                        est.struct_bytes.saturating_add(size);
                    if unbounded {
                        est.unbounded_slots.push(p.name.name.clone());
                    }
                }
            }
        }
    }

    // Walk capacity { } slots and multiply cap × cell_stride.
    // cap comes from the @form annotation's `cap = N` arg when
    // present; absent means the slot grows dynamically and the
    // slot name lands in unbounded_slots.
    let form_cap = form_cap_from_annotation(locus);
    for member in &locus.members {
        if let LocusMember::Capacity(cb) = member {
            for slot in &cb.slots {
                let stride = type_size(&slot.elem_ty, idx).0;
                if let Some(cap) = form_cap {
                    est.capacity_bytes = est
                        .capacity_bytes
                        .saturating_add(stride.saturating_mul(cap));
                } else {
                    est.unbounded_slots.push(slot_label(slot));
                }
            }
        }
    }

    visited.pop();
    est
}

fn slot_label(slot: &CapacitySlot) -> String {
    format!("capacity:{}", slot.name.name)
}

/// Returns the matching locus declaration if `ty` is `Named`
/// and resolves to a locus in the program. `None` for any other
/// shape (primitive, type alias, array, projection, etc.).
fn locus_type_name<'a>(
    ty: &TypeExpr,
    idx: &Index<'a>,
) -> Option<&'a LocusDecl> {
    let TypeExpr::Named { path, .. } = ty else {
        return None;
    };
    let first = path.segments.first()?;
    idx.loci.get(first.name.as_str()).copied()
}

/// Approximate byte size of a type expression. Returns
/// `(bytes, unbounded)` — the second tuple member flags slots
/// whose size couldn't be bounded (unbounded arrays, opaque
/// named types) so the caller can name them in the report.
fn type_size(ty: &TypeExpr, idx: &Index<'_>) -> (u64, bool) {
    match ty {
        TypeExpr::Primitive(p, _) => (primitive_size(*p), false),
        TypeExpr::Tuple(parts, _) => {
            let mut total: u64 = 0;
            let mut unbounded = false;
            for t in parts {
                let (s, u) = type_size(t, idx);
                total = total.saturating_add(s);
                unbounded |= u;
            }
            (total, unbounded)
        }
        TypeExpr::Array { elem, size, .. } => {
            let (elem_size, elem_unbounded) = type_size(elem, idx);
            let cap = size.as_ref().and_then(literal_int);
            match cap {
                Some(n) => (
                    elem_size.saturating_mul(n),
                    elem_unbounded,
                ),
                None => (0, true),
            }
        }
        TypeExpr::Named { path, .. } => {
            let Some(first) = path.segments.first() else {
                return (16, true);
            };
            if let Some(td) = idx.types.get(first.name.as_str()) {
                return type_decl_size(td, idx);
            }
            // Locus-typed names are handled by the caller
            // (which recurses through estimate_locus); we get
            // here when the named ref isn't a known struct,
            // alias, or enum. Conservative pointer-sized
            // placeholder, flagged unbounded.
            (16, true)
        }
        TypeExpr::Projection { inner, .. } => type_size(inner, idx),
        TypeExpr::Function { .. } => (8, false),
    }
}

fn type_decl_size(td: &TypeDecl, idx: &Index<'_>) -> (u64, bool) {
    match &td.body {
        TypeDeclBody::Alias(inner) => type_size(inner, idx),
        TypeDeclBody::Struct(fields) => {
            let mut total: u64 = 0;
            let mut unbounded = false;
            for f in fields {
                let (s, u) = type_size(&f.ty, idx);
                total = total.saturating_add(s);
                unbounded |= u;
            }
            (total, unbounded)
        }
        TypeDeclBody::Enum(variants) => {
            // Largest payload + tag. Tag = 8 bytes for
            // alignment; payload = max(sum-of-fields across
            // variants).
            let mut max_payload: u64 = 0;
            let mut unbounded = false;
            for v in variants {
                let mut vsize: u64 = 0;
                for ty in &v.fields {
                    let (s, u) = type_size(ty, idx);
                    vsize = vsize.saturating_add(s);
                    unbounded |= u;
                }
                if vsize > max_payload {
                    max_payload = vsize;
                }
            }
            (max_payload.saturating_add(8), unbounded)
        }
    }
}

fn primitive_size(p: PrimType) -> u64 {
    match p {
        PrimType::Int
        | PrimType::Uint
        | PrimType::Float
        | PrimType::Time
        | PrimType::Duration => 8,
        PrimType::Decimal => 16,
        PrimType::Bool => 1,
        // Heap-managed sequences. Approximated as pointer + len
        // = 16 bytes resident on the struct; the heap buffer
        // itself is not counted (lives in the locus's arena,
        // already covered by the per-method scratch heuristic
        // we elided in v0.1).
        PrimType::String
        | PrimType::Bytes
        | PrimType::StringView
        | PrimType::BytesView => 16,
    }
}

fn literal_int(e: &Expr) -> Option<u64> {
    match e {
        Expr::Literal(Literal::Int(n), _) => {
            if *n < 0 {
                None
            } else {
                Some(*n as u64)
            }
        }
        _ => None,
    }
}

/// Read `cap = N` off the locus's `@form(<name>, cap = N)`
/// annotation, if present. The form annotation is the only
/// place v1 surfaces capacity caps; future surface (a slot-
/// level `cap = N` kwarg) would extend this.
fn form_cap_from_annotation(locus: &LocusDecl) -> Option<u64> {
    let form = locus.form.as_ref()?;
    for arg in &form.args {
        if arg.name.name == "cap" {
            return literal_int(&arg.value);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use hale_syntax::parse_source;

    fn estimate(src: &str, name: &str) -> WorkingSetEstimate {
        let p = parse_source(src).expect("parse");
        compute_locus_working_set(name, &p.items)
            .unwrap_or_else(|| panic!("no locus `{}`", name))
    }

    #[test]
    fn empty_locus_is_arena_overhead_only() {
        let est = estimate(
            r#"
                locus Empty { }
                fn main() { Empty { }; }
            "#,
            "Empty",
        );
        assert_eq!(est.struct_bytes, ARENA_OVERHEAD);
        assert_eq!(est.capacity_bytes, 0);
        assert_eq!(est.child_bytes, 0);
        assert!(est.unbounded_slots.is_empty());
    }

    #[test]
    fn primitive_param_fields_contribute_bytes() {
        // 4 Ints × 8B + 1 Bool × 1B + 1 Decimal × 16B = 49 B
        let est = estimate(
            r#"
                locus L {
                    params {
                        a: Int = 0;
                        b: Int = 0;
                        c: Int = 0;
                        d: Int = 0;
                        flag: Bool = false;
                        money: Decimal = 0.0;
                    }
                }
                fn main() { L { }; }
            "#,
            "L",
        );
        assert_eq!(est.struct_bytes, ARENA_OVERHEAD + 4 * 8 + 1 + 16);
    }

    #[test]
    fn string_field_counts_as_ptr_plus_len() {
        let est = estimate(
            r#"
                locus L { params { name: String = ""; } }
                fn main() { L { }; }
            "#,
            "L",
        );
        assert_eq!(est.struct_bytes, ARENA_OVERHEAD + 16);
    }

    #[test]
    fn array_with_literal_cap_bounds_size() {
        // [Int; 8] = 8 × 8 = 64 B
        let est = estimate(
            r#"
                locus L { params { buf: [Int; 8] = []; } }
                fn main() { L { }; }
            "#,
            "L",
        );
        assert_eq!(est.struct_bytes, ARENA_OVERHEAD + 64);
        assert!(est.unbounded_slots.is_empty());
    }

    #[test]
    fn unbounded_array_flags_unbounded() {
        // Array with no literal size — the field's size can't
        // be bounded; lands in unbounded_slots.
        let est = estimate(
            r#"
                locus L { params { buf: [Int] = []; } }
                fn main() { L { }; }
            "#,
            "L",
        );
        assert_eq!(est.unbounded_slots, vec!["buf".to_string()]);
    }

    #[test]
    fn user_struct_size_recurses_into_fields() {
        // Quote = Decimal (16) + Decimal (16) + Int (8) = 40 B
        let est = estimate(
            r#"
                type Quote { bid: Decimal; ask: Decimal; venue: Int; }
                locus L { params { latest: Quote = Quote { bid: 0.0, ask: 0.0, venue: 0 }; } }
                fn main() { L { }; }
            "#,
            "L",
        );
        assert_eq!(est.struct_bytes, ARENA_OVERHEAD + 40);
    }

    #[test]
    fn locus_typed_param_field_routes_to_child_bytes() {
        // Inner empty locus: ARENA_OVERHEAD only.
        // Outer locus has a field of type Inner — that goes
        // into child_bytes, not struct_bytes. Outer's struct
        // still has its own arena overhead.
        let est = estimate(
            r#"
                locus Inner { }
                locus Outer { params { i: Inner = Inner { }; } }
                fn main() { Outer { }; }
            "#,
            "Outer",
        );
        assert_eq!(est.struct_bytes, ARENA_OVERHEAD);
        assert_eq!(est.child_bytes, ARENA_OVERHEAD);
        assert_eq!(est.total(), ARENA_OVERHEAD * 2);
    }

    #[test]
    fn hashmap_form_with_cap_bounds_capacity_bytes() {
        // @form(hashmap, cap = 64) with Entry cells of
        // Int (8) + Int (8) = 16 B stride. 64 × 16 = 1024 B.
        let est = estimate(
            r#"
                type Entry { k: Int; v: Int; }
                @form(hashmap, sync = lockfree, cap = 64)
                locus Registry {
                    capacity { pool entries of Entry indexed_by k; }
                }
                fn main() { Registry { }; }
            "#,
            "Registry",
        );
        assert_eq!(est.capacity_bytes, 64 * 16);
        assert!(est.unbounded_slots.is_empty());
    }

    #[test]
    fn hashmap_form_without_cap_flags_unbounded() {
        // No `cap = N` → slot lands in unbounded_slots.
        let est = estimate(
            r#"
                type Entry { k: Int; v: Int; }
                @form(hashmap, sync = serialized)
                locus Registry {
                    capacity { pool entries of Entry indexed_by k; }
                }
                fn main() { Registry { }; }
            "#,
            "Registry",
        );
        assert_eq!(est.capacity_bytes, 0);
        assert!(
            est.unbounded_slots.contains(&"capacity:entries".to_string()),
            "unbounded_slots = {:?}",
            est.unbounded_slots
        );
    }

    #[test]
    fn cache_tier_budget_constants() {
        assert_eq!(CacheTier::L1.budget_bytes(), 32 * 1024);
        assert_eq!(CacheTier::L2.budget_bytes(), 512 * 1024);
        assert_eq!(CacheTier::L3.budget_bytes(), 8 * 1024 * 1024);
    }

    #[test]
    fn nearest_tier_picks_smallest_fitting() {
        assert_eq!(nearest_tier(0), Some(CacheTier::L1));
        assert_eq!(nearest_tier(32 * 1024), Some(CacheTier::L1));
        assert_eq!(nearest_tier(32 * 1024 + 1), Some(CacheTier::L2));
        assert_eq!(nearest_tier(512 * 1024), Some(CacheTier::L2));
        assert_eq!(nearest_tier(512 * 1024 + 1), Some(CacheTier::L3));
        assert_eq!(nearest_tier(8 * 1024 * 1024), Some(CacheTier::L3));
        assert_eq!(nearest_tier(8 * 1024 * 1024 + 1), None);
    }

    #[test]
    fn render_report_contains_locus_names_and_totals() {
        let src = r#"
            locus Alpha { params { x: Int = 0; } }
            locus Beta  { params { s: String = ""; } }
            fn main() { Alpha { }; Beta { }; }
        "#;
        let p = parse_source(src).expect("parse");
        let map = compute_program_working_set(&p.items);
        let report = render_locality_report(&map);
        assert!(report.contains("Alpha"), "got:\n{}", report);
        assert!(report.contains("Beta"), "got:\n{}", report);
        assert!(report.contains("fits L1"), "got:\n{}", report);
    }

    #[test]
    fn compute_program_returns_entry_per_locus() {
        let src = r#"
            locus A { params { x: Int = 0; } }
            locus B { params { y: Int = 0; } }
            locus C { }
            fn main() { A { }; B { }; C { }; }
        "#;
        let p = parse_source(src).expect("parse");
        let map = compute_program_working_set(&p.items);
        assert_eq!(map.len(), 3);
        assert!(map.contains_key("A"));
        assert!(map.contains_key("B"));
        assert!(map.contains_key("C"));
    }

    #[test]
    fn missing_locus_returns_none() {
        let p = parse_source(
            r#"
                locus L { }
                fn main() { L { }; }
            "#,
        )
        .expect("parse");
        assert!(compute_locus_working_set("Nope", &p.items).is_none());
    }

    #[test]
    fn cyclic_locus_reference_is_handled() {
        // A → B → A. Once visited, the second visit returns
        // empty (cycle guard). No infinite recursion; estimator
        // terminates.
        let src = r#"
            locus A { params { b: B = B { }; } }
            locus B { params { a: A = A { }; } }
            fn main() { A { }; }
        "#;
        let p = parse_source(src).expect("parse");
        let est = compute_locus_working_set("A", &p.items).unwrap();
        // Just assert termination + non-degenerate output. The
        // exact byte count is sensitive to ordering, which is
        // implementation detail.
        assert!(est.total() >= ARENA_OVERHEAD);
    }
}
