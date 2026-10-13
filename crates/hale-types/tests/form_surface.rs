//! The form surface the checker types (friction sweep 2026-10-12): a
//! `@form(vec)`'s `clear` / `truncate`, a `@form(lru_cache)`'s `remove`
//! and `on_evict` hook.

#[path = "support/entries.rs"]
mod entries;
use entries::check_program;
use hale_syntax::parse_source;

fn messages(src: &str) -> Vec<String> {
    let prog = parse_source(src).expect("parse");
    check_program(&prog).into_iter().map(|d| d.message).collect()
}

fn assert_clean(src: &str) {
    let m = messages(src);
    assert!(m.is_empty(), "expected a clean check, got: {m:?}");
}

fn assert_error(src: &str, needle: &str) {
    let m = messages(src);
    assert!(m.iter().any(|d| d.contains(needle)), "expected a diagnostic containing {needle:?}, got: {m:?}");
}

#[test]
fn vec_clear_and_truncate_are_typed() {
    let vec = "@form(vec) locus V { capacity { heap xs of String; } }\n";
    assert_clean(&format!("{vec}fn main() {{ let v = V {{ }}; v.push(\"a\"); v.truncate(0); v.clear(); }}\n"));
    assert_error(&format!("{vec}fn main() {{ let v = V {{ }}; v.truncate(\"x\"); }}\n"), "expected `Int`, got `String`");
    assert_error(&format!("{vec}fn main() {{ let v = V {{ }}; let n = v.clear(); println(n + 1); }}\n"), "");
}

const LRU: &str = "type Conn { host: String = \"\"; fd: Int = -1; }\n";

#[test]
fn lru_remove_returns_the_value_and_is_fallible() {
    assert_clean(&format!(
        "{LRU}@form(lru_cache, cap = 2) locus P {{ capacity {{ pool cs of Conn indexed_by host; }} }}\n\
         fn main() {{ let p = P {{ }}; p.put(Conn {{ host: \"a\" }}); let c = p.remove(\"a\") or Conn {{ }}; println(c.fd); }}\n"
    ));
    assert_error(
        &format!(
            "{LRU}@form(lru_cache, cap = 2) locus P {{ capacity {{ pool cs of Conn indexed_by host; }} }}\n\
             fn main() {{ let p = P {{ }}; let c = p.remove(\"a\"); }}\n"
        ),
        "`p.remove` can fail (KeyError)",
    );
}

#[test]
fn lru_on_evict_with_the_hook_signature_is_accepted() {
    assert_clean(&format!(
        "{LRU}@form(lru_cache, cap = 2) locus P {{\n\
             capacity {{ pool cs of Conn indexed_by host; }}\n\
             fn on_evict(k: String, v: Conn) {{ let _r = std::io::tcp::close_fd(v.fd); }}\n\
         }}\n\
         fn main() {{ let p = P {{ }}; p.put(Conn {{ host: \"a\", fd: 3 }}); }}\n"
    ));
}

#[test]
fn lru_on_evict_with_another_signature_is_an_error() {
    for hook in [
        "fn on_evict(k: Int, v: Conn) { }",
        "fn on_evict(v: Conn) { }",
        "fn on_evict(k: String, v: Conn) -> Int { return 0; }",
        "fn on_evict(k: String, v: String) { }",
    ] {
        assert_error(
            &format!(
                "{LRU}@form(lru_cache, cap = 2) locus P {{ capacity {{ pool cs of Conn indexed_by host; }} {hook} }}\n\
                 fn main() {{ let p = P {{ }}; p.put(Conn {{ host: \"a\" }}); }}\n"
            ),
            "is the eviction hook of `@form(lru_cache)` and must be `fn on_evict(k: String, v: Conn)`",
        );
    }
}

/// The hook runs inside `put`, so what it does is what the `put`'s
/// caller reaches: a `@no_syscall` fn that puts into a cache whose hook
/// closes an fd is refused, naming the hook on the path.
#[test]
fn a_put_reaches_what_the_eviction_hook_does() {
    assert_error(
        &format!(
            "{LRU}@form(lru_cache, cap = 2) locus P {{\n\
                 capacity {{ pool cs of Conn indexed_by host; }}\n\
                 fn on_evict(k: String, v: Conn) {{ let _r = std::io::tcp::close_fd(v.fd); }}\n\
             }}\n\
             @no_syscall\n\
             fn add(p: P, h: String) {{ p.put(Conn {{ host: h }}); }}\n\
             fn main() {{ let p = P {{ }}; add(p, \"a\"); }}\n"
        ),
        "reaches add -> P::on_evict",
    );
    // Without the hook the same put reaches nothing.
    assert_clean(&format!(
        "{LRU}@form(lru_cache, cap = 2) locus P {{ capacity {{ pool cs of Conn indexed_by host; }} }}\n\
         @no_syscall\n\
         fn add(p: P, h: String) {{ p.put(Conn {{ host: h }}); }}\n\
         fn main() {{ let p = P {{ }}; add(p, \"a\"); }}\n"
    ));
}
