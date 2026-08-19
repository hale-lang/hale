//! The CLOSED external law decoder (GH #476 Change 6, round 4).
//!
//! Deserializes a schema-1.11 `law.rows[*].law` payload into a
//! typed vocabulary — exact enums for kinds, verbs, comparators,
//! via edges, dimensions — validates every variant's complete
//! shape and its references against the artifact's own sections,
//! and canonically re-renders the claims-tier forms so admission
//! can require byte agreement with the compatibility `claims`
//! rows. A payload that decodes is exactly one the emitter could
//! have produced; anything else refuses with its location named.

use serde_json::Value;

/// A decoded reference: display spelling + resolution status
/// (the raw identity is carried but not needed for rendering).
pub struct Ref {
    pub display: String,
    pub resolved: bool,
}

fn decode_ref(v: &Value, what: &str) -> Result<Ref, String> {
    let name_ok = v["name"].is_string();
    let display = v["display"].as_str();
    let resolved = v["resolved"].as_bool();
    match (name_ok, display, resolved) {
        (true, Some(d), Some(r)) => Ok(Ref {
            display: d.to_string(),
            resolved: r,
        }),
        _ => Err(format!("{} is not a typed reference", what)),
    }
}

pub struct ClassRef {
    pub class: String,
    pub builtin: bool,
    pub resolved: bool,
}

fn decode_class(v: &Value, what: &str) -> Result<ClassRef, String> {
    let class = v["class"]
        .as_str()
        .ok_or_else(|| format!("{}: class must be a string", what))?;
    let builtin = v["builtin"]
        .as_bool()
        .ok_or_else(|| format!("{}: builtin must be a bool", what))?;
    let resolved = v["resolved"]
        .as_bool()
        .ok_or_else(|| format!("{}: resolved must be a bool", what))?;
    // The builtin flag must AGREE with the language's closed
    // builtin set — a user class cannot claim builtin, and a
    // builtin is always resolved.
    if builtin != hale_model::is_builtin_effect_class(class) {
        return Err(format!(
            "{}: class `{}` mislabels its builtin status",
            what, class
        ));
    }
    if builtin && !resolved {
        return Err(format!(
            "{}: builtin class `{}` must be resolved",
            what, class
        ));
    }
    Ok(ClassRef {
        class: class.to_string(),
        builtin,
        resolved,
    })
}

fn decode_class_list(
    v: &Value,
    what: &str,
) -> Result<Vec<ClassRef>, String> {
    v.as_array()
        .ok_or_else(|| format!("{}: must be an array", what))?
        .iter()
        .map(|c| decode_class(c, what))
        .collect()
}

pub enum SetRef {
    Group(Ref),
    Effects(ClassRef),
}

pub enum Dim {
    Builtin(String),
    UserClass(ClassRef),
}

pub struct Grant {
    pub publish: bool,
    pub topic: Ref,
}

/// The closed law vocabulary — one variant per emitter kind.
pub enum Law {
    ForbidReaches {
        src: SetRef,
        dst: SetRef,
        via_calls: bool,
        via_bus: bool,
        during: Option<Ref>,
        avoiding: Option<Ref>,
    },
    OnlyEdges {
        src: Ref,
        dst: Ref,
        grants: Vec<Grant>,
    },
    Bound {
        class: ClassRef,
        limit: u64,
        from: Ref,
    },
    RequireEndpoint {
        publishers: bool,
        group: Ref,
        topic: Ref,
    },
    RequireSealed {
        group: Ref,
    },
    RequireAttributed {
        class: ClassRef,
    },
    Cover {
        seed: Ref,
        group: Ref,
    },
    Count {
        publishers: bool,
        topic: Ref,
        cmp: &'static str,
        n: u64,
    },
    EffectForbid,
    EffectOnly,
    EffectPublishSet,
    EffectCauses,
    NoPanic,
    DependsSet,
    PhaseEffects,
    AllocBudget,
    QuantBudget,
    Fleet,
}

/// The artifact context references must resolve against.
pub struct RefContext<'a> {
    pub groups: &'a Value,
    pub topic_names: &'a [String],
    pub fn_names: &'a [String],
    pub locus_names: &'a [String],
}

impl RefContext<'_> {
    fn group(&self, v: &Value, what: &str) -> Result<Ref, String> {
        let r = decode_ref(v, what)?;
        if r.resolved && !self.groups[&r.display].is_array() {
            return Err(format!(
                "{}: resolved group `{}` is not in this artifact",
                what, r.display
            ));
        }
        Ok(r)
    }
    fn topic(&self, v: &Value, what: &str) -> Result<Ref, String> {
        let r = decode_ref(v, what)?;
        if r.resolved
            && !self.topic_names.iter().any(|n| *n == r.display)
        {
            return Err(format!(
                "{}: resolved topic `{}` is not in this artifact",
                what, r.display
            ));
        }
        Ok(r)
    }
    fn function(
        &self,
        v: &Value,
        what: &str,
    ) -> Result<Ref, String> {
        let r = decode_ref(v, what)?;
        if r.resolved
            && !self.fn_names.iter().any(|n| *n == r.display)
        {
            return Err(format!(
                "{}: resolved fn `{}` is not in this artifact",
                what, r.display
            ));
        }
        Ok(r)
    }
    fn locus(&self, v: &Value, what: &str) -> Result<Ref, String> {
        let r = decode_ref(v, what)?;
        if r.resolved
            && !self.locus_names.iter().any(|n| *n == r.display)
        {
            return Err(format!(
                "{}: resolved locus `{}` is not in this artifact",
                what, r.display
            ));
        }
        Ok(r)
    }
    fn set(&self, v: &Value, what: &str) -> Result<SetRef, String> {
        if v["group"].is_object() {
            Ok(SetRef::Group(self.group(&v["group"], what)?))
        } else if v["effects"].is_object() {
            Ok(SetRef::Effects(decode_class(
                &v["effects"],
                what,
            )?))
        } else {
            Err(format!("{}: must be a group or effects set", what))
        }
    }
    fn selectors(
        &self,
        v: &Value,
        what: &str,
    ) -> Result<(), String> {
        for (i, s) in v
            .as_array()
            .ok_or_else(|| format!("{}: must be an array", what))?
            .iter()
            .enumerate()
        {
            let ok = s["name"].is_string()
                && s["topics"].as_array().is_some_and(|ts| {
                    ts.iter().all(|t| {
                        t["name"].is_string()
                            && t["display"].is_string()
                    })
                })
                && s["subjects"].as_array().is_some_and(|ss| {
                    ss.iter().all(|x| x.is_string())
                });
            if !ok {
                return Err(format!(
                    "{}[{}]: malformed bus selector",
                    what, i
                ));
            }
        }
        Ok(())
    }
}

/// Decode one payload against the closed vocabulary.
pub fn decode_law(
    law: &Value,
    cx: &RefContext<'_>,
) -> Result<Law, String> {
    match law["kind"].as_str() {
        Some("forbid_reaches") => {
            let via = law["via"].as_array().ok_or("via missing")?;
            let mut via_calls = false;
            let mut via_bus = false;
            if via.is_empty() {
                return Err("via must not be empty".to_string());
            }
            for v in via {
                match v.as_str() {
                    Some("calls") => via_calls = true,
                    Some("bus") => via_bus = true,
                    other => {
                        return Err(format!(
                            "via edge `{:?}` is not calls|bus",
                            other
                        ))
                    }
                }
            }
            let during = if law["during"].is_object() {
                Some(decode_ref(&law["during"], "during")?)
            } else if law.get("during").is_some_and(|d| !d.is_null())
            {
                return Err("during must be a phase reference"
                    .to_string());
            } else {
                None
            };
            let avoiding = if law["avoiding"].is_object() {
                Some(cx.group(&law["avoiding"], "avoiding")?)
            } else if law
                .get("avoiding")
                .is_some_and(|d| !d.is_null())
            {
                return Err("avoiding must be a group reference"
                    .to_string());
            } else {
                None
            };
            Ok(Law::ForbidReaches {
                src: cx.set(&law["src"], "src")?,
                dst: cx.set(&law["dst"], "dst")?,
                via_calls,
                via_bus,
                during,
                avoiding,
            })
        }
        Some("only_edges") => {
            let mut grants = Vec::new();
            for (i, g) in law["grants"]
                .as_array()
                .ok_or("grants missing")?
                .iter()
                .enumerate()
            {
                let publish = match g["verb"].as_str() {
                    Some("publish") => true,
                    Some("subscribe") => false,
                    other => {
                        return Err(format!(
                            "grants[{}]: verb `{:?}` is not \
                             publish|subscribe",
                            i, other
                        ))
                    }
                };
                grants.push(Grant {
                    publish,
                    topic: cx
                        .topic(&g["topic"], "grant topic")?,
                });
            }
            Ok(Law::OnlyEdges {
                src: cx.group(&law["src"], "src")?,
                dst: cx.group(&law["dst"], "dst")?,
                grants,
            })
        }
        Some("bound") => Ok(Law::Bound {
            class: decode_class(&law["class"], "class")?,
            limit: law["limit"].as_u64().ok_or("limit missing")?,
            from: cx.group(&law["from"], "from")?,
        }),
        Some("require_endpoint") => Ok(Law::RequireEndpoint {
            publishers: law["publishers"]
                .as_bool()
                .ok_or("publishers missing")?,
            group: cx.group(&law["group"], "group")?,
            topic: cx.topic(&law["topic"], "topic")?,
        }),
        Some("require_sealed") => Ok(Law::RequireSealed {
            group: cx.group(&law["group"], "group")?,
        }),
        Some("require_attributed") => {
            Ok(Law::RequireAttributed {
                class: decode_class(&law["class"], "class")?,
            })
        }
        Some("cover") => Ok(Law::Cover {
            seed: decode_ref(&law["seed"], "seed")?,
            group: cx.group(&law["group"], "group")?,
        }),
        Some("count") => {
            let cmp = match law["cmp"].as_str() {
                Some("==") => "==",
                Some("<=") => "<=",
                Some(">=") => ">=",
                other => {
                    return Err(format!(
                        "cmp `{:?}` is not ==|<=|>=",
                        other
                    ))
                }
            };
            Ok(Law::Count {
                publishers: law["publishers"]
                    .as_bool()
                    .ok_or("publishers missing")?,
                topic: cx.topic(&law["topic"], "topic")?,
                cmp,
                n: law["n"].as_u64().ok_or("n missing")?,
            })
        }
        Some("effect_forbid") => {
            cx.function(&law["at"], "at")?;
            decode_class_list(&law["classes"], "classes")?;
            Ok(Law::EffectForbid)
        }
        Some("effect_only") => {
            cx.function(&law["at"], "at")?;
            decode_class_list(&law["classes"], "classes")?;
            Ok(Law::EffectOnly)
        }
        Some("effect_causes") => {
            cx.function(&law["at"], "at")?;
            decode_class_list(&law["classes"], "classes")?;
            Ok(Law::EffectCauses)
        }
        Some("effect_publish_set") => {
            cx.function(&law["at"], "at")?;
            cx.selectors(&law["entries"], "entries")?;
            Ok(Law::EffectPublishSet)
        }
        Some("no_panic") => {
            cx.function(&law["at"], "at")?;
            Ok(Law::NoPanic)
        }
        Some("depends_set") => {
            cx.locus(&law["locus"], "locus")?;
            cx.selectors(&law["entries"], "entries")?;
            Ok(Law::DependsSet)
        }
        Some("phase_effects") => {
            cx.locus(&law["locus"], "locus")?;
            for (i, p) in law["phases"]
                .as_array()
                .ok_or("phases missing")?
                .iter()
                .enumerate()
            {
                if !p["phase"].is_string() {
                    return Err(format!(
                        "phases[{}]: phase must be a string",
                        i
                    ));
                }
                decode_class_list(&p["allowed"], "allowed")?;
            }
            Ok(Law::PhaseEffects)
        }
        Some("alloc_budget") => {
            cx.function(&law["at"], "at")?;
            law["per_call"].as_u64().ok_or("per_call missing")?;
            Ok(Law::AllocBudget)
        }
        Some("quant_budget") => {
            cx.function(&law["at"], "at")?;
            law["limit"].as_u64().ok_or("limit missing")?;
            let dim = &law["dim"];
            let _dim = if let Some(b) = dim["builtin"].as_str() {
                match b {
                    "stack_bytes" | "block_points" | "publish"
                    | "fanout" => Dim::Builtin(b.to_string()),
                    other => {
                        return Err(format!(
                            "dim `{}` is not a quantitative \
                             dimension",
                            other
                        ))
                    }
                }
            } else if dim["user_class"].is_object() {
                Dim::UserClass(decode_class(
                    &dim["user_class"],
                    "dim",
                )?)
            } else {
                return Err(
                    "dim must be a builtin tag or a user-class \
                     reference"
                        .to_string(),
                );
            };
            Ok(Law::QuantBudget)
        }
        Some(
            "fleet_forbid_reaches"
            | "fleet_only_edges"
            | "fleet_require_endpoint"
            | "fleet_count_instances",
        ) => Ok(Law::Fleet),
        other => Err(format!(
            "law kind `{:?}` is not in the closed vocabulary",
            other
        )),
    }
}

/// The judgment family a decoded kind belongs to — admission
/// requires the row's declared family to agree.
pub fn family_of(law: &Law) -> &'static str {
    match law {
        Law::ForbidReaches { .. } => "reachability",
        Law::OnlyEdges { .. } => "boundary",
        Law::RequireEndpoint { .. }
        | Law::RequireSealed { .. }
        | Law::RequireAttributed { .. }
        | Law::Cover { .. }
        | Law::Count { .. } => "endpoint",
        Law::Bound { .. } => "bound",
        Law::EffectForbid
        | Law::EffectOnly
        | Law::EffectPublishSet
        | Law::NoPanic
        | Law::PhaseEffects => "certificate",
        Law::EffectCauses
        | Law::DependsSet
        | Law::AllocBudget
        | Law::QuantBudget => "unmigrated",
        Law::Fleet => "fleet",
    }
}

/// Canonically re-render the compatibility `claims` form from the
/// decoded law — admission requires byte agreement, so an operand
/// swap under an unchanged form string is refused. `None` for
/// non-claims-tier kinds.
pub fn render_claims_form(law: &Law) -> Option<String> {
    let set = |s: &SetRef| -> String {
        match s {
            SetRef::Group(g) => g.display.clone(),
            SetRef::Effects(c) => format!("effects({})", c.class),
        }
    };
    Some(match law {
        Law::ForbidReaches {
            src,
            dst,
            via_calls,
            via_bus,
            during,
            avoiding,
        } => {
            let mut out = format!(
                "forbid reaches({}, {})",
                set(src),
                set(dst)
            );
            match (via_calls, via_bus) {
                (true, true) => {}
                (true, false) => out.push_str(" via { calls }"),
                (false, true) => out.push_str(" via { bus }"),
                (false, false) => {}
            }
            if let Some(p) = during {
                out.push_str(&format!(" during {}", p.display));
            }
            if let Some(a) = avoiding {
                out.push_str(&format!(" avoiding {}", a.display));
            }
            out
        }
        Law::OnlyEdges { src, dst, grants } => {
            let gs: Vec<String> = grants
                .iter()
                .map(|g| {
                    format!(
                        "{} {}",
                        if g.publish {
                            "publish"
                        } else {
                            "subscribe"
                        },
                        g.topic.display
                    )
                })
                .collect();
            format!(
                "only edges {} -> {} {{ {} }}",
                src.display,
                dst.display,
                gs.join("; ")
            )
        }
        Law::Bound { class, limit, from } => format!(
            "bound {} <= {} on paths from {}",
            class.class, limit, from.display
        ),
        Law::RequireEndpoint {
            publishers,
            group,
            topic,
        } => format!(
            "require {}(some {}, topic {})",
            if *publishers { "publishes" } else { "subscribes" },
            group.display,
            topic.display
        ),
        Law::RequireSealed { group } => {
            format!("require sealed(all {})", group.display)
        }
        Law::RequireAttributed { class } => {
            format!("require attributed(all {})", class.class)
        }
        Law::Cover { seed, group } => format!(
            "cover topic in seed({}): subscribed_by(some {})",
            seed.display, group.display
        ),
        Law::Count {
            publishers,
            topic,
            cmp,
            n,
        } => format!(
            "count {}(topic {}) {} {}",
            if *publishers {
                "publishers"
            } else {
                "subscribers"
            },
            topic.display,
            cmp,
            n
        ),
        _ => return None,
    })
}
