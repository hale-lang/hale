//! GH #711 + GH #812 — an unowned locus literal in EXPRESSION
//! position must outlive the expression that needs it.
//!
//! A locus literal nothing binds used to take the eager path:
//! `lower_locus_instantiation` emitted drain → dissolve →
//! arena_destroy at the end of the LITERAL, before the expression
//! consuming it ran. GH #710 (PR #743) gave the RECEIVER position an
//! owner and left the other two positions alone:
//!
//!   * ARGUMENT (GH #711, a downstream handoff): a provider passed
//!     inline through an interface-typed parameter to a callee that
//!     RETAINS it — a server built on it that lives on — died by a
//!     signal at the provider's first retained write. Birth and the
//!     early reads had already succeeded; the storage was gone by
//!     the time the retained write happened. Naming the provider
//!     with `let` first answered.
//!   * FIELD (GH #812): `Holder { tag: "hello" }.tag` destroyed the
//!     literal and THEN GEP'd and loaded the field. Benign only
//!     because `lotus_arena_destroy` returns chunks to the pool with
//!     their bytes intact — a second literal in the same statement
//!     recycles the chunk and the first read answers with the
//!     second's string. A silent wrong answer, no signal.
//!
//! One rule now covers all three: an unowned locus literal in any
//! expression position is owned by the enclosing fn's scope, exactly
//! as `let` owns its RHS. A bare `LocusName { ... };` STATEMENT is
//! unaffected — it never goes through expression lowering — and keeps
//! its fire-and-forget teardown, which `bare_statement_literal_*`
//! below pins.
//!
//! Each shape asserts against its let-bound control: the two
//! spellings have to be the same program. `dissolve()` printing its
//! tag makes the teardown POINT observable on stdout, so the
//! statement-position half bites without a sanitizer too.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

/// Every program here answers in milliseconds. The deadline is not
/// about slow machines: with the eager teardown restored, the
/// retained-argument shape did not merely crash — the corrupted
/// `@form(vec)` header sent it spinning, and a bare
/// `Command::output()` would hang a CI job instead of failing it.
const DEADLINE: Duration = Duration::from_secs(60);

fn build_and_run(tag: &str, src: &str) -> (String, String) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("unowned_lit_{}", tag));
    build_executable(&program, &bin).expect("build");
    let mut child = Command::new(&bin)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("run");
    let start = Instant::now();
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(s) => break Some(s),
            None if start.elapsed() > DEADLINE => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    let mut out = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        use std::io::Read;
        let mut buf = Vec::new();
        let _ = pipe.read_to_end(&mut buf);
        out = String::from_utf8_lossy(&buf).to_string();
    }
    let _ = std::fs::remove_file(&bin);
    let verdict = match status {
        Some(s) if s.success() => String::new(),
        Some(s) => format!("exited {:?}", s),
        None => format!("did not finish within {:?}", DEADLINE),
    };
    (out, verdict)
}

fn dump_ir(tag: &str, src: &str) -> String {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin: PathBuf = harness::unique_bin(&format!("unowned_lit_ir_{}", tag));
    let ir = bin.with_extension("ll");
    std::env::set_var("LOTUS_DUMP_IR", "1");
    let result = build_executable(&program, &bin);
    std::env::remove_var("LOTUS_DUMP_IR");
    result.expect("build");
    let text = std::fs::read_to_string(&ir).expect("read IR");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&ir);
    text
}

/// The `main` body, from its `define` to the first `\n}` after it.
fn carve_main(ir: &str) -> &str {
    let start = ir
        .find("define i32 @main(")
        .expect("`main` not defined in IR");
    let end = ir[start..]
        .find("\n}")
        .map(|i| start + i)
        .unwrap_or(ir.len());
    &ir[start..end]
}

/// An interface, a provider whose `submit` writes to a `@form(vec)`
/// child, and a server that retains the provider for its whole life.
/// The vec child is what made the corruption fatal rather than merely
/// wrong: its buffer goes back with the provider's arena.
const RETAINING_PRELUDE: &str = r#"
    interface Commands {
        fn submit(n: Int) -> Int;
    }

    type Row { v: Int = 0; }

    @form(vec)
    locus Rows {
        capacity { heap rows of Row; }
    }

    locus Provider {
        params {
            tag: String = "p";
            seen: Rows = Rows { };
        }
        fn submit(n: Int) -> Int {
            self.seen.push(Row { v: n });
            return self.seen.len() + len(self.tag);
        }
    }

    locus Server {
        params {
            commands: Commands = Provider { };
            port: Int = 0;
        }
        fn handle(n: Int) -> Int {
            return self.commands.submit(n);
        }
    }

    fn churn() -> Int {
        let mut s = "";
        let mut i = 0;
        while i < 200 {
            s = s + "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx" + to_string(i);
            i = i + 1;
        }
        return len(s);
    }

    fn serve(c: Commands) -> Int {
        let server = Server { commands: c, port: 1 };
        let first = server.handle(1);
        let noise = churn();
        let second = server.handle(2);
        if noise < 200 {
            return -1;
        }
        return first + second;
    }

    fn call_through(c: Commands) -> Int {
        return c.submit(1);
    }
"#;

/// A locus whose `birth()` allocates in its own arena and rewrites
/// the field that the literal is read for.
const HOLDER_PRELUDE: &str = r#"
    type Row { v: Int = 0; }

    @form(vec)
    locus Rows {
        capacity { heap rows of Row; }
    }

    locus Holder {
        params {
            tag: String = "none";
            seen: Rows = Rows { };
        }
        birth() {
            self.seen.push(Row { v: len(self.tag) });
            self.tag = self.tag + "-" + to_string(self.seen.len());
        }
    }
"#;

/// GH #711, the reduced report: the literal is a call argument and
/// the callee retains it across allocation churn. SIGSEGV before the
/// fix; the `let`-bound control always answered.
#[test]
fn inline_argument_retained_by_callee_matches_the_bound_control() {
    let src = format!(
        "{}
        fn main() {{
            println(\"inline=\", serve(Provider {{ tag: \"inline\" }}));
            let bound = Provider {{ tag: \"inline\" }};
            println(\"bound=\", serve(bound));
        }}",
        RETAINING_PRELUDE
    );
    let (out, verdict) = build_and_run("retained_arg", &src);
    assert!(
        verdict.is_empty(),
        "inline argument retained by the callee {}\n{}",
        verdict,
        out
    );
    assert_eq!(out, "inline=15\nbound=15\n", "got:\n{}", out);
}

/// The same argument position without retention: the callee only
/// calls through. This crashed too — the provider's vec child was
/// already reclaimed when `submit` pushed — so the defect was never
/// about retention, only about how long the literal lived.
#[test]
fn inline_argument_called_through_immediately_answers() {
    let src = format!(
        "{}
        fn main() {{
            println(\"through=\", call_through(Provider {{ tag: \"inline\" }}));
            let bound = Provider {{ tag: \"inline\" }};
            println(\"bound=\", call_through(bound));
        }}",
        RETAINING_PRELUDE
    );
    let (out, verdict) = build_and_run("through_arg", &src);
    assert!(verdict.is_empty(), "call-through {}\n{}", verdict, out);
    assert_eq!(out, "through=7\nbound=7\n", "got:\n{}", out);
}

/// GH #812: a field read of an unowned literal, with a SECOND literal
/// in the same statement so the chunk pool cannot mask the reclaim.
/// Before the fix this printed the second literal's string twice —
/// `a=again-1 b=again-1` — a silent wrong answer with no signal and
/// nothing for a sanitizer to see.
#[test]
fn field_read_of_unowned_literal_sees_its_own_literal() {
    let src = format!(
        "{}
        fn main() {{
            println(\"a=\", Holder {{ tag: \"hello\" }}.tag, \" b=\", Holder {{ tag: \"again\" }}.tag);
        }}",
        HOLDER_PRELUDE
    );
    let (out, verdict) = build_and_run("field_pair", &src);
    assert!(verdict.is_empty(), "field-read pair {}", verdict);
    assert_eq!(out, "a=hello-1 b=again-1\n", "got:\n{}", out);

    // The let-bound control is the definition of right.
    let bound_src = format!(
        "{}
        fn main() {{
            let h1 = Holder {{ tag: \"hello\" }};
            let h2 = Holder {{ tag: \"again\" }};
            println(\"a=\", h1.tag, \" b=\", h2.tag);
        }}",
        HOLDER_PRELUDE
    );
    let (bound_out, bound_verdict) =
        build_and_run("field_pair_bound", &bound_src);
    assert!(bound_verdict.is_empty(), "bound control {}", bound_verdict);
    assert_eq!(bound_out, out, "the two spellings must be the same program");
}

/// GH #710's shape, kept here so the three positions are pinned by one
/// binary: the receiver's owner now comes from expression position
/// rather than from the method-call site.
#[test]
fn receiver_position_literal_still_outlives_its_call() {
    let src = format!(
        "{}
        fn main() {{
            println(\"recv=\", Provider {{ tag: \"inline\" }}.submit(4));
            let bound = Provider {{ tag: \"inline\" }};
            println(\"bound=\", bound.submit(4));
        }}",
        RETAINING_PRELUDE
    );
    let (out, verdict) = build_and_run("receiver", &src);
    assert!(verdict.is_empty(), "receiver literal {}\n{}", verdict, out);
    assert_eq!(out, "recv=7\nbound=7\n", "got:\n{}", out);
}

/// The other half of the rule, and the reason it is stated in terms of
/// expression position: a bare `LocusName { ... };` STATEMENT is
/// fire-and-forget and still tears down where it stands, while the
/// same literal in an expression waits for the fn's scope exit.
/// `dissolve()` printing its tag makes both points observable.
#[test]
fn bare_statement_literal_still_tears_down_at_the_statement() {
    let src = r#"
        locus Noisy {
            params { tag: String = "?"; }
            dissolve() { println("dissolved ", self.tag); }
            fn width() -> Int { return len(self.tag); }
        }

        fn main() {
            Noisy { tag: "stmt" };
            println("after-stmt");
            println("w=", Noisy { tag: "expr" }.width());
            println("after-expr");
        }
    "#;
    let (out, verdict) = build_and_run("bare_stmt", src);
    assert!(verdict.is_empty(), "bare-statement program {}", verdict);
    assert_eq!(
        out,
        "dissolved stmt\nafter-stmt\nw=4\nafter-expr\ndissolved expr\n",
        "got:\n{}",
        out
    );
}

/// The IR claim GH #812 was filed on: the teardown must not precede
/// the field load. Before the fix the literal's reclaim block held the
/// `load` of `.tag` and the `printf` that consumes it; now the literal
/// is parked in its deferred slot and the teardown is emitted at the
/// fn's scope exit, after both.
#[test]
fn field_read_is_emitted_before_the_teardown() {
    let src = r#"
        locus Holder {
            params { tag: String = "none"; }
        }

        fn main() {
            println("a=", Holder { tag: "hello" }.tag);
        }
    "#;
    let ir = dump_ir("field_order", src);
    let body = carve_main(&ir);

    let slot_store = body
        .find("ptr %Holder.deferred.slot")
        .expect("the literal must be parked in a deferred-dissolve slot");
    let field_load = body
        .find("%field.tag = load")
        .expect("`.tag` must be loaded in main");
    let printf = body[field_load..]
        .find("@printf")
        .map(|i| field_load + i)
        .expect("the load must feed a printf");
    let reclaim = body
        .find("Holder.dissolve.")
        .expect("the deferred teardown must be emitted in main");

    assert!(
        slot_store < field_load,
        "the literal must be parked before its field is read\n{}",
        body
    );
    assert!(
        field_load < printf && printf < reclaim,
        "the field load and its use must precede the teardown\n{}",
        body
    );
}

/// The masking shape, recorded so the reduction table's "looked fine"
/// row stays pinned: with a scalar-only provider the freed bytes were
/// still intact, so the very same defect answered correctly. Nothing
/// about this program changes — it just must not start being wrong.
#[test]
fn scalar_only_provider_answers_the_same_either_way() {
    let src = r#"
        interface Commands {
            fn submit(n: Int) -> Int;
        }

        locus Provider {
            params {
                tag: String = "p";
                count: Int = 0;
            }
            fn submit(n: Int) -> Int {
                self.count = self.count + n;
                return self.count + len(self.tag);
            }
        }

        fn call_through(c: Commands) -> Int {
            return c.submit(1);
        }

        fn main() {
            println("through=", call_through(Provider { tag: "inline" }));
            let bound = Provider { tag: "inline" };
            println("bound=", call_through(bound));
        }
    "#;
    let (out, verdict) = build_and_run("scalar", src);
    assert!(verdict.is_empty(), "scalar provider {}", verdict);
    assert_eq!(out, "through=7\nbound=7\n", "got:\n{}", out);
}
