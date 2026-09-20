//! GH #816 — `LOTUS_NO_CHUNK_POOL`, the knob that lets the ASan
//! corpus oracle see an arena use-after-free.
//!
//! `lotus_arena_destroy` returns a dying arena's 64 KiB chunks to a
//! thread-local pool with their bytes intact, and the next
//! `lotus_arena_create` hands the same bytes back out. A load from a
//! destroyed arena therefore reads memory the process still owns:
//! ASan never saw a `free`, so it reported nothing. Four arena
//! use-after-frees (GH #710, #750, #711, #812) went through the ASan
//! corpus oracle undetected for exactly that reason — the #812 shape
//! produced no sanitizer output at all, only a wrong answer once a
//! second literal in the same statement claimed the recycled chunk.
//!
//! What this file pins is the knob itself, not any one of those bugs:
//! that recycling really stops, that an instrumented build turns it
//! off without being asked, and that the env var overrides the
//! compile-time default in both directions. Whether a *given*
//! use-after-free is then reported is the corpus oracle's job
//! (`corpus_oracle.rs`, ASan pass) — and the detection evidence for
//! #816 is in the PR: with PR #814's owner rule reverted, the #812
//! shape is a `heap-use-after-free` with both stacks under the knob
//! and silent without it.
//!
//! The measurement is the runtime's own `LOTUS_CHUNK_POOL_STATS`
//! dump, which reports per-thread `hits` (a chunk request served
//! from the pool), `misses` (served by `malloc`), `stores` (a
//! destroyed arena's chunk kept) and the pool's residual size at
//! exit. "Recycling is off" is exactly `hits == 0 && stores == 0 &&
//! pool_size == 0` with `misses > 0` — chunks were allocated, and
//! every one of them went back to libc.
//!
//! The instrumented build goes through `harness::build_asan`
//! (`BuildOptions::asan`, GH #843) — nothing here touches the
//! process environment; the knob itself is exercised on the CHILD's
//! env via `Command::env`.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

/// An allocating program: a `@form(vec)` child grown a chunk's worth
/// of rows, a String field rebuilt per call, and a fn-level scratch
/// that opens and closes on every iteration. Enough arena traffic
/// that the pool is exercised in both directions (chunks handed out
/// AND chunks returned by a destroy), and its printed answers are
/// checked so a run that allocates differently can't pass silently.
const PROGRAM: &str = r#"
    type Row {
        v: Int = 0;
    }

    @form(vec)
    locus Rows {
        capacity {
            heap rows of Row;
        }
    }

    locus Tally {
        params {
            tag: String = "t";
            seen: Rows = Rows { };
        }
        fn add(n: Int) -> Int {
            self.seen.push(Row { v: n });
            self.tag = "t" + to_string(n);
            return self.seen.len();
        }
    }

    // A fn frame that allocates and returns: its scratch arena is
    // created and destroyed on every call, which is the churn the
    // chunk pool exists to serve.
    fn churn(n: Int) -> Int {
        let mut s = "";
        let mut i = 0;
        while i < n {
            s = s + "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx" + to_string(i);
            i = i + 1;
        }
        return len(s);
    }

    fn main() {
        let t = Tally { };
        let mut i = 0;
        let mut rows = 0;
        let mut total = 0;
        while i < 64 {
            rows = t.add(i);
            total = total + churn(32);
            i = i + 1;
        }
        println("rows=", rows);
        println("churn=", total);
        println("tag=", t.tag);
    }
"#;

/// The four per-thread counters out of one `LOTUS_CHUNK_POOL_STATS`
/// line: `[chunk_pool main-thread tid=N] hits=.. misses=.. stores=..
/// overflows=.. pool_size=..`.
#[derive(Debug)]
struct PoolStats {
    hits: u64,
    misses: u64,
    stores: u64,
    pool_size: i64,
}

fn field(line: &str, key: &str) -> Option<i64> {
    let at = line.find(&format!("{key}="))? + key.len() + 1;
    let rest = &line[at..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit() && c != '-')
        .unwrap_or(rest.len());
    rest[..end].parse::<i64>().ok()
}

/// Run `bin` with the pool diagnostic on plus `extra` env, check the
/// program's own answers, and return the MAIN thread's counters. The
/// dump is an `atexit` hook, so it only appears on a normal exit —
/// which is itself part of what is being asserted.
fn stats_for(bin: &std::path::Path, extra: &[(&str, &str)]) -> PoolStats {
    let mut cmd = Command::new(bin);
    cmd.env("LOTUS_CHUNK_POOL_STATS", "1")
        // Leak detection is not what this file measures, and an
        // unrelated leak would turn a counter assertion into a
        // confusing non-zero exit.
        .env("ASAN_OPTIONS", "detect_leaks=0");
    for (k, v) in extra {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run program");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let label = format!("{extra:?}");
    assert!(
        out.status.success(),
        "{label}: non-zero exit {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        out.status
    );
    // The answers have to stay right whichever allocator is in play:
    // 64 pushes, and a String rebuilt from the last index.
    assert!(
        stdout.contains("rows=64"),
        "{label}: wrong row count\nstdout:\n{stdout}"
    );
    assert!(
        stdout.contains("tag=t63"),
        "{label}: wrong tag\nstdout:\n{stdout}"
    );
    let line = stderr
        .lines()
        .find(|l| l.contains("[chunk_pool main-thread"))
        .unwrap_or_else(|| {
            panic!("{label}: no main-thread chunk_pool line\nstderr:\n{stderr}")
        });
    PoolStats {
        hits: field(line, "hits").expect("hits") as u64,
        misses: field(line, "misses").expect("misses") as u64,
        stores: field(line, "stores").expect("stores") as u64,
        pool_size: field(line, "pool_size").expect("pool_size"),
    }
}

fn assert_recycling_off(what: &str, s: &PoolStats) {
    assert_eq!(
        s.hits, 0,
        "{what}: a chunk request was served from the pool — recycled \
         bytes are handed back out and ASan cannot see the free ({s:?})"
    );
    assert_eq!(
        s.stores, 0,
        "{what}: a destroyed arena's chunk was kept instead of freed — \
         this is the masking #816 removes ({s:?})"
    );
    assert_eq!(
        s.pool_size, 0,
        "{what}: the pool holds chunks at exit; with recycling off it \
         is never touched ({s:?})"
    );
    assert!(
        s.misses > 0,
        "{what}: no chunk was allocated at all, so the run proves \
         nothing about recycling ({s:?})"
    );
}

#[test]
fn no_chunk_pool_really_stops_recycling_and_asan_defaults_it_on() {
    let program = hale_syntax::parse_source(PROGRAM).expect("parse");

    // --- ordinary build -------------------------------------------
    let plain = harness::unique_bin("no_chunk_pool_plain");
    build_executable(&program, &plain).expect("build plain");

    // Default: the pool recycles. The prefill alone guarantees the
    // first default-sized request is a hit and that chunks are
    // resident at exit, so this is a real measurement of the state
    // #816 is about, not an artifact of how much the program churns.
    let on = stats_for(&plain, &[]);
    assert!(
        on.hits > 0 && on.pool_size > 0,
        "the default build should recycle chunks — if it no longer \
         does, the knob below proves nothing ({on:?})"
    );

    // The knob, on an ordinary build: every chunk goes back to libc.
    let off = stats_for(&plain, &[("LOTUS_NO_CHUNK_POOL", "1")]);
    assert_recycling_off("LOTUS_NO_CHUNK_POOL=1 on a plain build", &off);
    let _ = std::fs::remove_file(&plain);

    // --- instrumented build ---------------------------------------
    // The sanitizer cflags carry -DLOTUS_NO_CHUNK_POOL_DEFAULT=1, so
    // an ASan binary starts with recycling off and no harness has to
    // remember to ask. This is the half that keeps the corpus
    // oracle's guarantee from quietly lapsing.
    let asan = harness::unique_bin("no_chunk_pool_asan");
    // `BuildOptions::asan` through the harness — no test mutates the
    // process environment (GH #843), and the helper checks the
    // artifact really carries the ASan runtime.
    harness::build_asan(&program, &asan);

    let asan_default = stats_for(&asan, &[]);
    assert_recycling_off("an ASan build with no env set", &asan_default);

    // ... and the env var overrides the compile-time default the
    // other way, for a run that wants the pooled allocator under the
    // sanitizer (comparing a suspected bug against its masked form).
    let asan_opt_out = stats_for(&asan, &[("LOTUS_NO_CHUNK_POOL", "0")]);
    assert!(
        asan_opt_out.hits > 0 && asan_opt_out.pool_size > 0,
        "LOTUS_NO_CHUNK_POOL=0 should restore recycling under ASan \
         ({asan_opt_out:?})"
    );
    let _ = std::fs::remove_file(&asan);
}
