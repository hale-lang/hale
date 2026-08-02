//! Every shape a certified fn can reach an effect through.
//!
//! The effect system's holes have all been the same kind: a route the
//! callgraph did not walk, so an assertion passed over a real effect.
//! v0.12.0 closed four (handles, seed boundaries, interface slots,
//! absent frontier rows); later work closed sync-form access. This is
//! the standing sweep, so the next one is found by a test rather than
//! by a downstream report.

use hale_syntax::parse_source;

fn violates(src: &str) -> bool {
    let program = parse_source(src).expect("parse");
    hale_types::check_program(&program).iter().any(|d| {
        d.message.contains("effect assertion violated")
            || d.message.contains("causal set violated")
    })
}

macro_rules! reaches {
    ($name:ident, $src:expr) => {
        #[test]
        fn $name() {
            assert!(
                violates($src),
                "an effect reached through this shape must be caught"
            );
        }
    };
}

reaches!(direct_stdlib_call, "@no_syscall\n\
    fn f() -> Int { return len(std::io::fs::read_file(\"/x\") or \"\"); }\n\
    fn main() { println(f()); }");

reaches!(through_a_free_fn, "@no_syscall\n\
    fn f() -> Int { return g(); }\n\
    fn g() -> Int { return len(std::io::fs::read_file(\"/x\") or \"\"); }\n\
    fn main() { println(f()); }");

reaches!(through_a_handle, "locus R { params { p: String = \"/x\"; }\n\
      fn slurp() -> Int { return len(std::io::fs::read_file(self.p) or \"\"); } }\n\
    @no_syscall\n\
    fn f(r: R) -> Int { return r.slurp(); }\n\
    fn main() { let r = R { }; println(f(r)); }");

reaches!(through_self_method, "locus R { params { p: String = \"/x\"; }\n\
      fn slurp() -> Int { return len(std::io::fs::read_file(self.p) or \"\"); }\n\
      @no_syscall fn certified() -> Int { return self.slurp(); } }\n\
    main locus App { params { r: R = R { }; } birth() { println(self.r.certified()); } }\n\
    fn main() { App { }; }");

reaches!(through_an_interface_slot, "interface E { fn emit() -> Int; }\n\
    locus Loud { params { n: Int = 0; } fn emit() -> Int { println(\"x\"); return 1; } }\n\
    locus U { params { s: E = Loud { }; } fn go() -> Int { return self.s.emit(); } }\n\
    @no_syscall\n\
    fn f(u: U) -> Int { return u.go(); }\n\
    fn main() { let u = U { }; println(f(u)); }");

/// An UNREGISTERED `std::` path must fail closed. Absent and
/// unclassified are the same claim: the compiler cannot vouch for it.
reaches!(absent_frontier_row, "@no_syscall\n\
    fn f() -> Int { return std::nowhere::mystery(1); }\n\
    fn main() { println(f()); }");

reaches!(an_ffi_leaf, "@ffi(\"c\") fn raw_thing() -> Int;\n\
    @no_ffi\n\
    fn f() -> Int { return raw_thing(); }\n\
    fn main() { println(f()); }");

reaches!(two_hops_through_two_loci, "locus A { params { n: Int = 0; }\n\
      fn deep() -> Int { return len(std::io::fs::read_file(\"/x\") or \"\"); } }\n\
    locus B { params { a: A = A { }; } fn mid() -> Int { return self.a.deep(); } }\n\
    locus C { params { b: B = B { }; }\n\
      @no_syscall fn top() -> Int { return self.b.mid(); } }\n\
    main locus App { params { c: C = C { }; } birth() { println(self.c.top()); } }\n\
    fn main() { App { }; }");

reaches!(a_recursive_cycle, "@no_syscall\n\
    fn f(n: Int) -> Int { if n <= 0 { return len(std::io::fs::read_file(\"/x\") or \"\"); }\n\
      return f(n - 1); }\n\
    fn main() { println(f(3)); }");

/// A `mode` body was never collected into the callgraph, so its
/// callees were invisible and an assertion passed straight through
/// one. Modes are called like methods, so they key the same way.
reaches!(through_a_mode, "locus L { params { s: Float = 1.0; }\n\
      mode bulk() -> Float { println(\"io\"); return self.s; }\n\
      @no_syscall fn c() -> Float { return self.bulk(); } }\n\
    main locus App { params { l: L = L { }; } birth() { println(self.l.c()); } }\n\
    fn main() { App { }; }");

/// Through the BUS, which a call graph alone cannot follow — the
/// reason `causes:` exists.
reaches!(through_a_bus_subscriber, "type P { n: Int; }\n\
    topic T { payload: P; }\n\
    locus Sub { bus { subscribe T as on_t; } params { n: Int = 0; }\n\
      fn on_t(p: P) { println(\"side effect\"); } }\n\
    locus Pub { bus { publish T; }\n\
      @effects(causes: {}) fn go() { T <- P { n: 1 }; } }\n\
    main locus App { params { s: Sub = Sub { }; p: Pub = Pub { }; } }\n\
    fn main() { App { }; }");

/// A `sync`-bearing form takes a lock, and placement is not static —
/// so contention cannot be ruled out at compile time.
reaches!(a_sync_bearing_form, "type E { k: Int; v: Int; }\n\
    @form(hashmap, sync = serialized)\n\
    locus C { capacity { pool entries of E indexed_by k; } }\n\
    locus W { params { c: C = C { }; } @no_block fn hot(e: E) { self.c.set(e); } }\n\
    main locus App { params { w: W = W { }; } }\n\
    fn main() { App { }; }");

/// The control. Without it, an over-broad attribution would satisfy
/// every test above while certifying nothing.
#[test]
fn a_genuinely_pure_path_still_certifies() {
    assert!(
        !violates(
            "fn helper(a: Int, b: Int) -> Int { return a + b; }\n\
             @no_syscall @no_block @deterministic @no_ffi\n\
             fn f(x: Int) -> Int { return helper(x, 1); }\n\
             fn main() { println(f(1)); }"
        ),
        "a pure path must stay certifiable"
    );
}
