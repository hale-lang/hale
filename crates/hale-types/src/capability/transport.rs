//! What a binding's transport can carry (F.40 phase 3, P2 2 of 4): the
//! `bindings` family's capability data, beside the matrix it joins.
//!
//! A `bindings { }` entry may assert operational constraints on its route
//! (`where intra_machine, zero_copy`, spec/semantics.md § Operational
//! constraints (Form K)); the checker holds the entry's transport to
//! them. The verdict is a function of the transport kind and the
//! constraint, and of nothing else: it does not vary by target, as an FFI
//! type's does not (design §2.6), so it is one table, not a column per
//! target class. Whether a target lowers the transport at all is the
//! matrix's own `RemoteTransport(kind)` row ([`transport_cell`]).
//!
//! The table carries the checker's former `transport_satisfies` verbatim:
//! each cell is a `Satisfies`, a `Trusted` (an adapter's scope is the
//! adapter body's to know, so the checker takes the assertion on trust)
//! or a `Refuses` with the diagnostic's reason.

use hale_syntax::ast::BindingConstraint;

use super::{derive_capability_matrix, w, Behaviour, Capability, TargetClass, Transport, Witness};

/// A transport's verdict on one constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Guarantee {
    /// The transport satisfies the constraint intrinsically.
    Satisfies,
    /// The transport cannot say: a user adapter's scope is its body's.
    /// Taken on trust.
    Trusted,
    /// The transport cannot satisfy the constraint; the witness's reason
    /// is the diagnostic, as the checker words it after the topic.
    Refuses(Witness),
}

impl Guarantee {
    /// The diagnostic's reason when the transport cannot satisfy the
    /// constraint.
    pub fn refusal(&self) -> Option<&'static str> {
        match self {
            Guarantee::Refuses(w) => Some(w.reason),
            _ => None,
        }
    }
}

/// One transport's four cells, in [`BindingConstraint`]'s order of
/// declaration.
#[derive(Debug, Clone, Copy)]
pub struct GuaranteeRow {
    pub transport: Transport,
    pub intra_process: Guarantee,
    pub intra_machine: Guarantee,
    pub cross_machine: Guarantee,
    pub zero_copy: Guarantee,
}

impl GuaranteeRow {
    fn get(&self, c: BindingConstraint) -> &Guarantee {
        match c {
            BindingConstraint::IntraProcess => &self.intra_process,
            BindingConstraint::IntraMachine => &self.intra_machine,
            BindingConstraint::CrossMachine => &self.cross_machine,
            BindingConstraint::ZeroCopy => &self.zero_copy,
        }
    }
}

const SITE: &str = "crates/hale-types/src/check.rs::check_binding_constraints";
const SPEC: &str = "spec/semantics.md § Operational constraints (Form K)";

const fn refuses(reason: &'static str) -> Guarantee {
    Guarantee::Refuses(w(SITE, reason, SPEC))
}

pub const GUARANTEES: &[GuaranteeRow] = &[
    // unix: intra-machine substrate, kernel-memcpy at the socket boundary.
    GuaranteeRow {
        transport: Transport::Unix,
        intra_process: refuses("`unix` transport crosses OS process boundaries; cannot satisfy `intra_process`"),
        intra_machine: Guarantee::Satisfies,
        cross_machine: refuses("`unix` transport is host-local (AF_UNIX); cannot satisfy `cross_machine`"),
        zero_copy: refuses("`unix` transport memcpys at the kernel boundary; cannot satisfy `zero_copy`"),
    },
    // shm_ring: POSIX SHM ring substrate. Cross-process by design
    // (different procs mmap the same fd); host-local (POSIX SHM doesn't
    // traverse the network); satisfies zero_copy intrinsically.
    GuaranteeRow {
        transport: Transport::ShmRing,
        intra_process: refuses("`shm_ring` is cross-process by design (POSIX SHM); cannot satisfy `intra_process`"),
        intra_machine: Guarantee::Satisfies,
        cross_machine: refuses("`shm_ring` is host-local (POSIX SHM); cannot satisfy `cross_machine`"),
        zero_copy: Guarantee::Satisfies,
    },
    // Adapter: user-supplied. Trust for scope constraints (the adapter
    // body knows where it routes). Reject zero_copy: the Adapter contract
    // (`fn send(subject, bytes)`) requires serialization.
    GuaranteeRow {
        transport: Transport::Adapter,
        intra_process: Guarantee::Trusted,
        intra_machine: Guarantee::Trusted,
        cross_machine: Guarantee::Trusted,
        zero_copy: refuses(
            "`Adapter` transports cannot satisfy `zero_copy` — the Adapter contract (`fn send(subject, bytes)`) \
             requires serialization to Bytes",
        ),
    },
];

/// The transport's verdict on a constraint. Every transport has a row
/// (the test below), so there is no default arm.
pub fn guarantee(t: Transport, c: BindingConstraint) -> &'static Guarantee {
    GUARANTEES
        .iter()
        .find(|r| r.transport == t)
        .map(|r| r.get(c))
        .expect("every transport has a guarantee row")
}

/// What the matrix says of a transport kind on a target: its
/// `RemoteTransport(kind)` cell, whose verdict is `Lower` where the
/// target realizes the transport and `Reject` where it does not (the
/// adapter's on wasm32, today a late link refusal).
pub fn transport_cell(class: TargetClass, t: Transport) -> &'static Behaviour {
    derive_capability_matrix()
        .behaviour(class, Capability::RemoteTransport(t))
        .expect("every transport has a matrix row (law 1)")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table is the checker's former `transport_satisfies`, cell for
    /// cell: the same verdicts, the same words.
    #[test]
    fn the_table_carries_the_former_checkers_verdicts() {
        use BindingConstraint::*;
        use Transport::*;
        let want: &[(Transport, BindingConstraint, Option<&str>)] = &[
            (Unix, IntraProcess, Some("`unix` transport crosses OS process boundaries; cannot satisfy `intra_process`")),
            (Unix, IntraMachine, None),
            (Unix, CrossMachine, Some("`unix` transport is host-local (AF_UNIX); cannot satisfy `cross_machine`")),
            (Unix, ZeroCopy, Some("`unix` transport memcpys at the kernel boundary; cannot satisfy `zero_copy`")),
            (Adapter, IntraProcess, None),
            (Adapter, IntraMachine, None),
            (Adapter, CrossMachine, None),
            (
                Adapter,
                ZeroCopy,
                Some(
                    "`Adapter` transports cannot satisfy `zero_copy` — the Adapter contract (`fn send(subject, bytes)`) \
                     requires serialization to Bytes",
                ),
            ),
            (ShmRing, IntraProcess, Some("`shm_ring` is cross-process by design (POSIX SHM); cannot satisfy `intra_process`")),
            (ShmRing, IntraMachine, None),
            (ShmRing, CrossMachine, Some("`shm_ring` is host-local (POSIX SHM); cannot satisfy `cross_machine`")),
            (ShmRing, ZeroCopy, None),
        ];
        assert_eq!(want.len(), Transport::ALL.len() * 4, "every cell is pinned");
        for (t, c, refusal) in want {
            assert_eq!(guarantee(*t, *c).refusal(), *refusal, "{} × {}", t.name(), c.name());
        }
        for t in Transport::ALL {
            assert_eq!(GUARANTEES.iter().filter(|r| r.transport == t).count(), 1, "{}: one row", t.name());
        }
    }

    /// The matrix's transport rows: today's cells, on each target class.
    #[test]
    fn the_matrix_admits_each_transport_where_lowering_does() {
        for t in Transport::ALL {
            for class in [TargetClass::PosixAsync, TargetClass::PosixNoAsync] {
                assert!(transport_cell(class, t).is_lower(), "{} lowers on {}", t.name(), class.name());
            }
        }
        assert!(transport_cell(TargetClass::Wasm32, Transport::Unix).is_lower());
        assert!(transport_cell(TargetClass::Wasm32, Transport::ShmRing).is_lower());
        assert!(!transport_cell(TargetClass::Wasm32, Transport::Adapter).is_lower(), "the adapter's thread has no wasm");
    }
}
