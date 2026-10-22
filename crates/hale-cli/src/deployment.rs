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

/// The fleet seed `path`, checked over its own source and what each of
/// its imports declares, never merged with them (they are its
/// instances): the fleet as resolved source, the seed's directory, and
/// each import alias's seed. `Err` holds each diagnostic, rendered at
/// its line.
pub fn check_fleet_seed(
    path: &Path,
) -> Result<(FleetSource, PathBuf, BTreeMap<String, PathBuf>), Vec<String>> {
    let disk = hale_frontend::source::Disk;
    let render_failure = |f: hale_frontend::frontend::CheckableFailure| -> Vec<String> {
        let mut out: Vec<String> = f.io.iter().map(|i| i.text.clone()).collect();
        out.extend(f.diags.iter().map(|d| render(d, &f.file_bases, &f.sources)));
        out
    };
    let (_, _, programs, sources, file_bases, _) =
        hale_frontend::frontend::parse_checkable(path, &disk).map_err(render_failure)?;
    let root = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().unwrap_or(Path::new(".")).to_path_buf()
    };
    let root = root.canonicalize().unwrap_or(root);
    let workspace = hale_frontend::workspace::find_workspace_root(&root);
    // Each import alias's seed, and what its own files declare.
    let mut seeds: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut declared: BTreeMap<String, BTreeMap<String, Imported>> = BTreeMap::new();
    let mut errs: Vec<String> = Vec::new();
    for prog in programs.values() {
        for i in &prog.imports {
            let Some(alias) = &i.alias else { continue };
            if seeds.contains_key(alias) {
                continue;
            }
            let seed = match hale_frontend::imports::resolve_import(
                &root,
                workspace.as_deref(),
                &i.path,
                &disk,
            ) {
                Some(hale_frontend::imports::ImportTarget::Directory(d)) => {
                    d.canonicalize().unwrap_or(d)
                }
                Some(hale_frontend::imports::ImportTarget::SingleFile(f)) => f,
                None => {
                    let d = hale_syntax::Diag::ty(
                        i.path_span,
                        format!("import `{}` names no seed on disk", i.path),
                    );
                    errs.push(render(&d, &file_bases, &sources));
                    continue;
                }
            };
            let mut names: BTreeMap<String, Imported> = BTreeMap::new();
            match hale_frontend::frontend::parse_checkable(&seed, &disk) {
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
                                        Imported::Main
                                    } else {
                                        Imported::Locus
                                    };
                                    names.insert(l.name.name.clone(), kind);
                                }
                                TopDecl::Topic(t) => {
                                    names.insert(
                                        t.name.name.clone(),
                                        Imported::Topic {
                                            subject: t.subject.clone(),
                                        },
                                    );
                                }
                                _ => {}
                            }
                        }
                    }
                }
                Err(f) => errs.extend(render_failure(f)),
            }
            declared.insert(alias.clone(), names);
            seeds.insert(alias.clone(), seed);
        }
    }
    if !errs.is_empty() {
        return Err(errs);
    }
    let own: Vec<&hale_syntax::ast::Program> = programs.values().collect();
    let resolve = |alias: &str, name: &str| declared.get(alias).and_then(|m| m.get(name)).cloned();
    let (source, diags) = hale_types::fleet::fleet_source(&own, &resolve);
    match source {
        Some(s) => Ok((s, root, seeds)),
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
        let Some(seed) = seeds.get(&inst.alias) else {
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
