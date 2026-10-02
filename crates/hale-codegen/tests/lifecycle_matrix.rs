//! The lifecycle matrix (F.40 phase 3, L3): failure phase × tree
//! position × domain, every cell a generated program held to the
//! lifecycle's obligations.
//!
//! ## Why a generator
//!
//! The lifecycle fixtures (`lifecycle_fixtures.rs`, L1 and L2) hold one
//! program per decision line, each written for the shape its line was
//! about. A failure's delivery, the hold that keeps a failed child and
//! its payload alive until the handler completes, and the teardown that
//! follows are the same protocol wherever the child sits and whatever
//! thread runs it, so a fixture per line leaves the combinations nobody
//! wrote untested. This file is the ownership matrix's shape
//! (`ownership_matrix.rs`) over the lifecycle: three axes and a
//! renderer.
//!
//!   * **phase**: where the subject `Subj` fails. `params_settle`, a
//!     birth-epoch closure, which fails while the owner's params are
//!     open and is held to settle (decision line 1); `birth`, a
//!     `birth_check` (line 8); `run`, a `violate` in `run()` (line 9);
//!     `handler`, a `violate` in the subject's bus handler, on the cell
//!     its own `run()` publishes (inventory C39 and C40); `drain`, a
//!     `violate` in `drain()` (line 4's route in every spine); and
//!     `none`. (`bubble` from an `on_failure` is not a phase: it is
//!     lowered as the report and the structural exit, never as a
//!     delivery to the grandparent, where `spec/semantics.md` has it
//!     climb the tree until a handler absorbs it. Inventory row C47
//!     records the difference, corrected by L4's failure-delivery
//!     spine; until then the `handler` phase raises with a bus
//!     handler's `violate`, not a bubble.)
//!   * **position**: where the subject sits in its owner's tree. A
//!     field of the root (`root_child`); a field of a field
//!     (`grandchild`, whose owner is the middle locus); a statement
//!     literal its owner `accept`s (`accepted_child`); a field typed by
//!     an interface (`iface_field`) or a perspective (`persp_slot`); a
//!     `pinned(replicas = 2)` field (`replica`); a literal in the
//!     owner's `on_failure` (`handler_born`); a `let`-bound literal in
//!     the owner's `run()` (`let_literal`).
//!   * **domain**: where the subject and its owner run. `main`; `pool`,
//!     the owner built by a locus placed `cooperative(pool = side)`, so
//!     owner and subject share the worker; `pinned`, the subject placed
//!     `pinned` (a field position) or its owner built on a pinned
//!     thread (the others); `cross_pool`, the subject placed on pool
//!     `side` and its owner on main.
//!
//! Each cell's program names the obligations it exercises in its
//! header, as the fixtures do, with its expected outcome word; its
//! expected trace plan is derived from L1's plan for the lines it
//! touches ([`plan_for`]).
//!
//! ## Cells that are not programs
//!
//! Some combinations have no program. A cell the front end refuses as
//! a construction is [`Form::Unwritable`] with the rule that refuses
//! it: the generator renders the nearest spelling, the parser or the
//! checker refuses it, and [`UNWRITABLE`] lists every such cell, so a
//! rule that starts accepting one (or a new refusal) shows. A cell
//! whose phase names a path its position does not have is
//! [`Form::NoPath`]: a literal written after its owner settled cannot
//! fail while that owner's params are open. Neither is skipped
//! silently: both are counted, and the guard test holds the tables to
//! what the generator and the checker say.
//!
//! ## The oracles
//!
//! Each selected cell's program is written beside its binary, both
//! named through `harness::unique_bin` (the program is kept when the
//! cell fails), built with the lifecycle trace
//! (`BuildOptions::lifecycle_trace`) and run under a deadline.
//!
//!   1. **outcome**: the owner's handler heard each subject's failure
//!      once, each subject dissolved once, and `fn main` reached its
//!      end (`delivered-once`; `clean` for phase `none`; `not-started`
//!      where the run() that would raise the failure is canceled before
//!      it starts, decision line 19, and the owner hears nothing).
//!   2. **trace**: the laws every trace owes (`trace::laws`, among them
//!      that nothing is done to an instance nothing built: a step on a
//!      reclaimed struct), and the cell's plan.
//!   3. **ASan**, on the cells of the default sample:
//!      `harness::build_asan`, chunk pooling off (GH #816), and the
//!      instrumented build must give the same outcome.
//!   4. **differential**, where it applies: the `let_literal`
//!      position's inline twin, the receiver literal, is the same
//!      program by `spec/semantics.md` § "Dissolve timing rules", and
//!      must print the same.
//!
//! [`KNOWN_OPEN`] names the cells that fail today, each with its
//! inventory row (two, for a cell that shows two known defects), the
//! reason, and the complete profile of departures it shows; a run is
//! asserted to show exactly that profile, so a departure outside it
//! fails the cell, and when the fix lands the entry has to change or
//! go. A cell whose defect is undefined behaviour lists every profile
//! it has been seen to show, and a run shows exactly one of them. 18
//! cells in three families:
//! a handler run in place off the owner's domain (C36, L5's), a pinned
//! locus's fields undrained (C9), and a pinned locus's `birth_check`
//! never evaluated (C38). `handler/grandchild/pinned` shows both C9 and
//! C36.
//!
//! A family whose fix has landed leaves [`KNOWN_OPEN`], and its cells
//! assert the adopted outcome and plan; its first cell stays in the
//! sample, under ASan, as one of the [`REGRESSIONS`]. R19's 21 cells
//! (L5's first part) are these: a child's `run()` posted to the worker
//! that is tearing its owner down ran on the reclaimed struct, or, for
//! a subscriber, was freed unrun and unnamed. The teardown now cancels
//! it before the reclaim, and the cell's plan owes the named terminal,
//! `Run=NotStarted(Acknowledged)`, as the first step of the subject's
//! Reclaim, before its arena goes (L5's second part). A `cross_pool`
//! cell owes no cancellation: the subject's run on `side` is joined
//! before its owner is torn down on main, so nothing is queued at its
//! reclaim; the run queued on one pool and reclaimed from another is
//! `l19_cross_pool_queued_run_canceled.hl`'s, and `HALE_MATRIX_ASAN=full`
//! holds the cross-pool cells to no step on a reclaimed struct.
//!
//! ## Size
//!
//! 6 × 8 × 4 = 192 cells: 121 programs, 59 unwritable, 12 with no
//! path. The default runs a deterministic sample ([`default_sample`])
//! of sixty programs, ASan included, in about 10 s;
//! `HALE_MATRIX=full` runs all 121, for a nightly job, in about 12 s
//! (ASan stays on the sample; `HALE_MATRIX_ASAN=full` puts it on every
//! cell run). `HALE_MATRIX_CELL=<id>,<id>` prints the named cells'
//! programs, plans and traces.
//!
//! ## Corpus note
//!
//! Programs are assembled from ordinary string constants, never a raw
//! string literal, because `hale_corpus::embedded` harvests raw string
//! literals that look like a program out of every Rust file under a
//! `tests` directory (the ownership matrix's note).

use std::collections::BTreeSet;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hale_codegen::build_executable_with_options;
use hale_types::lifecycle::trace::{self, Trace, Violation};
use hale_types::lifecycle::ObligationKind;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/lifecycle_plan.rs"]
mod lifecycle_plan;

// ===================================================================
// The axes
// ===================================================================

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Phase {
    ParamsSettle,
    Birth,
    Run,
    Handler,
    Drain,
    None,
}

const PHASES: &[Phase] = &[Phase::ParamsSettle, Phase::Birth, Phase::Run, Phase::Handler, Phase::Drain, Phase::None];

impl Phase {
    fn id(self) -> &'static str {
        match self {
            Phase::ParamsSettle => "params_settle",
            Phase::Birth => "birth",
            Phase::Run => "run",
            Phase::Handler => "handler",
            Phase::Drain => "drain",
            Phase::None => "none",
        }
    }

    fn fails(self) -> bool {
        self != Phase::None
    }

    fn prose(self) -> &'static str {
        match self {
            Phase::ParamsSettle => "fails in a birth-epoch closure",
            Phase::Birth => "fails its birth_check",
            Phase::Run => "violates in run()",
            Phase::Handler => "violates in its bus handler, on the cell its run() publishes",
            Phase::Drain => "violates in drain()",
            Phase::None => "does not fail",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Position {
    RootChild,
    Grandchild,
    AcceptedChild,
    IfaceField,
    PerspSlot,
    Replica,
    HandlerBorn,
    LetLiteral,
}

const POSITIONS: &[Position] = &[
    Position::RootChild,
    Position::Grandchild,
    Position::AcceptedChild,
    Position::IfaceField,
    Position::PerspSlot,
    Position::Replica,
    Position::HandlerBorn,
    Position::LetLiteral,
];

impl Position {
    fn id(self) -> &'static str {
        match self {
            Position::RootChild => "root_child",
            Position::Grandchild => "grandchild",
            Position::AcceptedChild => "accepted_child",
            Position::IfaceField => "iface_field",
            Position::PerspSlot => "persp_slot",
            Position::Replica => "replica",
            Position::HandlerBorn => "handler_born",
            Position::LetLiteral => "let_literal",
        }
    }

    /// Built by its owner's params, so a failure at birth is raised
    /// while the owner's params are open.
    fn is_field(self) -> bool {
        !matches!(self, Position::AcceptedChild | Position::HandlerBorn | Position::LetLiteral)
    }

    /// The subject is a field of the deployment root, so a placement
    /// entry can name it.
    fn is_placeable(self) -> bool {
        matches!(self, Position::RootChild | Position::IfaceField | Position::PerspSlot | Position::Replica)
    }

    fn instances(self) -> usize {
        if self == Position::Replica { 2 } else { 1 }
    }

    fn prose(self) -> &'static str {
        match self {
            Position::RootChild => "a field of the root",
            Position::Grandchild => "a field of the root's field `Mid`, which handles it",
            Position::AcceptedChild => "a statement literal its owner accepts",
            Position::IfaceField => "a field typed by the interface `Probe`",
            Position::PerspSlot => "a field typed `perspective(Route)`",
            Position::Replica => "a `pinned(replicas = 2)` field",
            Position::HandlerBorn => "a literal in its owner's on_failure",
            Position::LetLiteral => "a let-bound literal in its owner's run()",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Domain {
    Main,
    Pool,
    Pinned,
    CrossPool,
}

const DOMAINS: &[Domain] = &[Domain::Main, Domain::Pool, Domain::Pinned, Domain::CrossPool];

impl Domain {
    fn id(self) -> &'static str {
        match self {
            Domain::Main => "main",
            Domain::Pool => "pool",
            Domain::Pinned => "pinned",
            Domain::CrossPool => "cross_pool",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct Cell {
    phase: Phase,
    position: Position,
    domain: Domain,
}

fn cell_id(c: Cell) -> String {
    [c.phase.id(), "/", c.position.id(), "/", c.domain.id()].concat()
}

fn all_cells() -> Vec<Cell> {
    let mut out = Vec::new();
    for &phase in PHASES {
        for &position in POSITIONS {
            for &domain in DOMAINS {
                out.push(Cell { phase, position, domain });
            }
        }
    }
    out
}

// ===================================================================
// Cells that are not programs
// ===================================================================

/// What a cell is.
enum Form {
    Program(Program),
    /// The front end refuses the nearest spelling, under this rule.
    Unwritable { rule: &'static str },
    /// The phase names a path the position does not have.
    NoPath { why: &'static str },
}

/// The refusals a construction can meet, as (rule, the start of the
/// diagnostic). A refusal that matches none is a generator bug.
const REFUSALS: &[(&str, &str)] = &[
    ("placement rule 6", "but declares a closure whose epoch is `birth` or `dissolve`"),
    ("placement rule 3", "which is not a locus type; placement applies only to locus instances"),
    ("placement rule 15", "`replicas` is only valid on a `pinned` placement"),
    ("placement rule 18", "names a field no locus literal initialises"),
];

/// The cells the front end refuses, as `phase/position/domain`
/// patterns (`*` is any value of its axis) and the rule; the first
/// pattern a cell matches is its. Held equal to what the generator and
/// the checker say, cell by cell, and every pattern matches a cell.
const UNWRITABLE: &[(&str, &str)] = &[
    // `replicas` fans out pinned threads only.
    ("*/replica/main", "placement rule 15"),
    ("*/replica/pool", "placement rule 15"),
    ("*/replica/cross_pool", "placement rule 15"),
    // A placement entry names a locus-typed field; an interface or a
    // perspective is not one.
    ("*/iface_field/pinned", "placement rule 3"),
    ("*/iface_field/cross_pool", "placement rule 3"),
    ("*/persp_slot/pinned", "placement rule 3"),
    ("*/persp_slot/cross_pool", "placement rule 3"),
    // A pinned locus may not declare a birth-epoch closure.
    ("params_settle/root_child/pinned", "placement rule 6"),
    ("params_settle/replica/pinned", "placement rule 6"),
    // A literal outside the static tower reaches a placed field only
    // through a call, which leaves the entry untaken.
    ("*/accepted_child/cross_pool", "placement rule 18"),
    ("*/handler_born/cross_pool", "placement rule 18"),
    ("*/let_literal/cross_pool", "placement rule 18"),
];

fn matches_pattern(pattern: &str, id: &str) -> bool {
    pattern.split('/').zip(id.split('/')).all(|(p, v)| p == "*" || p == v)
}

fn declared_unwritable(c: Cell) -> Option<&'static str> {
    let id = cell_id(c);
    UNWRITABLE.iter().find(|(p, _)| matches_pattern(p, &id)).map(|(_, r)| *r)
}

const NO_PARAMS_BRACKET: &str =
    "the literal is written after its owner's params settled, so no failure of it is raised while they are open";

fn no_path(c: Cell) -> Option<&'static str> {
    (c.phase == Phase::ParamsSettle && !c.position.is_field()).then_some(NO_PARAMS_BRACKET)
}

// ===================================================================
// Expected failures
// ===================================================================

/// One known defect a cell shows: (inventory row, the reason in one
/// sentence).
type Open = (&'static str, &'static str);

/// One complete set of departures a known-open cell's run shows: its
/// rows' defects and their consequences in this cell, whole violations
/// as [`departures`] writes them, with the instance written `_`.
type Profile = &'static [&'static str];

/// Cells that fail today: (cell, the defects it shows, the profiles it
/// shows them in). Almost always one defect; a cell exhibiting two known
/// defects carries both rows. Almost always one profile: a run is
/// asserted to show exactly one of its entry's profiles, so a departure
/// outside the profile fails it as it would any other cell, and when a
/// fix lands its departures go, the assertion fails, and the entry has
/// to change or go. A cell whose defect is undefined behaviour says so
/// in its reason and lists every profile it has been seen to show, since
/// a use-after-free can finish on one platform and crash on another; its
/// entry goes with the fix. An `asan:` departure is owed only where ASan
/// ran.
const KNOWN_OPEN: &[(&str, &[Open], &[Profile])] = &[
    // L5's: a failure raised off the owner's thread.
    ("run/root_child/pinned", &[("C36", IN_PLACE)], &[&[RAN_ON_PINNED_1]]),
    ("run/replica/pinned", &[("C36", IN_PLACE)], &[&[RAN_ON_PINNED_1, RAN_ON_PINNED_2]]),
    ("run/root_child/cross_pool", &[("C36", IN_PLACE)], &[&[RAN_ON_SIDE]]),
    ("handler/root_child/pinned", &[("C36", IN_PLACE)], &[&[RAN_ON_PINNED_1]]),
    ("handler/replica/pinned", &[("C36", IN_PLACE)], &[&[RAN_ON_PINNED_1, RAN_ON_PINNED_2]]),
    ("handler/root_child/cross_pool", &[("C36", IN_PLACE)], &[&[RAN_ON_SIDE]]),
    ("handler/grandchild/cross_pool", &[("C36", IN_PLACE)], &[&[RAN_ON_MAIN_FOR_SIDE]]),
    ("drain/root_child/pinned", &[("C36", IN_PLACE)], &[&[RAN_ON_PINNED_1]]),
    ("drain/replica/pinned", &[("C36", IN_PLACE)], &[&[RAN_ON_PINNED_1, RAN_ON_PINNED_2]]),
    ("drain/grandchild/cross_pool", &[("C36", IN_PLACE)], &[&[RAN_ON_MAIN_FOR_SIDE]]),
    // A pinned locus's own fields.
    ("params_settle/grandchild/pinned", &[("C9", UNDRAINED)], &[&[NO_DRAIN]]),
    ("birth/grandchild/pinned", &[("C9", UNDRAINED)], &[&[NO_DRAIN]]),
    ("run/grandchild/pinned", &[("C9", UNDRAINED)], &[&[NO_DRAIN]]),
    // `Subj`'s handler, on main, delivers the failure in place there
    // while its owner `Mid` is pinned: C36 as well as C9.
    ("handler/grandchild/pinned", &[("C9", UNDRAINED), ("C36", IN_PLACE)], &[&[NO_DRAIN, RAN_ON_MAIN_FOR_PINNED]]),
    // The drain that would raise the failure never runs.
    ("drain/grandchild/pinned", &[("C9", UNDRAINED)], &[&[UNHEARD, NO_DRAIN, NO_DELIVERY, RECLAIMED_UNHEARD, ASAN_UNHEARD]]),
    ("none/grandchild/pinned", &[("C9", UNDRAINED)], &[&[NO_DRAIN]]),
    ("birth/root_child/pinned", &[("C38", NO_BIRTH_CHECK)], &[&[UNHEARD, NO_DELIVERY, RECLAIMED_UNHEARD, ASAN_UNHEARD]]),
    ("birth/replica/pinned", &[("C38", NO_BIRTH_CHECK)], &[&[UNHEARD_2, NO_DELIVERY, RECLAIMED_UNHEARD, ASAN_UNHEARD_2]]),
];

/// Cells a fixed defect is held to, sampled with ASan on every PR: a
/// family's first cell, as it was while the family was in
/// [`KNOWN_OPEN`]. R19 (L5's first part): a child's run() posted to the
/// worker that tears its owner down started on the reclaimed struct, or
/// a subscriber's was freed unrun and unnamed; each is now canceled by
/// the teardown, `NotStarted(Acknowledged)` ([`run_canceled`]).
const REGRESSIONS: &[&str] = &["birth/accepted_child/pool", "handler/root_child/pool"];

const IN_PLACE: &str = "the owner's handler runs in place on the thread that raised the failure (the subject's pinned thread or pool worker, or the teardown thread), not on the owner's domain (decision L0-1)";
const RAN_ON_PINNED_1: &str = "trace: domain: Subj.FailureDelivery (inst _ inc 0) ran on pinned:1, claimed main";
const RAN_ON_PINNED_2: &str = "trace: domain: Subj.FailureDelivery (inst _ inc 0) ran on pinned:2, claimed main";
const RAN_ON_SIDE: &str = "trace: domain: Subj.FailureDelivery (inst _ inc 0) ran on pool:side, claimed main";
const RAN_ON_MAIN_FOR_SIDE: &str = "trace: domain: Subj.FailureDelivery (inst _ inc 0) ran on main, claimed pool:side";
const RAN_ON_MAIN_FOR_PINNED: &str = "trace: domain: Subj.FailureDelivery (inst _ inc 0) ran on main, claimed pinned";

const UNDRAINED: &str = "a pinned locus's own fields are never drained, so `Mid`'s field `Subj` is dissolved without its drain";
const NO_DRAIN: &str = "trace: missing: Subj.Drain";

// Inventory C38 said the check agrees; it is never evaluated on a pinned
// locus. The checker does not refuse it: L4's birth spine runs the check
// on the pinned thread before run().
const NO_BIRTH_CHECK: &str = "the pinned thread function runs birth() without the locus's birth_check, so the check never fires and the owner hears nothing";

// The owner hears nothing: no handler runs, the failure is never
// delivered, and each subject is reclaimed without its delivery.
const UNHEARD: &str = "outcome: handler 0/1, dissolve 1/1, adopted delivered-once";
const UNHEARD_2: &str = "outcome: handler 0/2, dissolve 2/2, adopted delivered-once";
const NO_DELIVERY: &str = "trace: missing: Subj.FailureDelivery";
const RECLAIMED_UNHEARD: &str = "trace: edge: Subj.Reclaim.Entered (inst _ inc 0) with Subj.FailureDelivery.Completed not reached";
const ASAN_UNHEARD: &str = "asan: the instrumented build gives handler 0/1, dissolve 1/1";
const ASAN_UNHEARD_2: &str = "asan: the instrumented build gives handler 0/2, dissolve 2/2";

// ===================================================================
// Rendering
// ===================================================================

/// One cell's program and what it owes.
struct Program {
    src: String,
    /// The receiver-literal spelling of a `let_literal` cell.
    twin: Option<String>,
    plan: String,
    outcome: &'static str,
}

/// The lines a cell prints, as the harness counts them.
const SUBJ_DISSOLVE: &str = "ev subj-dissolve";
const HANDLER: &str = "ev handler";
const END: &str = "ev end";

/// The owner that handles the subject's failure: the middle locus for a
/// grandchild, else `Own`.
fn handler_owner(c: Cell) -> &'static str {
    if c.position == Position::Grandchild { "Mid" } else { "Own" }
}

/// Whether `Own` is the deployment root, or a locus a placed `Spawner`
/// builds in its run().
fn own_is_root(c: Cell) -> bool {
    if c.position == Position::Replica {
        // A replica is a placement entry's, on the root, in every domain.
        return true;
    }
    match c.domain {
        Domain::Main | Domain::CrossPool => true,
        Domain::Pool => false,
        Domain::Pinned => c.position.is_field(),
    }
}

/// The placement entry `Own` carries for its subject (or `Mid`), when
/// `Own` is the root.
fn own_placement(c: Cell) -> Option<String> {
    let key = if c.position == Position::Grandchild { "m" } else { "s" };
    let class = match (c.domain, c.position) {
        (Domain::Main, Position::Replica) => "cooperative(pool = main, replicas = 2)",
        (Domain::Pool | Domain::CrossPool, Position::Replica) => "cooperative(pool = side, replicas = 2)",
        (Domain::Pinned, Position::Replica) => "pinned(replicas = 2)",
        (Domain::Main, _) => return None,
        (Domain::Pinned, _) => "pinned",
        (Domain::CrossPool, _) => "cooperative(pool = side)",
        (Domain::Pool, _) => return None,
    };
    Some(["    placement { ", key, ": ", class, "; }\n"].concat())
}

fn subj_decl(c: Cell) -> String {
    let head = if c.position == Position::PerspSlot { "locus Subj : serves Route {\n" } else { "locus Subj {\n" };
    let mut body = vec!["    params { n: Int = 0; }\n".to_string()];
    match c.phase {
        Phase::ParamsSettle => body.push("    closure ready { self.n ~~ 1 within 0; epoch birth; }\n".into()),
        Phase::None => {}
        _ => body.push("    closure fuse { captures: n; epoch inline; }\n".into()),
    }
    if c.phase == Phase::Handler {
        body.push("    bus { subscribe Pings as on_ping; publish Pings; }\n".into());
    }
    if c.phase == Phase::Birth {
        body.push("    birth_check { self.n == 0 } -> violate fuse;\n".into());
    }
    body.push(
        match c.phase {
            Phase::Run => "    run() { violate fuse; }\n",
            Phase::Handler => "    run() { Pings <- Ping { n: 1 }; }\n",
            _ => "    run() { println(\"ev subj-run\"); }\n",
        }
        .into(),
    );
    if c.phase == Phase::Handler {
        // A bus handler's `violate`, not a `bubble` from a handler:
        // `bubble` is lowered as the report and the exit, never climbing
        // the tree (inventory C47, until L4's failure-delivery spine).
        body.push("    fn on_ping(p: Ping) { violate fuse; }\n".into());
    }
    if c.phase == Phase::Drain {
        body.push("    drain() { violate fuse; }\n".into());
    }
    body.push(["    dissolve() { println(\"", SUBJ_DISSOLVE, "\"); }\n"].concat());
    body.push("    fn v() -> Int { return self.n; }\n".into());
    [head.to_string(), body.join(""), "}\n".to_string()].concat()
}

const PING_DECL: &str = "type Ping { n: Int; }
topic Pings { payload: Ping; subject: \"lcmatrix.ping\"; }
";

const TRIG_DECL: &str = "locus Trig {
    params { n: Int = 0; }
    closure trig { captures: n; epoch inline; }
    run() { violate trig; }
}
";

const PROBE_DECL: &str = "interface Probe { fn v() -> Int; }\n";

const ROUTE_DECL: &str = "perspective Route { fn v() -> Int; }\n";

const MAKE_SUBJ: &str = "fn make_subj() -> Subj { return Subj { }; }\n";

/// The handler owner's `on_failure` for the subject.
fn owner_handlers() -> String {
    ["    on_failure(c: Subj, err: ClosureViolation) { println(\"", HANDLER, "\"); }\n"].concat()
}

/// `Own`'s members for the cell's position (and `Mid`'s declaration
/// for a grandchild), `twin` giving the receiver-literal spelling.
fn own_members(c: Cell, twin: bool) -> (String, String) {
    let mut mid = String::new();
    let mut m: Vec<String> = Vec::new();
    // The cross-pool spelling of a literal outside the static tower: a
    // placed field fed by a call (placement rule 18).
    if c.domain == Domain::CrossPool && !c.position.is_field() {
        m.push("    params { s: Subj = make_subj(); }\n".into());
        m.push(own_placement(Cell { position: Position::RootChild, ..c }).expect("a cross-pool entry"));
        m.push(owner_handlers());
        return (mid, m.concat());
    }
    match c.position {
        Position::RootChild | Position::Replica => m.push("    params { s: Subj = Subj { }; }\n".into()),
        Position::IfaceField => m.push("    params { s: Probe = Subj { }; }\n".into()),
        Position::PerspSlot => m.push("    params { s: perspective(Route) = Subj { }; }\n".into()),
        Position::Grandchild => {
            mid = [
                "locus Mid {\n",
                "    params { s: Subj = Subj { }; }\n",
                &owner_handlers(),
                "    dissolve() { println(\"ev mid-dissolve\"); }\n",
                "}\n",
            ]
            .concat();
            m.push("    params { m: Mid = Mid { }; }\n".into());
        }
        _ => {}
    }
    if own_is_root(c) {
        if let Some(p) = own_placement(c) {
            m.push(p);
        }
    }
    match c.position {
        Position::AcceptedChild => {
            m.push("    accept(c: Subj) { }\n".into());
            m.push("    release (c: Subj) { }\n".into());
        }
        Position::HandlerBorn => {
            m.push("    on_failure(c: Trig, err: ClosureViolation) { Subj { }; }\n".into());
        }
        _ => {}
    }
    if c.position != Position::Grandchild {
        m.push(owner_handlers());
    }
    match c.position {
        Position::AcceptedChild => m.push("    run() { Subj { }; }\n".into()),
        Position::HandlerBorn => m.push("    run() { Trig { }; }\n".into()),
        Position::LetLiteral if twin => m.push("    run() { println(\"ev let \", Subj { }.v()); }\n".into()),
        Position::LetLiteral => m.push(
            [
                "    run() {\n",
                "        let s = Subj { };\n",
                "        println(\"ev let \", s.v());\n",
                "    }\n",
            ]
            .concat(),
        ),
        _ => {}
    }
    (mid, m.concat())
}

fn source(c: Cell, twin: bool, header: &str) -> String {
    let mut parts: Vec<String> = vec![header.to_string()];
    if c.position == Position::IfaceField {
        parts.push(PROBE_DECL.into());
    }
    if c.position == Position::PerspSlot {
        parts.push(ROUTE_DECL.into());
    }
    if c.phase == Phase::Handler {
        parts.push(PING_DECL.into());
    }
    if c.position == Position::HandlerBorn && c.domain != Domain::CrossPool {
        parts.push(TRIG_DECL.into());
    }
    parts.push(subj_decl(c));
    if c.domain == Domain::CrossPool && !c.position.is_field() {
        parts.push(MAKE_SUBJ.into());
    }
    let (mid, members) = own_members(c, twin);
    if !mid.is_empty() {
        parts.push(mid);
    }
    let root = own_is_root(c);
    parts.push([if root { "main locus Own {\n" } else { "locus Own {\n" }, &members, "}\n"].concat());
    if !root {
        let class = if c.domain == Domain::Pool { "cooperative(pool = side)" } else { "pinned" };
        parts.push("locus Spawner {\n    run() { Own { }; }\n}\n".into());
        parts.push(
            [
                "main locus App {\n",
                "    params { sp: Spawner = Spawner { }; }\n",
                "    placement { sp: ",
                class,
                "; }\n",
                "}\n",
            ]
            .concat(),
        );
    }
    let top = if root { "Own" } else { "App" };
    parts.push(["fn main() {\n    ", top, " { };\n    println(\"", END, "\");\n}\n"].concat());
    parts.join("\n")
}

/// The obligation kinds a plan names, in declaration order.
fn kinds_of(plan: &str) -> Vec<&'static str> {
    let exp = lifecycle_plan::plan(plan);
    let mut kinds: Vec<ObligationKind> = exp.owed.iter().map(|o| o.kind).collect();
    kinds.sort();
    kinds.dedup();
    kinds.into_iter().map(ObligationKind::name).collect()
}

fn header(c: Cell, plan: &str, outcome: &str) -> String {
    [
        "// Lifecycle matrix cell ",
        &cell_id(c),
        " (",
        &kinds_of(plan).join(", "),
        ").\n//\n// `Subj` ",
        c.phase.prose(),
        "; it is ",
        c.position.prose(),
        ", in domain ",
        c.domain.id(),
        ".\n//\n// Expected terminal outcome: `",
        outcome,
        "`.\n",
    ]
    .concat()
}

/// Render a cell: its form, before the front end has judged it.
fn render(c: Cell) -> Result<Program, &'static str> {
    if let Some(why) = no_path(c) {
        return Err(why);
    }
    let plan = plan_for(c);
    let outcome = if raises(c) {
        "delivered-once"
    } else if c.phase.fails() {
        "not-started"
    } else {
        "clean"
    };
    let head = header(c, &plan, outcome);
    let src = source(c, false, &head);
    let twin = (c.position == Position::LetLiteral).then(|| source(c, true, &head));
    Ok(Program { src, twin, plan, outcome })
}

// ===================================================================
// The plan
// ===================================================================

/// The domain the subject's `run()` is claimed on, when the cell
/// determines it: its own placement, or the one thread its whole tree
/// is built on.
fn subj_run_domain(c: Cell) -> Option<&'static str> {
    match c.domain {
        Domain::Main => Some("main"),
        Domain::Pool => Some("pool:side"),
        Domain::Pinned if c.position == Position::Grandchild => None,
        Domain::Pinned => Some("pinned"),
        Domain::CrossPool if c.position.is_placeable() => Some("pool:side"),
        Domain::CrossPool => None,
    }
}

/// The handler owner's domain: where a failure's delivery completes
/// (decision L0-1).
fn owner_domain(c: Cell) -> &'static str {
    match c.domain {
        Domain::Main => "main",
        Domain::Pool => "pool:side",
        Domain::Pinned if c.position.is_field() && c.position != Position::Grandchild => "main",
        Domain::Pinned => "pinned",
        Domain::CrossPool if c.position == Position::Grandchild => "pool:side",
        Domain::CrossPool => "main",
    }
}

/// Whether the failure is raised while the handler owner's params are
/// open on the thread that settles them, so it is held and delivered
/// at settle (decision line 1).
fn held(c: Cell) -> bool {
    if !c.position.is_field() {
        return false;
    }
    let at_birth = matches!(c.phase, Phase::ParamsSettle | Phase::Birth);
    // A pinned subject is born on its own thread, after the
    // instantiation returned (inventory C9): its owner may have settled.
    let born_on_it = !(c.domain == Domain::Pinned && c.position != Position::Grandchild);
    // A run() inline on the instantiating thread runs inside the params
    // loop (the fixture l01_held_failure_settle): on main, and for a
    // grandchild whose placed parent's params run on main (inventory
    // C9). On a pool worker the field's run() is posted, and where a
    // pool locus's lifecycle runs waits on decision line 3.
    let run_inline =
        c.domain == Domain::Main || (c.position == Position::Grandchild && c.domain != Domain::Pool);
    born_on_it && (at_birth || (c.phase == Phase::Run && run_inline))
}

/// Whether the subject's run() is canceled before it starts (decision
/// line 19, inventory R19). In the pool domain `Own` is a statement
/// literal in `Spawner`'s run(), so the worker tears it down as soon as
/// it is built; a subject whose run() was posted to that worker (a
/// field, or an accepted child; a literal in a handler or a `let` runs
/// inline) is still queued behind the teardown, which cancels the run,
/// `NotStarted(Acknowledged)`, before it reclaims the subject. A field
/// that failed at birth has no run posted; an accepted child that
/// failed its `birth_check` does.
fn run_canceled(c: Cell) -> bool {
    if c.domain != Domain::Pool {
        return false;
    }
    match c.position {
        Position::RootChild | Position::Grandchild | Position::IfaceField | Position::PerspSlot => {
            matches!(c.phase, Phase::Run | Phase::Handler | Phase::Drain | Phase::None)
        }
        Position::AcceptedChild => true,
        _ => false,
    }
}

/// Whether the subject's failure is raised: it fails, and not in a
/// run() (or the handler of the cell its run() publishes) that never
/// starts.
fn raises(c: Cell) -> bool {
    c.phase.fails() && !(run_canceled(c) && matches!(c.phase, Phase::Run | Phase::Handler))
}

/// The cell's plan, from L1's plans for the lines it touches.
fn plan_for(c: Cell) -> String {
    let n = c.position.instances();
    let subj = if n == 1 { "Subj".to_string() } else { format!("Subj*{n}") };
    let owner = handler_owner(c);
    let mut steps: Vec<String> = vec!["Birth".into()];
    let run = match subj_run_domain(c) {
        Some(d) => format!("Run!{d}"),
        None => "Run".into(),
    };
    let delivery = if held(c) {
        // A held failure's delivery completes on the settling thread;
        // where that is for an owner placed off main waits on line 1.
        if c.domain == Domain::Main { "FailureDelivery!main ConstructionDelivery".to_string() } else { "FailureDelivery ConstructionDelivery".to_string() }
    } else {
        format!("FailureDelivery!{}", owner_domain(c))
    };
    // A canceled run has no entry to order; its own line below.
    let canceled = run_canceled(c);
    match c.phase {
        Phase::ParamsSettle | Phase::Birth => steps.push(delivery.clone()),
        Phase::Run | Phase::Handler if canceled => {}
        Phase::Run | Phase::Handler => {
            steps.push(run.clone());
            steps.push(delivery.clone());
        }
        Phase::Drain => {
            if !canceled {
                steps.push(run.clone());
            }
            steps.push("Drain".into());
            steps.push(delivery.clone());
        }
        Phase::None if canceled => {}
        Phase::None => steps.push(run.clone()),
    }
    if c.phase != Phase::Drain {
        steps.push("Drain".into());
    }
    steps.push("Dissolve".into());
    steps.push("Reclaim".into());
    let mut lines = vec![
        format!("{owner}: Birth Drain Dissolve Reclaim"),
        format!("{subj}: {}", steps.join(" ")),
    ];
    if raises(c) {
        // The failed child is kept until its handler completes
        // (`lotus_failure_hold`, decision line 8).
        lines.push("edge Subj.FailureDelivery.Completed -> Subj.Reclaim.Entered".into());
    }
    if canceled {
        // The teardown that reclaims the subject cancels its queued run
        // and names it, on the worker, inside the reclaim's bracket.
        lines.push(format!("{subj}: Run=NotStarted(Acknowledged)!pool:side Cancellation!pool:side"));
        lines.push("edge Subj.Run.Ended -> Subj.Reclaim.Completed".into());
        lines.push("edge Subj.Reclaim.Entered -> Subj.Cancellation.Entered".into());
        lines.push("edge Subj.Cancellation.Completed -> Subj.Reclaim.Completed".into());
    }
    if raises(c) && held(c) {
        lines[0] = format!("{owner}: ParamsSettle Birth Drain Dissolve Reclaim");
        lines.push(format!("edge {owner}.ParamsSettle.Completed -> Subj.FailureDelivery.Completed"));
        lines.push(format!("edge Subj.FailureDelivery.Completed -> {owner}.Birth.Entered"));
    }
    // Children before their owner's arena (decision line 14).
    lines.push(format!("edge Subj.Reclaim.Completed -> {owner}.Reclaim.Entered"));
    lines.join("\n")
}

// ===================================================================
// Running
// ===================================================================

/// Generated programs answer in milliseconds; a hang is a failure of
/// its own, not a stalled suite.
const DEADLINE: Duration = Duration::from_secs(10);

struct Ran {
    stdout: String,
    /// stderr without the trace's lines.
    stderr: String,
    trace: Trace,
    code: Option<i32>,
    timed_out: bool,
}

impl Ran {
    fn complete(&self) -> bool {
        !self.timed_out && self.code == Some(0)
    }
}

/// Run `bin` with output to files, so a large report can never fill a
/// pipe, killing it at [`DEADLINE`].
fn run_bin(bin: &Path, envs: &[(&str, &str)]) -> Ran {
    let out_path = bin.with_extension("out");
    let err_path = bin.with_extension("err");
    let mut cmd = Command::new(bin);
    cmd.stdin(Stdio::null())
        .stdout(File::create(&out_path).expect("create stdout file"))
        .stderr(File::create(&err_path).expect("create stderr file"));
    cmd.envs(envs.iter().copied());
    let mut child = cmd.spawn().expect("spawn the generated binary");
    let start = Instant::now();
    let (code, timed_out) = loop {
        match child.try_wait().expect("try_wait") {
            Some(s) => break (s.code(), false),
            None if start.elapsed() > DEADLINE => {
                let _ = child.kill();
                let _ = child.wait();
                break (None, true);
            }
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    };
    let read = |p: &Path| -> String {
        let mut s = String::new();
        let _ = File::open(p).and_then(|mut f| f.read_to_string(&mut s));
        let _ = std::fs::remove_file(p);
        s
    };
    let stdout = read(&out_path);
    let stderr = read(&err_path);
    let trace = trace::parse(&stderr).unwrap_or_else(|e| panic!("{}: the trace does not parse: {e}", bin.display()));
    Ran { stdout, stderr: trace.rest.clone(), trace, code, timed_out }
}

fn count(stdout: &str, line: &str) -> usize {
    stdout.lines().filter(|l| l.trim_end() == line).count()
}

/// The outcome word.
fn outcome(c: Cell, r: &Ran) -> String {
    if r.timed_out {
        return "timeout".into();
    }
    match r.code {
        Some(0) => {}
        Some(_) if r.stderr.contains("ClosureViolation") => return "structural-exit".into(),
        Some(n) => return format!("exit-{n}"),
        None => return "signal".into(),
    }
    if count(&r.stdout, END) != 1 {
        return "main-did-not-end".into();
    }
    let want = c.position.instances();
    let heard = count(&r.stdout, HANDLER);
    let dissolved = count(&r.stdout, SUBJ_DISSOLVE);
    let want_heard = if raises(c) { want } else { 0 };
    match (heard == want_heard, dissolved == want) {
        (true, true) if raises(c) => "delivered-once".into(),
        (true, true) if c.phase.fails() => "not-started".into(),
        (true, true) => "clean".into(),
        _ => format!("handler {heard}/{want_heard}, dissolve {dissolved}/{want}"),
    }
}

const SANITIZER_MARKERS: &[&str] = &[
    "ERROR: AddressSanitizer",
    "ERROR: LeakSanitizer",
    "heap-use-after-free",
    "double-free",
    "heap-buffer-overflow",
    "stack-buffer-overflow",
    "SEGV on unknown address",
];

fn slug(c: Cell) -> String {
    cell_id(c).replace('/', "_")
}

fn trace_build(program: &hale_syntax::ast::Program, bin: &Path) -> Result<(), String> {
    let opts = hale_codegen::BuildOptions { lifecycle_trace: true, ..build_opts::options() };
    build_executable_with_options(program, bin, &[], &opts).map_err(|e| format!("{e:?}"))
}

/// Parse and check a cell's source: `Ok` when the front end takes it,
/// the refusal's diagnostics otherwise.
fn front_end(src: &str) -> Result<hale_syntax::ast::Program, Vec<String>> {
    let program = hale_syntax::parse_source(src).map_err(|e| vec![format!("{e:?}")])?;
    let errs: Vec<String> = hale_types::check_program(&program)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| d.message.clone())
        .collect();
    if errs.is_empty() { Ok(program) } else { Err(errs) }
}

/// The rule a refusal is under, or a panic: a refusal the table does
/// not know is a generator bug.
fn refusal_rule(c: Cell, errs: &[String], src: &str) -> &'static str {
    for (rule, text) in REFUSALS {
        if errs.iter().any(|e| e.contains(text)) {
            return rule;
        }
    }
    panic!("{}: the front end refuses the generated program, and no construction rule names it: {errs:#?}\n{src}", cell_id(c))
}

/// A cell's form, judged by the front end.
fn form(c: Cell) -> Form {
    match render(c) {
        Err(why) => Form::NoPath { why },
        Ok(p) => match front_end(&p.src) {
            Ok(_) => Form::Program(p),
            Err(errs) => Form::Unwritable { rule: refusal_rule(c, &errs, &p.src) },
        },
    }
}

const KEPT_NOTE: &str = "(the program is kept at ";

/// Every way a cell failed, each as `<oracle>: <what>`.
fn run_cell(c: Cell, asan: bool) -> Vec<String> {
    let id = cell_id(c);
    let Form::Program(p) = form(c) else { panic!("{id} is not a program") };
    let program = front_end(&p.src).expect("judged already");
    let mut failures = Vec::new();
    let bin = harness::unique_bin(&["lcmatrix_", &slug(c)].concat());
    let _ = std::fs::write(bin.with_extension("hl"), &p.src);
    if let Err(e) = trace_build(&program, &bin) {
        return vec![format!("build: {e}")];
    }
    let ran = run_bin(&bin, &[]);
    let _ = std::fs::remove_file(&bin);
    if verbose(c) {
        dump(&id, &p, &ran);
    }

    // --- 1: the outcome word ---------------------------------------
    let got = outcome(c, &ran);
    if got != p.outcome {
        failures.push(format!("outcome: {got}, adopted {}", p.outcome));
    }

    // --- 2: the trace ----------------------------------------------
    let mut v: Vec<Violation> = trace::laws(&ran.trace, ran.complete());
    v.extend(lifecycle_plan::plan(&p.plan).check(&ran.trace, ran.complete()));
    failures.extend(v.iter().map(|v| format!("trace: {v}")));

    // --- 4: the differential ---------------------------------------
    if let Some(twin) = &p.twin {
        let tprogram = front_end(twin).unwrap_or_else(|e| panic!("{id}: the twin is refused: {e:#?}\n{twin}"));
        let tbin = harness::unique_bin(&["lcmatrix_twin_", &slug(c)].concat());
        match trace_build(&tprogram, &tbin) {
            Err(e) => failures.push(format!("differential: the twin does not build: {e}")),
            Ok(()) => {
                let tran = run_bin(&tbin, &[]);
                let _ = std::fs::remove_file(&tbin);
                if tran.stdout != ran.stdout || tran.code != ran.code {
                    failures.push(format!(
                        "differential: the let-bound and receiver spellings differ\nlet:\n{}receiver:\n{}",
                        ran.stdout, tran.stdout
                    ));
                }
            }
        }
    }

    // --- 3: the sanitizer ------------------------------------------
    if asan {
        let abin = harness::unique_bin(&["lcmatrix_asan_", &slug(c)].concat());
        harness::build_asan(&program, &abin);
        let arun = run_bin(&abin, &[("ASAN_OPTIONS", "detect_leaks=1"), ("LOTUS_NO_CHUNK_POOL", "1")]);
        let _ = std::fs::remove_file(&abin);
        let report = [arun.stdout.as_str(), arun.stderr.as_str()].concat();
        let hits: Vec<&str> = SANITIZER_MARKERS.iter().copied().filter(|m| report.contains(m)).collect();
        if !hits.is_empty() {
            failures.push(format!("asan: {hits:?}\n{report}"));
        } else if outcome(c, &arun) != p.outcome {
            failures.push(format!("asan: the instrumented build gives {}", outcome(c, &arun)));
        }
    }
    if failures.is_empty() {
        let _ = std::fs::remove_file(bin.with_extension("hl"));
    } else {
        failures.push(format!("{KEPT_NOTE}{})", bin.with_extension("hl").display()));
    }
    failures
}

/// `HALE_MATRIX_CELL=<substrings of cell ids, comma-separated>` prints
/// the matching cells' programs, plans and traces.
fn verbose(c: Cell) -> bool {
    std::env::var("HALE_MATRIX_CELL").is_ok_and(|f| f.split(',').any(|s| cell_id(c).contains(s)))
}

fn dump(id: &str, p: &Program, ran: &Ran) {
    eprintln!("==== {id}\n{}\n---- plan\n{}\n---- stdout\n{}---- stderr\n{}---- trace", p.src, p.plan, ran.stdout, ran.stderr);
    for e in &ran.trace.events {
        eprintln!(
            "{} {} {} {} {} {} {:?}",
            e.seq,
            e.kind.name(),
            e.point.name(),
            e.spine.map(|s| s.name()).unwrap_or("-"),
            e.domain,
            e.decl.as_deref().unwrap_or("-"),
            e.subject.map(|s| (s.instance.raw(), s.incarnation.raw()))
        );
    }
}

// ===================================================================
// The sample
// ===================================================================

/// How many programs the default sample aims for.
const TARGET_SAMPLE: usize = 60;

/// A stride co-prime with 192 = 2^6 · 3.
const FILL_STRIDE: usize = 73;

fn full_mode() -> bool {
    matches!(std::env::var("HALE_MATRIX").as_deref(), Ok("full"))
}

fn is_program(c: Cell) -> bool {
    matches!(form(c), Form::Program(_))
}

/// The default sample: the first `KNOWN_OPEN` cell of each family (one
/// reason), so every known defect is held to its failure on every PR,
/// and the [`REGRESSIONS`], so every fixed one is held to its fix;
/// one program per (phase, position) pair, walking the domains so each
/// appears; then a co-prime stride over the programs to
/// [`TARGET_SAMPLE`]. Per family rather than per open cell: a family
/// fails across a whole axis, and its cells are all held in the full
/// run.
fn default_sample() -> Vec<Cell> {
    let all = all_cells();
    let programs: Vec<Cell> = all.iter().copied().filter(|c| is_program(*c)).collect();
    let mut picked: BTreeSet<Cell> = BTreeSet::new();
    let mut families: BTreeSet<&str> = BTreeSet::new();
    for (id, opens, _) in KNOWN_OPEN {
        let c = *programs.iter().find(|c| cell_id(**c) == *id).unwrap_or_else(|| panic!("KNOWN_OPEN names {id}, not a program"));
        for (_, reason) in *opens {
            if families.insert(reason) {
                picked.insert(c);
            }
        }
    }
    for id in REGRESSIONS {
        let c = *programs.iter().find(|c| cell_id(**c) == *id).unwrap_or_else(|| panic!("REGRESSIONS names {id}, not a program"));
        picked.insert(c);
    }
    for (i, &phase) in PHASES.iter().enumerate() {
        for (j, &position) in POSITIONS.iter().enumerate() {
            for k in 0..DOMAINS.len() {
                let domain = DOMAINS[(i + j + k) % DOMAINS.len()];
                let c = Cell { phase, position, domain };
                if programs.contains(&c) {
                    picked.insert(c);
                    break;
                }
            }
        }
    }
    let mut k = 0;
    while picked.len() < TARGET_SAMPLE && k < programs.len() {
        picked.insert(programs[(k * FILL_STRIDE) % programs.len()]);
        k += 1;
    }
    picked.into_iter().collect()
}

/// `HALE_MATRIX_ASAN=full`: ASan on every selected cell, not only the
/// sample's.
fn asan_everywhere() -> bool {
    matches!(std::env::var("HALE_MATRIX_ASAN").as_deref(), Ok("full"))
}

fn selected_cells() -> Vec<Cell> {
    if full_mode() { all_cells().into_iter().filter(|c| is_program(*c)).collect() } else { default_sample() }
}

// ===================================================================
// The shards
// ===================================================================

fn open_entry(id: &str) -> Option<(&'static [Open], &'static [Profile])> {
    KNOWN_OPEN.iter().find(|(k, ..)| *k == id).map(|(_, opens, profiles)| (*opens, *profiles))
}

/// A sanitizer departure is owed only where ASan ran.
fn owed(d: &str, asan: bool) -> bool {
    asan || !d.starts_with("asan: ")
}

/// A run's failures as departures: each failure's first line (the rest
/// is its evidence: a sanitizer report, two spellings' output), its
/// instance numbers written `_`, without the note saying where the
/// program is kept.
fn departures(failures: &[String]) -> Vec<String> {
    failures
        .iter()
        .filter(|f| !f.starts_with(KEPT_NOTE))
        .map(|f| lifecycle_plan::normalized(f.lines().next().unwrap_or_default()))
        .collect()
}

/// Run the selected cells of one domain and one phase, each held to the
/// verdict `KNOWN_OPEN` declares for it.
fn run_shard(domain: Domain, phase: Phase) {
    let sample: BTreeSet<Cell> = default_sample().into_iter().collect();
    let cells: Vec<Cell> = selected_cells().into_iter().filter(|c| c.domain == domain && c.phase == phase).collect();
    let mut unexpected_fail = Vec::new();
    let mut unexpected_pass = Vec::new();
    for c in cells {
        let id = cell_id(c);
        let started = Instant::now();
        let asan = sample.contains(&c) || asan_everywhere();
        let failures = run_cell(c, asan);
        eprintln!("{id}: {} failure(s) in {:?}", failures.len(), started.elapsed());
        match open_entry(&id) {
            Some((opens, profiles)) => {
                let shown = departures(&failures);
                let rows: Vec<&str> = opens.iter().map(|(row, _)| *row).collect();
                // A run matches a profile when it shows nothing outside
                // it and everything in it that is owed; the guard keeps
                // the profiles distinct, so at most one matches.
                let gone = |p: Profile| -> Vec<&str> {
                    p.iter().copied().filter(|d| owed(d, asan) && !shown.iter().any(|s| s == d)).collect()
                };
                let matches = |p: Profile| shown.iter().all(|s| p.contains(&s.as_str())) && gone(p).is_empty();
                if profiles.iter().any(|p| matches(p)) {
                    continue;
                }
                let unmatched: Vec<&String> =
                    shown.iter().filter(|s| !profiles.iter().any(|p| p.contains(&s.as_str()))).collect();
                if !unmatched.is_empty() {
                    unexpected_fail.push(format!(
                        "{id} fails beyond the departures of inventory row(s) {rows:?}: {unmatched:#?}\n  {}",
                        failures.join("\n  ")
                    ));
                } else {
                    let gone: Vec<Vec<&str>> = profiles.iter().map(|p| gone(p)).collect();
                    unexpected_pass.push(format!(
                        "{id} (KNOWN_OPEN at {opens:?}) shows none of its profiles: of each it no longer shows {gone:#?}: the fix has landed, so the entry has to change or go; it shows {shown:#?}"
                    ));
                }
            }
            None if failures.is_empty() => {}
            None => unexpected_fail.push(format!("{id}\n  {}", failures.join("\n  "))),
        }
    }
    assert!(
        unexpected_fail.is_empty(),
        "{} cell(s) fail a lifecycle oracle and are not in KNOWN_OPEN:\n\n{}",
        unexpected_fail.len(),
        unexpected_fail.join("\n\n")
    );
    assert!(unexpected_pass.is_empty(), "{}", unexpected_pass.join("\n\n"));
}

macro_rules! shards {
    ($($name:ident => $domain:ident, $phase:ident;)*) => {
        mod matrix {
            use super::*;
            $(
                #[test]
                fn $name() { run_shard(Domain::$domain, Phase::$phase); }
            )*
        }
    };
}

shards! {
    main_params_settle => Main, ParamsSettle;
    main_birth => Main, Birth;
    main_run => Main, Run;
    main_handler => Main, Handler;
    main_drain => Main, Drain;
    main_none => Main, None;
    pool_params_settle => Pool, ParamsSettle;
    pool_birth => Pool, Birth;
    pool_run => Pool, Run;
    pool_handler => Pool, Handler;
    pool_drain => Pool, Drain;
    pool_none => Pool, None;
    pinned_params_settle => Pinned, ParamsSettle;
    pinned_birth => Pinned, Birth;
    pinned_run => Pinned, Run;
    pinned_handler => Pinned, Handler;
    pinned_drain => Pinned, Drain;
    pinned_none => Pinned, None;
    cross_pool_params_settle => CrossPool, ParamsSettle;
    cross_pool_birth => CrossPool, Birth;
    cross_pool_run => CrossPool, Run;
    cross_pool_handler => CrossPool, Handler;
    cross_pool_drain => CrossPool, Drain;
    cross_pool_none => CrossPool, None;
}

// ===================================================================
// The matrix itself
// ===================================================================

/// Every cell is a program, `Unwritable` under a rule `UNWRITABLE`
/// names, or `NoPath`; every program is `hale fmt` clean and its plan
/// parses; the sample covers every axis value and stays a sample.
#[test]
fn every_cell_is_written_or_named() {
    let all = all_cells();
    assert_eq!(all.len(), PHASES.len() * POSITIONS.len() * DOMAINS.len());
    let ids: BTreeSet<String> = all.iter().map(|c| cell_id(*c)).collect();
    assert_eq!(ids.len(), all.len(), "two cells share an id");

    let mut unwritable: Vec<(String, &str)> = Vec::new();
    let mut no_paths = 0;
    let mut programs = 0;
    for c in &all {
        match form(*c) {
            Form::Program(p) => {
                programs += 1;
                for src in std::iter::once(&p.src).chain(p.twin.as_ref()) {
                    let formatted = hale_syntax::fmt::format_source(src).unwrap_or_else(|e| panic!("{}: fmt: {e:?}", cell_id(*c)));
                    assert!(formatted == *src, "{} is not `hale fmt` clean; formatted:\n{formatted}\ngenerated:\n{src}", cell_id(*c));
                }
                assert!(!lifecycle_plan::plan(&p.plan).owed.is_empty(), "{}: an empty plan", cell_id(*c));
                // The header names the cell, its obligations and its
                // expected outcome, as a fixture's does.
                let head = format!("// Lifecycle matrix cell {} (", cell_id(*c));
                let word = format!("// Expected terminal outcome: `{}`.", p.outcome);
                assert!(p.src.starts_with(&head) && p.src.contains(&word), "{}: the header is not the convention:\n{}", cell_id(*c), p.src);
            }
            Form::Unwritable { rule } => unwritable.push((cell_id(*c), rule)),
            Form::NoPath { why } => {
                assert!(!why.is_empty(), "{}: a cell with no path says why", cell_id(*c));
                no_paths += 1;
            }
        }
    }
    let declared: Vec<(String, &str)> =
        all.iter().filter(|c| no_path(**c).is_none()).filter_map(|c| declared_unwritable(*c).map(|r| (cell_id(*c), r))).collect();
    assert_eq!(unwritable, declared, "the cells the front end refuses are not the ones UNWRITABLE names");
    for (pattern, _) in UNWRITABLE {
        assert!(ids.iter().any(|id| matches_pattern(pattern, id)), "UNWRITABLE's {pattern} matches no cell");
    }
    eprintln!(
        "{} cells: {programs} programs, {} unwritable, {no_paths} with no path; the default sample runs {}",
        all.len(),
        unwritable.len(),
        default_sample().len()
    );

    let mut open_ids = BTreeSet::new();
    for id in REGRESSIONS {
        assert!(ids.contains(*id), "REGRESSIONS names {id}, which is not a cell");
        assert!(open_entry(id).is_none(), "{id} is both a regression and KNOWN_OPEN");
    }
    for (id, opens, profiles) in KNOWN_OPEN {
        assert!(ids.contains(*id), "KNOWN_OPEN names {id}, which is not a cell");
        assert!(open_ids.insert(*id), "KNOWN_OPEN names {id} twice; a cell's rows go in one entry");
        assert!(!opens.is_empty(), "{id}: an entry owes at least one row");
        let mut rows = BTreeSet::new();
        for (row, reason) in *opens {
            assert!(!row.is_empty() && !reason.is_empty(), "{id}: a row owes its name and reason");
            assert!(rows.insert(*row), "{id} names row {row} twice");
        }
        assert!(!profiles.is_empty(), "{id}: an entry owes at least one profile");
        assert!(
            profiles.len() == 1 || opens.iter().any(|(_, reason)| reason.contains("undefined behaviour")),
            "{id}: only a defect that is undefined behaviour, and says so in its reason, shows alternative profiles"
        );
        // Distinct with the sanitizer's departures set aside, so a run
        // without ASan matches at most one profile too.
        let mut seen = BTreeSet::new();
        for p in *profiles {
            assert!(!p.is_empty(), "{id}: a profile owes at least one departure");
            let mut listed = BTreeSet::new();
            for d in *p {
                assert_eq!(lifecycle_plan::normalized(d), *d, "{id}: a departure names its instance as `_`");
                assert!(!d.contains('\n'), "{id}: a departure is one line");
                assert!(listed.insert(*d), "{id}: `{d}` is listed twice in one profile");
            }
            let unsanitized: BTreeSet<&str> = p.iter().copied().filter(|d| owed(d, false)).collect();
            assert!(seen.insert(unsanitized), "{id}: two profiles are the same, the sanitizer's departures aside");
        }
    }

    let sample = default_sample();
    assert!(sample.len() >= TARGET_SAMPLE, "the sample shrank to {}", sample.len());
    assert!(sample.len() <= programs / 2, "the sample grew to {} of {programs} programs", sample.len());
    let seen = |f: &dyn Fn(&Cell) -> &'static str| -> BTreeSet<&'static str> { sample.iter().map(f).collect() };
    assert_eq!(seen(&|c| c.phase.id()).len(), PHASES.len(), "a phase is not sampled");
    assert_eq!(seen(&|c| c.position.id()).len(), POSITIONS.len(), "a position is not sampled");
    assert_eq!(seen(&|c| c.domain.id()).len(), DOMAINS.len(), "a domain is not sampled");
}
