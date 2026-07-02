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
    CapacitySlot, Expr, LocalityTier, LocusDecl, LocusMember, Literal,
    PrimType, TopDecl, TypeDecl, TypeDeclBody, TypeExpr,
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
    /// FUv0.8.2 #3 (2026-05-25): conservative method-scratch
    /// high-water mark — the largest sum of transient allocs
    /// observed across any one method body on this locus.
    /// Method scratch is destroyed at each method return so
    /// it doesn't contribute long-term resident bytes, but the
    /// instantaneous footprint during a method call does sit
    /// in cache alongside the locus's struct + capacity. The
    /// estimator counts struct/tuple/array literals + Binary
    /// String-concat sites + ArrayRepeat — each contributes
    /// its byte size to the running per-method sum; the locus's
    /// scratch_bytes is the max sum across all its methods
    /// (since only one method body runs in a single scratch
    /// subregion at a time).
    pub scratch_bytes: u64,
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
            .saturating_add(self.scratch_bytes)
    }
}

/// Cache-tier budget guidance. The numerical budgets come
/// from sysfs (`/sys/devices/system/cpu/cpu0/cache/index{0,2,3}/size`)
/// on the build host when available; falls back to static
/// defaults (32 KB L1, 512 KB L2-per-core, 8 MB shared L3)
/// when sysfs isn't present (non-Linux build host, container
/// without `/sys`, parse failure on an exotic format). The
/// detected value is cached in a `OnceLock` so the first
/// `budget_bytes()` call pays the I/O and subsequent calls
/// are a constant load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheTier {
    L1,
    L2,
    L3,
}

impl CacheTier {
    pub fn budget_bytes(self) -> u64 {
        let budgets = detect_cache_budgets();
        match self {
            CacheTier::L1 => budgets.l1,
            CacheTier::L2 => budgets.l2,
            CacheTier::L3 => budgets.l3,
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

/// Static fallback when sysfs is unavailable / unparseable.
/// Conservative typical-x86_64 defaults as of 2026.
const FALLBACK_L1_BYTES: u64 = 32 * 1024;
const FALLBACK_L2_BYTES: u64 = 512 * 1024;
const FALLBACK_L3_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
struct CacheBudgets {
    l1: u64,
    l2: u64,
    l3: u64,
}

/// F.32-2 v0.2 (2026-05-25): one-shot sysfs probe.
/// `/sys/devices/system/cpu/cpu0/cache/index{0,2,3}/size`
/// holds the per-CPU L1d / L2 / L3 cache size as a string
/// like `"32K"` / `"512K"` / `"8M"`. Each index is parsed
/// independently; any failure (file missing, unrecognized
/// suffix, integer parse error) falls back to the
/// corresponding static `FALLBACK_*` constant. Cached in a
/// `OnceLock` so the first call to any `CacheTier::budget_bytes`
/// pays the three open/read syscalls; later calls are a load.
fn detect_cache_budgets() -> CacheBudgets {
    static CACHE: std::sync::OnceLock<CacheBudgets> =
        std::sync::OnceLock::new();
    *CACHE.get_or_init(probe_cache_budgets_from_sysfs)
}

fn probe_cache_budgets_from_sysfs() -> CacheBudgets {
    // Linux convention:
    //   index0 = L1d (data cache, 32K typical)
    //   index1 = L1i (instruction cache; not relevant here)
    //   index2 = L2 (typical 512K-1M per core)
    //   index3 = L3 (typical 8-128M shared)
    // The indices' meanings come from `/sys/.../type` (Data /
    // Instruction / Unified) but in practice index 0/2/3 are
    // the right answers on every x86_64 / aarch64 box we care
    // about. Read all three; fall back per-tier on any
    // failure so a partial sysfs (e.g., a VM without an L3
    // entry) still gets honest L1/L2 numbers.
    let base = "/sys/devices/system/cpu/cpu0/cache";
    let l1 = read_cache_size(&format!("{}/index0/size", base))
        .unwrap_or(FALLBACK_L1_BYTES);
    let l2 = read_cache_size(&format!("{}/index2/size", base))
        .unwrap_or(FALLBACK_L2_BYTES);
    let l3 = read_cache_size(&format!("{}/index3/size", base))
        .unwrap_or(FALLBACK_L3_BYTES);
    CacheBudgets { l1, l2, l3 }
}

/// Parse the sysfs `size` file. Format is a decimal integer
/// followed by an optional unit suffix (`K`, `M`, `G`).
/// Trailing newline is stripped. Returns `None` on any
/// parse / IO failure.
fn read_cache_size(path: &str) -> Option<u64> {
    let s = std::fs::read_to_string(path).ok()?;
    parse_sysfs_cache_size(s.trim())
}

fn parse_sysfs_cache_size(s: &str) -> Option<u64> {
    if s.is_empty() {
        return None;
    }
    let (num, suffix) = match s.chars().last()? {
        c if c.is_ascii_alphabetic() => {
            (&s[..s.len() - c.len_utf8()], Some(c.to_ascii_uppercase()))
        }
        _ => (s, None),
    };
    let n: u64 = num.parse().ok()?;
    let scale: u64 = match suffix {
        None => 1,
        Some('K') => 1024,
        Some('M') => 1024 * 1024,
        Some('G') => 1024 * 1024 * 1024,
        Some(_) => return None,
    };
    n.checked_mul(scale)
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
            "  {:<width$}  ~{:>8} B  ({})  struct={} capacity={} children={} scratch={}\n",
            name,
            total,
            tier_label,
            est.struct_bytes,
            est.capacity_bytes,
            est.child_bytes,
            est.scratch_bytes,
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

/// F.32-2 v0.2 (2026-05-25): parse a `--target-cache` value
/// (case-insensitive `l1` / `l2` / `l3`). Returns `None` for
/// unrecognized values; the CLI surfaces the diagnostic with
/// the unknown spelling.
pub fn parse_cache_tier(s: &str) -> Option<CacheTier> {
    match s.to_ascii_lowercase().as_str() {
        "l1" => Some(CacheTier::L1),
        "l2" => Some(CacheTier::L2),
        "l3" => Some(CacheTier::L3),
        _ => None,
    }
}

/// F.32-2 v0.2 budget breach record: one locus that exceeds
/// its effective budget. `excess_bytes` is total − budget.
/// `source` names where the budget came from (per-locus
/// annotation vs global `--target-cache`) so the diagnostic
/// can attribute the contract correctly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetBreach {
    pub locus_name: String,
    pub total_bytes: u64,
    pub budget_bytes: u64,
    pub excess_bytes: u64,
    pub tier: CacheTier,
    pub source: BudgetSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetSource {
    /// Tier came from a per-locus `@locality(L1|L2|L3)`
    /// annotation. The annotation is a hard contract; breaches
    /// surface even without `--target-cache` on the command
    /// line.
    LocalityAnnotation,
    /// Tier came from the global `--target-cache` CLI flag.
    GlobalTargetCache,
}

impl BudgetSource {
    pub fn label(self) -> &'static str {
        match self {
            BudgetSource::LocalityAnnotation => "@locality",
            BudgetSource::GlobalTargetCache => "--target-cache",
        }
    }
}

/// F.32-2 v0.2: evaluate every locus in `map` against the
/// named cache tier's budget. Returns the set of breaches in
/// alphabetical locus order (matching the report's sort
/// stability). Empty vec = clean. All breaches are attributed
/// to `GlobalTargetCache`; the per-locus-annotation path uses
/// `breaches_with_per_locus_budgets`.
pub fn breaches_against_tier(
    map: &BTreeMap<String, WorkingSetEstimate>,
    tier: CacheTier,
) -> Vec<BudgetBreach> {
    let budget = tier.budget_bytes();
    map.iter()
        .filter_map(|(name, est)| {
            let total = est.total();
            if total > budget {
                Some(BudgetBreach {
                    locus_name: name.clone(),
                    total_bytes: total,
                    budget_bytes: budget,
                    excess_bytes: total - budget,
                    tier,
                    source: BudgetSource::GlobalTargetCache,
                })
            } else {
                None
            }
        })
        .collect()
}

/// F.32-2 v0.2 (2026-05-25): resolve a locus's effective
/// budget. Per-locus `@locality(L1|L2|L3)` wins over the
/// global `--target-cache`; `@locality(any)` explicitly opts
/// out of the global gate; an absent annotation falls through
/// to the global tier (or `None` if no global is set).
///
/// Returned tuple's second component names the source so a
/// downstream breach record can attribute the contract.
/// Returns `None` when the locus has no effective budget.
pub fn effective_locus_budget(
    locus: &LocusDecl,
    global_target: Option<CacheTier>,
) -> Option<(CacheTier, BudgetSource)> {
    match locus.locality.as_ref().map(|a| a.tier) {
        Some(LocalityTier::L1) => {
            Some((CacheTier::L1, BudgetSource::LocalityAnnotation))
        }
        Some(LocalityTier::L2) => {
            Some((CacheTier::L2, BudgetSource::LocalityAnnotation))
        }
        Some(LocalityTier::L3) => {
            Some((CacheTier::L3, BudgetSource::LocalityAnnotation))
        }
        Some(LocalityTier::Any) => None,
        None => global_target.map(|t| (t, BudgetSource::GlobalTargetCache)),
    }
}

/// F.32-2 v0.2: walk the program's loci, resolve each one's
/// effective budget (per-locus annotation overrides
/// `global_target`; `@locality(any)` opts out), and report
/// every breach. Returns alphabetical-by-locus-name order.
/// `global_target = None` still evaluates loci carrying
/// explicit `@locality` annotations — those are a hard
/// contract regardless of CLI flags.
pub fn breaches_with_per_locus_budgets(
    map: &BTreeMap<String, WorkingSetEstimate>,
    items: &[TopDecl],
    global_target: Option<CacheTier>,
) -> Vec<BudgetBreach> {
    let idx = build_index(items);
    let mut out: Vec<BudgetBreach> = Vec::new();
    for (name, est) in map {
        let Some(locus) = idx.loci.get(name.as_str()) else {
            continue;
        };
        let Some((tier, source)) =
            effective_locus_budget(locus, global_target)
        else {
            continue;
        };
        let total = est.total();
        let budget = tier.budget_bytes();
        if total > budget {
            out.push(BudgetBreach {
                locus_name: name.clone(),
                total_bytes: total,
                budget_bytes: budget,
                excess_bytes: total - budget,
                tier,
                source,
            });
        }
    }
    out
}

/// F.32-2 v0.2: format a breach list as a stderr-friendly
/// diagnostic. `severity` is "error" or "warning" — the
/// caller picks based on whether `--strict` is set. Each
/// breach line attributes its budget to either the per-locus
/// annotation or the global `--target-cache` flag.
pub fn render_breach_diagnostic(
    breaches: &[BudgetBreach],
    severity: &str,
) -> String {
    let mut out = String::new();
    if breaches.is_empty() {
        return out;
    }
    out.push_str(&format!(
        "{}: {} locus(es) exceed their working-set budget:\n",
        severity,
        breaches.len(),
    ));
    for b in breaches {
        out.push_str(&format!(
            "  {}: estimated {} B (+ {} B over {} = {} B; from {})\n",
            b.locus_name,
            b.total_bytes,
            b.excess_bytes,
            b.tier.label(),
            b.budget_bytes,
            b.source.label(),
        ));
    }
    out
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

    let mut est = WorkingSetEstimate::default();

    // Walk params { } — every user field contributes either to
    // struct_bytes (primitive / type) or to child_bytes (locus-
    // typed field, recursive expansion). F.32-2 v0.2: lay out
    // user fields with alignment-aware offset accumulation
    // (round each field's offset up to its natural alignment
    // before adding its size, round the final total up to the
    // struct's own alignment). Packed-layout assumption from
    // v0.1 under-estimated structs by 10-20% on
    // mixed-alignment shapes; alignment-correct accumulation
    // catches that.
    let mut user_offset: u64 = 0;
    let mut user_align: u64 = 1;
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
                    let info = type_size_info(ty, idx);
                    user_offset = round_up(user_offset, info.align);
                    user_offset = user_offset.saturating_add(info.size);
                    if info.align > user_align {
                        user_align = info.align;
                    }
                    if info.unbounded {
                        est.unbounded_slots.push(p.name.name.clone());
                    }
                }
            }
        }
    }
    let user_section_size = round_up(user_offset, user_align);
    est.struct_bytes =
        ARENA_OVERHEAD.saturating_add(user_section_size);

    // Walk capacity { } slots and multiply cap × cell_stride.
    // cap comes from the @form annotation's `cap = N` arg when
    // present; absent means the slot grows dynamically and the
    // slot name lands in unbounded_slots. Stride includes
    // alignment padding (`type_size_info` already rounds up).
    let form_cap = form_cap_from_annotation(locus);
    for member in &locus.members {
        if let LocusMember::Capacity(cb) = member {
            for slot in &cb.slots {
                let info = type_size_info(&slot.elem_ty, idx);
                let stride = round_up(info.size, info.align);
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

    // FUv0.8.2 #3: method-scratch high-water mark. Walk each
    // method body (lifecycle / mode / fn / on_failure), sum
    // the transient allocations a single execution would put
    // in scratch, and take the max across methods. Only one
    // method body runs per scratch subregion at a time, so
    // the max bounds the instantaneous footprint.
    let mut high_water: u64 = 0;
    for member in &locus.members {
        let body_opt = match member {
            LocusMember::Lifecycle(lc) => Some(&lc.body),
            LocusMember::Mode(md) => Some(&md.body),
            LocusMember::Fn(fd) => Some(&fd.body),
            LocusMember::Failure(ff) => Some(&ff.body),
            _ => None,
        };
        if let Some(body) = body_opt {
            let bytes = block_scratch_bytes(body, idx);
            if bytes > high_water {
                high_water = bytes;
            }
        }
    }
    est.scratch_bytes = high_water;

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

/// F.32-2 v0.2 (2026-05-25): size + alignment + unbounded
/// flag for a type expression. v0.1 tracked only `(size,
/// unbounded)` and assumed packed layout, which under-
/// estimated structs by ~10-20% (every Bool / Int adjacent to
/// a Decimal lost 8-15 bytes of padding).
///
/// `size` is the layout-correct byte size (interior padding
/// applied for structs, final padding rounded up to the
/// type's own alignment). `align` is the type's natural
/// alignment — used by enclosing structs to round their
/// running offset up to a field boundary.
#[derive(Debug, Clone, Copy)]
struct TypeSizeInfo {
    size: u64,
    align: u64,
    unbounded: bool,
}

fn type_size_info(ty: &TypeExpr, idx: &Index<'_>) -> TypeSizeInfo {
    match ty {
        TypeExpr::Primitive(p, _) => primitive_size_info(*p),
        TypeExpr::Bounded { elem, cap, .. } => {
            // { i64 len, [N x T] } inline.
            let e = type_size_info(elem, idx);
            let data = round_up(8, e.align) + e.size * *cap;
            TypeSizeInfo {
                size: round_up(data, 8.max(e.align)),
                align: 8.max(e.align),
                unbounded: e.unbounded,
            }
        }
        TypeExpr::Tuple(parts, _) => {
            let mut size: u64 = 0;
            let mut align: u64 = 1;
            let mut unbounded = false;
            for t in parts {
                let info = type_size_info(t, idx);
                size = round_up(size, info.align);
                size = size.saturating_add(info.size);
                if info.align > align {
                    align = info.align;
                }
                unbounded |= info.unbounded;
            }
            size = round_up(size, align);
            TypeSizeInfo { size, align, unbounded }
        }
        TypeExpr::Array { elem, size, .. } => {
            let elem_info = type_size_info(elem, idx);
            // Each element of a `[T; N]` is laid out at the
            // element's natural alignment. Effective stride =
            // round_up(elem_size, elem_align).
            let stride = round_up(elem_info.size, elem_info.align);
            let cap = size.as_ref().and_then(literal_int);
            match cap {
                Some(n) => TypeSizeInfo {
                    size: stride.saturating_mul(n),
                    align: elem_info.align,
                    unbounded: elem_info.unbounded,
                },
                None => TypeSizeInfo {
                    size: 0,
                    align: elem_info.align,
                    unbounded: true,
                },
            }
        }
        TypeExpr::Named { path, .. } => {
            let Some(first) = path.segments.first() else {
                return TypeSizeInfo { size: 16, align: 8, unbounded: true };
            };
            if let Some(td) = idx.types.get(first.name.as_str()) {
                return type_decl_size_info(td, idx);
            }
            // Locus-typed names are handled by the caller
            // (which recurses through estimate_locus). Other
            // unknown names: conservative pointer-sized
            // placeholder, flagged unbounded.
            TypeSizeInfo { size: 16, align: 8, unbounded: true }
        }
        TypeExpr::Projection { inner, .. } => type_size_info(inner, idx),
        TypeExpr::Function { .. } => {
            TypeSizeInfo { size: 8, align: 8, unbounded: false }
        }
    }
}

fn type_decl_size_info(td: &TypeDecl, idx: &Index<'_>) -> TypeSizeInfo {
    match &td.body {
        TypeDeclBody::Alias(inner) => type_size_info(inner, idx),
        TypeDeclBody::Struct(fields) => {
            // Walk declaration order, accumulating with
            // alignment padding. Final size is rounded up to
            // the struct's own alignment so back-to-back
            // arrays of this struct also pad correctly.
            let mut size: u64 = 0;
            let mut align: u64 = 1;
            let mut unbounded = false;
            for f in fields {
                let info = type_size_info(&f.ty, idx);
                size = round_up(size, info.align);
                size = size.saturating_add(info.size);
                if info.align > align {
                    align = info.align;
                }
                unbounded |= info.unbounded;
            }
            size = round_up(size, align);
            TypeSizeInfo { size, align, unbounded }
        }
        TypeDeclBody::Enum(variants) => {
            // Largest payload (with internal padding) + 8-byte
            // tag (also padded to its alignment).
            let mut max_payload_size: u64 = 0;
            let mut max_payload_align: u64 = 1;
            let mut unbounded = false;
            for v in variants {
                let mut vsize: u64 = 0;
                let mut valign: u64 = 1;
                for ty in &v.fields {
                    let info = type_size_info(ty, idx);
                    vsize = round_up(vsize, info.align);
                    vsize = vsize.saturating_add(info.size);
                    if info.align > valign {
                        valign = info.align;
                    }
                    unbounded |= info.unbounded;
                }
                vsize = round_up(vsize, valign);
                if vsize > max_payload_size {
                    max_payload_size = vsize;
                }
                if valign > max_payload_align {
                    max_payload_align = valign;
                }
            }
            // Tag = i64 (8B, align 8). Effective enum size:
            // round_up(tag, max_payload_align) + payload, then
            // round_up to enum align (max of tag-align and
            // payload-align).
            let align = max_payload_align.max(8);
            let tag_size = round_up(8, max_payload_align.max(1));
            let size = round_up(
                tag_size.saturating_add(max_payload_size),
                align,
            );
            TypeSizeInfo { size, align, unbounded }
        }
    }
}

fn primitive_size_info(p: PrimType) -> TypeSizeInfo {
    let (size, align) = match p {
        PrimType::Int
        | PrimType::Uint
        | PrimType::Float
        | PrimType::Time
        | PrimType::Duration => (8, 8),
        PrimType::Decimal => (16, 16),
        PrimType::Bool => (1, 1),
        // Heap-managed sequences. Approximated as pointer + len
        // = 16 bytes resident on the struct; the heap buffer
        // itself is not counted (lives in the locus's arena,
        // already covered by the per-method scratch heuristic
        // we elided in v0.1). Alignment = 8 (pointer).
        PrimType::String
        | PrimType::Bytes
        | PrimType::StringView
        | PrimType::BytesView
        | PrimType::BytesMut => (16, 8),
    };
    TypeSizeInfo { size, align, unbounded: false }
}

fn round_up(n: u64, align: u64) -> u64 {
    if align <= 1 {
        return n;
    }
    let r = n % align;
    if r == 0 { n } else { n.saturating_add(align - r) }
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
/// FUv0.8.2 #3 (2026-05-25): walk a method body summing the
/// transient bytes a single execution would put in scratch.
/// Each Expr-shape that allocates in the current arena adds
/// its byte size. Sums conservatively (worst-case
/// simultaneously live) rather than tracking lifetimes — the
/// estimator's purpose is "does this fit in cache" and a
/// loose upper bound is the right error direction.
fn block_scratch_bytes(b: &hale_syntax::ast::Block, idx: &Index<'_>) -> u64 {
    let mut total: u64 = 0;
    for s in &b.stmts {
        total = total.saturating_add(stmt_scratch_bytes(s, idx));
    }
    total
}

fn stmt_scratch_bytes(s: &hale_syntax::ast::Stmt, idx: &Index<'_>) -> u64 {
    use hale_syntax::ast::Stmt;
    match s {
        Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } => {
            expr_scratch_bytes(value, idx)
        }
        Stmt::Assign { value, .. } => expr_scratch_bytes(value, idx),
        Stmt::If(s) => {
            let mut t = expr_scratch_bytes(&s.cond, idx);
            t = t.saturating_add(block_scratch_bytes(&s.then_block, idx));
            t = t.saturating_add(else_branch_scratch_bytes(
                s.else_block.as_deref(),
                idx,
            ));
            t
        }
        Stmt::Match(m) => {
            let mut t = expr_scratch_bytes(&m.scrutinee, idx);
            for arm in &m.arms {
                if let Some(g) = &arm.guard {
                    t = t.saturating_add(expr_scratch_bytes(g, idx));
                }
                t = t.saturating_add(match_arm_body_scratch_bytes(
                    &arm.body, idx,
                ));
            }
            t
        }
        Stmt::For { iter, body, .. } => {
            // Inner allocations inside a loop are reset every
            // iteration in scratch terms — but the loop's
            // simultaneous footprint may include 1× the
            // per-iteration bytes plus the iter expr's
            // bytes. Conservative: add both.
            expr_scratch_bytes(iter, idx)
                .saturating_add(block_scratch_bytes(body, idx))
        }
        Stmt::While { cond, body, .. } => expr_scratch_bytes(cond, idx)
            .saturating_add(block_scratch_bytes(body, idx)),
        Stmt::Return(Some(e), _) => expr_scratch_bytes(e, idx),
        Stmt::Fail { value, .. } => expr_scratch_bytes(value, idx),
        Stmt::Block(b) => block_scratch_bytes(b, idx),
        Stmt::ShmWrite { max, body, .. } => {
            expr_scratch_bytes(max, idx).saturating_add(block_scratch_bytes(body, idx))
        }
        Stmt::Send { subject, value, .. } => {
            expr_scratch_bytes(subject, idx)
                .saturating_add(expr_scratch_bytes(value, idx))
        }
        Stmt::Expr(e) => expr_scratch_bytes(e, idx),
        Stmt::Recovery { args, .. } => {
            let mut t: u64 = 0;
            for a in args {
                t = t.saturating_add(expr_scratch_bytes(a, idx));
            }
            t
        }
        Stmt::Violate { payload, .. } => match payload {
            Some(p) => expr_scratch_bytes(p, idx),
            None => 0,
        },
        Stmt::Return(None, _)
        | Stmt::Break(_)
        | Stmt::Continue(_)
        | Stmt::Yield(_) | Stmt::Terminate(_) => 0,
    }
}

fn else_branch_scratch_bytes(
    branch: Option<&hale_syntax::ast::ElseBranch>,
    idx: &Index<'_>,
) -> u64 {
    use hale_syntax::ast::ElseBranch;
    match branch {
        None => 0,
        Some(ElseBranch::Else(b)) => block_scratch_bytes(b, idx),
        Some(ElseBranch::ElseIf(inner)) => stmt_scratch_bytes(
            &hale_syntax::ast::Stmt::If(inner.clone()),
            idx,
        ),
    }
}

fn match_arm_body_scratch_bytes(
    body: &hale_syntax::ast::MatchArmBody,
    idx: &Index<'_>,
) -> u64 {
    use hale_syntax::ast::MatchArmBody;
    match body {
        MatchArmBody::Expr(e) => expr_scratch_bytes(e, idx),
        MatchArmBody::Block(b) => block_scratch_bytes(b, idx),
    }
}

/// Heuristic per-concat cost for binary `+` where either operand
/// might be heap-typed. Without typecheck info available here
/// we can't know exact operand types; a 32-byte budget per
/// concat is conservatively low (covers a ~16-char result + 16
/// bytes of header).
const STRING_CONCAT_BYTES: u64 = 32;

fn expr_scratch_bytes(e: &Expr, idx: &Index<'_>) -> u64 {
    use hale_syntax::ast::BinOp;
    match e {
        // Struct literal: allocates a fresh value of the named
        // type in scratch. Children's allocations also count
        // (they evaluate in the same scope).
        Expr::Struct { path, inits, .. } => {
            let base = path
                .segments
                .first()
                .and_then(|s| idx.types.get(s.name.as_str()))
                .map(|td| type_decl_size_info(td, idx).size)
                .unwrap_or(0);
            let mut t = base;
            for i in inits {
                t = t.saturating_add(expr_scratch_bytes(&i.value, idx));
            }
            t
        }
        // Tuple / Array literals: same shape, but anonymous —
        // size is the sum of part sizes (the tuple's struct).
        // For Array, the elements' sizes contribute via the
        // recursive expr walk below; the array itself is
        // sized to elem_size × n.
        Expr::Tuple(parts, _) => {
            let mut t: u64 = 0;
            for p in parts {
                t = t.saturating_add(expr_scratch_bytes(p, idx));
                // Estimate part's storage in the tuple: walk
                // through if it's a struct/tuple/array, else
                // primitive-sized.
                t = t.saturating_add(literal_storage_size(p, idx));
            }
            t
        }
        Expr::Array(parts, _) => {
            let mut t: u64 = 0;
            for p in parts {
                t = t.saturating_add(expr_scratch_bytes(p, idx));
                t = t.saturating_add(literal_storage_size(p, idx));
            }
            t
        }
        Expr::ArrayRepeat { val, count, .. } => {
            let elem = literal_storage_size(val, idx);
            elem.saturating_mul(*count)
                .saturating_add(expr_scratch_bytes(val, idx))
        }
        // Binary + on possibly-heap operands → likely string
        // concat. Without typecheck info we can't know for
        // sure; charge the small heuristic per Binary+ and
        // recurse.
        Expr::Binary { op, left, right, .. } => {
            let mut t = expr_scratch_bytes(left, idx)
                .saturating_add(expr_scratch_bytes(right, idx));
            if matches!(op, BinOp::Add) {
                t = t.saturating_add(STRING_CONCAT_BYTES);
            }
            t
        }
        Expr::Unary { operand, .. } => expr_scratch_bytes(operand, idx),
        Expr::Field { receiver, .. } => expr_scratch_bytes(receiver, idx),
        Expr::Index { receiver, index, .. } => {
            expr_scratch_bytes(receiver, idx)
                .saturating_add(expr_scratch_bytes(index, idx))
        }
        Expr::Path2 { receiver, .. } => expr_scratch_bytes(receiver, idx),
        Expr::Call { callee, args, .. } => {
            let mut t = expr_scratch_bytes(callee, idx);
            for a in args {
                t = t.saturating_add(expr_scratch_bytes(a, idx));
            }
            t
        }
        Expr::Block(b) => block_scratch_bytes(b, idx),
        Expr::If(s) => {
            let mut t = expr_scratch_bytes(&s.cond, idx);
            t = t.saturating_add(block_scratch_bytes(&s.then_block, idx));
            t = t.saturating_add(else_branch_scratch_bytes(
                s.else_block.as_deref(),
                idx,
            ));
            t
        }
        Expr::Match(m) => {
            let mut t = expr_scratch_bytes(&m.scrutinee, idx);
            for arm in &m.arms {
                if let Some(g) = &arm.guard {
                    t = t.saturating_add(expr_scratch_bytes(g, idx));
                }
                t = t.saturating_add(match_arm_body_scratch_bytes(
                    &arm.body, idx,
                ));
            }
            t
        }
        Expr::Or { inner, disposition, .. } => {
            use hale_syntax::ast::OrDisposition;
            let mut t = expr_scratch_bytes(inner, idx);
            match disposition {
                OrDisposition::Substitute(rhs) => {
                    t = t.saturating_add(expr_scratch_bytes(rhs, idx))
                }
                OrDisposition::Fail(payload, _) => {
                    t = t.saturating_add(expr_scratch_bytes(payload, idx))
                }
                OrDisposition::Raise(_) | OrDisposition::Discard(_) => {}
            }
            t
        }
        Expr::Sum(inner, _) | Expr::Prod(inner, _) => {
            expr_scratch_bytes(inner, idx)
        }
        Expr::Approx { left, right, tolerance, .. } => {
            expr_scratch_bytes(left, idx)
                .saturating_add(expr_scratch_bytes(right, idx))
                .saturating_add(expr_scratch_bytes(tolerance, idx))
        }
        Expr::Range { lo, hi, .. } => expr_scratch_bytes(lo, idx)
            .saturating_add(expr_scratch_bytes(hi, idx)),
        _ => 0,
    }
}

/// Approximate the byte footprint a literal expression
/// occupies as a value (vs. recursively in scratch). Used by
/// Tuple / Array / ArrayRepeat to size the container itself.
fn literal_storage_size(e: &Expr, idx: &Index<'_>) -> u64 {
    use hale_syntax::ast::{Literal, PrimType};
    match e {
        Expr::Literal(lit, _) => match lit {
            Literal::Int(_)
            | Literal::Float(_)
            | Literal::Duration(_) => 8,
            Literal::Decimal(_) => 16,
            Literal::Bool(_) => 1,
            Literal::Nil => 8,
            Literal::String(s) => {
                // ptr + len header (16 B) + payload bytes,
                // capped at 64 for sanity.
                16u64.saturating_add((s.len() as u64).min(64))
            }
            Literal::Time(_) => 8,
            Literal::Bytes(b) => 16u64.saturating_add(b.len() as u64),
        },
        Expr::Struct { path, .. } => path
            .segments
            .first()
            .and_then(|s| idx.types.get(s.name.as_str()))
            .map(|td| type_decl_size_info(td, idx).size)
            .unwrap_or(0),
        // Fallback for non-literal sub-expressions inside a
        // literal container: assume pointer-sized.
        _ => {
            let _ = primitive_size_info(PrimType::Int);
            8
        }
    }
}

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
        // 4 Ints × 8B (align 8) + 1 Bool × 1B (align 1) + 1
        // Decimal × 16B (align 16). With v0.2 alignment-aware
        // layout, offsets advance:
        //   Int   0 .. 8    (round_up 0,8 = 0)
        //   Int   8 .. 16
        //   Int  16 .. 24
        //   Int  24 .. 32
        //   Bool 32 .. 33   (round_up 32,1 = 32)
        //   Pad  33 .. 48   (round_up 33,16 = 48 — Decimal
        //                    alignment forces 15B pad)
        //   Decimal 48 .. 64
        //   Final round_up(64, 16) = 64
        // User section = 64 bytes. Struct = ARENA_OVERHEAD + 64.
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
        assert_eq!(est.struct_bytes, ARENA_OVERHEAD + 64);
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
        // Quote interior with alignment:
        //   bid     Decimal 16, align 16 → 0  .. 16
        //   ask     Decimal 16, align 16 → 16 .. 32
        //   venue   Int      8, align  8 → 32 .. 40
        //   final round_up(40, max_align=16) = 48
        // Struct align = 16 ⇒ Quote size = 48 B, align 16.
        // Outer L: user_offset 0 + 48 = 48, user_align 16,
        // round_up(48, 16) = 48. struct = ARENA_OVERHEAD + 48.
        let est = estimate(
            r#"
                type Quote { bid: Decimal; ask: Decimal; venue: Int; }
                locus L { params { latest: Quote = Quote { bid: 0.0, ask: 0.0, venue: 0 }; } }
                fn main() { L { }; }
            "#,
            "L",
        );
        assert_eq!(est.struct_bytes, ARENA_OVERHEAD + 48);
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
    fn fallback_cache_tier_constants() {
        // The runtime tiers may come from sysfs on Linux build
        // hosts (the f32-2-v02 sysfs detect path), so these
        // values can differ across machines. Pin the fallback
        // constants instead — those are what builds on non-
        // Linux hosts / containers without /sys see.
        assert_eq!(FALLBACK_L1_BYTES, 32 * 1024);
        assert_eq!(FALLBACK_L2_BYTES, 512 * 1024);
        assert_eq!(FALLBACK_L3_BYTES, 8 * 1024 * 1024);
    }

    #[test]
    fn budget_bytes_at_least_fallback() {
        // Sysfs values should never be SMALLER than the
        // fallback (any modern x86_64 / aarch64 box has at
        // least 32K L1 / 512K L2 / 8M L3). Lock the lower
        // bound so a future sysfs parser regression that
        // silently halves the budget gets caught.
        assert!(CacheTier::L1.budget_bytes() >= FALLBACK_L1_BYTES);
        assert!(CacheTier::L2.budget_bytes() >= FALLBACK_L2_BYTES);
        assert!(CacheTier::L3.budget_bytes() >= FALLBACK_L3_BYTES);
    }

    #[test]
    fn parse_sysfs_cache_size_units() {
        assert_eq!(parse_sysfs_cache_size("32K"), Some(32 * 1024));
        assert_eq!(parse_sysfs_cache_size("512K"), Some(512 * 1024));
        assert_eq!(parse_sysfs_cache_size("8M"), Some(8 * 1024 * 1024));
        assert_eq!(
            parse_sysfs_cache_size("128M"),
            Some(128 * 1024 * 1024)
        );
        assert_eq!(
            parse_sysfs_cache_size("1G"),
            Some(1024 * 1024 * 1024)
        );
        // Bare integer = bytes.
        assert_eq!(parse_sysfs_cache_size("4096"), Some(4096));
        // Whitespace already stripped by `read_cache_size`'s
        // .trim(); these are the raw-parser cases.
        assert_eq!(parse_sysfs_cache_size(""), None);
        assert_eq!(parse_sysfs_cache_size("32X"), None);
        assert_eq!(parse_sysfs_cache_size("notanumber"), None);
    }

    #[test]
    fn nearest_tier_picks_smallest_fitting() {
        // Use FALLBACK_* directly so the test is environment-
        // independent (sysfs detect can shift the runtime tier
        // sizes).
        let l1 = FALLBACK_L1_BYTES;
        let l2 = FALLBACK_L2_BYTES;
        let l3 = FALLBACK_L3_BYTES;
        // The actual tiers are at LEAST these values; use the
        // runtime budgets for the equality boundary cases.
        let l1_runtime = CacheTier::L1.budget_bytes();
        let l2_runtime = CacheTier::L2.budget_bytes();
        let l3_runtime = CacheTier::L3.budget_bytes();
        assert_eq!(nearest_tier(0), Some(CacheTier::L1));
        assert_eq!(nearest_tier(l1_runtime), Some(CacheTier::L1));
        assert_eq!(nearest_tier(l1_runtime + 1), Some(CacheTier::L2));
        assert_eq!(nearest_tier(l2_runtime), Some(CacheTier::L2));
        assert_eq!(nearest_tier(l3_runtime), Some(CacheTier::L3));
        assert_eq!(nearest_tier(l3_runtime + 1), None);
        // Touch the fallback constants so a future regression
        // that drops them from the source surfaces here.
        let _ = (l1, l2, l3);
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

    // === FUv0.8.2 #3 method-scratch tests ===============

    #[test]
    fn empty_methods_have_zero_scratch() {
        let est = estimate(
            r#"
                locus L {
                    params { x: Int = 0; }
                    run() { let _ = self.x; }
                }
                fn main() { L { }; }
            "#,
            "L",
        );
        assert_eq!(est.scratch_bytes, 0, "got: {:?}", est);
    }

    #[test]
    fn struct_literal_in_method_charges_scratch() {
        // Foo = 8B (Int). Constructing `Foo { v: 1 }` inside
        // a method body charges the type's storage size to
        // scratch_bytes.
        let est = estimate(
            r#"
                type Foo { v: Int; }
                locus L {
                    fn make() -> Int {
                        let f = Foo { v: 1 };
                        return f.v;
                    }
                }
                fn main() { L { }; }
            "#,
            "L",
        );
        assert!(
            est.scratch_bytes >= 8,
            "expected at least 8 B for Foo literal; got: {:?}",
            est
        );
    }

    #[test]
    fn high_water_is_max_across_methods() {
        // Method `small` builds an 8B struct; method `big`
        // builds a 24B struct. The locus's scratch_bytes is
        // the max (24B), not the sum.
        let est = estimate(
            r#"
                type Small { a: Int; }
                type Big   { a: Int; b: Int; c: Int; }
                locus L {
                    fn small() -> Int {
                        let s = Small { a: 1 };
                        return s.a;
                    }
                    fn big() -> Int {
                        let b = Big { a: 1, b: 2, c: 3 };
                        return b.a;
                    }
                }
                fn main() { L { }; }
            "#,
            "L",
        );
        // Small contributes 8; Big contributes 24. Max is 24
        // (plus the recursive Int literals' scratch which is
        // 0). Assert lower bound.
        assert!(
            est.scratch_bytes >= 24,
            "expected high-water ≥ 24 B; got: {:?}",
            est
        );
        // And < small + big (i.e., we took the max, not sum).
        assert!(
            est.scratch_bytes < 8 + 24 + 32,
            "expected max-across-methods not sum; got: {:?}",
            est
        );
    }

    #[test]
    fn total_includes_scratch() {
        let est = estimate(
            r#"
                type Big { a: Int; b: Int; c: Int; }
                locus L {
                    fn make() -> Int {
                        let b = Big { a: 1, b: 2, c: 3 };
                        return b.a;
                    }
                }
                fn main() { L { }; }
            "#,
            "L",
        );
        assert_eq!(
            est.total(),
            est.struct_bytes
                + est.capacity_bytes
                + est.child_bytes
                + est.scratch_bytes,
            "total should sum all four fields",
        );
        assert!(est.scratch_bytes > 0);
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

    // === F.32-2 v0.2 strict-gate tests ====================

    #[test]
    fn parse_cache_tier_recognizes_canonical_labels() {
        assert_eq!(parse_cache_tier("l1"), Some(CacheTier::L1));
        assert_eq!(parse_cache_tier("l2"), Some(CacheTier::L2));
        assert_eq!(parse_cache_tier("l3"), Some(CacheTier::L3));
        // Case-insensitive — operator likely types "L1".
        assert_eq!(parse_cache_tier("L1"), Some(CacheTier::L1));
        assert_eq!(parse_cache_tier("L2"), Some(CacheTier::L2));
        assert_eq!(parse_cache_tier("L3"), Some(CacheTier::L3));
        assert_eq!(parse_cache_tier("l4"), None);
        assert_eq!(parse_cache_tier(""), None);
        assert_eq!(parse_cache_tier("32k"), None);
    }

    #[test]
    fn breaches_against_tier_picks_over_budget_loci() {
        let src = r#"
            type Entry { k: Int; v: Int; }
            @form(hashmap, sync = lockfree, cap = 4096)
            locus Big {
                capacity { pool entries of Entry indexed_by k; }
            }
            locus Tiny { params { x: Int = 0; } }
            fn main() { Big { }; Tiny { }; }
        "#;
        let p = parse_source(src).expect("parse");
        let map = compute_program_working_set(&p.items);
        // Big = 64 arena + 4096 × 16 cell = 65600 B → exceeds
        // L1 (32K) but fits L2 (512K).
        let l1 = breaches_against_tier(&map, CacheTier::L1);
        assert_eq!(l1.len(), 1, "got: {:?}", l1);
        assert_eq!(l1[0].locus_name, "Big");
        assert!(l1[0].total_bytes > l1[0].budget_bytes);
        assert_eq!(l1[0].excess_bytes, l1[0].total_bytes - l1[0].budget_bytes);
        // Tiny fits all tiers — no breach.
        let l2 = breaches_against_tier(&map, CacheTier::L2);
        assert!(l2.is_empty(), "got: {:?}", l2);
        let l3 = breaches_against_tier(&map, CacheTier::L3);
        assert!(l3.is_empty(), "got: {:?}", l3);
    }

    #[test]
    fn render_breach_diagnostic_uses_severity_label() {
        let breach = BudgetBreach {
            locus_name: "Big".to_string(),
            total_bytes: 100_000,
            budget_bytes: 32 * 1024,
            excess_bytes: 100_000 - 32 * 1024,
            tier: CacheTier::L1,
            source: BudgetSource::GlobalTargetCache,
        };
        let err =
            render_breach_diagnostic(&[breach.clone()], "error");
        assert!(err.starts_with("error:"), "got: {}", err);
        assert!(err.contains("L1"), "got: {}", err);
        assert!(err.contains("Big"), "got: {}", err);
        assert!(err.contains("--target-cache"), "got: {}", err);
        let warn = render_breach_diagnostic(&[breach], "warning");
        assert!(warn.starts_with("warning:"), "got: {}", warn);
    }

    #[test]
    fn render_breach_diagnostic_empty_returns_empty_string() {
        let out = render_breach_diagnostic(&[], "error");
        assert!(out.is_empty(), "got: {}", out);
    }

    #[test]
    fn locality_annotation_overrides_global_target() {
        // Two loci: BigL1 has `@locality(L1)`; BigL2 has no
        // annotation. With `--target-cache l2` global:
        //   - BigL1 evaluated against L1 (its annotation wins),
        //     breach if it exceeds 32K.
        //   - BigL2 evaluated against L2 (global tier).
        let src = r#"
            type Entry { k: Int; v: Int; }
            @form(hashmap, sync = lockfree, cap = 4096)
            @locality(L1)
            locus BigL1 {
                capacity { pool entries of Entry indexed_by k; }
            }
            @form(hashmap, sync = lockfree, cap = 4096)
            locus BigL2 {
                capacity { pool entries of Entry indexed_by k; }
            }
            fn main() { BigL1 { }; BigL2 { }; }
        "#;
        let p = parse_source(src).expect("parse");
        let map = compute_program_working_set(&p.items);
        // 4096 × 16 + 64 = 65600 B per locus.
        // Global L2 (524288) — neither exceeds. BigL1's L1
        // annotation (32768) is exceeded.
        let breaches = breaches_with_per_locus_budgets(
            &map,
            &p.items,
            Some(CacheTier::L2),
        );
        let names: Vec<&str> = breaches
            .iter()
            .map(|b| b.locus_name.as_str())
            .collect();
        assert_eq!(names, vec!["BigL1"], "got: {:?}", breaches);
        assert_eq!(breaches[0].tier, CacheTier::L1);
        assert_eq!(breaches[0].source, BudgetSource::LocalityAnnotation);
    }

    #[test]
    fn locality_any_opts_out_of_global_gate() {
        // Locus has `@locality(any)`. Global --target-cache=L1
        // would normally flag it; the explicit opt-out
        // exempts it.
        let src = r#"
            type Entry { k: Int; v: Int; }
            @form(hashmap, sync = lockfree, cap = 4096)
            @locality(any)
            locus Big {
                capacity { pool entries of Entry indexed_by k; }
            }
            fn main() { Big { }; }
        "#;
        let p = parse_source(src).expect("parse");
        let map = compute_program_working_set(&p.items);
        let breaches = breaches_with_per_locus_budgets(
            &map,
            &p.items,
            Some(CacheTier::L1),
        );
        assert!(breaches.is_empty(), "got: {:?}", breaches);
    }

    #[test]
    fn locality_annotation_checked_without_global_target() {
        // Without --target-cache, loci without annotations
        // get no budget. Loci with @locality(Lx) are still
        // checked — the annotation is a hard contract.
        let src = r#"
            type Entry { k: Int; v: Int; }
            @form(hashmap, sync = lockfree, cap = 4096)
            @locality(L1)
            locus Big {
                capacity { pool entries of Entry indexed_by k; }
            }
            locus Tiny { params { x: Int = 0; } }
            fn main() { Big { }; Tiny { }; }
        "#;
        let p = parse_source(src).expect("parse");
        let map = compute_program_working_set(&p.items);
        let breaches = breaches_with_per_locus_budgets(
            &map,
            &p.items,
            None,
        );
        let names: Vec<&str> = breaches
            .iter()
            .map(|b| b.locus_name.as_str())
            .collect();
        assert_eq!(names, vec!["Big"], "got: {:?}", breaches);
        assert_eq!(breaches[0].source, BudgetSource::LocalityAnnotation);
    }

    #[test]
    fn effective_budget_resolution() {
        // Pin the precedence directly: locality wins; any
        // opts out; no annotation falls through to global.
        let parse_locus = |src: &str| {
            let p = parse_source(src).expect("parse");
            for item in p.items {
                if let TopDecl::Locus(l) = item {
                    return l;
                }
            }
            panic!("no locus");
        };
        let l1 = parse_locus("@locality(L1) locus X { }");
        let l2 = parse_locus("@locality(L2) locus X { }");
        let any = parse_locus("@locality(any) locus X { }");
        let bare = parse_locus("locus X { }");

        assert_eq!(
            effective_locus_budget(&l1, Some(CacheTier::L3)),
            Some((CacheTier::L1, BudgetSource::LocalityAnnotation)),
        );
        assert_eq!(
            effective_locus_budget(&l2, None),
            Some((CacheTier::L2, BudgetSource::LocalityAnnotation)),
        );
        assert_eq!(
            effective_locus_budget(&any, Some(CacheTier::L1)),
            None,
            "any opts out even with global target",
        );
        assert_eq!(
            effective_locus_budget(&bare, Some(CacheTier::L2)),
            Some((CacheTier::L2, BudgetSource::GlobalTargetCache)),
        );
        assert_eq!(effective_locus_budget(&bare, None), None);
    }
}
