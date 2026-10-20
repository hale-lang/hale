//! The compose backend: one service per program instance.

use std::fmt::Write;
use std::path::Path;

use super::{q, relative, Model, LIVENESS_PATH};

/// `bash` alone is needed in the image: the check is a GET over /dev/tcp.
fn healthcheck(port: i64) -> String {
    let probe = format!(
        "exec 3<>/dev/tcp/127.0.0.1/{port} && printf 'GET {LIVENESS_PATH} HTTP/1.0\\r\\n\\r\\n' >&3 && head -n1 <&3 | grep -q ' 200 '"
    );
    format!(
        "    healthcheck:\n      test: [\"CMD\", \"bash\", \"-c\", {}]\n      interval: 2s\n      timeout: 2s\n      retries: 30\n",
        q(&probe)
    )
}

pub(super) fn render(m: &Model, dest: &Path, notes: &mut Vec<String>) -> String {
    let mut o = String::new();
    let mount = relative(dest, &m.base);
    // a bind source is a path; a bare name is a named volume in the short syntax
    let mount = if mount.starts_with('.') || mount.starts_with('/') {
        mount
    } else {
        format!("./{mount}")
    };
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
        "# Build each program first (`hale build <seed>`); a service runs its built binary from"
    );
    let _ = writeln!(
        o,
        "# the mount, in a glibc image compatible with the build host."
    );
    let _ = writeln!(o, "services:");
    for inst in &m.insts {
        let p = &m.progs[inst.prog];
        let _ = writeln!(o, "  {}:", inst.id);
        let _ = writeln!(o, "    image: debian:stable-slim");
        let _ = writeln!(o, "    working_dir: /app");
        let _ = writeln!(
            o,
            "    volumes:\n      - {}",
            q(&format!("{mount}:/app:ro"))
        );
        match &inst.binary {
            Some(b) => {
                let _ = writeln!(o, "    command: [{}]", q(&format!("/app/{b}")));
            }
            None => notes.push(format!(
                "{}: no binary named (a plan row's `binary`): no command rendered",
                inst.id
            )),
        }
        if !m.env.is_empty() {
            let _ = writeln!(o, "    environment:");
            for (k, v) in &m.env {
                let _ = writeln!(o, "      {k}: {}", q(v));
            }
        }
        if let Some(s) = &p.scrape {
            let n = m.insts.iter().filter(|x| x.prog == inst.prog).count();
            let published = if n == 1 {
                format!("127.0.0.1:{0}:{0}", s.port)
            } else {
                s.port.to_string()
            };
            let _ = writeln!(o, "    ports:\n      - {}", q(&published));
        }
        if let Some(port) = p.probes_port {
            o.push_str(&healthcheck(port));
        }
        let deps: Vec<&str> = m
            .edges
            .iter()
            .filter(|(_, s)| *s == inst.id)
            .map(|(p, _)| p.as_str())
            .collect();
        if !deps.is_empty() {
            let _ = writeln!(o, "    depends_on:");
            for d in deps {
                let healthy = m
                    .insts
                    .iter()
                    .find(|x| x.id == d)
                    .map(|x| m.progs[x.prog].probes_port.is_some())
                    .unwrap_or(false);
                let cond = if healthy {
                    "service_healthy"
                } else {
                    "service_started"
                };
                let _ = writeln!(o, "      {d}:\n        condition: {cond}");
            }
        }
    }
    o
}
