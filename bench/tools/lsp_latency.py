#!/usr/bin/env python3
"""lsp_latency — editor latency of `hale lsp`, measured from a client.

Starts `<hale> lsp`, opens one file of a seed and times the diagnostics
that follow, then makes `--edits` ordinary edits (a comment line added at
the end of the file, then removed, alternately; full-document sync, as
the server takes it) and times each.

For every event two times are recorded, from the moment the client
wrote the message:
  first: the first `publishDiagnostics` for the edited file (the
         typing stage's publication; the editor's squiggles update);
  done:  the last `publishDiagnostics` of the pass, the server having
         been silent for `--quiet` seconds after it (the laws' stage,
         which republishes only the files it adds a finding to, is over).
Peak RSS is the server's VmHWM at the end. Prints one JSON object.

Usage: lsp_latency.py <hale> <file.hl> [--edits 20] [--quiet 1.0]
"""

import json
import os
import statistics
import subprocess
import sys
import threading
import time
from pathlib import Path


def frame(msg):
    body = json.dumps(msg).encode()
    return b"Content-Length: %d\r\n\r\n" % len(body) + body


class Server:
    def __init__(self, hale, root):
        self.p = subprocess.Popen([hale, "lsp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.DEVNULL, cwd=root)
        self.events = []  # (monotonic time, message)
        self.cv = threading.Condition()
        threading.Thread(target=self.reader, daemon=True).start()

    def reader(self):
        out = self.p.stdout
        while True:
            headers = {}
            while True:
                line = out.readline()
                if not line:
                    return
                line = line.strip()
                if not line:
                    break
                k, _, v = line.decode().partition(":")
                headers[k.lower()] = v.strip()
            body = out.read(int(headers["content-length"]))
            msg = json.loads(body)
            with self.cv:
                self.events.append((time.monotonic(), msg))
                self.cv.notify_all()

    def send(self, msg):
        self.p.stdin.write(frame(msg))
        self.p.stdin.flush()
        return time.monotonic()

    def request(self, rid, method, params):
        self.send({"jsonrpc": "2.0", "id": rid, "method": method, "params": params})
        with self.cv:
            self.cv.wait_for(lambda: any(m.get("id") == rid for _, m in self.events), timeout=120)

    def measure(self, start_index, uri, quiet, timeout=300):
        """Times from the event's write to its first diagnostics for `uri`
        and to the pass's last publication."""
        deadline = time.monotonic() + timeout
        first = None
        while True:
            with self.cv:
                self.cv.wait(timeout=0.05)
                pubs = [(t, m) for t, m in self.events[start_index:]
                        if m.get("method") == "textDocument/publishDiagnostics"]
            if first is None:
                mine = [t for t, m in pubs if m["params"]["uri"] == uri]
                if mine:
                    first = mine[0]
            if first is not None and pubs and time.monotonic() - pubs[-1][0] >= quiet:
                return first, pubs[-1][0]
            if time.monotonic() > deadline:
                raise SystemExit(f"no diagnostics for {uri} within {timeout}s")

    def peak_rss_kib(self):
        for line in Path(f"/proc/{self.p.pid}/status").read_text().splitlines():
            if line.startswith("VmHWM:"):
                return int(line.split()[1])
        return 0


def main():
    args = sys.argv[1:]
    hale, path = args[0], Path(args[1]).resolve()
    edits = int(args[args.index("--edits") + 1]) if "--edits" in args else 20
    quiet = float(args[args.index("--quiet") + 1]) if "--quiet" in args else 1.0
    text = path.read_text()
    uri = path.as_uri()
    root = path.parent
    s = Server(hale, root)
    s.request(1, "initialize", {"processId": os.getpid(), "rootUri": root.as_uri(), "capabilities": {}})
    s.send({"jsonrpc": "2.0", "method": "initialized", "params": {}})

    n = len(s.events)
    t0 = s.send({"jsonrpc": "2.0", "method": "textDocument/didOpen",
                 "params": {"textDocument": {"uri": uri, "languageId": "hale", "version": 1, "text": text}}})
    first, done = s.measure(n, uri, quiet)
    open_first, open_done = (first - t0) * 1000, (done - t0) * 1000

    firsts, dones = [], []
    for v in range(edits):
        body = text + "// edit\n" if v % 2 == 0 else text
        n = len(s.events)
        t0 = s.send({"jsonrpc": "2.0", "method": "textDocument/didChange",
                     "params": {"textDocument": {"uri": uri, "version": v + 2}, "contentChanges": [{"text": body}]}})
        first, done = s.measure(n, uri, quiet)
        firsts.append((first - t0) * 1000)
        dones.append((done - t0) * 1000)

    rss = s.peak_rss_kib()
    s.request(99, "shutdown", None)
    s.send({"jsonrpc": "2.0", "method": "exit"})
    try:
        s.p.wait(timeout=10)
    except subprocess.TimeoutExpired:
        s.p.kill()

    def q(xs, p):
        xs = sorted(xs)
        return round(xs[min(len(xs) - 1, max(0, int(round(p * len(xs) + 0.5)) - 1))], 1)

    print(json.dumps({
        "file": str(path),
        "open_first_ms": round(open_first, 1),
        "open_done_ms": round(open_done, 1),
        "edits": edits,
        "edit_first_ms_p50": round(statistics.median(firsts), 1),
        "edit_first_ms_p95": q(firsts, 0.95),
        "edit_done_ms_p50": round(statistics.median(dones), 1),
        "edit_done_ms_p95": q(dones, 0.95),
        "peak_rss_mib": round(rss / 1024, 1),
    }))


if __name__ == "__main__":
    main()
