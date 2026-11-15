//! The deprecated stdlib callables (STABILITY.md § Deprecation): a
//! stable-tier form whose meaning will change, or which will be refused,
//! warns for at least one minor release first. Each row names the
//! callable and the warning a call of it raises, a `DiagKind::Warn`
//! whose message begins `deprecated:` and names the replacement and the
//! release that changes it. The checker reads the table where a method
//! call on a stdlib locus resolves (`check.rs`, the field arm of a
//! callee), and reports the warning at the call.
//!
//! A row is removed in the release that makes the change, when the
//! warning becomes the new meaning or a refusal.

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
    // GH #1943: `field` and `value` write their argument raw, and the
    // spec has always said they take a string. v0.33 makes them quote it,
    // so a call that hands them encoded JSON moves to the raw variant now.
    DeprecatedMethod {
        locus: "__StdJsonBuilder",
        method: "field",
        message: "`std::json::Builder.field(name, value)` writes `value` raw, as JSON already encoded; write \
                  `raw_field(name, json)` for encoded JSON or `string_field(name, s)` for a string. v0.33 makes \
                  `field` quote and escape its value as a string",
    },
    DeprecatedMethod {
        locus: "__StdJsonBuilder",
        method: "value",
        message: "`std::json::Builder.value(v)` writes `v` raw, as JSON already encoded; write `raw_value(json)` \
                  for encoded JSON or `string_value(s)` for a string. v0.33 makes `value` quote and escape `v` \
                  as a string",
    },
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
