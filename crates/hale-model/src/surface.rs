//! GH #1417: the `surface` family — the program's API as rows
//! (spec/api.md § Surfaces and their rows, § The contract digest).
//!
//! A [`Surface`] is a named table of operations; each [`SurfaceRow`] is
//! one `rpc` line of an `api` block or one `@rpc` handler, and both
//! spellings produce the same row. A surface's digest is the fold of its
//! rows in canonical order, each reduced to its member, the contract
//! shape hashes of its request, response and error types
//! (`spec/model.md` § The shape of a type) and its required roles; the
//! framing is [`surface_digest_input`]'s, byte for byte. A hub's stream
//! rows fold the same way under their own framing
//! ([`stream_digest_input`]).
//!
//! The rows are `hale-types`' (`surfaces::surface_rows`), projected; the
//! model hashes nothing it did not receive, and its law
//! (`ApplicationModel::validate`) holds each surface's digest to the fold
//! of its own rows.

use crate::ids::{FunctionId, ProvenanceId, SurfaceId};

/// A surface: an `api` block, or the seed's default surface its `@rpc`
/// handlers feed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Surface {
    /// The block's name, or the seed's for its default surface.
    pub name: String,
    /// The contract digest: [`surface_digest`] over its rows.
    pub digest: u64,
    /// The `api` block, or the first `@rpc` of a default surface.
    pub provenance: ProvenanceId,
}

/// A type a row names: its spelling, its contract shape and that
/// shape's hash.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RowType {
    /// The type as the handler's signature writes it.
    pub display: String,
    /// Its contract shape (`spec/model.md` § The shape of a type).
    pub shape: String,
    /// The FNV-1a/64 fold of `shape`.
    pub hash: u64,
}

/// One row of a surface.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SurfaceRow {
    /// The surface the row is a row of.
    pub surface: SurfaceId,
    /// The handler as a caller names it: `Locus::fn`, the locus in the
    /// row author's spelling.
    pub member: String,
    /// The handler's function row.
    pub handler: Option<FunctionId>,
    /// The handler's one value parameter's type, if it takes one.
    pub request: Option<RowType>,
    /// Its return type; `None` for `()`.
    pub response: Option<RowType>,
    /// `E` of `fallible(E)`; `None` when it is not fallible.
    pub error: Option<RowType>,
    /// The error type is `ClosureViolation`: a failure is the server
    /// error, and a description carries no error schema for the member.
    pub server_error: bool,
    /// Every pool an instance of the handler's locus runs on, sorted:
    /// `main`, a pool's name, `pinned:<path>`. Which one answers is the
    /// receiver instance a serve site binds.
    pub pools: Vec<String>,
    /// The roles a caller must hold, as written.
    pub requires: Vec<String>,
    /// The `rpc` line, or the `@rpc` attribute.
    pub provenance: ProvenanceId,
}

/// What one row contributes to its surface's digest.
#[derive(Clone, Copy, Debug)]
pub struct DigestLine<'a> {
    /// The row's member name.
    pub member: &'a str,
    /// The request's shape hash; `None` writes `-`.
    pub request: Option<u64>,
    /// The response's shape hash; `None` (a `()` handler) writes `-`.
    pub response: Option<u64>,
    /// The error type's shape hash; `None` (not fallible) writes `-`.
    pub error: Option<u64>,
    /// The required roles as written; the digest sorts them.
    pub requires: &'a [String],
}

impl<'a> From<&'a SurfaceRow> for DigestLine<'a> {
    fn from(r: &'a SurfaceRow) -> DigestLine<'a> {
        DigestLine {
            member: &r.member,
            request: r.request.as_ref().map(|t| t.hash),
            response: r.response.as_ref().map(|t| t.hash),
            error: r.error.as_ref().map(|t| t.hash),
            requires: &r.requires,
        }
    }
}

fn hash_field(h: Option<u64>) -> String {
    h.map_or_else(|| "-".to_string(), |h| format!("{h:016x}"))
}

/// Required roles as a digest field: sorted as bytes, each once, joined
/// by `,`, or `-` for none.
fn roles_field(roles: &[String]) -> String {
    let set: std::collections::BTreeSet<&str> = roles.iter().map(String::as_str).collect();
    if set.is_empty() {
        "-".to_string()
    } else {
        set.into_iter().collect::<Vec<_>>().join(",")
    }
}

/// The hash input of a surface's digest (spec/api.md § The contract
/// digest): the header line `hale-api-surface 1`, then one line per row
/// ordered by member name as bytes, five TAB-separated fields (member,
/// request, response and error shape hashes as sixteen lowercase hex
/// digits or `-`, the sorted required roles joined by `,` or `-`), every
/// line ended by one LF.
pub fn surface_digest_input(lines: &[DigestLine<'_>]) -> String {
    let mut sorted: Vec<&DigestLine<'_>> = lines.iter().collect();
    sorted.sort_by(|a, b| a.member.as_bytes().cmp(b.member.as_bytes()));
    let mut out = String::from("hale-api-surface 1\n");
    for l in sorted {
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\n",
            l.member,
            hash_field(l.request),
            hash_field(l.response),
            hash_field(l.error),
            roles_field(l.requires)
        ));
    }
    out
}

/// A surface's contract digest: the FNV-1a/64 fold of
/// [`surface_digest_input`].
pub fn surface_digest(lines: &[DigestLine<'_>]) -> u64 {
    hale_graph::identity::fnv64(surface_digest_input(lines).as_bytes())
}

/// A digest as every document writes it: `fnv1a64:` and sixteen
/// lowercase hex digits.
pub fn digest_text(d: u64) -> String {
    format!("fnv1a64:{d:016x}")
}

/// One stream row of a hub (spec/api.md § Streams), as its digest reads
/// it.
#[derive(Clone, Copy, Debug)]
pub struct StreamLine<'a> {
    /// The topic's name.
    pub topic: &'a str,
    /// The payload's contract shape hash.
    pub payload: u64,
    /// `out` for a topic the program publishes, `in` for one it
    /// subscribes.
    pub direction: &'a str,
    /// The codec the stream crosses under.
    pub codec: &'a str,
    /// The frames each admitted subscriber's queue holds.
    pub bound: u64,
    /// `drop_old` or `drop_new`.
    pub on_full: &'a str,
    /// Whether the binding replays to a reconnecting subscriber.
    pub replay: bool,
    /// The required roles as written; the digest sorts them.
    pub requires: &'a [String],
}

/// The hash input of a hub's stream digest (spec/api.md § Streams, the
/// hub exposure): the header line `hale-api-hub 1`, then one line per
/// stream row ordered by topic as bytes, eight TAB-separated fields
/// (topic, payload shape hash, direction, codec, bound in decimal,
/// on_full, replay `1` or `0`, the sorted required roles or `-`), every
/// line ended by one LF.
pub fn stream_digest_input(lines: &[StreamLine<'_>]) -> String {
    let mut sorted: Vec<&StreamLine<'_>> = lines.iter().collect();
    sorted.sort_by(|a, b| a.topic.as_bytes().cmp(b.topic.as_bytes()));
    let mut out = String::from("hale-api-hub 1\n");
    for l in sorted {
        out.push_str(&format!(
            "{}\t{:016x}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            l.topic,
            l.payload,
            l.direction,
            l.codec,
            l.bound,
            l.on_full,
            if l.replay { 1 } else { 0 },
            roles_field(l.requires)
        ));
    }
    out
}

/// A hub's stream digest: the FNV-1a/64 fold of [`stream_digest_input`].
pub fn stream_digest(lines: &[StreamLine<'_>]) -> u64 {
    hale_graph::identity::fnv64(stream_digest_input(lines).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roles(r: &[&str]) -> Vec<String> {
        r.iter().map(|s| s.to_string()).collect()
    }

    /// tests/api-contract/digest.md, worked by hand: `Public`'s 159
    /// bytes and their fold, `Admin`'s, and the hub `fills`'s.
    #[test]
    fn the_fixture_digests_fold_as_digest_md_works_them() {
        let trader = roles(&["trader"]);
        let none = roles(&[]);
        let public = [
            DigestLine {
                member: "Orders::place",
                request: Some(0xcb5775974312c858),
                response: Some(0xbb4f99639cf069af),
                error: Some(0x36c7f0561125943e),
                requires: &none,
            },
            DigestLine {
                member: "Orders::cancel",
                request: Some(0xdeb8489f34994e5a),
                response: Some(0xe1506381a35c8ced),
                error: Some(0xdb0311924c0e7333),
                requires: &trader,
            },
        ];
        let input = surface_digest_input(&public);
        assert_eq!(input.len(), 159);
        assert_eq!(
            input,
            "hale-api-surface 1\n\
             Orders::cancel\tdeb8489f34994e5a\te1506381a35c8ced\tdb0311924c0e7333\ttrader\n\
             Orders::place\tcb5775974312c858\tbb4f99639cf069af\t36c7f0561125943e\t-\n"
        );
        assert_eq!(digest_text(surface_digest(&public)), "fnv1a64:a8930d6e7998e986");

        let operator = roles(&["operator"]);
        let admin = [
            DigestLine {
                member: "Orders::cancel",
                request: Some(0xdeb8489f34994e5a),
                response: Some(0xe1506381a35c8ced),
                error: Some(0xdb0311924c0e7333),
                requires: &operator,
            },
            DigestLine {
                member: "Ledger::rebalance",
                request: Some(0x19611780fbd68ecf),
                response: Some(0x3193bf68569ed280),
                error: Some(0x36c7f0561125943e),
                requires: &operator,
            },
        ];
        assert_eq!(surface_digest_input(&admin).len(), 172);
        assert_eq!(digest_text(surface_digest(&admin)), "fnv1a64:40381db6685c9f75");

        let fills = [StreamLine {
            topic: "Fills",
            payload: 0x32e4848051d36e16,
            direction: "out",
            codec: "json",
            bound: 64,
            on_full: "drop_old",
            replay: false,
            requires: &operator,
        }];
        assert_eq!(stream_digest_input(&fills).len(), 70);
        assert_eq!(digest_text(stream_digest(&fills)), "fnv1a64:26970854397ab154");
    }

    /// The requires field is a set: order and repetition as written move
    /// no digest; `-` stands for none in every slot.
    #[test]
    fn requires_is_a_sorted_set_and_none_is_a_dash() {
        let ab = roles(&["b", "a", "b"]);
        let ba = roles(&["a", "b"]);
        let l = |r: &[String]| surface_digest_input(&[DigestLine { member: "L::f", request: None, response: None, error: None, requires: r }]);
        assert_eq!(l(&ab), l(&ba));
        assert_eq!(l(&ab), "hale-api-surface 1\nL::f\t-\t-\t-\ta,b\n");
        assert_eq!(l(&[]), "hale-api-surface 1\nL::f\t-\t-\t-\t-\n");
    }
}
