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

/// A routing-key value — one variant per key-eligible field type in
/// the shipped routing contract (spec/semantics.md "Width is
/// inherited from the keyed_by field's type"): `Bool`, `Int`,
/// `Time`/`Duration` (ns), no-payload `enum` (by variant name — the
/// source-level identity; the runtime tag is derivation), `Decimal`
/// (the spec-defined u128 comparison pair), `String`. Typecheck
/// fixes ONE key type per topic, so a topic's publishes and
/// subscriptions never mix variants.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum KeyValue {
    Bool(bool),
    Int(i64),
    /// Nanoseconds since epoch.
    Time(i64),
    /// Nanoseconds.
    Duration(i64),
    /// A no-payload enum key, by variant name.
    EnumTag(String),
    /// The spec-defined u128 comparison pair (`key_lo`, `key_hi`) —
    /// equality over Decimal keys IS two i64 compares by contract.
    Decimal {
        lo: u64,
        hi: u64,
    },
    Str(String),
}

/// A topic's declared key: the `keyed_by` payload field plus the
/// topic's `on_unmatched` policy — what happens when a keyed
/// publish matches no subscriber. Both are routing-contract facts
/// the delivery judgments need (a `Fail` policy changes the send
/// contract; a `Fallback` policy adds a delivery edge to the
/// catch-unmatched subscriber).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TopicKey {
    pub field: String,
    pub on_unmatched: KeyOnUnmatched,
}

/// `on_unmatched:` policy of a keyed topic (spec/semantics.md).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyOnUnmatched {
    /// Drop silently (the default).
    Swallow,
    /// The publish becomes fallible at the send site.
    Fail,
    /// A `where key == _` subscriber receives unmatched messages.
    /// The model law (validated): a Fallback topic has at least one
    /// [`KeyPredicate::Fallback`] subscription, and the `_`
    /// sentinel is legal ONLY on Fallback topics.
    Fallback,
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
    /// The `where key == _` catch-unmatched subscription — receives
    /// exactly the messages no other filter matched. Legal only on
    /// topics with `on_unmatched: fallback` (validated). Distinct
    /// from [`KeyPredicate::Any`], which is the ordinary unkeyed
    /// full-delivery subscription.
    Fallback,
    /// The filter exists but its value is not statically known.
    /// Adds possible edges; never supports a guarantee.
    Unknown,
}

/// What key values a publication site can produce. Only meaningful
/// on a KEYED topic: an unkeyed publish carries NO KeyDomain
/// (`Publish.key_domain: Option<KeyDomain>` is `None`), and the
/// validator enforces the correspondence both ways — inventing a
/// domain for an unkeyed publish (or omitting one on a keyed topic)
/// is not a model.
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
