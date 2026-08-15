//! Keyed delivery, bounds, and loss policy — recorded as authored,
//! before any claim consumes them.
//!
//! The polarity law (crate docs, "Keyed delivery"): judgments derive
//! `may_deliver` (unknown keys ADD possible edges — conservative for
//! negative claims) and `must_deliver` (exact coverage only) from
//! these raw facts. Three failure modes this schema exists to
//! prevent: treating keyed delivery as broadcast (false
//! reachability), dropping filtered edges (false isolation), and
//! discarding the filter syntax so every keyed-topic judgment is
//! permanently `uncertified`.
//!
//! The first implementation proves no arithmetic: it preserves the
//! authored filter and says what it does not know (`Unknown`).

/// A routing-key value. Typecheck fixes ONE key type per topic, so a
/// topic's publishes and subscriptions never mix variants.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum KeyValue {
    Int(i64),
    Str(String),
}

/// A topic's declared key: the `keyed_by` payload field.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TopicKey {
    pub field: String,
}

/// What a subscription's `where key == …` filter admits.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum KeyPredicate {
    /// Unkeyed subscription: every message on the subject.
    Any,
    /// `where key == <literal>`.
    EqLiteral(KeyValue),
    /// `where key == replica` — satisfied by this instance's
    /// replica index (the instance population comes from placement;
    /// the delivery semantics stay on the subscription — placement
    /// remains semantics-free).
    EqReplica,
    /// The filter exists but its value is not statically known.
    /// Adds possible edges; never supports a guarantee.
    Unknown,
}

/// What key values a publication site can produce.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum KeyDomain {
    /// Statically exact value set (sorted, deduplicated).
    Exact(Vec<KeyValue>),
    /// An integer interval, inclusive.
    IntRange { min: i64, max: i64 },
    /// Any value of the key's type — exact *type*, unknown values.
    AnyOfType(String),
    /// Nothing is known about the produced keys.
    Unknown,
}

/// A queue bound. `Unbounded` is the declared default, not an
/// unknown — an unknown capacity would be a [`Hole`].
/// `Bounded(0)` is invalid (the settled #255 design excludes it);
/// [`ApplicationModel::validate`] rejects it.
///
/// [`Hole`]: crate::hole::Hole
/// [`ApplicationModel::validate`]: crate::application::ApplicationModel::validate
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Capacity {
    Unbounded,
    Bounded(u64),
}

/// The topic-level half of the #255 bounds design: `bounded(N)`
/// declared ON THE TOPIC, applying to every publisher, with the
/// topic's `on_full` contract selected before any per-site
/// disposition. Not reconstructible from publish rows — the N and
/// the refusal contract are topic facts, which is why they are
/// schema now (Change 1 pins what is expensive to retrofit).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TopicBound {
    /// Must be > 0; validate rejects `0`.
    pub capacity: u64,
    pub on_full: TopicOnFull,
}

/// What a full bounded topic does to a publish.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TopicOnFull {
    /// `on_full: fail` — the send contract refuses at the publish
    /// site (before any per-site disposition applies).
    Fail,
}

/// What a bounded subscription does when full (GH #255 vocabulary).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShedPolicy {
    /// Unbounded, or bounded-with-refusal: nothing is shed.
    None,
    DropOld,
    DropNew,
}

/// The publish site's declared disposition when delivery cannot be
/// accepted (GH #255): the send-site half of the loss contract.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PublishDisposition {
    /// No explicit disposition authored at the site.
    Default,
    Raise,
    Discard,
    Handler,
    Wait,
}

/// A transport binding's declared loss behavior — what the publish
/// contract says "accepted" obligates it to when the link degrades.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BindingLossBehavior {
    /// Lossy by declaration (udp-class): downstream loss is within
    /// contract.
    Drop,
    /// Loss is structural but `or wait` can park through a
    /// reconnect window (unix connect-class).
    WaitCapable,
    /// Loss fails the binding (structural exit / supervision).
    Fail,
}
