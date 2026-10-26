//! Certificate rejection is independent of the graph search that proposed it.
use hale_types::reachability_proof::{Certificate, GraphProof, Vertex};

fn graph() -> GraphProof {
    GraphProof {
        vertices: (0..4)
            .map(|id| Vertex {
                id,
                identity: id.to_string(),
                display: id.to_string(),
                provenance: None,
                expanded: id <= 1,
            })
            .collect(),
        sources: vec![0],
        targets: vec![3],
        edges: vec![(0, 1), (1, 1)],
        certificate: Certificate::Closed { closed: vec![0, 1] },
    }
}

#[test]
fn omitted_frontier_and_omitted_roots_cannot_certify_absence() {
    let good = graph();
    good.validate().unwrap();
    let mut skipped_frontier = good.clone();
    skipped_frontier.edges.push((1, 2));
    assert!(skipped_frontier
        .validate()
        .unwrap_err()
        .contains("successor"));
    let mut skipped_root = good.clone();
    skipped_root.sources.push(2);
    assert!(skipped_root.validate().unwrap_err().contains("source"));
    let mut target = good;
    target.targets = vec![1];
    assert!(target.validate().unwrap_err().contains("target"));
}

#[test]
fn paths_need_a_selected_source_real_edges_and_a_target() {
    let mut proof = graph();
    proof.certificate = Certificate::Path {
        source: 0,
        path: vec![1],
    };
    assert!(proof.validate().unwrap_err().contains("target"));
    proof.targets = vec![1];
    proof.validate().unwrap();
    proof.certificate = Certificate::Path {
        source: 0,
        path: vec![1, 2],
    };
    assert!(proof.validate().unwrap_err().contains("absent edge"));
    proof.certificate = Certificate::Path {
        source: 1,
        path: vec![],
    };
    assert!(proof.validate().unwrap_err().contains("source set"));
    proof.sources = vec![1];
    proof.validate().unwrap(); // reflexive violation
}

#[test]
fn identities_and_expansion_account_are_checked() {
    let mut proof = graph();
    proof.vertices.push(proof.vertices[0].clone());
    assert!(proof.validate().unwrap_err().contains("duplicate"));
    proof.vertices.pop();
    proof.edges.push((1, 9));
    assert!(proof.validate().unwrap_err().contains("dangling"));
    proof.edges.pop();
    proof.vertices[1].expanded = false;
    assert!(proof.validate().unwrap_err().contains("unexpanded"));
    proof.vertices[1].expanded = true;
    proof.certificate = Certificate::NoTargets;
    assert!(proof.validate().unwrap_err().contains("has targets"));
    proof.targets.clear();
    proof.validate().unwrap();
}
