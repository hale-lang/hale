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

const JOB: &str = "type Job { name: String = \"\"; pri: Int = 0; tags: Bytes = b\"\"; }\n";

#[test]
fn heap_surface_is_typed() {
    assert_clean(&format!(
        "{JOB}@form(heap) locus Q {{ capacity {{ heap jobs of Job ordered_by min pri; }} }}\n\
         fn main() {{\n\
             let q = Q {{ }};\n\
             q.push(Job {{ pri: 2 }});\n\
             let a = q.peek() or Job {{ }};\n\
             let b = q.pop() or Job {{ }};\n\
             println(a.pri + b.pri + q.len());\n\
             if q.is_empty() {{ q.clear(); }}\n\
         }}\n"
    ));
    // pop and peek are fallible (EmptyError); a vec's index surface is absent.
    assert_error(
        &format!("{JOB}@form(heap) locus Q {{ capacity {{ heap jobs of Job ordered_by max pri; }} }}\nfn main() {{ let q = Q {{ }}; let a = q.pop(); }}\n"),
        "`q.pop` can fail (EmptyError)",
    );
    assert_error(
        &format!("{JOB}@form(heap) locus Q {{ capacity {{ heap jobs of Job ordered_by max pri; }} }}\nfn main() {{ let q = Q {{ }}; let a = q.get(0) or Job {{ }}; }}\n"),
        "no field `get` on `Q`",
    );
}

#[test]
fn heap_shape_errors() {
    for (slot, needle) in [
        ("heap jobs of Job;", "must declare `ordered_by min <field>` or `ordered_by max <field>`"),
        ("heap jobs of Job ordered_by min tags;", "the `ordered_by` field must be an `Int`, `Float`, `String`, `Time` or `Duration`"),
        ("heap jobs of Job ordered_by max nope;", "cell type `Job` has no field `nope` to be ordered by"),
        ("pool jobs of Job ordered_by max pri;", "got `pool jobs of ...`"),
        ("heap jobs of Int ordered_by max pri;", "cell type must be a user-declared record"),
        ("heap jobs of Job indexed_by name ordered_by min pri;", "drop `indexed_by`"),
    ] {
        assert_error(
            &format!("{JOB}@form(heap) locus Q {{ capacity {{ {slot} }} }}\nfn main() {{ let q = Q {{ }}; }}\n"),
            needle,
        );
    }
    assert_error(
        &format!("{JOB}@form(heap, cap = 3) locus Q {{ capacity {{ heap jobs of Job ordered_by min pri; }} }}\nfn main() {{ }}\n"),
        "@form(heap) takes no arguments",
    );
}

#[test]
fn ordered_by_outside_a_heap_is_an_error() {
    for decl in [
        "@form(vec) locus Q { capacity { heap jobs of Job ordered_by max pri; } }",
        "locus Q { capacity { heap jobs of Job ordered_by max pri; } }",
    ] {
        assert_error(
            &format!("{JOB}{decl}\nfn main() {{ let q = Q {{ }}; }}\n"),
            "`ordered_by` is the slot clause of `@form(heap)`",
        );
    }
}

#[test]
fn ordered_by_names_min_or_max() {
    let err = parse_source(&format!(
        "{JOB}@form(heap) locus Q {{ capacity {{ heap jobs of Job ordered_by up pri; }} }}\nfn main() {{ }}\n"
    ))
    .expect_err("`up` is not a direction");
    assert!(
        err.iter().any(|d| d.message.contains("expected `min` or `max` after `ordered_by`")),
        "{:?}",
        err.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}
