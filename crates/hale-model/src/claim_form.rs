//! The spelling of every law form, over its operands' display text.
//!
//! A law's rendered form is what binds a stated row to its typed
//! operands: the topology artifact states each claims-block form,
//! certificate form and legacy fingerprint, and admission re-renders
//! them from the row's payload, refusing a row whose operands were
//! edited under an unchanged form. The two renderings must agree byte
//! for byte, so the spelling is written once, here, over plain text:
//! [`crate::ClaimRow`]'s renderers call these with a lowered row's
//! operands, and admission (`hale-cli`'s `topology_law`) calls them
//! with the operands it decodes from the artifact, which carry no
//! model ids (F.40 phase 4, A5).

use crate::claim_ir::CountCmpIr;

/// `effects(C)` — a class's declared carriers, in set position.
pub fn effect_carriers(class: &str) -> String {
    format!("effects({})", class)
}

/// `forbid reaches(SRC, DST) [via { … }] [during P] [avoiding G]`.
/// Both edge kinds (or neither) is the default and writes no `via`.
pub fn forbid_reaches(
    src: &str,
    dst: &str,
    via_calls: bool,
    via_bus: bool,
    during: Option<&str>,
    avoiding: Option<&str>,
) -> String {
    let mut out = format!("forbid reaches({}, {})", src, dst);
    match (via_calls, via_bus) {
        (true, false) => out.push_str(" via { calls }"),
        (false, true) => out.push_str(" via { bus }"),
        _ => {}
    }
    if let Some(p) = during {
        out.push_str(&format!(" during {}", p));
    }
    if let Some(a) = avoiding {
        out.push_str(&format!(" avoiding {}", a));
    }
    out
}

/// `only edges SRC -> DST { publish T; subscribe U }`; each grant is
/// (publish, topic).
pub fn only_edges<'a>(
    src: &str,
    dst: &str,
    grants: impl IntoIterator<Item = (bool, &'a str)>,
) -> String {
    let gs: Vec<String> = grants
        .into_iter()
        .map(|(publish, topic)| {
            format!(
                "{} {}",
                if publish { "publish" } else { "subscribe" },
                topic
            )
        })
        .collect();
    format!("only edges {} -> {} {{ {} }}", src, dst, gs.join("; "))
}

/// `bound C <= N on paths from G`.
pub fn bound(class: &str, limit: u64, from: &str) -> String {
    format!("bound {} <= {} on paths from {}", class, limit, from)
}

/// `require publishes(some G, topic T)` / `require subscribes(…)`.
pub fn require_endpoint(publishers: bool, group: &str, topic: &str) -> String {
    format!(
        "require {}(some {}, topic {})",
        if publishers { "publishes" } else { "subscribes" },
        group,
        topic
    )
}

/// `require sealed(all G)`.
pub fn require_sealed(group: &str) -> String {
    format!("require sealed(all {})", group)
}

/// `require attributed(all C)`.
pub fn require_attributed(class: &str) -> String {
    format!("require attributed(all {})", class)
}

/// `cover topic in seed(S): subscribed_by(some G)`.
pub fn cover(seed: &str, group: &str) -> String {
    format!("cover topic in seed({}): subscribed_by(some {})", seed, group)
}

/// `cover keys(topic T [in LO..=HI]): delivered_to(exactly_one G)`.
pub fn route_coverage(
    topic: &str,
    range: Option<(i64, i64)>,
    group: &str,
) -> String {
    let keys = match range {
        Some((lo, hi)) => format!(" in {}..={}", lo, hi),
        None => String::new(),
    };
    format!(
        "cover keys(topic {}{}): delivered_to(exactly_one {})",
        topic, keys, group
    )
}

/// `count publishers(topic T) <cmp> N` / `count subscribers(…)`.
pub fn count(publishers: bool, topic: &str, cmp: CountCmpIr, n: u64) -> String {
    format!(
        "count {}(topic {}) {} {}",
        if publishers { "publishers" } else { "subscribers" },
        topic,
        cmp.as_str(),
        n
    )
}

/// The certificate `@effects(none: { C })` generates per class.
pub fn forbid_effect(at: &str, class: &str) -> String {
    format!("forbid reaches({{{}}}, effects({}))", at, class)
}

/// The certificate `@effects(only: { … })` generates.
pub fn only_effects<'a>(
    classes: impl IntoIterator<Item = &'a str>,
    at: &str,
) -> String {
    format!("only effects {{{}}} on {{{}}}", join(classes), at)
}

/// The certificate `@effects(publish: { … })` generates.
pub fn only_publishes<'a>(
    entries: impl IntoIterator<Item = &'a str>,
    at: &str,
) -> String {
    format!("only publishes {{{}}} from {{{}}}", join(entries), at)
}

/// The certificate `@no_panic` generates.
pub fn no_panic(at: &str) -> String {
    format!("forbid reaches({{{}}}, panic)", at)
}

/// The certificate `@phase_effects` generates per phase.
pub fn phase_effects<'a>(
    classes: impl IntoIterator<Item = &'a str>,
    locus: &str,
    phase: &str,
) -> String {
    format!(
        "only effects {{{}}} on {{{}}} during {}",
        join(classes),
        locus,
        phase
    )
}

/// The certificate a `@budget` contract generates, over its dimension
/// (`alloc` for `alloc_per_call`).
pub fn budget(dim: &str, limit: u64, at: &str) -> String {
    format!("bound {} <= {} on paths from {{{}}}", dim, limit, at)
}

/// The fingerprint of `@effects(causes: { … })`.
pub fn causes<'a>(
    classes: impl IntoIterator<Item = &'a str>,
    at: &str,
) -> String {
    format!("causes {{{}}} from {{{}}}", join(classes), at)
}

/// The fingerprint of `@effects(depends: { … })`.
pub fn depends<'a>(
    entries: impl IntoIterator<Item = &'a str>,
    locus: &str,
) -> String {
    format!("depends {{{}}} on {{{}}}", join(entries), locus)
}

fn join<'a>(items: impl IntoIterator<Item = &'a str>) -> String {
    items.into_iter().collect::<Vec<_>>().join(", ")
}
