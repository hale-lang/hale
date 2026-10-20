//! The kubernetes backend: a Deployment and a Service per program.

use std::fmt::Write;

use super::{dns, q, Model, LIVENESS_PATH, READINESS_PATH};

pub(super) fn render(m: &Model, notes: &mut Vec<String>) -> String {
    let mut o = String::new();
    let _ = writeln!(
        o,
        "# Rendered by `hale deploy render` from {}: a pure function of the checked program.",
        m.source
    );
    if let Some(e) = &m.env_name {
        let _ = writeln!(o, "# environment: {e}");
    }
    let _ = writeln!(
        o,
        "# Each image <name>:latest holds the built binary under /app (`hale build <seed>`); it is"
    );
    let _ = writeln!(o, "# named here and never built or pushed.");
    let _ = writeln!(
        o,
        "# No HorizontalPodAutoscaler: no scale policy is declared for this environment."
    );
    let mut progs: Vec<usize> = (0..m.progs.len()).collect();
    progs.sort_by(|a, b| m.progs[*a].name.cmp(&m.progs[*b].name));
    for pi in progs {
        let p = &m.progs[pi];
        let name = dns(&p.name);
        let replicas = m.insts.iter().filter(|i| i.prog == pi).count();
        let _ = writeln!(o, "---");
        let _ = writeln!(o, "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: {name}\n  labels:\n    app: {name}");
        let _ = writeln!(
            o,
            "spec:\n  replicas: {replicas}\n  selector:\n    matchLabels:\n      app: {name}"
        );
        let _ = writeln!(
            o,
            "  template:\n    metadata:\n      labels:\n        app: {name}"
        );
        if let Some(s) = &p.scrape {
            let _ = writeln!(
                o,
                "      annotations:\n        prometheus.io/scrape: \"true\"\n        prometheus.io/port: {}\n        prometheus.io/path: {}",
                q(&s.port.to_string()),
                q(&s.path)
            );
        }
        let _ = writeln!(
            o,
            "    spec:\n      containers:\n        - name: {name}\n          image: {}",
            q(&format!("{name}:latest"))
        );
        match &p.binary {
            Some(b) => {
                let _ = writeln!(o, "          command: [{}]", q(&format!("/app/{b}")));
            }
            None => notes.push(format!(
                "{}: no binary named (a plan row's `binary`): no command rendered",
                p.name
            )),
        }
        let mut ports: Vec<(&str, i64)> = Vec::new();
        if let Some(port) = p.probes_port {
            ports.push(("probes", port));
        }
        if let Some(s) = &p.scrape {
            ports.push(("metrics", s.port));
        }
        if !ports.is_empty() {
            let _ = writeln!(o, "          ports:");
            for (n, port) in &ports {
                let _ = writeln!(
                    o,
                    "            - name: {n}\n              containerPort: {port}"
                );
            }
        }
        if !m.env.is_empty() {
            let _ = writeln!(o, "          env:");
            for (k, v) in &m.env {
                let _ = writeln!(o, "            - name: {k}\n              value: {}", q(v));
            }
        }
        if p.probes_port.is_some() {
            for (field, path) in [
                ("livenessProbe", LIVENESS_PATH),
                ("readinessProbe", READINESS_PATH),
            ] {
                let _ = writeln!(o, "          {field}:\n            httpGet:\n              path: {path}\n              port: probes");
            }
        }
        if ports.is_empty() {
            notes.push(format!(
                "{}: declares no port, so no Service is rendered for it",
                p.name
            ));
            continue;
        }
        let _ = writeln!(o, "---");
        let _ = writeln!(
            o,
            "apiVersion: v1\nkind: Service\nmetadata:\n  name: {name}\n  labels:\n    app: {name}"
        );
        let _ = writeln!(o, "spec:\n  selector:\n    app: {name}\n  ports:");
        for (n, port) in &ports {
            let _ = writeln!(
                o,
                "    - name: {n}\n      port: {port}\n      targetPort: {n}"
            );
        }
    }
    o
}
