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
//! artifacts publish and subscribe that subject and bind it out of their
//! process.
//!
//! Composition reads the rows as the plan it composes
//! ([`Deployment::composition`]); the seeds stay artifacts, never merged
//! source, so a fleet is checked as one graph without a super-main.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_syntax::ast::{CountCmp, TopDecl};
use hale_types::fleet::{FleetClaim, FleetSource};

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
}

/// A component's placement default: the host it names, or its replica
/// count. The habitat's allocation rows bind instances to hosts.
#[derive(Debug, Clone)]
pub struct PlacementRow {
    pub component: String,
    pub host: Option<String>,
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

/// The `deployment` family's producer: the rows of the fleet `path`
/// declares, or every reason it has none.
pub fn derive_deployment(path: &Path) -> Result<Deployment, Vec<String>> {
    // The fleet seed checks first, as `hale check` reports it: its laws
    // (fleet_laws) are the check's.
    let me = std::env::current_exe().map_err(|e| vec![e.to_string()])?;
    let r = std::process::Command::new(me)
        .arg("check")
        .arg(path)
        .output()
        .map_err(|e| vec![format!("hale check {}: {e}", path.display())])?;
    if !r.status.success() {
        return Err(vec![format!(
            "fleet seed `{}` does not check:\n{}{}",
            path.display(),
            String::from_utf8_lossy(&r.stdout).trim(),
            String::from_utf8_lossy(&r.stderr).trim()
        )]);
    }
    let snap = Snapshot::load(
        path,
        LoadMode::WholeSeed,
        &hale_frontend::source::Disk,
        Config::check(false, false),
    )
    .map_err(|_| vec![format!("{}: the fleet seed does not load", path.display())])?;
    let Ok(top) = snap.demand_scope() else {
        return Err(vec![format!(
            "{}: the fleet seed does not resolve",
            path.display()
        )]);
    };
    let bundle = snap.bundle();
    let (source, diags) = hale_types::fleet::fleet_source(&bundle, top);
    let Some(source) = source else {
        let mut errs: Vec<String> = diags.iter().map(|d| d.message.clone()).collect();
        if errs.is_empty() {
            errs.push(format!("{}: declares no `fleet locus`", path.display()));
        }
        return Err(errs);
    };
    let root = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().unwrap_or(Path::new(".")).to_path_buf()
    };
    let root = root.canonicalize().unwrap_or(root);
    let workspace = hale_frontend::workspace::find_workspace_root(&root);
    // Each import alias's seed, resolved as the load resolved it.
    let mut seeds: BTreeMap<String, PathBuf> = BTreeMap::new();
    for i in snap.entry_imports() {
        let Some(alias) = &i.alias else { continue };
        let target = hale_frontend::imports::resolve_import(
            &root,
            workspace.as_deref(),
            &i.path,
            &hale_frontend::source::Disk,
        );
        let seed = match target {
            Some(hale_frontend::imports::ImportTarget::Directory(d)) => {
                d.canonicalize().unwrap_or(d)
            }
            Some(hale_frontend::imports::ImportTarget::SingleFile(f)) => f,
            None => continue,
        };
        seeds.insert(alias.clone(), seed);
    }
    deployment_rows(&source, &root, &seeds)
}

fn deployment_rows(
    source: &FleetSource,
    root: &Path,
    seeds: &BTreeMap<String, PathBuf>,
) -> Result<Deployment, Vec<String>> {
    let mut errs = Vec::new();
    let mut components = Vec::new();
    let mut models = BTreeMap::new();
    let none = crate::sign::Trust::load(&[]).map_err(|e| vec![e.to_string()])?;
    for inst in &source.instances {
        let Some(seed) = seeds.get(&inst.alias) else {
            errs.push(format!(
                "instance `{}`: import `{}` names no seed on disk",
                inst.param, inst.alias
            ));
            continue;
        };
        let artifact = root
            .join(".hale/fleet")
            .join(&source.name)
            .join(format!("{}.topology.json", inst.param));
        if let Err(e) = crate::fleet::cut_artifact(seed, &artifact) {
            errs.push(format!("instance `{}`: {e}", inst.param));
            continue;
        }
        match crate::fleet::load_artifact(&artifact, &none) {
            Ok((model, _, _)) => {
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
            host: inst.host.clone(),
            replicas: inst.replicas,
        });
        if inst.replicas > 1 {
            for i in 0..inst.replicas {
                instances.push(InstanceRow {
                    id: format!("{}-{i}", inst.param),
                    component: inst.param.clone(),
                });
            }
        } else {
            instances.push(InstanceRow {
                id: inst.param.clone(),
                component: inst.param.clone(),
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
            match &ep.bound {
                None => {
                    errs.push(format!(
                        "binding `{}`: instance `{}` {} subject `{}`, but its main binds no transport for it, \
                         so the topic never leaves its process; bind `{}` in its `bindings`",
                        b.topic,
                        ep.component,
                        if ep.publish { "publishes" } else { "subscribes" },
                        b.subject,
                        ep.topic
                    ));
                    continue;
                }
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
        let instances = self
            .instances
            .iter()
            .map(|i| InstanceSpec {
                id: i.id.clone(),
                artifact: artifact_of(&i.component),
                labels: vec![i.component.clone()],
                seed: None,
                node: None,
                binary: None,
                binary_sha256: None,
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
            schema: "1.2".to_string(),
            name: self.fleet.clone(),
            instances,
            routes,
            groups,
            claims,
        }
    }
}
