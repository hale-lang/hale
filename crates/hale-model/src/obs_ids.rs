//! GH #476 Change 8 — canonical entity ids for the observation
//! manifest.
//!
//! A live process's manifest identifies entities by NAME plus a
//! registration-order id: the topic row for `sense.reading` is
//! whichever row that subject happened to register into. A consumer
//! (iris) holding the source-derived model had no way to join the
//! two except by matching strings and trusting that the compiler's
//! spelling and the runtime's spelling agree — which is exactly the
//! kind of second authority this epic exists to delete.
//!
//! So the compiler answers instead. This module projects the model
//! down to `(manifest kind, manifest name, canonical id)` rows; the
//! CLI hands them to codegen, codegen stamps them in the observation
//! prelude, and the runtime writes the id into the manifest entry's
//! `aux_b` when that entry is created. The ids are indices into THIS
//! model's tables, so they are only meaningful alongside the
//! header's `model_hash`.
//!
//! Encoding: `index + 1`, because `aux_b == 0` is the pre-existing
//! "nothing here" value and every unstamped build must keep reading
//! that way.

use crate::application::ApplicationModel;

/// The manifest kinds an entity id can be attached to. Values are
/// the runtime's `MK_*` constants (`lotus_obs.c`) — this enum is
/// the Rust half of that shared numbering.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ObsEntityKind {
    /// `MK_TOPIC` — keyed by WIRE SUBJECT (the manifest fuses every
    /// publisher of a subject into one row), so the canonical id is
    /// the model's `SubjectId`, not `TopicId`.
    Topic = 0,
    /// `MK_LOCUS_TYPE` — keyed by locus type name; `LocusDeclId`.
    LocusType = 1,
    /// `MK_BINDING` — keyed by the bound subject; `BindingId`.
    Binding = 2,
}

/// One stamp: "in this model, the manifest row of `kind` named
/// `name` is entity `id`".
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ObsEntityId {
    pub kind: ObsEntityKind,
    pub name: String,
    /// `index + 1` — see the module note on the zero value.
    pub id: u64,
}

/// Project the model's entity tables into manifest stamps, in a
/// deterministic order (kind, then name).
pub fn obs_entity_ids(m: &ApplicationModel) -> Vec<ObsEntityId> {
    let mut out: Vec<ObsEntityId> = Vec::new();
    for (i, s) in m.entities.subjects.iter().enumerate() {
        // Wildcard patterns are not manifest rows: the runtime
        // registers the CONCRETE subject a message was published
        // on, never the pattern a subscriber matched with.
        if !s.exact {
            continue;
        }
        out.push(ObsEntityId {
            kind: ObsEntityKind::Topic,
            name: s.pattern.clone(),
            id: i as u64 + 1,
        });
    }
    for (i, l) in m.entities.loci.iter().enumerate() {
        // The RAW name: what codegen emits as the locus type name
        // and what `lotus_obs_locus_birth` therefore receives. The
        // author-facing `display` (alias-qualified for imports) is
        // never on the wire.
        out.push(ObsEntityId {
            kind: ObsEntityKind::LocusType,
            name: l.name.clone(),
            id: i as u64 + 1,
        });
    }
    for (i, b) in m.entities.bindings.iter().enumerate() {
        let Some(s) = m.entities.subjects.get(b.subject.index()) else {
            continue;
        };
        out.push(ObsEntityId {
            kind: ObsEntityKind::Binding,
            name: s.pattern.clone(),
            id: i as u64 + 1,
        });
    }
    out.sort_by(|a, b| {
        (a.kind as u8, &a.name).cmp(&(b.kind as u8, &b.name))
    });
    out.dedup_by(|a, b| a.kind == b.kind && a.name == b.name);
    out
}
