//! A user effect class must travel over the BUS, not just the call
//! graph (#345).
//!
//! The docs, the spec and the published article all stated that a user
//! class "travels over the bus under `causes:`" like a built-in. It
//! did not. `causes_diags` infers each subscriber's effect set from
//! `frontier::infer_effects`, which unioned a leaf's `carries` only
//! when something CALLED it — a fn's own `is: {…}` was invisible to
//! its own set. A subscriber declaring `is: {money}` therefore
//! contributed nothing, and the publisher's `causes:` contract was
//! satisfied without naming it.
//!
//! Two things made this hard to notice. The identical shape with a
//! built-in class (`println` in the handler) reports the violation
//! correctly, so any spot-check with a built-in passes. And
//! `@effects(causes: { money })` did not PARSE — the `causes` arm
//! never learned to intern user classes — so the diagnostic's own
//! advice ("add the class to the declaration") led to a parse error.
//! The feature was unreachable from both ends.

use std::process::Command;

fn check_src(name: &str, src: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "hale-ue-bus-{}-{}",
        std::process::id(),
        name
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let f = dir.join("main.hl");
    std::fs::write(&f, src).expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(&f)
        .output()
        .expect("run hale check");
    let _ = std::fs::remove_dir_all(&dir);
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// `is: {…}` on the handler, `causes:` on the publisher omitting it.
fn program(causes: &str) -> String {
    format!(
        "effect money;\n\
         \n\
         type Payment {{ cents: Int; }}\n\
         topic Settled {{ payload: Payment; subject: \"settled\"; }}\n\
         \n\
         locus Ledger {{\n\
             params {{ total: Int = 0; }}\n\
             bus {{ subscribe Settled as on_settled; }}\n\
         \n\
             @effects(is: {{ money }})\n\
             fn on_settled(p: Payment) {{ self.total = self.total + p.cents; }}\n\
         }}\n\
         \n\
         main locus App {{\n\
             params {{ l: Ledger = Ledger {{ }}; }}\n\
             bus {{ publish Settled; }}\n\
         \n\
             @effects(causes: {{ {causes} }})\n\
             fn fire() {{ Settled <- Payment {{ cents: 1 }}; }}\n\
         \n\
             birth() {{ self.fire(); }}\n\
         }}\n\
         \n\
         fn main() {{ App {{ }}; }}\n"
    )
}

#[test]
fn a_user_class_reached_through_the_bus_violates_causes() {
    let out = check_src("omitted", &program("publish"));
    assert!(
        out.contains("declared causal set violated"),
        "publishing reaches a subscriber carrying `money`, which the \
         `causes:` set omits — this must be a violation:\n{}",
        out
    );
    assert!(
        out.contains("money"),
        "the causal diagnostic must NAME the class; `render_effects` \
         knows only built-ins, so a user class rendered as nothing and \
         the message read `can transitively cause  through the bus`:\n{}",
        out
    );
}

/// The other half: the advice the diagnostic gives must actually work.
#[test]
fn declaring_the_user_class_in_causes_satisfies_it() {
    let out = check_src("declared", &program("publish, money"));
    assert!(
        out.contains("typechecked") && !out.contains("error"),
        "`@effects(causes: {{ publish, money }})` must parse AND \
         satisfy the contract:\n{}",
        out
    );
}
