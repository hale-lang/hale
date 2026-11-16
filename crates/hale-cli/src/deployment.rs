//! Phase 4 (C2): the `deployment` family's producer.
//!
//! A fleet seed declares `fleet locus NAME { params { … } placement
//! { … } bindings { … } claims { … } }` (spec/verification.md § Fleet
//! composition). [`derive_deployment`] reads that source, checked and
//! resolved (`hale_types::fleet::fleet_source`), cuts each instance's
//! artifact from the seed its param names, and derives the rows: the
//! components, the instances, the placement defaults, the endpoints and
//! the routes. A route is derived, never written: a binding names a
//! subject, and its publishers and subscribers are the instances whose
//! artifacts publish and subscribe that subject; an instance whose own
//! main binds the subject must bind it over the route's transport.
//!
//! Composition reads the rows as the plan it composes
//! ([`Deployment::composition`]); the seeds stay artifacts, never merged
//! source, so a fleet is checked as one graph without a super-main.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use hale_syntax::ast::{CountCmp, TopDecl};
use hale_types::fleet::{FleetClaim, FleetSource, Imported};

use crate::fleet::{
    ClaimSpec, CountSpec, Endpoint, FleetPlan, ForbidReaches, GroupSpec, InstanceSpec, OnlyEdges,
    RequireEndpoint, RouteSpec,
};

/// A deployment as typed rows.
#[derive(Debug, Clone)]
pub struct Deployment {
    /// The fleet locus's name.
    pub fleet: String,
    /// The fleet seed's directory: where artifacts are cut, beside it.
    pub root: PathBuf,
    pub components: Vec<ComponentRow>,
    pub instances: Vec<InstanceRow>,
    pub placement: Vec<PlacementRow>,
    pub endpoints: Vec<EndpointRow>,
    pub routes: Vec<RouteRow>,
    /// The fleet's law, judged over the composed rows.
    pub claims: Vec<FleetClaim>,
}

/// One component: a param's main locus, the seed it is built from and
/// the artifact cut from that seed.
#[derive(Debug, Clone)]
pub struct ComponentRow {
    pub param: String,
    pub main: String,
    pub seed: PathBuf,
    pub artifact: PathBuf,
    pub shape_hash: String,
}

/// One deployed instance of a component.
#[derive(Debug, Clone)]
pub struct InstanceRow {
    pub id: String,
    pub component: String,
    /// The host its component's placement default names, if any; the
    /// habitat's allocation rows override it per instance.
    pub host: Option<String>,
}

/// A component's placement default: the hosts it names (one instance
/// on each), or its replica count. The habitat's allocation rows bind
/// instances to hosts.
#[derive(Debug, Clone)]
pub struct PlacementRow {
    pub component: String,
    pub hosts: Vec<String>,
    pub replicas: u64,
}

/// A topic a component uses, at the topic grain its artifact records:
/// its local name, its wire subject, the direction, and the transport
/// its own main binds it to (`None`: in-process only).
#[derive(Debug, Clone)]
pub struct EndpointRow {
    pub component: String,
    pub topic: String,
    pub subject: String,
    pub publish: bool,
    pub bound: Option<String>,
}

/// A derived route: one fleet binding's subject, from every instance
/// that publishes it to every instance that subscribes it, each named
/// with the topic as its own artifact spells it.
#[derive(Debug, Clone)]
pub struct RouteRow {
    pub id: String,
    pub subject: String,
    pub transport: String,
    pub publishers: Vec<(String, String)>,
    pub subscribers: Vec<(String, String)>,
}

/// Whether `path` is a fleet seed: a directory or `.hl` file whose own
/// source declares a `fleet locus`.
pub fn is_fleet_seed(path: &Path) -> bool {
    let files: Vec<PathBuf> = if path.is_dir() {
        match std::fs::read_dir(path) {
            Ok(rd) => rd
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("hl"))
                .collect(),
            Err(_) => return false,
        }
    } else if path.extension().and_then(|x| x.to_str()) == Some("hl") {
        vec![path.to_path_buf()]
    } else {
        return false;
    };
    files.iter().any(|f| {
        std::fs::read_to_string(f)
            .ok()
            .and_then(|src| hale_syntax::parse_source(&src).ok())
            .is_some_and(|p| {
                hale_syntax::ast::flat_decls(&p.items).any(|d| matches!(d, TopDecl::Fleet(_)))
            })
    })
}

/// One diagnostic of the fleet seed, at its file and line.
fn render(
    d: &hale_syntax::Diag,
    file_bases: &[(u32, PathBuf, u32)],
    sources: &BTreeMap<PathBuf, String>,
) -> String {
    let at = d.span.start.0;
    match file_bases.iter().find(|(b, _, l)| at >= *b && at <= b + l) {
        Some((base, path, _)) => d.render_located(
            &path.display().to_string(),
            sources.get(path).map(String::as_str).unwrap_or(""),
            *base,
        ),
        None => d.message.clone(),
    }
}

/// The fleet seed `path`, checked over its own source, never merged with
/// the seeds it imports (they are its instances): the fleet as resolved
/// source, the seed's directory, and each instance's seed by the key its
/// [`hale_types::fleet::FleetInstance::seed`] carries. `Err` holds each
/// diagnostic, rendered at its line.
///
/// A name the fleet spells through its imports (`alias::Main`, a bare
/// `Main` an import list brings in, a name an imported seed re-exports
/// from its own import list) resolves through the rename and re-export
/// rows the compiler builds for the same imports, the seed linked as
/// every verb links it, to the declaration it names in the seed that
/// declares it (GH #1987). The link is read for those rows alone; the
/// fleet's laws are judged over its own source.
pub fn check_fleet_seed(
    path: &Path,
) -> Result<(FleetSource, PathBuf, BTreeMap<String, PathBuf>), Vec<String>> {
    let disk = hale_frontend::source::Disk;
    let render_failure = |f: hale_frontend::frontend::CheckableFailure| -> Vec<String> {
        let mut out: Vec<String> = f.io.iter().map(|i| i.text.clone()).collect();
        out.extend(f.diags.iter().map(|d| render(d, &f.file_bases, &f.sources)));
        out
    };
    let (files, own_files, programs, sources, file_bases, effects) =
        hale_frontend::frontend::parse_checkable(path, &disk).map_err(render_failure)?;
    let root = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().unwrap_or(Path::new(".")).to_path_buf()
    };
    let root = root.canonicalize().unwrap_or(root);
    let workspace = hale_frontend::workspace::find_workspace_root(&root);
    // Each import alias's seed on disk: an import that names none is
    // refused at its path.
    let mut direct: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut errs: Vec<String> = Vec::new();
    for prog in programs.values() {
        for i in &prog.imports {
            let Some(alias) = &i.alias else { continue };
            if direct.contains_key(alias) {
                continue;
            }
            match hale_frontend::imports::resolve_import(&root, workspace.as_deref(), &i.path, &disk) {
                Some(hale_frontend::imports::ImportTarget::Directory(d)) => {
                    direct.insert(alias.clone(), d.canonicalize().unwrap_or(d));
                }
                Some(hale_frontend::imports::ImportTarget::SingleFile(f)) => {
                    direct.insert(alias.clone(), f.canonicalize().unwrap_or(f));
                }
                None => {
                    let d = hale_syntax::Diag::ty(i.path_span, format!("import `{}` names no seed on disk", i.path));
                    errs.push(render(&d, &file_bases, &sources));
                }
            }
        }
    }
    // What each alias's seed declares itself, read from its own files:
    // the resolution every fleet seed has had, and the one a name falls
    // back to when the compiler's rows have no answer for it.
    let mut declared: BTreeMap<String, BTreeMap<String, Imported>> = BTreeMap::new();
    for (alias, seed) in &direct {
        let mut names: BTreeMap<String, Imported> = BTreeMap::new();
        match hale_frontend::frontend::parse_checkable(seed, &disk) {
            Ok((_, _, theirs, _, _, _)) => {
                // the seed's entry, as every consumer reads it: the entry row
                let progs: Vec<&hale_syntax::ast::Program> = theirs.values().collect();
                let row = hale_types::entry::entry_row_in(&progs);
                let main = row.entry().map(|m| m.name.clone());
                for p in theirs.values() {
                    for d in hale_syntax::ast::flat_decls(&p.items) {
                        match d {
                            TopDecl::Locus(l) => {
                                let kind = if main.as_deref() == Some(l.name.name.as_str()) {
                                    Imported::Main { seed: seed.display().to_string() }
                                } else {
                                    Imported::Locus
                                };
                                names.insert(l.name.name.clone(), kind);
                            }
                            TopDecl::Topic(t) => {
                                names.insert(t.name.name.clone(), Imported::Topic { subject: t.subject.clone() });
                            }
                            _ => {}
                        }
                    }
                }
            }
            Err(f) => errs.extend(render_failure(f)),
        }
        declared.insert(alias.clone(), names);
    }
    if !errs.is_empty() {
        return Err(errs);
    }
    // The compiler's rows: the seed's imports linked as `hale check`
    // links them. The fleet itself stays out of the link: it is no
    // program, and its bindings may spell an adapter through an alias
    // only an instance's seed declares (`nats::NatsAdapter`), which the
    // fleet never resolves and the link's alias scoping would refuse. A
    // link that fails anyway leaves the rows empty, and every name then
    // resolves as it always has.
    let unfleeted: BTreeMap<PathBuf, hale_syntax::ast::Program> = programs
        .iter()
        .map(|(k, p)| {
            let mut p = p.clone();
            p.items.retain(|d| !matches!(d, TopDecl::Fleet(_)));
            (k.clone(), p)
        })
        .collect();
    // (the alias scoping reads the text too: the fleet's is blanked,
    // its offsets kept)
    let mut unfleeted_sources = sources.clone();
    for (file, p) in &programs {
        let Some(text) = unfleeted_sources.get_mut(file) else { continue };
        let Some((base, _, _)) = file_bases.iter().find(|(_, f, _)| f == file) else { continue };
        for d in p.items.iter().filter(|d| matches!(d, TopDecl::Fleet(_))) {
            let (lo, hi) = (d.span().start.0.saturating_sub(*base) as usize, d.span().end.0.saturating_sub(*base) as usize);
            if lo < hi && hi <= text.len() && text.is_char_boundary(lo) && text.is_char_boundary(hi) {
                let blank: String = text[lo..hi].chars().map(|c| if c == '\n' { '\n' } else { ' ' }).collect();
                text.replace_range(lo..hi, &blank);
            }
        }
    }
    let link = hale_frontend::frontend::link_checkable(
        path,
        &files,
        own_files,
        unfleeted,
        unfleeted_sources,
        file_bases.clone(),
        effects,
        &disk,
    );
    // An import list's own refusals (an item the library does not
    // declare, a listed name that collides) are the fleet's: import lists
    // are new (F.51), so no fleet that checked before writes one. Every
    // other refusal of the link only empties the rows.
    let link = match link {
        Ok(l) => Some(l),
        Err(f) => {
            let listed: Vec<String> = f
                .diags
                .iter()
                .filter(|d| {
                    d.message.contains("is not a top-level name of the library imported as")
                        || d.message.contains("is listed on the import of")
                })
                .map(|d| render(d, &f.file_bases, &f.sources))
                .collect();
            if !listed.is_empty() {
                return Err(listed);
            }
            None
        }
    };
    let (linked, linked_bases, renames) = match link {
        Some((linked, _, bases, renames, _, _)) => (linked, bases, renames),
        None => (BTreeMap::new(), Vec::new(), Vec::new()),
    };
    let merged: Vec<&TopDecl> =
        linked.values().flat_map(|p| hale_syntax::ast::flat_decls(&p.items)).collect();
    // The seed a merged declaration is in: the alias's own when the file
    // is that single-file seed, else the file's directory.
    let seed_of = |pos: u32| -> Option<PathBuf> {
        let f = hale_frontend::frontend::file_of_span(pos, &linked_bases)?;
        let f = f.canonicalize().unwrap_or(f);
        if direct.values().any(|s| s == &f) {
            return Some(f);
        }
        f.parent().map(Path::to_path_buf)
    };
    // The one row an alias names `name` by. A contested alias (two seeds
    // of the graph bind it to two libraries) is scoped per seed
    // (`alias$N`); the fleet's own is the row whose declaration is in
    // the seed the fleet's import names.
    let row = |alias: &str, name: &str| -> Option<String> {
        let scoped = format!("{alias}$");
        let mut found: Vec<&String> = renames
            .iter()
            .filter(|(k, _)| k.len() == 2 && k[1] == name && (k[0] == alias || k[0].starts_with(&scoped)))
            .map(|(_, m)| m)
            .collect();
        found.sort();
        found.dedup();
        match found.as_slice() {
            [one] => Some((*one).clone()),
            many => {
                let want = direct.get(alias)?;
                many.iter()
                    .find(|m| {
                        merged.iter().any(|d| decl_name(d) == Some(m.as_str()) && seed_of(d.span().start.0).as_ref() == Some(want))
                    })
                    .map(|m| (*m).clone())
            }
        }
    };
    // A bare name: the import list that brings it in, as `alias::name`.
    let listed: BTreeMap<String, (String, String)> = programs
        .values()
        .flat_map(|p| p.imports.iter())
        .filter_map(|i| i.alias.as_ref().map(|a| (a, i)))
        .flat_map(|(a, i)| i.items.iter().map(move |it| (it.local().name.clone(), (a.clone(), it.name.name.clone()))))
        .collect();
    let seeds: std::cell::RefCell<BTreeMap<String, PathBuf>> = std::cell::RefCell::new(BTreeMap::new());
    let by_rows = |segs: &[&str]| -> Option<Imported> {
        let mangled = match segs {
            [name] => {
                let (alias, theirs) = listed.get(*name)?;
                row(alias, theirs)?
            }
            [alias, name] => row(alias, name)?,
            _ => return None,
        };
        let decl = merged.iter().find(|d| decl_name(d) == Some(mangled.as_str()))?;
        match decl {
            TopDecl::Topic(t) => Some(Imported::Topic { subject: t.subject.clone() }),
            TopDecl::Locus(l) => {
                let seed = seed_of(l.name.span.start.0)?;
                // The declaring seed's entry, as every consumer reads it:
                // the entry row of that seed's own load. The two loads
                // number their files apart, so a declaration is matched
                // by its file and its offset in it.
                let at = |pos: u32, bases: &[(u32, PathBuf, u32)]| -> Option<(PathBuf, u32)> {
                    let (base, f, _) = bases.iter().filter(|(b, _, len)| pos >= *b && pos <= b + len).max_by_key(|(b, _, _)| *b)?;
                    Some((f.canonicalize().unwrap_or_else(|_| f.clone()), pos - base))
                };
                let here = at(l.name.span.start.0, &linked_bases)?;
                let is_main = match hale_frontend::frontend::parse_checkable(&seed, &disk) {
                    Ok((_, _, theirs, _, their_bases, _)) => {
                        let progs: Vec<&hale_syntax::ast::Program> = theirs.values().collect();
                        let row = hale_types::entry::entry_row_in(&progs);
                        hale_types::entry::root_decl(&progs)
                            .filter(|_| row.entry().is_some())
                            .and_then(|m| at(m.name.span.start.0, &their_bases))
                            == Some(here)
                    }
                    Err(_) => false,
                };
                if is_main {
                    let key = seed.display().to_string();
                    seeds.borrow_mut().insert(key.clone(), seed);
                    Some(Imported::Main { seed: key })
                } else {
                    Some(Imported::Locus)
                }
            }
            _ => None,
        }
    };
    // The compiler's rows first, then what the alias's seed declares
    // itself (GH #1987): nothing a fleet resolved before is refused.
    let resolve = |segs: &[&str]| -> Option<Imported> {
        let found = by_rows(segs);
        if found.is_some() {
            return found;
        }
        let [alias, name] = segs else { return None };
        let imported = declared.get(*alias)?.get(*name)?.clone();
        if let Imported::Main { seed } = &imported {
            seeds.borrow_mut().insert(seed.clone(), PathBuf::from(seed));
        }
        Some(imported)
    };
    let own: Vec<&hale_syntax::ast::Program> = programs.values().collect();
    let (source, diags) = hale_types::fleet::fleet_source(&own, &resolve);
    match source {
        Some(s) => Ok((s, root, seeds.into_inner())),
        None if diags.is_empty() => Err(vec![format!(
            "{}: declares no `fleet locus`",
            path.display()
        )]),
        None => Err(diags
            .iter()
            .map(|d| render(d, &file_bases, &sources))
            .collect()),
    }
}

/// A locus's or a topic's declared (merged) name.
fn decl_name(d: &TopDecl) -> Option<&str> {
    match d {
        TopDecl::Locus(l) => Some(l.name.name.as_str()),
        TopDecl::Topic(t) => Some(t.name.name.as_str()),
        _ => None,
    }
}

/// The `deployment` family's producer: the rows of the fleet `path`
/// declares, or every reason it has none.
pub fn derive_deployment(path: &Path) -> Result<Deployment, Vec<String>> {
    let (source, root, seeds) = check_fleet_seed(path)?;
    deployment_rows(&source, &root, &seeds)
}

fn deployment_rows(
    source: &FleetSource,
    root: &Path,
    seeds: &BTreeMap<String, PathBuf>,
) -> Result<Deployment, Vec<String>> {
    let mut errs = Vec::new();
    let mut components = Vec::new();
    // The artifacts are a cache in the workspace's `.hale/` (a clone
    // ignores it), under the fleet seed's place in the workspace.
    let workspace =
        hale_frontend::workspace::find_workspace_root(root).unwrap_or_else(|| root.to_path_buf());
    let workspace = workspace.canonicalize().unwrap_or(workspace);
    let place = match root.strip_prefix(&workspace) {
        Ok(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("_"),
    };
    let cache = workspace.join(".hale/fleet").join(place);
    let mut models = BTreeMap::new();
    for inst in &source.instances {
        let Some(seed) = seeds.get(&inst.seed) else {
            errs.push(format!(
                "instance `{}`: import `{}` names no seed on disk",
                inst.param, inst.alias
            ));
            continue;
        };
        let artifact = cache.join(format!("{}.topology.json", inst.param));
        if let Err(e) = crate::fleet::cut_artifact(seed, &artifact) {
            errs.push(format!("instance `{}`: {e}", inst.param));
            continue;
        }
        match crate::fleet::load_artifact(&artifact) {
            Ok((model, _)) => {
                components.push(ComponentRow {
                    param: inst.param.clone(),
                    main: inst.main.clone(),
                    seed: seed.clone(),
                    artifact: artifact.clone(),
                    shape_hash: model.shape_hash.clone(),
                });
                models.insert(inst.param.clone(), model);
            }
            Err(e) => errs.push(format!("instance `{}`: {e}", inst.param)),
        }
    }
    if !errs.is_empty() {
        return Err(errs);
    }

    let mut instances = Vec::new();
    let mut placement = Vec::new();
    for inst in &source.instances {
        placement.push(PlacementRow {
            component: inst.param.clone(),
            hosts: inst.hosts.clone(),
            replicas: inst.replicas,
        });
        // an instance is a process: `<param>-<i>`, one per replica or host
        for i in 0..inst.replicas {
            instances.push(InstanceRow {
                id: format!("{}-{i}", inst.param),
                component: inst.param.clone(),
                host: inst.hosts.get(i as usize).cloned(),
            });
        }
    }

    // Endpoints: every topic-grain endpoint each component's artifact
    // records, with the transport its own main binds the subject to.
    let mut endpoints: Vec<EndpointRow> = Vec::new();
    for (param, model) in &models {
        for e in &model.endpoints {
            let Some(topic) = &e.topic else { continue };
            // a publisher end is a send site; a subscriber end is its declaration
            if e.publish && e.site.is_none() {
                continue;
            }
            if endpoints
                .iter()
                .any(|x| &x.component == param && &x.topic == topic && x.publish == e.publish)
            {
                continue;
            }
            let bound = model
                .bindings
                .iter()
                .find(|(s, _)| *s == e.wire)
                .map(|(_, t)| t.clone());
            endpoints.push(EndpointRow {
                component: param.clone(),
                topic: topic.clone(),
                subject: e.wire.clone(),
                publish: e.publish,
                bound,
            });
        }
    }

    // Routes: one per binding, derived from the endpoints.
    let kind = |t: &str| t.split(':').next().unwrap_or(t).to_string();
    let mut routes = Vec::new();
    for b in &source.bindings {
        let mut publishers = Vec::new();
        let mut subscribers = Vec::new();
        for ep in endpoints.iter().filter(|e| e.subject == b.subject) {
            // An end its own main binds must agree with the route's
            // transport; an end it does not bind is carried by the fleet's
            // binding, the deployment's configuration of the route.
            match &ep.bound {
                None => {}
                Some(t) if kind(t) != kind(&b.transport) => {
                    errs.push(format!(
                        "binding `{}`: the fleet routes subject `{}` over `{}`, and instance `{}`'s main binds it to `{}`",
                        b.topic, b.subject, b.transport, ep.component, t
                    ));
                    continue;
                }
                Some(_) => {}
            }
            for inst in instances.iter().filter(|i| i.component == ep.component) {
                let end = (inst.id.clone(), ep.topic.clone());
                if ep.publish {
                    publishers.push(end);
                } else {
                    subscribers.push(end);
                }
            }
        }
        if publishers.is_empty() || subscribers.is_empty() {
            errs.push(format!(
                "binding `{}` wires nothing: no instance of the fleet {} subject `{}`",
                b.topic,
                if publishers.is_empty() {
                    "publishes"
                } else {
                    "subscribes"
                },
                b.subject
            ));
            continue;
        }
        routes.push(RouteRow {
            id: b.topic.clone(),
            subject: b.subject.clone(),
            transport: b.transport.clone(),
            publishers,
            subscribers,
        });
    }
    if !errs.is_empty() {
        return Err(errs);
    }
    Ok(Deployment {
        fleet: source.name.clone(),
        root: root.to_path_buf(),
        components,
        instances,
        placement,
        endpoints,
        routes,
        claims: source.claims.clone(),
    })
}

impl Deployment {
    /// The rows as the document `hale fleet export` writes, the bytes a
    /// signature over the rows covers: the instances with their seeds,
    /// hosts and component shapes, the derived routes, the groups and the
    /// claims. The same source derives the same bytes.
    pub fn export(&self) -> String {
        let mut s = serde_json::to_string_pretty(&self.composition()).unwrap_or_default();
        s.push('\n');
        s
    }

    /// Where a signature over the rows lives: `fleet.sig` in the fleet
    /// seed's directory, committed beside its source.
    pub fn signature_path(&self) -> PathBuf {
        self.root.join("fleet.sig")
    }

    /// The rows as the plan composition reads: each instance with its
    /// component's artifact and its param as its label, each param a
    /// group, each derived route, each claim.
    pub fn composition(&self) -> FleetPlan {
        let artifact_of = |c: &str| {
            self.components
                .iter()
                .find(|x| x.param == c)
                .map(|x| x.artifact.display().to_string())
                .unwrap_or_default()
        };
        let component = |c: &str| self.components.iter().find(|x| x.param == c);
        let instances = self
            .instances
            .iter()
            .map(|i| InstanceSpec {
                id: i.id.clone(),
                artifact: artifact_of(&i.component),
                labels: vec![i.component.clone()],
                seed: component(&i.component).map(|c| crate::deploy::relative(&self.root, &c.seed)),
                node: i.host.clone(),
                app_shape_hash: component(&i.component)
                    .map(|c| c.shape_hash.clone())
                    .unwrap_or_default(),
            })
            .collect();
        let routes = self
            .routes
            .iter()
            .map(|r| RouteSpec {
                id: r.id.clone(),
                publishers: r
                    .publishers
                    .iter()
                    .map(|(i, t)| Endpoint {
                        instance: i.clone(),
                        topic: t.clone(),
                    })
                    .collect(),
                subscribers: r
                    .subscribers
                    .iter()
                    .map(|(i, t)| Endpoint {
                        instance: i.clone(),
                        topic: t.clone(),
                    })
                    .collect(),
                transport: Some(r.transport.clone()),
            })
            .collect();
        let groups = self
            .components
            .iter()
            .map(|c| {
                (
                    c.param.clone(),
                    GroupSpec {
                        instances: Vec::new(),
                        labels: vec![c.param.clone()],
                    },
                )
            })
            .collect();
        let none = || ClaimSpec {
            name: String::new(),
            forbid_reaches: None,
            require_subscribes: None,
            require_publishes: None,
            count_publisher_instances: None,
            count_subscriber_instances: None,
            only_edges: None,
        };
        let claims = self
            .claims
            .iter()
            .map(|c| match c {
                FleetClaim::ForbidReaches {
                    name,
                    from,
                    to,
                    avoiding,
                } => ClaimSpec {
                    name: name.clone(),
                    forbid_reaches: Some(ForbidReaches {
                        from: from.clone(),
                        to: to.clone(),
                        avoiding: avoiding.clone(),
                    }),
                    ..none()
                },
                FleetClaim::OnlyEdges {
                    name,
                    from,
                    to,
                    grant_subjects,
                } => ClaimSpec {
                    name: name.clone(),
                    only_edges: Some(OnlyEdges {
                        from: from.clone(),
                        to: to.clone(),
                        grant_subjects: grant_subjects.clone(),
                    }),
                    ..none()
                },
                FleetClaim::Require {
                    name,
                    publishes,
                    group,
                    subject,
                } => {
                    let e = RequireEndpoint {
                        group: group.clone(),
                        subject: subject.clone(),
                    };
                    if *publishes {
                        ClaimSpec {
                            name: name.clone(),
                            require_publishes: Some(e),
                            ..none()
                        }
                    } else {
                        ClaimSpec {
                            name: name.clone(),
                            require_subscribes: Some(e),
                            ..none()
                        }
                    }
                }
                FleetClaim::Count {
                    name,
                    publishers,
                    subject,
                    cmp,
                    n,
                } => {
                    let n = *n as usize;
                    let k = CountSpec {
                        subject: subject.clone(),
                        eq: matches!(cmp, CountCmp::Eq).then_some(n),
                        max: matches!(cmp, CountCmp::Le).then_some(n),
                        min: matches!(cmp, CountCmp::Ge).then_some(n),
                    };
                    if *publishers {
                        ClaimSpec {
                            name: name.clone(),
                            count_publisher_instances: Some(k),
                            ..none()
                        }
                    } else {
                        ClaimSpec {
                            name: name.clone(),
                            count_subscriber_instances: Some(k),
                            ..none()
                        }
                    }
                }
            })
            .collect();
        FleetPlan {
            schema: crate::fleet::FLEET_PLAN_SCHEMA.to_string(),
            name: self.fleet.clone(),
            instances,
            routes,
            groups,
            claims,
        }
    }
}
