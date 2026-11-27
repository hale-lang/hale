//! A stdlib module as a set of declarations: what a group member
//! `std::log::*` names.
//!
//! A module is a namespace the stdlib declares in Hale source. Its
//! declarations are every fn-bearing declaration (a locus, a free fn)
//! of each stdlib seed file that declares one of the module's public
//! names, bundled or appended on demand (`http_read.hl` declares
//! `std::http::BodyReader`), its file-private helpers included: `std::log::*` is the
//! whole of `log.hl`, the sinks that print and the helpers they print
//! through, so masking it masks the logger's own route to the
//! terminal. A namespace lowered natively (`std::str`) declares
//! nothing in Hale source and is no module here.
//!
//! [`built_subscribers`] answers which stdlib subscriptions a program's
//! bus holds: those of the stdlib loci it builds.
//!
//! The names are the stdlib's own (`__StdLogStdoutSink`), the spelling
//! an absorbed stdlib body carries ([`hale_model::AbsorbedNode::decl`]).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use hale_syntax::ast::TopDecl;

/// Every module's declarations, keyed by its path (`std::log`), built
/// once per process from the parsed stdlib.
fn modules() -> &'static BTreeMap<String, BTreeSet<String>> {
    static MODULES: OnceLock<BTreeMap<String, BTreeSet<String>>> = OnceLock::new();
    MODULES.get_or_init(|| {
        let Some(program) = crate::stdlib_bodies::program() else {
            return BTreeMap::new();
        };
        // Each seed file's fn-bearing declarations, and the file each
        // name is declared in.
        let mut by_file: BTreeMap<&'static str, BTreeSet<String>> = BTreeMap::new();
        let mut file_of: BTreeMap<String, &'static str> = BTreeMap::new();
        for item in &program.items {
            let Some((name, span)) = fn_bearing(item) else { continue };
            let Some((file, _, _)) = hale_stdlib::ap_file_at(span.start.as_usize()) else {
                continue;
            };
            by_file.entry(file).or_default().insert(name.clone());
            file_of.insert(name.clone(), file);
        }
        // GH #2218: the files appended only to a program that names what
        // they declare (`io_tcp_listener.hl`, `http_read.hl`, …) are
        // stdlib seed files like the bundled ones. Leaving them out left
        // `std::io::tcp::Listener` out of `std::io::tcp::*`, so a target
        // naming the module never hit a walk entering it, and a gate
        // naming it never masked one.
        for (_, file, src) in crate::rpc_expand::RUNTIME_FILES {
            let Ok(parsed) = hale_syntax::parse_source_at(src, 0) else { continue };
            for item in &parsed.items {
                let Some((name, _)) = fn_bearing(item) else { continue };
                by_file.entry(file).or_default().insert(name.clone());
                file_of.entry(name.clone()).or_insert(file);
            }
        }
        let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (segs, target) in hale_stdlib::PATH_RENAMES {
            if segs.len() < 3 || segs[0] != "std" {
                continue;
            }
            let Some(file) = file_of.get(*target) else { continue };
            let module = segs[..segs.len() - 1].join("::");
            out.entry(module).or_default().extend(by_file[file].iter().cloned());
        }
        out
    })
}

/// A locus or free fn's name and where it is declared; `None` for any
/// declaration that bears no fn.
fn fn_bearing(item: &TopDecl) -> Option<(&String, hale_syntax::Span)> {
    match item {
        TopDecl::Locus(l) => Some((&l.name.name, l.name.span)),
        TopDecl::Fn(f) => Some((&f.name.name, f.name.span)),
        _ => None,
    }
}

/// The declarations of the module `path` (`std::log`), or `None` when
/// the stdlib declares no such module in Hale source.
pub fn module_decls(path: &str) -> Option<&'static BTreeSet<String>> {
    modules().get(path)
}

/// Whether `name` is declared by a stdlib file appended on demand
/// ([`crate::rpc_expand::RUNTIME_FILES`]): merged into the program that
/// names it, so its bodies are the program's functions rather than
/// absorbed stdlib interiors.
pub fn appended_decl(name: &str) -> bool {
    static APPENDED: OnceLock<BTreeSet<String>> = OnceLock::new();
    APPENDED
        .get_or_init(|| {
            let mut out = BTreeSet::new();
            for (_, _, src) in crate::rpc_expand::RUNTIME_FILES {
                let Ok(parsed) = hale_syntax::parse_source_at(src, 0) else { continue };
                out.extend(parsed.items.iter().filter_map(fn_bearing).map(|(n, _)| n.clone()));
            }
            out
        })
        .contains(name)
}

/// The modules a group member may name, for a did-you-mean.
pub fn module_paths() -> impl Iterator<Item = &'static str> {
    modules().keys().map(String::as_str)
}

/// One subscription of a stdlib locus the program builds: its subject
/// as the stdlib declares it, the locus, and the handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltSubscriber {
    pub pattern: String,
    pub locus: String,
    pub handler: String,
}

/// The stdlib subscribers the program builds: every subscription of a
/// stdlib locus the placement table holds an instance of. A stdlib locus
/// no instance realizes subscribes to nothing. The one rule the claims
/// walk (a publish reaching a stdlib subscriber) and the orphan-subject
/// lint (a publish with a subscriber) both read.
pub fn built_subscribers(placement: &crate::placement::PlacementTable) -> Vec<BuiltSubscriber> {
    use hale_syntax::ast::{BusMember, LocusMember};
    let built: std::collections::BTreeSet<&str> = placement
        .instances
        .values()
        .filter_map(|r| r.realizes.as_ref().map(|d| d.lowered.as_str()))
        .collect();
    let mut out = Vec::new();
    let Some(sp) = crate::stdlib_bodies::program() else { return out };
    for item in &sp.items {
        let TopDecl::Locus(l) = item else { continue };
        if !built.contains(l.name.name.as_str()) {
            continue;
        }
        for mem in &l.members {
            let LocusMember::Bus(bus) = mem else { continue };
            for bm in &bus.members {
                if let BusMember::Subscribe { subject, handler, .. } = bm {
                    out.push(BuiltSubscriber {
                        pattern: subject.canonical().to_string(),
                        locus: l.name.name.clone(),
                        handler: handler.name.clone(),
                    });
                }
            }
        }
    }
    out
}
