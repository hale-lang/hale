#!/usr/bin/env python3
"""The v1 performance baseline: build, runtime and editor measurements.

Measures only; it sets no targets and fails on no number. Run from the
repository root after `cargo build --release`:

    python3 bench/run.py --hale target/release/hale --out bench/results.json

`--only build,bus,http,json,memory,lsp` runs a subset. Every measurement
that is repeated reports its median and its range. The JSON written to
`--out` holds every raw run; the Markdown table printed at the end is what
bench/BASELINE.md records.
"""

import argparse
import json
import os
import platform
import re
import shutil
import socket
import statistics
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BENCH = ROOT / "bench"

# The build references: one representative example, and the largest DNA
# program (the host: the core, the API and its own seed, ~ the size of a
# real organism's).
BUILD_SMALL = "crates/hale-codegen/tests/fixtures/examples/92-build-an-api"
BUILD_LARGE = "dna/host"
EDIT_SMALL = "crates/hale-codegen/tests/fixtures/examples/92-build-an-api/main.hl"
EDIT_LARGE = "dna/host/main.hl"
# The editor references: a file of a small seed and of the largest seed.
LSP_SMALL = "crates/hale-codegen/tests/fixtures/examples/92-build-an-api/main.hl"
LSP_LARGE = "dna/core/assembly.hl"


def sh(cmd, **kw):
    return subprocess.run(cmd, check=True, capture_output=True, text=True, **kw)


def timed(cmd, env=None, cwd=ROOT, rss=False):
    """Runs `cmd`; its wall time and stdout, and with `rss` its peak RSS
    in MiB (the child's own, from wait4)."""
    with tempfile.TemporaryFile("w+") as out, tempfile.TemporaryFile("w+") as err:
        t = time.monotonic()
        p = subprocess.Popen(cmd, stdout=out, stderr=err, env=env, cwd=cwd)
        _, status, usage = os.wait4(p.pid, 0)
        took = time.monotonic() - t
        p.returncode = os.waitstatus_to_exitcode(status)
        out.seek(0)
        err.seek(0)
        stdout = out.read()
        if p.returncode != 0:
            raise SystemExit(f"failed: {' '.join(map(str, cmd))}\n{stdout}\n{err.read()}")
    if rss:
        return took, stdout, usage.ru_maxrss / 1024
    return took, stdout


def summary(xs, nd=1):
    return {"median": round(statistics.median(xs), nd), "min": round(min(xs), nd), "max": round(max(xs), nd), "runs": [round(x, nd) for x in xs]}


def machine(hale):
    cpu = next((l.split(":", 1)[1].strip() for l in Path("/proc/cpuinfo").read_text().splitlines() if l.startswith("model name")), platform.processor())
    mem = next((int(l.split()[1]) for l in Path("/proc/meminfo").read_text().splitlines() if l.startswith("MemTotal")), 0)
    try:
        rev = sh(["git", "rev-parse", "--short=9", "HEAD"], cwd=ROOT).stdout.strip()
    except Exception:
        rev = "?"
    ver = subprocess.run([hale, "--version"], capture_output=True, text=True).stdout.strip().splitlines()[0]
    return {
        "cpu": cpu,
        "logical_cpus": os.cpu_count(),
        "mem_gib": round(mem / 1024 / 1024),
        "kernel": platform.release(),
        "hale": ver,
        "revision": rev,
    }


# ---- build ---------------------------------------------------------------

def bench_build(hale, reps):
    out = {}
    for name, target, edit in [("small", BUILD_SMALL, EDIT_SMALL), ("large", BUILD_LARGE, EDIT_LARGE)]:
        cold, warm, edited, peak = [], [], [], []
        for _ in range(reps):
            with tempfile.TemporaryDirectory() as d:
                env = dict(os.environ, XDG_CACHE_HOME=d)
                exe = Path(d) / "out"
                took, _, mib = timed([hale, "build", target, "-o", exe], env, rss=True)
                cold.append(took)
                peak.append(mib)
                warm.append(timed([hale, "build", target, "-o", exe], env)[0])
                src = ROOT / edit
                text = src.read_text()
                try:
                    src.write_text(text + "// bench edit\n")
                    edited.append(timed([hale, "build", target, "-o", exe], env)[0])
                finally:
                    src.write_text(text)
        out[name] = {"target": target, "cold_s": summary(cold, 2), "warm_s": summary(warm, 2), "edit_s": summary(edited, 2),
                     "peak_rss_mib": summary(peak, 0)}
    return out


# ---- runtime: bus round trips -------------------------------------------

def build_program(hale, src, d):
    exe = Path(d) / Path(src).stem
    timed([hale, "build", src, "-o", exe])
    return exe


def bench_bus(hale, reps):
    out = {}
    with tempfile.TemporaryDirectory() as d:
        for name, src in [("same_thread", "bench/bus_rt_same.hl"), ("cross_pool", "bench/bus_rt_xpool.hl")]:
            exe = build_program(hale, src, d)
            ns = []
            for _ in range(reps):
                _, stdout = timed([exe])
                vals = dict(l.split("=", 1) for l in stdout.split() if "=" in l)
                assert vals["got"] == vals["iters"], stdout
                ns.append(float(vals["ns_per_rt"]))
            out[name] = {"ns_per_round_trip": summary(ns, 0), "iters": int(vals["iters"])}
    return out


# ---- runtime: JSON -------------------------------------------------------

def bench_json(hale, reps):
    runs = {}
    for _ in range(reps):
        _, stdout = timed([hale, "bench", "bench/json_bench.hl", "--json"])
        doc = json.loads(stdout[stdout.index("["):]) if "[" in stdout else json.loads(stdout)
        items = doc if isinstance(doc, list) else doc.get("results", [])
        for it in items:
            runs.setdefault(it["name"], {"ns": [], "allocs": []})
            runs[it["name"]]["ns"].append(float(it["ns_per_op"]))
            runs[it["name"]]["allocs"].append(it.get("allocs_per_op"))
    return {k: {"ns_per_op": summary(v["ns"], 0), "allocs_per_op": v["allocs"][-1]} for k, v in runs.items()}


# ---- runtime: HTTP and memory ---------------------------------------------

def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def rss_kib(pid):
    for l in Path(f"/proc/{pid}/status").read_text().splitlines():
        if l.startswith("VmRSS:"):
            return int(l.split()[1])
    return 0


class Served:
    def __init__(self, exe):
        self.port = free_port()
        self.p = subprocess.Popen([exe, str(self.port)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            try:
                socket.create_connection(("127.0.0.1", self.port), timeout=0.2).close()
                return
            except OSError:
                time.sleep(0.05)
        raise SystemExit("the HTTP server did not come up")

    def stop(self):
        self.p.terminate()
        try:
            self.p.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.p.kill()


def httpload(load, port, conns, duration, warmup):
    p = sh([load, "--addr", f"127.0.0.1:{port}", "--conns", str(conns), "--duration", str(duration), "--warmup", str(warmup)])
    return json.loads(p.stdout)


def build_httpload(d):
    exe = Path(d) / "httpload"
    sh(["rustc", "-O", "--edition", "2021", str(BENCH / "tools" / "httpload.rs"), "-o", str(exe)])
    return exe


def bench_http(hale, reps, conns):
    out = {}
    with tempfile.TemporaryDirectory() as d:
        exe = build_program(hale, "bench/http_serve.hl", d)
        load = build_httpload(d)
        for c in conns:
            rps, p50, p99, runs = [], [], [], []
            for _ in range(reps):
                srv = Served(exe)
                try:
                    r = httpload(load, srv.port, c, 10, 2)
                finally:
                    srv.stop()
                runs.append(r)
                rps.append(r["rps"])
                p50.append(r["p50_us"])
                p99.append(r["p99_us"])
            out[f"conns_{c}"] = {"rps": summary(rps, 0), "p50_us": summary(p50, 0), "p99_us": summary(p99, 0),
                                 "errors": sum(r["errors"] for r in runs), "dropped": sum(r["dropped"] for r in runs),
                                 "reconnects": sum(r["reconnects"] for r in runs)}
    return out


def bench_memory(hale, seconds, conns):
    """The HTTP server under constant load for `seconds`: its RSS each
    second, and the growth over the run after a warm-up."""
    with tempfile.TemporaryDirectory() as d:
        exe = build_program(hale, "bench/http_serve.hl", d)
        load = build_httpload(d)
        srv = Served(exe)
        samples = []
        idle = rss_kib(srv.p.pid)
        done = threading.Event()
        result = {}

        def run_load():
            result.update(httpload(load, srv.port, conns, seconds, 0))
            done.set()

        threading.Thread(target=run_load, daemon=True).start()
        t0 = time.monotonic()
        while not done.is_set():
            samples.append((time.monotonic() - t0, rss_kib(srv.p.pid)))
            done.wait(1.0)
        after = rss_kib(srv.p.pid)
        srv.stop()
    warm = [(t, r) for t, r in samples if t >= 10]
    n = len(warm)
    mt = sum(t for t, _ in warm) / n
    mr = sum(r for _, r in warm) / n
    slope = sum((t - mt) * (r - mr) for t, r in warm) / max(1e-9, sum((t - mt) ** 2 for t, _ in warm))
    pick = lambda at: min(samples, key=lambda s: abs(s[0] - at))[1]
    return {
        "seconds": seconds,
        "conns": conns,
        "requests": result.get("requests"),
        "errors": result.get("errors"),
        "dropped": result.get("dropped"),
        "rss_mib_idle": round(idle / 1024, 1),
        "rss_mib_at_10s": round(pick(10) / 1024, 1),
        "rss_mib_at_end": round(samples[-1][1] / 1024, 1),
        "rss_mib_after_load": round(after / 1024, 1),
        "growth_kib_per_min_after_10s": round(slope * 60, 1),
        "samples": [(round(t, 1), r) for t, r in samples],
    }


# ---- editor --------------------------------------------------------------

def bench_lsp(hale, edits):
    out = {}
    for name, f in [("small_seed", LSP_SMALL), ("large_seed", LSP_LARGE)]:
        p = sh([sys.executable, str(BENCH / "tools" / "lsp_latency.py"), hale, str(ROOT / f), "--edits", str(edits)], cwd=ROOT)
        r = json.loads(p.stdout)
        r["file"] = f
        out[name] = r
    return out


# ---- report --------------------------------------------------------------

def table(res):
    m = res["machine"]
    rows = [f"Machine: {m['cpu']}, {m['logical_cpus']} logical CPUs, {m['mem_gib']} GiB, Linux {m['kernel']}; {m['hale']} at {m['revision']}", ""]
    rows += ["| Area | Measure | Median | Range |", "|---|---|---|---|"]
    def row(area, what, s, unit):
        rows.append(f"| {area} | {what} | {s['median']} {unit} | {s['min']} to {s['max']} |")
    if "build" in res:
        for k, v in res["build"].items():
            for m_, label in [("cold_s", "cold"), ("warm_s", "warm, unchanged"), ("edit_s", "after a one-line edit")]:
                row("build", f"`hale build {v['target']}`, {label}", v[m_], "s")
            row("build", f"`hale build {v['target']}`, cold, peak RSS", v["peak_rss_mib"], "MiB")
    if "bus" in res:
        for k, v in res["bus"].items():
            row("bus", f"round trip, {k.replace('_', ' ')}", v["ns_per_round_trip"], "ns")
    if "json" in res:
        for k, v in res["json"].items():
            row("json", f"{k} ({v['allocs_per_op']} allocs/op)", v["ns_per_op"], "ns/op")
    if "http" in res:
        for k, v in res["http"].items():
            c = k.split("_")[1]
            row("http", f"{c} connections, requests/s", v["rps"], "")
            row("http", f"{c} connections, p50", v["p50_us"], "µs")
            row("http", f"{c} connections, p99", v["p99_us"], "µs")
            rows.append(f"| http | {c} connections, errors / dropped attempts, all runs | {v['errors']} / {v['dropped']} | |")
    if "memory" in res:
        v = res["memory"]
        rows.append(f"| memory | HTTP server RSS: idle / at 10 s / at {v['seconds']} s | {v['rss_mib_idle']} / {v['rss_mib_at_10s']} / {v['rss_mib_at_end']} MiB | |")
        rows.append(f"| memory | RSS growth after 10 s ({v['requests']} requests, {v['errors']} errors, {v['dropped']} dropped) | {v['growth_kib_per_min_after_10s']} KiB/min | |")
    if "lsp" in res:
        for k, v in res["lsp"].items():
            rows.append(f"| editor | {k.replace('_', ' ')} ({v['file']}): open, first / done | {v['open_first_ms']} / {v['open_done_ms']} ms | |")
            rows.append(f"| editor | {k.replace('_', ' ')}: edit, first p50 / p95 | {v['edit_first_ms_p50']} / {v['edit_first_ms_p95']} ms | {v['edits']} edits |")
            rows.append(f"| editor | {k.replace('_', ' ')}: edit, done p50 / p95 | {v['edit_done_ms_p50']} / {v['edit_done_ms_p95']} ms | |")
            rows.append(f"| editor | {k.replace('_', ' ')}: server peak RSS | {v['peak_rss_mib']} MiB | |")
    return "\n".join(rows)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--hale", default=str(ROOT / "target/release/hale"))
    ap.add_argument("--out", default=str(BENCH / "results.json"))
    ap.add_argument("--only", default="build,bus,json,http,memory,lsp")
    ap.add_argument("--reps", type=int, default=5)
    ap.add_argument("--memory-seconds", type=int, default=120)
    ap.add_argument("--edits", type=int, default=20)
    a = ap.parse_args()
    hale = str(Path(a.hale).resolve())
    if not shutil.which("rustc") and ("http" in a.only or "memory" in a.only):
        raise SystemExit("rustc builds the load generator; install a Rust toolchain")
    only = set(a.only.split(","))
    res = {"machine": machine(hale)}
    steps = [("build", lambda: bench_build(hale, 3)), ("bus", lambda: bench_bus(hale, a.reps)),
             ("json", lambda: bench_json(hale, a.reps)), ("http", lambda: bench_http(hale, a.reps, [1, 16, 64])),
             ("memory", lambda: bench_memory(hale, a.memory_seconds, 16)), ("lsp", lambda: bench_lsp(hale, a.edits))]
    for name, f in steps:
        if name in only:
            print(f"bench: {name} ...", file=sys.stderr, flush=True)
            res[name] = f()
    Path(a.out).write_text(json.dumps(res, indent=1) + "\n")
    print(table(res))


if __name__ == "__main__":
    main()
