//! Aperio runtime — Phase 2.
//!
//! v0 cut: a tree-walking interpreter that runs parsed +
//! typechecked Aperio programs. Region allocator, cooperative
//! scheduler, and bus router come later in Phase 2; the
//! interpreter is the "is the language semantically real"
//! check that doesn't wait on codegen.
//!
//! Public surface:
//! - [`run_program`] / [`run_bundle`] — execute a parsed Program
//!   (or set of programs) starting from `fn main()`.

pub mod builtins;
pub mod bus;
pub mod env;
pub mod eval;
pub mod value;

pub use bus::{BusRouter, RingBuffer, SyncDispatch, Transport, TransportKind};
pub use builtins::set_user_args;
pub use eval::{run_bundle, run_bundle_with_bus, run_program};
pub use value::Value;

#[cfg(test)]
mod tests {
    use super::*;
    use aperio_syntax::parse_source;

    #[test]
    fn k_max_computes_from_framework_params() {
        // F.1: k_max = B / [(1-phi)c + phi*sigma].
        // B=10, c=2, sigma=1, phi=0.0 -> denom = 2 -> k_max = 5.
        let src = r#"
            locus L {
                params {
                    B: Int = 10;
                    c: Int = 2;
                    sigma: Int = 1;
                    phi: Float = 0.0;
                }
                birth() {
                    println("k_max=", self.k_max);
                }
            }
            fn main() { L { }; }
        "#;
        let program = parse_source(src).unwrap();
        assert_eq!(run_program(&program).unwrap(), 0);
    }

    #[test]
    fn k_max_in_closure_assertion() {
        // The framework primitive can be referenced as the
        // tolerance band in a closure: assert that count
        // never exceeds k_max. This is the F.1 invariant
        // operationalized in source.
        let src = r#"
            locus L {
                params {
                    B: Int = 10;
                    c: Int = 1;
                    sigma: Int = 1;
                    phi: Float = 1.0;
                    count: Int = 0;
                }
                bus { subscribe "ping" as on_ping of type Int; }
                fn on_ping(n: Int) {
                    self.count = self.count + 1;
                }
                closure within_capacity {
                    self.count ~~ 0 within self.k_max;
                }
            }
            fn main() { L { }; }
        "#;
        let program = parse_source(src).unwrap();
        // No pings sent → count stays 0 → 0 ~~ 0 within k_max → pass.
        assert_eq!(run_program(&program).unwrap(), 0);
    }

    #[test]
    fn long_lived_closure_passes_at_program_end() {
        // SubscriberL has a bus subscribe (long-lived) and a
        // closure that should pass at program-end dissolve.
        let src = r#"
            type Ping { n: Int; }

            locus SubscriberL {
                params { count: Int = 0; }
                bus { subscribe "p" as on_ping of type Ping; }
                fn on_ping(p: Ping) {
                    self.count = self.count + 1;
                }
                closure stays_zero_or_more {
                    self.count ~~ 0 within 100;
                }
            }

            fn main() {
                SubscriberL { };
            }
        "#;
        let program = parse_source(src).unwrap();
        assert_eq!(run_program(&program).unwrap(), 0);
    }

    #[test]
    fn match_dispatches_on_literal() {
        let src = r#"
            fn main() {
                let x = 2;
                match x {
                    1 -> println("one"),
                    2 -> println("two"),
                    _ -> println("other"),
                }
            }
        "#;
        let program = parse_source(src).unwrap();
        assert_eq!(run_program(&program).unwrap(), 0);
    }

    #[test]
    fn match_binds_wildcard_value() {
        // The binding pattern captures the scrutinee. Used as
        // a catch-all that names the value for the body.
        let src = r#"
            fn main() {
                let x = 42;
                match x {
                    1 -> println("one"),
                    n -> println("got: ", n),
                }
            }
        "#;
        let program = parse_source(src).unwrap();
        assert_eq!(run_program(&program).unwrap(), 0);
    }

    #[test]
    fn match_with_guard() {
        let src = r#"
            fn main() {
                let x = 7;
                match x {
                    n if n > 5 -> println("big: ", n),
                    n -> println("small: ", n),
                }
            }
        "#;
        let program = parse_source(src).unwrap();
        assert_eq!(run_program(&program).unwrap(), 0);
    }

    #[test]
    fn long_lived_closure_fails_at_program_end() {
        let src = r#"
            locus L {
                params { x: Int = 5; y: Int = 99; }
                bus { subscribe "_" as on_msg of type Int; }
                fn on_msg(_v: Int) { }
                closure xy_match {
                    self.x ~~ self.y within 0;
                }
            }
            fn main() { L { }; }
        "#;
        let program = parse_source(src).unwrap();
        let err = run_program(&program).expect_err("should fail at program end");
        assert!(
            err.contains("ClosureViolation") && err.contains("xy_match"),
            "expected ClosureViolation; got: {}",
            err
        );
    }
}
