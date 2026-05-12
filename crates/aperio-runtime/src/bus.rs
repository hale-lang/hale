//! Bus router and transport abstraction.
//!
//! The interpreter's view of the bus is uniform: every subject
//! has a [`Transport`] that knows how to enqueue published
//! payloads and drain them as deliveries to subscribers. The
//! interpreter loops over `drain` until empty, dispatching each
//! delivery as a handler invocation.
//!
//! Two transports ship with v0:
//!
//! - [`SyncDispatch`] — the default. Publishing a payload
//!   immediately enqueues one [`Delivery`] per subscriber; the
//!   interpreter drains after each `publish`, so the call
//!   chain returns only after every subscriber has run. Same
//!   semantics as the original direct dispatch.
//!
//! - [`RingBuffer`] — LMAX-style. Pre-allocated typed-slot
//!   ring with a producer cursor and one consumer cursor per
//!   subscriber. Publishing writes to the next slot and
//!   advances the producer cursor; draining reads up to a
//!   batch from each consumer cursor and advances. v0 keeps
//!   this in-process and single-threaded; the cross-process
//!   shared-memory variant is the natural Phase 2 extension.
//!
//! Source-level surface is unchanged. `bus { ... }` and `<-`
//! don't know which transport is bound; that's a deployment
//! decision. Default is [`SyncDispatch`]; specific subjects
//! opt into [`RingBuffer`] via [`BusRouter::with_transport`]
//! at runtime construction.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::rc::Rc;

use crate::value::{LocusHandle, Value};

#[derive(Clone, Debug)]
pub struct Subscription {
    pub locus: LocusHandle,
    pub handler: String,
    /// m42: the locus's parent at subscribe time, captured so
    /// tick-epoch closures fired at bus-handler completion can
    /// route violations to the parent's `on_failure(child, err)`
    /// without walking parent_stack (which during a bus dispatch
    /// has the subscriber itself on top, not the actual parent).
    pub parent: Option<LocusHandle>,
}

/// One pending delivery: the subscription that should receive
/// the payload, plus the payload itself.
#[derive(Debug)]
pub struct Delivery {
    pub subscription: Subscription,
    pub payload: Value,
}

/// Per-subject delivery semantics. Implementations decide
/// when a published payload becomes a [`Delivery`] returned
/// from `drain`.
pub trait Transport: std::fmt::Debug {
    fn subscribe(&mut self, sub: Subscription);
    fn publish(&mut self, payload: Value);
    fn drain(&mut self) -> Vec<Delivery>;
    /// Configuration label used in diagnostics.
    fn label(&self) -> &'static str;
}

// ============================================================
// SyncDispatch — direct delivery, the default.
// ============================================================

#[derive(Default, Debug)]
pub struct SyncDispatch {
    subscribers: Vec<Subscription>,
    pending: VecDeque<Delivery>,
}

impl SyncDispatch {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Transport for SyncDispatch {
    fn subscribe(&mut self, sub: Subscription) {
        self.subscribers.push(sub);
    }

    fn publish(&mut self, payload: Value) {
        for sub in &self.subscribers {
            self.pending.push_back(Delivery {
                subscription: sub.clone(),
                payload: payload.clone(),
            });
        }
    }

    fn drain(&mut self) -> Vec<Delivery> {
        std::mem::take(&mut self.pending).into_iter().collect()
    }

    fn label(&self) -> &'static str {
        "sync"
    }
}

// ============================================================
// RingBuffer — LMAX-style, pre-allocated slots, per-consumer
// cursors. v0: in-process, single-threaded. The data structure
// is the same shape as a multi-process shared-memory
// disruptor; the threading + memory-mapping work comes later.
// ============================================================

#[derive(Debug)]
pub struct RingBuffer {
    /// Slot capacity. Must be a power of two for the wrap mask
    /// to work; we just round up if not.
    capacity: usize,
    /// Mask = capacity - 1. Used for `index % capacity`.
    mask: usize,
    /// Pre-allocated slots. Each slot holds the most recent
    /// payload written to that ring position. v0 holds a
    /// `Value` — a typed shared-memory variant would hold
    /// fixed-layout typed bytes.
    slots: Vec<Option<Value>>,
    /// Monotonically-increasing producer cursor. Index into
    /// the ring by `producer & mask`.
    producer: u64,
    /// Subscribers and their consumer cursors. One cursor per
    /// subscriber; the cursor names the *next* sequence the
    /// subscriber should read.
    subscribers: Vec<(Subscription, u64)>,
}

impl RingBuffer {
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.next_power_of_two().max(2);
        Self {
            capacity: cap,
            mask: cap - 1,
            slots: (0..cap).map(|_| None).collect(),
            producer: 0,
            subscribers: Vec::new(),
        }
    }

    fn min_consumer(&self) -> u64 {
        self.subscribers
            .iter()
            .map(|(_, c)| *c)
            .min()
            .unwrap_or(self.producer)
    }
}

impl Transport for RingBuffer {
    fn subscribe(&mut self, sub: Subscription) {
        // New subscribers start at the current producer cursor —
        // they don't see history. Matches typical disruptor
        // semantics for late joiners.
        self.subscribers.push((sub, self.producer));
    }

    fn publish(&mut self, payload: Value) {
        // Backpressure: if the slowest consumer is `capacity`
        // sequences behind, the slot we'd overwrite still has
        // an unread message. v0 silently drops the old slot
        // (overrun); a production transport would block /
        // signal pressure / panic per a configured wait
        // strategy. v0 keeps it simple but reports if anyone
        // gets overrun via a crude check.
        if self.subscribers.is_empty() {
            // No subscribers; drop on the floor (sync would
            // also have no-op'd).
            return;
        }
        let oldest_unread = self.min_consumer();
        if self.producer.saturating_sub(oldest_unread) >= self.capacity as u64 {
            // Overrun. We're about to overwrite an unread
            // slot. v0 advances the lagging cursor to skip
            // the dropped slot.
            for (_, cursor) in self.subscribers.iter_mut() {
                if self.producer.saturating_sub(*cursor) >= self.capacity as u64 {
                    *cursor = self.producer.saturating_sub(self.capacity as u64 - 1);
                }
            }
        }
        let idx = (self.producer as usize) & self.mask;
        self.slots[idx] = Some(payload);
        self.producer = self.producer.wrapping_add(1);
    }

    fn drain(&mut self) -> Vec<Delivery> {
        let mut out = Vec::new();
        for (sub, cursor) in self.subscribers.iter_mut() {
            while *cursor < self.producer {
                let idx = (*cursor as usize) & self.mask;
                if let Some(payload) = self.slots[idx].clone() {
                    out.push(Delivery {
                        subscription: sub.clone(),
                        payload,
                    });
                }
                *cursor = cursor.wrapping_add(1);
            }
        }
        out
    }

    fn label(&self) -> &'static str {
        "ring"
    }
}

// ============================================================
// Configuration: which transport per subject.
// ============================================================

#[derive(Debug, Clone)]
pub enum TransportKind {
    Sync,
    Ring { capacity: usize },
}

impl Default for TransportKind {
    fn default() -> Self {
        TransportKind::Sync
    }
}

impl TransportKind {
    pub fn build(&self) -> Box<dyn Transport> {
        match self {
            TransportKind::Sync => Box::new(SyncDispatch::new()),
            TransportKind::Ring { capacity } => Box::new(RingBuffer::new(*capacity)),
        }
    }
}

// ============================================================
// BusRouter
// ============================================================

#[derive(Default)]
pub struct BusRouter {
    transports: Rc<RefCell<BTreeMap<String, Box<dyn Transport>>>>,
    /// Per-subject configuration override. Subjects without an
    /// override use [`TransportKind::Sync`] when first
    /// referenced.
    config: Rc<RefCell<BTreeMap<String, TransportKind>>>,
    /// m94: wildcard subscriptions, kept separately from the
    /// per-subject transport map. Each entry is `(pattern,
    /// subscription)`; on publish, every pattern is matched
    /// against the published subject and matching deliveries
    /// land in `wildcard_pending`.
    wildcard_subs: Rc<RefCell<Vec<(String, Subscription)>>>,
    wildcard_pending: Rc<RefCell<VecDeque<Delivery>>>,
}

/// m94: subject wildcard matching.
///
/// v0 supports one form: a trailing `**` that matches *zero or
/// more* remaining dot-separated segments. So `"log.app.**"`
/// matches `"log.app"`, `"log.app.db"`, `"log.app.db.query"` —
/// the publishing logger's own subject AND any descendant. This
/// is the cascade-friendly semantics: subscribing to `log.app.**`
/// captures the whole sub-tree including its root.
///
/// `**` must appear at the end of the pattern, preceded either
/// by `.` or by nothing (the bare `"**"` pattern matches every
/// subject). `**` in any other position rejects.
///
/// Patterns without `**` fall through to exact equality — the
/// cheap path stays cheap.
pub fn subject_match(pattern: &str, subject: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix("**") {
        if prefix.is_empty() {
            // Bare "**" — matches every subject.
            return true;
        }
        if !prefix.ends_with('.') {
            // "log**" or similar — invalid, no match.
            return false;
        }
        // The pattern is "<root>." + "**". Two valid forms:
        //   - subject equals root (no trailing segments)
        //   - subject starts with "<root>." and has a tail
        let root = &prefix[..prefix.len() - 1]; // strip trailing "."
        if subject == root {
            return true;
        }
        subject.starts_with(prefix) && subject.len() > prefix.len()
    } else if pattern.contains("**") {
        // ** somewhere other than the end — reject.
        false
    } else {
        pattern == subject
    }
}

impl std::fmt::Debug for BusRouter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BusRouter")
            .field("subjects", &self.transports.borrow().keys().collect::<Vec<_>>())
            .finish()
    }
}

impl BusRouter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Configure the transport for a subject. Must be called
    /// before the first publish/subscribe on that subject. If
    /// the subject is later referenced and no config exists,
    /// a [`SyncDispatch`] is built lazily.
    pub fn with_transport(&mut self, subject: impl Into<String>, kind: TransportKind) {
        self.config.borrow_mut().insert(subject.into(), kind);
    }

    fn ensure(&self, subject: &str) {
        let mut t = self.transports.borrow_mut();
        if !t.contains_key(subject) {
            let kind = self
                .config
                .borrow()
                .get(subject)
                .cloned()
                .unwrap_or_default();
            t.insert(subject.to_string(), kind.build());
        }
    }

    pub fn subscribe(
        &self,
        subject: String,
        locus: LocusHandle,
        handler: String,
        parent: Option<LocusHandle>,
    ) {
        let sub = Subscription {
            locus,
            handler,
            parent,
        };
        // m94: wildcard subscriptions live in a separate registry
        // and are checked against every publish, independent of
        // the per-subject transport map.
        if subject.contains("**") {
            self.wildcard_subs.borrow_mut().push((subject, sub));
            return;
        }
        self.ensure(&subject);
        self.transports
            .borrow_mut()
            .get_mut(&subject)
            .expect("subject ensured")
            .subscribe(sub);
    }

    pub fn publish(&self, subject: &str, payload: Value) {
        self.ensure(subject);
        self.transports
            .borrow_mut()
            .get_mut(subject)
            .expect("subject ensured")
            .publish(payload.clone());
        // m94: also fan out to any wildcard subscribers whose
        // pattern matches this subject. Deliveries land in
        // wildcard_pending and ride the same drain loop as
        // exact-match deliveries.
        let wildcards = self.wildcard_subs.borrow();
        if !wildcards.is_empty() {
            let mut pending = self.wildcard_pending.borrow_mut();
            for (pattern, sub) in wildcards.iter() {
                if subject_match(pattern, subject) {
                    pending.push_back(Delivery {
                        subscription: sub.clone(),
                        payload: payload.clone(),
                    });
                }
            }
        }
    }

    /// Drain pending deliveries across all configured
    /// transports. Returns one batch; the caller is expected
    /// to loop until drain returns empty (so re-entrant
    /// publishes from inside a handler get serviced too).
    pub fn drain_all(&self) -> Vec<Delivery> {
        let mut out = Vec::new();
        for transport in self.transports.borrow_mut().values_mut() {
            out.extend(transport.drain());
        }
        // m94: wildcard subscribers' deliveries.
        out.extend(self.wildcard_pending.borrow_mut().drain(..));
        out
    }
}

// ============================================================
// Tests for the transport implementations themselves.
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_subscription(name: &str, handler: &str) -> Subscription {
        // For tests we just need *some* LocusHandle. Build a
        // bogus one — these tests don't dispatch handlers,
        // they just inspect Delivery shape.
        use std::cell::RefCell;
        use std::rc::Rc;
        let decl = aperio_syntax::ast::LocusDecl {
            name: aperio_syntax::ast::Ident {
                name: name.to_string(),
                span: aperio_syntax::Span {
                start: aperio_syntax::Pos(0),
                end: aperio_syntax::Pos(0),
            },
            },
            generics: Vec::new(),
            annotations: Vec::new(),
            form: None,
            members: Vec::new(),
            span: aperio_syntax::Span {
                start: aperio_syntax::Pos(0),
                end: aperio_syntax::Pos(0),
            },
        };
        Subscription {
            locus: LocusHandle {
                name: name.to_string(),
                state: Rc::new(RefCell::new(BTreeMap::new())),
                children: Rc::new(RefCell::new(Vec::new())),
                decl: Rc::new(decl),
                dissolved: Rc::new(std::cell::Cell::new(false)),
                restart_count: Rc::new(std::cell::Cell::new(0)),
                quarantined: Rc::new(std::cell::Cell::new(false)),
                duration_last_fire: Rc::new(RefCell::new(Vec::new())),
                parent: Rc::new(RefCell::new(None)),
                restart_in_place_pending: Rc::new(std::cell::Cell::new(false)),
                accumulators: Rc::new(RefCell::new(BTreeMap::new())),
                slots: Rc::new(RefCell::new(BTreeMap::new())),
            },
            handler: handler.to_string(),
            parent: None,
        }
    }

    #[test]
    fn sync_delivers_to_each_subscriber() {
        let mut t = SyncDispatch::new();
        t.subscribe(fake_subscription("A", "ha"));
        t.subscribe(fake_subscription("B", "hb"));
        t.publish(Value::Int(42));
        let deliveries = t.drain();
        assert_eq!(deliveries.len(), 2);
        assert!(matches!(deliveries[0].payload, Value::Int(42)));
        assert!(matches!(deliveries[1].payload, Value::Int(42)));
    }

    #[test]
    fn ring_delivers_in_order() {
        let mut t = RingBuffer::new(4);
        t.subscribe(fake_subscription("C", "h"));
        t.publish(Value::Int(1));
        t.publish(Value::Int(2));
        t.publish(Value::Int(3));
        let deliveries = t.drain();
        let payloads: Vec<i64> = deliveries
            .iter()
            .map(|d| match d.payload {
                Value::Int(n) => n,
                _ => panic!("not int"),
            })
            .collect();
        assert_eq!(payloads, vec![1, 2, 3]);
    }

    #[test]
    fn ring_drain_is_idempotent_when_no_new_publish() {
        let mut t = RingBuffer::new(4);
        t.subscribe(fake_subscription("C", "h"));
        t.publish(Value::Int(1));
        let _ = t.drain();
        // Second drain should be empty: cursor advanced past
        // the only entry.
        assert!(t.drain().is_empty());
    }

    #[test]
    fn ring_overrun_skips_dropped_slot() {
        let mut t = RingBuffer::new(2); // capacity 2
        t.subscribe(fake_subscription("C", "h"));
        t.publish(Value::Int(1));
        t.publish(Value::Int(2));
        t.publish(Value::Int(3)); // overruns slot 0
        let deliveries = t.drain();
        // Slow consumer skipped the lost slot; got the live
        // ones (capacity 2 → after publish-3, slots hold [3,2]
        // with producer=3, consumer skips to 1).
        let payloads: Vec<i64> = deliveries
            .iter()
            .map(|d| match d.payload {
                Value::Int(n) => n,
                _ => panic!("not int"),
            })
            .collect();
        assert!(
            payloads == vec![2, 3] || payloads == vec![3],
            "got {:?}",
            payloads
        );
    }

    #[test]
    fn router_default_is_sync() {
        let router = BusRouter::new();
        router.subscribe("s".into(), fake_subscription("A", "h").locus, "h".into(), None);
        router.publish("s", Value::Int(7));
        let deliveries = router.drain_all();
        assert_eq!(deliveries.len(), 1);
        assert!(matches!(deliveries[0].payload, Value::Int(7)));
    }

    #[test]
    fn router_per_subject_ring_override() {
        let mut router = BusRouter::new();
        router.with_transport("hot", TransportKind::Ring { capacity: 16 });
        router.subscribe("hot".into(), fake_subscription("A", "h").locus, "h".into(), None);
        router.publish("hot", Value::Int(1));
        router.publish("hot", Value::Int(2));
        let deliveries = router.drain_all();
        assert_eq!(deliveries.len(), 2);
    }

    // ============================================================
    // m94: subject wildcard matching.
    // ============================================================

    #[test]
    fn subject_match_exact() {
        assert!(subject_match("log.app", "log.app"));
        assert!(!subject_match("log.app", "log.api"));
        assert!(!subject_match("log.app", "log"));
    }

    #[test]
    fn subject_match_trailing_double_star() {
        assert!(subject_match("log.**", "log.app"));
        assert!(subject_match("log.**", "log.app.db"));
        assert!(subject_match("log.**", "log.app.db.query"));
        // m94: zero+ trailing semantics — root subject matches too.
        assert!(subject_match("log.**", "log"));
        // Wrong root.
        assert!(!subject_match("log.**", "logs"));
        assert!(!subject_match("log.**", "logs.app"));
    }

    #[test]
    fn subject_match_bare_double_star() {
        assert!(subject_match("**", "anything"));
        assert!(subject_match("**", "a.b.c"));
        // Bare "**" matches every subject including empty.
        assert!(subject_match("**", ""));
    }

    #[test]
    fn subject_match_double_star_must_be_trailing() {
        // ** in the middle is rejected (no fancy multi-segment
        // matching in v0).
        assert!(!subject_match("log.**.error", "log.app.error"));
        // ** without preceding dot is rejected.
        assert!(!subject_match("log**", "logXapp"));
    }

    #[test]
    fn router_wildcard_subscriber_receives_matching_publish() {
        let router = BusRouter::new();
        router.subscribe(
            "log.**".into(),
            fake_subscription("Sink", "on_log").locus,
            "on_log".into(),
            None,
        );
        router.publish("log.app", Value::Int(1));
        router.publish("log.app.db", Value::Int(2));
        router.publish("other.thing", Value::Int(99));
        let deliveries = router.drain_all();
        assert_eq!(deliveries.len(), 2, "wildcard should receive 2 of 3");
        assert!(matches!(deliveries[0].payload, Value::Int(1)));
        assert!(matches!(deliveries[1].payload, Value::Int(2)));
    }

    #[test]
    fn router_exact_and_wildcard_both_fire() {
        let router = BusRouter::new();
        router.subscribe(
            "log.app".into(),
            fake_subscription("Exact", "on_app").locus,
            "on_app".into(),
            None,
        );
        router.subscribe(
            "log.**".into(),
            fake_subscription("Wild", "on_any").locus,
            "on_any".into(),
            None,
        );
        router.publish("log.app", Value::Int(7));
        let deliveries = router.drain_all();
        // Both subscribers see the payload exactly once.
        assert_eq!(deliveries.len(), 2);
        let handlers: Vec<&str> = deliveries
            .iter()
            .map(|d| d.subscription.handler.as_str())
            .collect();
        assert!(handlers.contains(&"on_app"));
        assert!(handlers.contains(&"on_any"));
    }
}
