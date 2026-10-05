//! A standing check (F.40 phase 4): the IR of three corpus programs stays
//! inside a band.
//!
//! Phase 3's build regression on `dna/host` (30 s to 53 s, then the
//! reclaim's guards emitted at every site: 1.85 M lines of IR to 2.13 M)
//! was visible in the size of the emitted IR, a number that does not
//! depend on the machine, days before anyone timed a build. This test
//! reads that number for three programs chosen to span the lowering:
//!
//! - `73-bounded-bus`: bus-heavy. A bounded topic with `on_full: fail`,
//!   an `or wait` publisher on a pinned child, a `drop_old` subscriber,
//!   and the main locus's children dissolving with dissolve bodies.
//! - `89-nested-locus-teardown`: lifecycle and children. A three-deep
//!   tree of field children (a `@form(vec)` leaf with `birth`, `drain`
//!   and `dissolve` bodies), torn down from an unowned literal, a method
//!   receiver and a `let`.
//! - `69-http-router`: stdlib-heavy. `std::http::Router`, its routes,
//!   middleware and dispatch: the stdlib's loci and functions called
//!   from user code, with no socket.
//!
//! For each: the pre-optimization IR's line count, its count of
//! `define`s and its count of `call` instructions, each inside ±10% of
//! the value measured beside it. Most of any program's IR is the floor
//! every program carries (the runtime's entry points and the stdlib's
//! loci: an empty `main` is 93-96% of each of the three), so a program's
//! own lowering could double inside a band on the total; each number is
//! therefore banded twice, as a total and as the program's share above
//! the floor, and the floor has a band of its own. `dna/host` has the
//! same three numbers checked where its one build happens:
//! `scripts/warm-dna-cache.sh` (the CI step that builds the host into the
//! DNA toolchain cache before the DNA suite and the CLI tests run).
//!
//! A number outside its band fails with the measured value and the band.
//! Moving a band on purpose is an edit of its constant here, with the
//! reason (and the new measurement) in the commit.

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// The pre-optimization IR's size: lines, `define`s, `call`s.
#[derive(Debug, Clone, Copy, PartialEq)]
struct IrSize {
    lines: usize,
    defines: usize,
    calls: usize,
}

impl IrSize {
    fn of(ir: &str) -> IrSize {
        IrSize {
            lines: ir.lines().count(),
            defines: ir.lines().filter(|l| l.starts_with("define ")).count(),
            calls: ir.lines().filter(|l| is_call(l)).count(),
        }
    }

    /// The part of `self` above `floor`.
    fn above(self, floor: IrSize) -> IrSize {
        IrSize {
            lines: self.lines.saturating_sub(floor.lines),
            defines: self.defines.saturating_sub(floor.defines),
            calls: self.calls.saturating_sub(floor.calls),
        }
    }

    fn numbers(self) -> [(&'static str, usize); 3] {
        [("lines", self.lines), ("defines", self.defines), ("calls", self.calls)]
    }
}

/// A `call` instruction: `call ...`, `tail call ...`, `%x = call ...`
/// (the warm script's `grep` counts the host's by the same rule).
fn is_call(line: &str) -> bool {
    let ins = line.trim_start();
    let ins = match ins.split_once(" = ") {
        Some((dest, rhs)) if dest.starts_with('%') => rhs,
        _ => ins,
    };
    let ins = ins.trim_start_matches("tail ").trim_start_matches("musttail ").trim_start_matches("notail ");
    ins.starts_with("call ")
}

/// The relative half-width of every band.
const BAND: f64 = 0.10;

/// The floor: a program with nothing in it.
const EMPTY: &str = "fn main() { }\n";

// Measured 2026-10-05 on F.40 phase 4's main. Each program's share above
// the floor, the second band: 73-bounded-bus 3473 lines, 41 defines,
// 602 calls; 89-nested-locus-teardown 4716, 50, 478; 69-http-router
// 2948, 30, 278.
const FLOOR: IrSize = IrSize { lines: 64421, defines: 709, calls: 7111 };
const BOUNDED_BUS: IrSize = IrSize { lines: 67894, defines: 750, calls: 7713 };
const NESTED_TEARDOWN: IrSize = IrSize { lines: 69137, defines: 759, calls: 7589 };
const HTTP_ROUTER: IrSize = IrSize { lines: 67369, defines: 739, calls: 7389 };

fn ir_size_of(tag: &str, src: &str) -> IrSize {
    let bin = harness::unique_bin(&format!("ir_size_band_{tag}"));
    let ir = harness::build_source_ir_text(src, &bin).unwrap_or_else(|e| panic!("{tag}: build: {e:?}"));
    let _ = std::fs::remove_file(&bin);
    IrSize::of(&ir)
}

/// `want` ±[`BAND`], rounded outward.
fn band(want: usize) -> (usize, usize) {
    let w = want as f64;
    ((w * (1.0 - BAND)).floor() as usize, (w * (1.0 + BAND)).ceil() as usize)
}

/// Each number of `got` outside `want`'s band, said.
fn out_of_band(what: &str, got: IrSize, want: IrSize) -> Vec<String> {
    let mut out = Vec::new();
    for ((name, measured), (_, pinned)) in got.numbers().into_iter().zip(want.numbers()) {
        let (lo, hi) = band(pinned);
        if measured < lo || measured > hi {
            out.push(format!(
                "{what} {name}: measured {measured}, pinned {pinned}, band {lo}..={hi} ({:+.1}%)",
                (measured as f64 / pinned.max(1) as f64 - 1.0) * 100.0
            ));
        }
    }
    out
}

fn fail_unless_empty(program: &str, out: Vec<String>, measured: String) {
    assert!(
        out.is_empty(),
        "the IR of `{program}` left its band:\n  {}\n{measured}\n\
         Phase 3's regressions on dna/host showed first in this number. If the change is meant to move it, \
         edit the constant in crates/hale-codegen/tests/ir_size_band.rs to the measured value, with the reason \
         in the commit.",
        out.join("\n  ")
    )
}

fn assert_in_band(example: &str, pinned: IrSize) {
    let path = format!("{}/tests/fixtures/examples/{example}/main.hl", env!("CARGO_MANIFEST_DIR"));
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let got = ir_size_of(example, &src);
    let floor = ir_size_of("floor", EMPTY);
    let mut out = out_of_band("total", got, pinned);
    out.extend(out_of_band("above the floor", got.above(floor), pinned.above(FLOOR)));
    fail_unless_empty(example, out, format!("measured {got:?}, floor {floor:?}"));
}

#[test]
fn the_floor_stays_in_its_band() {
    let got = ir_size_of("floor", EMPTY);
    fail_unless_empty("fn main() { }", out_of_band("total", got, FLOOR), format!("measured {got:?}"));
}

#[test]
fn a_bus_heavy_programs_ir_stays_in_its_band() {
    assert_in_band("73-bounded-bus", BOUNDED_BUS);
}

#[test]
fn a_lifecycle_programs_ir_stays_in_its_band() {
    assert_in_band("89-nested-locus-teardown", NESTED_TEARDOWN);
}

#[test]
fn a_stdlib_heavy_programs_ir_stays_in_its_band() {
    assert_in_band("69-http-router", HTTP_ROUTER);
}
