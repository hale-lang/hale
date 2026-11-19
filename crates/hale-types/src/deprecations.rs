//! The deprecated stdlib callables: a form whose meaning will change, or
//! which will be refused, may warn first. Before 1.0 a release may make
//! such a change at once (STABILITY.md), so a row here is a courtesy the
//! change chose, not a window it owes. Each row names the
//! callable and the warning a call of it raises, a `DiagKind::Warn`
//! whose message begins `deprecated:` and names the replacement and the
//! release that changes it. The checker reads the table where a method
//! call on a stdlib locus resolves (`check.rs`, the field arm of a
//! callee), and reports the warning at the call.
//!
//! A row is removed in the release that makes the change, when the
//! warning becomes the new meaning or a refusal.
//!
//! A deprecated language form, not a callable, keeps its warning here
//! too, as a fn the checker calls where it meets the form. None is
//! deprecated today: GH #2022's write through a `let` binding, the last
//! one, is an error now (`check.rs`, `let_write_error`).

/// One deprecated stdlib method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeprecatedMethod {
    /// The locus the method is declared on, as the stdlib source declares
    /// it (`__StdJsonBuilder`, what `std::json::Builder` resolves to).
    pub locus: &'static str,
    /// The method's name.
    pub method: &'static str,
    /// The warning, after `deprecated: `: what the call does today, what
    /// to write instead, and which release changes it.
    pub message: &'static str,
}

/// The deprecated stdlib methods.
pub const DEPRECATED_METHODS: &[DeprecatedMethod] = &[
    // GH #1943's rows (`std::json::Builder.field` / `.value` wrote their
    // argument raw) were removed in v0.35, which made both quote it.
];

/// The deprecation of `locus.method`, if the table has one.
pub fn deprecated_method(locus: &str, method: &str) -> Option<&'static DeprecatedMethod> {
    DEPRECATED_METHODS.iter().find(|d| d.locus == locus && d.method == method)
}

/// The warning a call of `d` raises, `deprecated:` first.
pub fn warning(d: &DeprecatedMethod) -> String {
    format!("deprecated: {}", d.message)
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_row_names_a_method_the_stdlib_declares() {
        for d in super::DEPRECATED_METHODS {
            let decl = format!("locus {} {{", d.locus);
            let at = hale_stdlib::AP_SOURCE.find(&decl).unwrap_or_else(|| panic!("the stdlib declares `{}`", d.locus));
            let body = &hale_stdlib::AP_SOURCE[at..];
            assert!(body.contains(&format!("fn {}(", d.method)), "`{}` declares `{}`", d.locus, d.method);
            assert!(super::warning(d).starts_with("deprecated: "));
        }
    }
}
