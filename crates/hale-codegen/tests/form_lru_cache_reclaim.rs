//! `@form(lru_cache)` `remove(k)`, the `on_evict(k, v)` hook, and cell
//! reclamation (friction sweep 2026-10-12, a downstream handoff: the
//! http Client's connection pool hand-rolled an LRU because the form had
//! no take-semantics and no eviction hook, so an fd-owning cell could
//! not be closed when the cache dropped it).
//!
//! - `remove(k)` takes the entry out: the caller gets its own copy, a
//!   miss is a `KeyError`.
//! - a locus that declares `fn on_evict(k: K, v: S)` has it called with
//!   each entry `put` evicts over `cap`, before the cell is reclaimed.
//! - a replaced, evicted or removed cell's blocks (its String and Bytes
//!   fields, nested records at any depth) retire, deferred to the
//!   activation boundary as a `@form(hashmap)` cell's do. Before, the
//!   cache reclaimed nothing: every put of a new value grew its arena
//!   for good.
//!
//! The churn shapes run 100 cycles, dump arena residency, run to
//! 100 000 and dump again: the cache's arena must hold the same bytes.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn run(name: &str, src: &str) -> String {
    let bin = harness::unique_bin(&format!("hale_lru_reclaim_{name}"));
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{name}: non-zero {:?}; stdout: {stdout}; stderr: {stderr}", out.status);
    stdout
}

/// An fd-owning cell closed on eviction: each of 1000 puts opens a
/// listening socket, the cache (cap 4) evicts 996 of them, and the hook
/// closes each — so the descriptors are reused and the highest one
/// stays small. The hook sees the evicted entry's key and value; a
/// `remove` hands the entry back (its fd still open) and a miss fails.
#[test]
fn on_evict_closes_an_fd_owning_cell_and_remove_takes_an_entry() {
    let out = run(
        "fd",
        r#"
        type Conn { host: String = ""; fd: Int = -1; note: String = ""; }
        @form(lru_cache, cap = 4)
        locus Pool {
            capacity { pool conns of Conn indexed_by host; }
            params { closed: Int = 0; bad: Int = 0; last: String = ""; }
            fn on_evict(k: String, v: Conn) {
                if k != v.host { self.bad = self.bad + 1; }
                if std::io::tcp::close_fd(v.fd) != 0 { self.bad = self.bad + 1; }
                self.closed = self.closed + 1;
                self.last = v.note;
            }
        }
        fn main() {
            let p = Pool { };
            let mut i = 0;
            let mut maxfd = 0;
            while i < 1000 {
                let fd = std::io::tcp::listen_socket("127.0.0.1", 0) or -1;
                if fd > maxfd { maxfd = fd; }
                let touch = p.get("h" + to_string(i - 3)) or Conn { };
                p.put(Conn { host: "h" + to_string(i), fd: fd, note: "n" + to_string(i) });
                i = i + 1;
            }
            let c = p.remove("h998") or Conn { host: "none" };
            let miss = p.remove("h1") or Conn { host: "miss" };
            let closed = std::io::tcp::close_fd(c.fd);
            println(p.len(), " ", p.closed, " ", p.bad, " ", maxfd < 40, " ", p.last, " ",
                c.host, c.note, " ", closed, " ", miss.host, " ", p.contains("h998"), " ", p.contains("h997"));
        }
        "#,
    );
    assert_eq!(out.trim(), "3 996 0 true n994 h998n998 0 miss false true");
}

/// A put over a present key replaces without the hook (the caller that
/// replaces a value owns what the old one held); `remove` does not call
/// it either.
#[test]
fn replace_and_remove_do_not_call_the_hook() {
    let out = run(
        "nohook",
        r#"
        type E { k: Int = 0; s: String = ""; }
        @form(lru_cache, cap = 2)
        locus C {
            capacity { pool es of E indexed_by k; }
            params { evicted: String = ""; }
            fn on_evict(k: Int, v: E) { self.evicted = self.evicted + to_string(k) + v.s + ","; }
        }
        fn main() {
            let c = C { };
            c.put(E { k: 1, s: "a" });
            c.put(E { k: 1, s: "b" });
            c.put(E { k: 2, s: "c" });
            let r = c.remove(2) or E { };
            c.put(E { k: 3, s: "d" });
            c.put(E { k: 4, s: "e" });
            c.put(E { k: 5, s: "f" });
            let g = c.get(5) or E { };
            println(c.evicted, " ", r.s, " ", g.s, " ", c.len());
        }
        "#,
    );
    assert_eq!(out.trim(), "1b,3d, c f 2");
}

/// The resident bytes of the arena labelled `label`, per residency
/// dump, in order.
fn arena_bytes(stderr: &str, label: &str) -> Vec<u64> {
    let tag = format!("label={label}]");
    stderr
        .lines()
        .filter(|l| l.contains(&tag))
        .map(|l| {
            l.split_whitespace()
                .find_map(|w| w.strip_prefix("bytes="))
                .and_then(|v| v.parse().ok())
                .expect("a residency row carries bytes=N")
        })
        .collect()
}

/// `decl` declares the `@form(lru_cache)` locus `M`; `body` is one cycle
/// of `H.step(x, b)` over `self.m`, with `self.n` counting cycles.
fn assert_cache_flat(name: &str, decl: &str, body: &str) {
    let src = format!(
        r#"
        type In {{ s: String = ""; b: Bytes = b""; }}
        type Cell {{ id: Int = 0; s: String = ""; b: Bytes = b""; inner: In = In {{ }}; }}
        type Named {{ key: String = ""; v: String = ""; }}
        {decl}
        locus H {{
            params {{ m: M = M {{ }}; n: Int = 0; }}
            fn step(x: String, b: Bytes) {{ self.n = self.n + 1; {body} }}
        }}
        fn main() {{
            let h = H {{ }};
            let x = std::str::upper("some string of forty characters or so...");
            let b = std::bytes::from_string(x);
            let mut i = 0;
            while i < 100 {{ h.step(x, b); i = i + 1; }}
            let _warm = std::process::dump_arena_residency();
            while i < 100000 {{ h.step(x, b); i = i + 1; }}
            let _late = std::process::dump_arena_residency();
        }}
        "#
    );
    let bin = harness::unique_bin(&format!("hale_lru_reclaim_flat_{name}"));
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin)
        .env("LOTUS_ARENA_RESIDENCY", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{name}: non-zero {:?}; stderr: {stderr}", out.status);
    let dumps = arena_bytes(&stderr, "M");
    assert!(dumps.len() >= 2, "{name}: expected two residency dumps; stderr: {stderr}");
    assert!(dumps[0] > 0, "{name}: the cache's arena holds nothing; stderr: {stderr}");
    assert_eq!(
        dumps[0], dumps[1],
        "{name}: the cache's arena grew between 100 and 100000 cycles; stderr: {stderr}"
    );
}

const CACHE: &str = "@form(lru_cache, cap = 4) locus M { capacity { pool cells of Cell indexed_by id; } }";

#[test]
fn eviction_churn_stays_flat() {
    assert_cache_flat(
        "evict",
        CACHE,
        "self.m.put(Cell { id: self.n, s: x, b: b, inner: In { s: x, b: b } });",
    );
}

#[test]
fn replace_churn_stays_flat() {
    assert_cache_flat(
        "replace",
        CACHE,
        "self.m.put(Cell { id: self.n % 4, s: x, b: b, inner: In { s: x, b: b } });",
    );
}

#[test]
fn remove_churn_stays_flat() {
    assert_cache_flat(
        "remove",
        CACHE,
        "self.m.put(Cell { id: self.n % 3, s: x, inner: In { s: x, b: b } }); \
         let r = self.m.remove((self.n + 1) % 3) or Cell { };",
    );
}

#[test]
fn get_modify_put_stays_flat() {
    assert_cache_flat(
        "rmw",
        CACHE,
        "let c = self.m.get(self.n % 4) or Cell { id: self.n % 4, s: x, inner: In { s: x } }; \
         self.m.put(Cell { id: c.id, s: c.s, b: b, inner: c.inner });",
    );
}

#[test]
fn eviction_through_the_hook_stays_flat() {
    assert_cache_flat(
        "hook",
        "@form(lru_cache, cap = 4) locus M { \
             capacity { pool cells of Named indexed_by key; } \
             params { seen: Int = 0; } \
             fn on_evict(k: String, v: Named) { self.seen = self.seen + len(k) + len(v.v); } \
         }",
        "self.m.put(Named { key: \"k\" + to_string(self.n), v: x });",
    );
}

/// Values read out of the cache — a `get` copy held across the put that
/// evicts or replaces its entry, a value the hook stores, a removed
/// entry kept in a field — stay whole while the cache reclaims the
/// cells they came from, under ASan.
#[test]
fn values_outlive_their_cells() {
    let src = r#"
        type In { s: String = ""; b: Bytes = b""; }
        type Cell { id: Int = 0; s: String = ""; inner: In = In { }; }
        @form(lru_cache, cap = 3)
        locus M {
            capacity { pool cells of Cell indexed_by id; }
            params { last: Cell = Cell { }; evicted: Int = 0; }
            fn on_evict(k: Int, v: Cell) {
                self.last = v;
                self.evicted = self.evicted + len(v.s) + len(v.inner.s) + len(v.inner.b);
            }
        }
        locus H {
            params { m: M = M { }; kept: Cell = Cell { }; seen: Int = 0; }
            fn make(i: Int, x: Bytes) -> Cell {
                return Cell { id: i % 7, s: "s" + to_string(i), inner: In { s: "i" + to_string(i), b: x } };
            }
            fn churn(i: Int, x: Bytes) {
                let g = self.m.get((i + 6) % 7) or Cell { s: "-" };
                self.m.put(self.make(i, x));
                self.m.put(self.make(i + 1, x));
                self.m.put(self.make(i + 2, x));
                let r = self.m.remove((i + 1) % 7) or Cell { s: "-" };
                if i % 3 == 0 { self.kept = r; }
                self.seen = self.seen + len(g.s) + len(g.inner.s) + len(r.s) + len(r.inner.b);
            }
        }
        fn main() {
            let h = H { };
            let x = std::bytes::from_string("payload-bytes");
            let mut i = 0;
            while i < 3000 { h.churn(i, x); i = i + 1; }
            println("kept=", h.kept.s, h.kept.inner.s, " last=", h.m.last.s, " ev=", h.m.evicted, " seen=", h.seen);
        }
    "#;
    let bin = harness::unique_bin("hale_lru_reclaim_outlive");
    harness::build_source_asan(src, &bin);
    let out = Command::new(&bin)
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "non-zero {:?}; stdout: {stdout}; stderr: {stderr}", out.status);
    assert!(stdout.contains("kept=s2998i2998 last=s2998 ev=66757 seen=80664"), "stdout: {stdout}; stderr: {stderr}");
}
