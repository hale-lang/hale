//! In-memory bus router for the v0 interpreter.
//!
//! v0 transport: synchronous in-process delivery. When a locus
//! is instantiated, every `subscribe SUBJECT as HANDLER ...` in
//! its `bus { ... }` block registers a [`Subscription`] in the
//! router. When `"subject" <- v` fires, the router looks up the
//! subject and invokes each subscribed handler in registration
//! order.
//!
//! Phase 2 production runtime will replace this with a
//! transport-bound multiplexer (NATS, UDP multicast, ...) and
//! push delivery onto the cooperative scheduler. The interface
//! shape (subjects + subscriptions) doesn't change.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use crate::value::LocusHandle;

#[derive(Default, Debug)]
pub struct BusRouter {
    /// `subject -> [(subscriber, handler_name)]`. Multiple
    /// subscribers per subject are permitted; they fire in
    /// registration order.
    subscribers: Rc<RefCell<BTreeMap<String, Vec<Subscription>>>>,
}

#[derive(Clone, Debug)]
pub struct Subscription {
    pub locus: LocusHandle,
    pub handler: String,
}

impl BusRouter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn subscribe(&self, subject: String, locus: LocusHandle, handler: String) {
        self.subscribers
            .borrow_mut()
            .entry(subject)
            .or_default()
            .push(Subscription { locus, handler });
    }

    /// Snapshot the subscribers for a subject. Returning a
    /// fresh Vec rather than holding the borrow protects
    /// against re-entrant publish (a handler that publishes
    /// while we're iterating).
    pub fn subscribers_for(&self, subject: &str) -> Vec<Subscription> {
        self.subscribers
            .borrow()
            .get(subject)
            .cloned()
            .unwrap_or_default()
    }
}
