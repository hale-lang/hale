//! Runs the DNA Phase 0 domain proof (`dna/tests/`, GH #526) through
//! `hale test`, and `hale verify` over its core, so a compiler change
//! that breaks the domain shapes fails the build here rather than in
//! a friction log. `dna/core` is a library seed (imported by the
//! tests); every fixture is an ordinary Hale program that exits 0.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

/// The DNA fixtures, run in slices so CI's hash partition can spread
/// them: one test over the whole `dna/tests` directory took eleven
/// minutes on one partition while the others' test phases took three
/// (2026-09-19), and every workflow card adds a fixture. Each slice
/// takes every SLICES-th fixture of the sorted listing and runs them in
/// turn, as `hale test <dir>` did.
///
/// The slices that land on one job run beside each other, four at a
/// time (the `dna-fixtures` test group in `.config/nextest.toml`):
/// ports are already picked free per fixture, and the knowledge
/// database a slice hands its fixtures is now the slice's own
/// (`SliceKnowledgeDb`), so nothing but the runner is shared. Four
/// because the suite has CI jobs of its own (`dna` in tests.yml) on
/// 4-core runners, and every fixture builds and boots an organism, so
/// one per core is where a fixture's bounded waits stop paying for
/// contention (GH #795). Sixteen slices of ~5 fixtures rather than
/// eight of ~10, so the hash partition over the slice tests spreads
/// them evenly across two jobs. One wedged fixture still ends in a
/// named failure within the group's timeout.
const SLICES: usize = 16;

fn fixture_files() -> Vec<PathBuf> {
    let dir = repo_root().join("dna/tests");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("dna/tests")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).map_or(false, |n| n.ends_with("_test.hl")))
        .collect();
    files.sort();
    files
}

/// The environment variable a slice stamps on every process its own
/// fixtures start (GH #872). `hale test` runs a fixture as a child of
/// this process, and the fixture spawns its organism, bodies and
/// services with `execvp`, which carries the environment through every
/// generation — so anything one of THIS slice's fixtures leaves running
/// still holds THIS slice's value, while the slice running beside it in
/// the same `cargo test` process (the slices run in parallel by
/// default) and another checkout's DNA run on the same box hold a
/// different value or none at all.
const SLICE_TAG: &str = "HALE_DNA_SUITE_TAG";

/// A value no other slice and no other run on this box can hold: this
/// process, this slice, this moment.
fn slice_tag(slice: usize) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}-slice{}-{}", std::process::id(), slice, now)
}

/// The DNA scratch root a command line or a working directory names, if
/// it names one: the fixtures build their roots as
/// `/tmp/dna-<what>-<pid>` and run their organisms from inside them, so
/// the root is what attributes a leaked process to a fixture.
fn dna_scratch_root(text: &str) -> Option<String> {
    let at = text.find("/tmp/dna-")?;
    let rest = &text[at..];
    let after_tmp = "/tmp/".len();
    let end = rest[after_tmp..].find('/').map_or(rest.len(), |i| after_tmp + i);
    // A command line arrives here as one string, and a removed
    // directory reads back from `cwd` as `<path> (deleted)`: either
    // way the root ends at the first blank.
    Some(rest[..end].split_whitespace().next()?.to_string())
}

/// The processes THIS slice's fixtures started that are still running,
/// one line each, named by the scratch root that owns them so a real
/// leak is attributable to the fixture that leaked it.
///
/// A process is this slice's when it carries this slice's stamp; it is
/// a leaked organism when a DNA scratch root is in its command line or
/// is its working directory. Both conditions, so that the slice's own
/// `hale test` children on their way out are not blamed, and so that
/// the scope is exactly what it was before #872 minus the neighbours.
fn leftover_processes(tag: &str) -> Vec<String> {
    let Ok(procfs) = std::fs::read_dir("/proc") else {
        return unscoped_leftover_processes();
    };
    let stamp = format!("{SLICE_TAG}={tag}");
    let mut left = Vec::new();
    for entry in procfs.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str().filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) else {
            continue;
        };
        let dir = entry.path();
        // Unreadable is not this slice's: either another user's
        // process, or one that ended between the listing and here.
        let Ok(environ) = std::fs::read(dir.join("environ")) else { continue };
        if !environ.split(|b| *b == 0).any(|v| v == stamp.as_bytes()) {
            continue;
        }
        let cmdline = std::fs::read(dir.join("cmdline")).unwrap_or_default();
        let cmdline = String::from_utf8_lossy(&cmdline).replace('\0', " ").trim().to_string();
        let cwd = std::fs::read_link(dir.join("cwd")).map(|p| p.display().to_string()).unwrap_or_default();
        let Some(root) = dna_scratch_root(&cmdline).or_else(|| dna_scratch_root(&cwd)) else { continue };
        left.push(format!("  {root}: pid {pid}: {cmdline}"));
    }
    left.sort();
    left
}

/// The pre-#872 filter, for a platform with no procfs to read a
/// process's environment from (macOS; CI runs this suite on Linux).
/// Nothing there can tell one runner's processes from another's, so
/// this keeps the unscoped filter rather than no guard at all — it
/// over-blames where it cannot attribute, which is the failure mode a
/// developer can see and diagnose.
fn unscoped_leftover_processes() -> Vec<String> {
    let ps = Command::new("ps").args(["-eo", "pid=,args="]).output().expect("ps");
    let me = std::process::id().to_string();
    let mut left: Vec<String> = String::from_utf8_lossy(&ps.stdout)
        .lines()
        .filter(|l| l.contains("/tmp/dna-") && l.split_whitespace().next() != Some(me.as_str()))
        .map(|l| format!("  {}", l.trim()))
        .collect();
    left.sort();
    left
}

/// The DSN `dsn` with its database path replaced by `name`, query
/// string and all: `postgres://dna:dna@host:5480/dna?sslmode=disable`
/// with `dna_1234_s2_…` becomes
/// `postgres://dna:dna@host:5480/dna_1234_s2_…?sslmode=disable`.
/// `None` when the DSN has no `scheme://host/database` shape to edit.
fn dsn_with_database(dsn: &str, name: &str) -> Option<String> {
    let authority = dsn.find("://")? + "://".len();
    let path = authority + dsn[authority..].find('/')?;
    let tail = dsn[path + 1..].find(['?', '#']).map_or(dsn.len(), |i| path + 1 + i);
    Some(format!("{}/{}{}", &dsn[..path], name, &dsn[tail..]))
}

/// A knowledge database of this slice's own, so that the slices of one
/// partition can run beside each other. CI's `test` job sets one
/// `HALE_DNA_KNOWLEDGE_DSN` for the whole job and four of the fixtures
/// write to it; sharing it is the reason the slices used to be
/// serialized. The slice creates a database named after itself, hands
/// its fixtures a DSN pointing at that one, and drops it on the way out
/// (including when a fixture fails: the drop runs while the assertion
/// unwinds).
///
/// Only for a `postgres` DSN. `memory`, an unset DSN, or a box with no
/// `psql` leaves the fixtures with what the environment gave them —
/// nothing about the suite requires the isolation, it only lets the
/// slices overlap.
struct SliceKnowledgeDb {
    /// The DSN the database was created from, and will be dropped from.
    admin: String,
    name: String,
    /// The DSN this slice's fixtures get: `admin` with its database
    /// replaced by `name`.
    dsn: String,
}

impl SliceKnowledgeDb {
    fn create(slice: usize) -> Option<SliceKnowledgeDb> {
        let admin = std::env::var("HALE_DNA_KNOWLEDGE_DSN").ok()?;
        if !admin.starts_with("postgres") {
            return None;
        }
        // Lowercase, unqualified, well under Postgres's 63-byte
        // identifier limit, and unique per process, slice and moment
        // for the same reason the slice tag is: another checkout may be
        // running its own DNA suite against this same server.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() % 1_000_000_000)
            .unwrap_or(0);
        let name = format!("dna_{}_s{}_{}", std::process::id(), slice, now);
        // Rewritten before it is created: a DSN this cannot edit is one
        // the fixtures could not be pointed at anyway, and an
        // unreachable database is not worth creating.
        let dsn = dsn_with_database(&admin, &name)?;
        match Command::new("psql").arg(&admin).args(["-v", "ON_ERROR_STOP=1", "-c", &format!("CREATE DATABASE {name}")]).output() {
            Ok(out) if out.status.success() => Some(SliceKnowledgeDb { admin, name, dsn }),
            Ok(out) => {
                eprintln!(
                    "dna slice {slice}: could not create its own knowledge database ({name}); \
                     falling back to the shared HALE_DNA_KNOWLEDGE_DSN:\n{}",
                    String::from_utf8_lossy(&out.stderr).trim()
                );
                None
            }
            Err(e) => {
                eprintln!(
                    "dna slice {slice}: no usable `psql` ({e}), so it cannot take a knowledge database of its own; \
                     falling back to the shared HALE_DNA_KNOWLEDGE_DSN. The slices of one partition then write to \
                     one database while they run beside each other."
                );
                None
            }
        }
    }
}

impl Drop for SliceKnowledgeDb {
    fn drop(&mut self) {
        // Best effort: a database left behind costs a CI runner that is
        // thrown away in minutes nothing, and failing the slice over
        // the cleanup would bury the failure that got us here.
        let sql = format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", self.name);
        let out = Command::new("psql").arg(&self.admin).args(["-v", "ON_ERROR_STOP=1", "-c", &sql]).output();
        if !matches!(&out, Ok(o) if o.status.success()) {
            eprintln!("dna slice: could not drop its knowledge database {} (harmless; it is a scratch database)", self.name);
        }
    }
}

/// What one fixture cost, for the slice's timing table.
struct FixtureTiming {
    fixture: String,
    seconds: f64,
}

/// The slice's fixtures, slowest first, on stdout and — under GitHub
/// Actions — in the job summary. Which fixture a slice's minutes went
/// to is otherwise invisible: the slices were sized by hashing, and the
/// spread between them (59s to 196s on the 2026-09-20 run) is a
/// property of their contents that nothing reported.
fn report_timings(slice: usize, timings: &mut Vec<FixtureTiming>) {
    if timings.is_empty() {
        return;
    }
    timings.sort_by(|a, b| b.seconds.total_cmp(&a.seconds).then_with(|| a.fixture.cmp(&b.fixture)));
    let total: f64 = timings.iter().map(|t| t.seconds).sum();
    let width = timings.iter().map(|t| t.fixture.len()).max().unwrap_or(0);
    println!("DNA fixtures, slice {slice}: {} fixtures, {total:.1}s", timings.len());
    for t in timings.iter() {
        println!("  {:<width$}  {:>7.1}s", t.fixture, t.seconds, width = width);
    }
    let Some(summary) = std::env::var_os("GITHUB_STEP_SUMMARY") else { return };
    let mut md = format!("### DNA fixtures, slice {slice}\n\n| fixture | seconds |\n| --- | ---: |\n");
    for t in timings.iter() {
        md.push_str(&format!("| {} | {:.1} |\n", t.fixture, t.seconds));
    }
    md.push_str(&format!("| **total** | **{total:.1}** |\n\n"));
    // Best effort, like the drop above: a summary that cannot be
    // written is not a test failure.
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&summary) {
        use std::io::Write;
        let _ = f.write_all(md.as_bytes());
    }
}

/// Run one fixture, returning what it cost or why it failed. `hale test
/// --json` is what carries the cost: the text form reports only the
/// pass/fail summary, and the wall time of the child here would fold
/// the compile of every import into the first fixture that needed it.
fn run_one_fixture(f: &PathBuf, tag: &str, dsn: Option<&str>) -> Result<FixtureTiming, String> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_hale"));
    cmd.arg("test").arg("--json").arg(f).env("HALE_BIN", env!("CARGO_BIN_EXE_hale"));
    cmd.env("HALE_DNA_SOURCE", repo_root());
    cmd.env(SLICE_TAG, tag);
    cmd.current_dir(std::env::temp_dir());
    if let Some(dsn) = dsn {
        cmd.env("HALE_DNA_KNOWLEDGE_DSN", dsn);
    }
    // GH #795: a fixture's bounded waits are written for a quiet
    // machine, and here they are not on one — the slices run beside the
    // rest of the workspace's tests, and beside one another, with an
    // organism, its bodies and their builds inside each fixture.
    // `dna::wait_scale` widens every wait at once, and the suite asks
    // for double by default; a slower runner can ask for more from the
    // environment. Two, not more: the widest wait in a fixture is 300s,
    // and the slice's own `terminate-after` allowance is 15 minutes for
    // every fixture in it, so one stuck wait must not be able to eat
    // the whole slice. Running a fixture by hand keeps the
    // quiet-machine bounds, so a real hang is still reported in seconds
    // rather than minutes.
    if std::env::var_os("HALE_DNA_WAIT_SCALE").is_none() {
        cmd.env("HALE_DNA_WAIT_SCALE", "2");
    }
    let out = cmd.output().expect("invoke hale test on a DNA fixture");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    // The whole of both streams, as before: what the organism said on
    // its way down is most of the diagnosis.
    let failed = |why: &str| format!("the DNA fixture {} {why}\n\nstdout:\n{}\nstderr:\n{}", f.display(), stdout, stderr);
    if !out.status.success() {
        return Err(failed("failed."));
    }
    // `hale test --json` prints the outcome array as its last line.
    let line = stdout.lines().rev().find(|l| l.trim_start().starts_with('[')).ok_or_else(|| failed("printed no JSON outcome array."))?;
    let outcomes: Vec<serde_json::Value> =
        serde_json::from_str(line.trim()).map_err(|e| failed(&format!("printed an unreadable JSON outcome array ({e}).")))?;
    // A fixture that ran nothing passes vacuously: `hale test` exits 0
    // over an empty set, so the count is part of the contract, as the
    // `, 0 failed` summary this replaces made it.
    if outcomes.is_empty() {
        return Err(failed("ran no test."));
    }
    // The JSON escapes a failure's own output; unescaped here so the
    // assertion that failed reads as it would from `hale test`.
    let not_passed: Vec<String> = outcomes
        .iter()
        .filter(|o| o["status"] != "pass")
        .map(|o| format!("{}:\n{}", o["file"].as_str().unwrap_or("?"), o["message"].as_str().unwrap_or("(no message)")))
        .collect();
    if !not_passed.is_empty() {
        return Err(failed(&format!("did not pass:\n{}", not_passed.join("\n"))));
    }
    let ms: u64 = outcomes.iter().map(|o| o["elapsed_ms"].as_u64().unwrap_or(0)).sum();
    Ok(FixtureTiming {
        fixture: f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| f.display().to_string()),
        seconds: ms as f64 / 1000.0,
    })
}

fn run_fixture_slice(slice: usize) {
    let files = fixture_files();
    assert!(!files.is_empty(), "no DNA fixtures under dna/tests");
    let mine: Vec<&PathBuf> = files.iter().enumerate().filter(|(i, _)| i % SLICES == slice).map(|(_, f)| f).collect();
    let tag = slice_tag(slice);
    let db = SliceKnowledgeDb::create(slice);
    // Without one of its own — no postgres DSN, no `psql` — the
    // fixtures get exactly what this process was given.
    let dsn = match &db {
        Some(db) => Some(db.dsn.clone()),
        None => std::env::var("HALE_DNA_KNOWLEDGE_DSN").ok(),
    };

    let mut timings: Vec<FixtureTiming> = Vec::with_capacity(mine.len());
    let mut failure = None;
    for f in &mine {
        match run_one_fixture(f, &tag, dsn.as_deref()) {
            // A fixture failure stops the slice, as it always has: the
            // fixtures after it would run against whatever the failing
            // one left behind.
            Err(why) => {
                failure = Some(why);
                break;
            }
            Ok(t) => timings.push(t),
        }
    }
    // Before the panic, so a slice that failed still says where its
    // minutes went.
    report_timings(slice, &mut timings);
    if let Some(why) = failure {
        panic!("{why}");
    }
    // Every process a fixture started must be gone once the fixture is:
    // a host, a body or a service left running is a leak the fixture
    // did not reclaim. The wait is for a child still tearing down; the
    // filter is scoped to the processes THIS slice started (GH #872),
    // because the slices of one `cargo test` run in parallel by default
    // and other checkouts run their own DNA fixtures on the same box —
    // unscoped, the slice that finished first was blamed for a
    // neighbour's live organism.
    std::thread::sleep(std::time::Duration::from_secs(5));
    let left = leftover_processes(&tag);
    assert!(
        left.is_empty(),
        "processes this slice's DNA fixtures started are still running ({SLICE_TAG}={tag}), by the scratch root that owns them:\n{}",
        left.join("\n")
    );
}

macro_rules! fixture_slices {
    ($($name:ident => $i:expr),* $(,)?) => { $( #[test] fn $name() { run_fixture_slice($i); } )* };
}
fixture_slices! {
    dna_fixtures_slice_0 => 0, dna_fixtures_slice_1 => 1, dna_fixtures_slice_2 => 2, dna_fixtures_slice_3 => 3,
    dna_fixtures_slice_4 => 4, dna_fixtures_slice_5 => 5, dna_fixtures_slice_6 => 6, dna_fixtures_slice_7 => 7,
    dna_fixtures_slice_8 => 8, dna_fixtures_slice_9 => 9, dna_fixtures_slice_10 => 10, dna_fixtures_slice_11 => 11,
    dna_fixtures_slice_12 => 12, dna_fixtures_slice_13 => 13, dna_fixtures_slice_14 => 14, dna_fixtures_slice_15 => 15,
}

/// The slice's own knowledge database is reached by editing exactly the
/// database of the job's DSN: the user, password, host, port and query
/// string are what let a fixture connect at all, and CI's DSN carries
/// `?sslmode=disable`.
#[test]
fn a_slice_database_edits_only_the_database_of_the_dsn() {
    assert_eq!(
        dsn_with_database("postgres://dna:dna@127.0.0.1:5480/dna?sslmode=disable", "dna_7_s2_9").as_deref(),
        Some("postgres://dna:dna@127.0.0.1:5480/dna_7_s2_9?sslmode=disable")
    );
    assert_eq!(dsn_with_database("postgresql://dna@host/dna", "s").as_deref(), Some("postgresql://dna@host/s"));
    // No database to replace: the caller keeps the DSN it was given.
    assert_eq!(dsn_with_database("postgres://dna@host", "s"), None);
    assert_eq!(dsn_with_database("memory", "s"), None);
}

#[test]
fn the_slices_cover_every_fixture_once() {
    let files = fixture_files();
    let mut covered = 0;
    for slice in 0..SLICES {
        covered += files.iter().enumerate().filter(|(i, _)| i % SLICES == slice).count();
    }
    assert_eq!(covered, files.len(), "every fixture belongs to exactly one slice");
}

/// GH #872: the leftover-process guard blames only what its own slice
/// started. Two slice bodies' guards run here at the same moment in one
/// process: slice A has a process still running under its own scratch
/// root, slice B has left nothing, and a third process under a DNA root
/// of its own stands in for another checkout's run on the same box.
/// Only A is blamed, its message names A's root, and B passes.
///
/// Before the scoping every guard saw every `/tmp/dna-` process on the
/// box, so B failed for A's leak and both failed for the stranger's.
/// Cheap by construction: no fixture runs, the leak is a `sleep`.
#[cfg(target_os = "linux")]
#[test]
fn the_leftover_guard_blames_only_its_own_slice() {
    let pid = std::process::id();
    let tag_a = slice_tag(0);
    let tag_b = slice_tag(1);
    let tag_stranger = format!("another-checkouts-slice-{pid}");
    let root_a = format!("/tmp/dna-guard872-a-{pid}");
    let root_stranger = format!("/tmp/dna-guard872-stranger-{pid}");
    let mut running = Vec::new();
    for (root, tag) in [(&root_a, tag_a.as_str()), (&root_stranger, tag_stranger.as_str())] {
        std::fs::create_dir_all(root).expect("a scratch root for the guard's regression");
        running.push(
            Command::new("sleep")
                .arg("120")
                .current_dir(root)
                .env(SLICE_TAG, tag)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("leave a process running under a DNA scratch root"),
        );
    }
    let leaked_pid = running[0].id();
    // The stamp reaches a process at exec, not at fork: wait for BOTH
    // to be visible before either guard looks, so what this pins is the
    // scoping and not a race — a stranger who had not exec'd yet would
    // pass the guard for having no environment rather than for holding
    // someone else's stamp.
    for tag in [tag_a.as_str(), tag_stranger.as_str()] {
        for _ in 0..100 {
            if !leftover_processes(tag).is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    let (a, b) = (tag_a.clone(), tag_b.clone());
    let guard_a = std::thread::spawn(move || leftover_processes(&a));
    let guard_b = std::thread::spawn(move || leftover_processes(&b));
    let left_a = guard_a.join().expect("slice A's guard");
    let left_b = guard_b.join().expect("slice B's guard");

    // Reclaim before asserting: this test must not itself leave a
    // process under a DNA root for the next slice to trip over.
    for mut child in running {
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = std::fs::remove_dir_all(&root_a);
    let _ = std::fs::remove_dir_all(&root_stranger);

    assert_eq!(left_a.len(), 1, "slice A is blamed for its own leak and for nothing else, got:\n{}", left_a.join("\n"));
    assert!(left_a[0].contains(&root_a), "slice A's message names the scratch root that owns the leak, got:\n{}", left_a[0]);
    assert!(
        left_a[0].contains(&format!("pid {leaked_pid}")),
        "slice A's message names the process that leaked, got:\n{}",
        left_a[0]
    );
    assert!(
        left_b.is_empty(),
        "slice B left nothing running: neither slice A's leak nor a stranger's process may fail it, got:\n{}",
        left_b.join("\n")
    );
}

#[test]
fn dna_core_verifies_clean() {
    let dir = repo_root().join("dna/core");
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("verify")
        .arg(&dir)
        .output()
        .expect("invoke hale verify dna/core");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "hale verify dna/core must report zero findings.\nstdout:\n{}\nstderr:\n{}",
        stdout,
        stderr
    );
}

/// A suite that quietly emptied would pass the check above by
/// running nothing. #526 names eight programs; seven are Hale-native,
/// #528 adds the hosted-model adapter's, #529 the gateways' and
/// the source-editing Attempt's, #583 the budget's and the Anthropic
/// adapter's.
#[test]
fn dna_fixture_set_is_complete() {
    let dir = repo_root().join("dna/tests");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .expect("dna/tests exists")
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.ends_with("_test.hl").then_some(n)
        })
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "acceptance_binding_test.hl",
            "anthropic_messages_test.hl",
            "apply_test.hl",
            "assembly_test.hl",
            "b1_team_test.hl",
            "body_claim_expired_test.hl",
            "body_lease_blocked_test.hl",
            "body_lease_start_test.hl",
            "body_provision_script_test.hl",
            "body_scan_test.hl",
            "books_slice_test.hl",
            "budget_test.hl",
            "concern_identity_test.hl",
            "deployment_test.hl",
            "editing_test.hl",
            "effect_outcomes_test.hl",
            "embedded_provenance_test.hl",
            "extensions_test.hl",
            "fanout_join_test.hl",
            "grant_layering_test.hl",
            "grant_resources_test.hl",
            "handed_task_test.hl",
            "handoff_test.hl",
            "harness_test.hl",
            "journal_contention_test.hl",
            "journal_test.hl",
            "knowledge_context_test.hl",
            "knowledge_events_test.hl",
            "knowledge_store_test.hl",
            "knowledge_test.hl",
            "lease_epoch_test.hl",
            "ledger_test.hl",
            "membrane_loss_test.hl",
            "mutation_review_test.hl",
            "openai_chat_test.hl",
            "optimize_test.hl",
            "org_test.hl",
            "owners_test.hl",
            "ownership_test.hl",
            "performers_test.hl",
            "plan_routing_test.hl",
            "practice_test.hl",
            "principal_oidc_test.hl",
            "principal_test.hl",
            "prompt_receipt_test.hl",
            "receipt_retention_test.hl",
            "receipt_vault_test.hl",
            "record_test.hl",
            "recorded_model_test.hl",
            "recovery_association_test.hl",
            "recovery_two_memories_test.hl",
            "recursion_settlement_test.hl",
            "rehydrate_work_test.hl",
            "relay_repeated_request_test.hl",
            "retired_admission_test.hl",
            "review_authority_test.hl",
            "routed_attempt_identity_test.hl",
            "routing_test.hl",
            "schedule_cli_test.hl",
            "schedule_test.hl",
            "supersession_test.hl",
            "task_decide_test.hl",
            "task_evidence_test.hl",
            "two_heads_test.hl",
            "two_owners_test.hl",
            "verification_test.hl",
            "work_correlation_test.hl",
            "workflow_admission_contention_test.hl",
            "workflow_admission_test.hl",
            "workflow_attempt_test.hl",
            "workflow_children_test.hl",
            "workflow_definition_test.hl",
            "workflow_edit_outcomes_test.hl",
            "workflow_events_test.hl",
            "workflow_lifetime_dna_test.hl",
            "workflow_lifetime_test.hl",
            "workflow_projection_test.hl",
            "workflow_reconcile_test.hl",
            "workflow_recovery_test.hl",
            "workflow_release_type_wide_test.hl",
            "workflow_restore_test.hl",
            "workflow_retry_test.hl",
            "workflow_run_test.hl",
            "workflow_step_test.hl",
            "workspace_test.hl",
        ]
    );
}

/// GH #646 stage 0 (#649): the record has an API of its own, and only its
/// implementation spells the tool. In the core and the host, `git` is
/// invoked from exactly the record's implementation and the genome's own
/// files (the Structure): a new call site anywhere else is a boundary
/// crossed, and this fails the build until it goes behind `dna::Record`
/// (or `genome.hl` in the host).
#[test]
fn only_the_record_and_the_genome_spell_git() {
    let root = repo_root();
    let allowed = ["dna/core/record.hl", "dna/core/workspace.hl", "dna/core/verification.hl", "dna/core/org.hl", "dna/host/genome.hl"];
    let mut offenders = Vec::new();
    for dir in ["dna/core", "dna/host"] {
        for e in std::fs::read_dir(root.join(dir)).expect("dna dir").flatten() {
            let p = e.path();
            if p.extension().map(|x| x != "hl").unwrap_or(true) {
                continue;
            }
            let rel = format!("{dir}/{}", p.file_name().unwrap().to_string_lossy());
            let text = std::fs::read_to_string(&p).unwrap();
            let spells = text.lines().enumerate().filter(|(_, l)| {
                let l = l.trim_start();
                !l.starts_with("//") && (l.contains("run_tool(\"git") || l.contains("\"git\\n") || l.contains("\\ngit\\n") || l.contains("process::run(\"git"))
            });
            for (i, l) in spells {
                if !allowed.contains(&rel.as_str()) {
                    offenders.push(format!("{rel}:{}: {}", i + 1, l.trim()));
                }
            }
        }
    }
    assert!(offenders.is_empty(), "git is spelled outside the record's implementation and the genome's files:\n{}", offenders.join("\n"));
}

/// GH #647: a body's infrastructure and transport are implementations
/// behind the core's interfaces. In the host, `ssh`, `systemctl`,
/// `journalctl` and `docker compose` are invoked from exactly one file.
#[test]
fn only_the_reference_infrastructure_spells_its_tools() {
    let root = repo_root();
    let mut offenders = Vec::new();
    for e in std::fs::read_dir(root.join("dna/host")).expect("dna/host").flatten() {
        let p = e.path();
        if p.extension().map(|x| x != "hl").unwrap_or(true) || p.file_name().unwrap() == "infra.hl" {
            continue;
        }
        let text = std::fs::read_to_string(&p).unwrap();
        for (i, l) in text.lines().enumerate() {
            let t = l.trim_start();
            if t.starts_with("//") {
                continue;
            }
            if t.contains("ssh -o") || t.contains("systemctl --user") || t.contains("journalctl") || t.contains("docker\\ncompose") {
                offenders.push(format!("dna/host/{}:{}: {}", p.file_name().unwrap().to_string_lossy(), i + 1, t));
            }
        }
    }
    assert!(offenders.is_empty(), "a tool of the body's infrastructure is spelled outside dna/host/infra.hl:\n{}", offenders.join("\n"));
}

/// GH #648: the code-review host is an implementation behind `Forge`;
/// `gh` is invoked from exactly one file in the host.
#[test]
fn only_the_github_forge_spells_gh() {
    let root = repo_root();
    let mut offenders = Vec::new();
    for dir in ["dna/core", "dna/host"] {
        for e in std::fs::read_dir(root.join(dir)).expect("dna dir").flatten() {
            let p = e.path();
            if p.extension().map(|x| x != "hl").unwrap_or(true) || p.file_name().unwrap() == "forge_github.hl" {
                continue;
            }
            let text = std::fs::read_to_string(&p).unwrap();
            for (i, l) in text.lines().enumerate() {
                let t = l.trim_start();
                if !t.starts_with("//") && (t.contains("run_tool(\"gh") || t.contains("\"gh\\n")) {
                    offenders.push(format!("{dir}/{}:{}: {}", p.file_name().unwrap().to_string_lossy(), i + 1, t));
                }
            }
        }
    }
    assert!(offenders.is_empty(), "gh is spelled outside dna/host/forge_github.hl:\n{}", offenders.join("\n"));
}
