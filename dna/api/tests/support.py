"""Owned, local fixtures for the read-only DNA API integration suite.

The fixture writes the existing GitRecord wire format, not a mock HTTP API:
one commit per journal event, cumulative journal.jsonl, sha256 receipt blobs.
These tests exercise projection/HTTP boundaries, not domain admission.
"""

from __future__ import annotations

import base64
import hashlib
import http.client
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import threading
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlencode, urlsplit


def encoded(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


def test_environment():
    env = os.environ.copy()
    # A fixture must never discover the developer's database or evidence store.
    for key in (
        "HALE_DNA_KNOWLEDGE_DSN",
        "HALE_DNA_KNOWLEDGE_URL",
        "HALE_DNA_EVIDENCE_KEY",
        "HALE_DNA_OIDC_SECRET",
    ):
        env.pop(key, None)
    env["HALE_DNA_DISCOVER"] = "off"
    env["GIT_CONFIG_NOSYSTEM"] = "1"
    env["GIT_CONFIG_GLOBAL"] = os.devnull
    env["GIT_TERMINAL_PROMPT"] = "0"
    return env


class Record:
    def __init__(self, path: Path):
        self.path = path
        path.mkdir(parents=True)
        self.env = test_environment()
        self.git("init", "-q", "-b", "main")
        self.git("config", "user.name", "api-fixture")
        self.git("config", "user.email", "api-fixture@example.invalid")
        self.git("config", "commit.gpgsign", "false")
        self.git("config", "gc.auto", "0")
        self.rows = []
        self.head = ""
        self.append("org.born", "org", "read-only API fixture " + str(uuid.uuid4()))
        self.identity = self.head

    def git(self, *args: str, input: str | None = None):
        result = subprocess.run(
            ["git", "-C", str(self.path), *args], input=input, text=True,
            capture_output=True, env=self.env, timeout=10, check=False,
        )
        if result.returncode:
            raise AssertionError(f"git {args}: {result.stderr}")
        return result.stdout.strip()

    def commit_text(self, text: str):
        blob = self.git("hash-object", "-w", "--stdin", input=text)
        tree = self.git("mktree", input=f"100644 blob {blob}\tjournal.jsonl\n")
        args = ["commit-tree", tree]
        if self.head:
            args += ["-p", self.head]
        commit = self.git(*args, input=f"fixture event {len(self.rows)}\n")
        self.git("update-ref", "refs/dna/journal", commit, self.head or "0" * 40)
        self.head = commit
        return commit

    def append(self, kind: str, entity: str, body, author: str = "alice"):
        self.rows.append({
            "seq": len(self.rows), "kind": kind, "entity": entity,
            "body": body if isinstance(body, str) else encoded(body),
            "author": author,
        })
        return self.commit_text("".join(encoded(row) + "\n" for row in self.rows))

    def receipt_text(self, text: str):
        digest = hashlib.sha256(text.encode()).hexdigest()
        blob = self.git("hash-object", "-w", "--stdin", input=text)
        self.git("update-ref", "refs/dna/receipts/" + digest, blob)
        return "sha256:" + digest

    def receipt(self, document: dict, *, compact=False):
        # knowledge_document uses the Builder's spaces; design_document has a
        # distinct compact native codec. Preserve the exact content-addressed bytes.
        text = encoded(document) if compact else json.dumps(document, ensure_ascii=False)
        return self.receipt_text(text)

    def practice(self, name="acceptance/résumé", text="Première ligne — ✓\n第二行 / preserve", *, target="org/support/équipe", supersedes="", review_id=None, design=False):
        if design:
            document = {"author": "org", "kind": "practice", "name": name,
                        "provenance": "design"}
            if supersedes:
                document["supersedes"] = supersedes
            document.update(target=target, text=text, toolchain="0.20.0")
        else:
            document = {"kind": "practice", "text": text, "author": "org",
                        "target": target, "provenance": "proposed", "name": name}
            if supersedes:
                document["supersedes"] = supersedes
        digest = self.receipt(document, compact=design)
        review_id = review_id or "k:" + digest[7:19]
        self.append("knowledge.proposed", digest, {
            "digest": digest, "review_id": review_id, "kind": "practice",
            "author": "org", "target": target, "class": "goal", "name": name,
            "supersedes": supersedes,
        }, "dna")
        # Native assembly review questions embed the proposed document text.
        question = f"Ratify {name}: {text}\nFor {target} — exact candidate"
        self.append("review.requested", "review:" + review_id, {
            "question": question, "subject_digest": digest,
            "required_authority": "board", "author": "org",
            "knowledge_digest": digest, "kind": "practice", "target": target,
            "class": "goal",
        }, "dna")
        self.append("practice.proposed", "request/α/1", {
            "name": name, "digest": digest, "review_id": review_id,
            "by": "alice", "because": "Two lines\nexplain why / pourquoi",
            "supersedes": supersedes,
        }, "dna")
        return {"digest": digest, "review_id": review_id, "name": name,
                "text": text, "target": target, "question": question}

    def refs(self):
        return self.git("show-ref")


def free_port():
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


class Response:
    def __init__(self, status, headers, body):
        self.status, self.headers, self.body = status, dict(headers), body

    def json(self):
        return json.loads(self.body)

    def header(self, name):
        return next((value for key, value in self.headers.items()
                     if key.lower() == name.lower()), "")


def request(port, path, method="GET", *, headers=None, body=None):
    client = http.client.HTTPConnection("127.0.0.1", port, timeout=10)
    try:
        client.request(method, path, body=body, headers=headers or {})
        response = client.getresponse()
        return Response(response.status, response.getheaders(), response.read().decode("utf-8"))
    finally:
        client.close()


class ApiServer:
    def __init__(self, executable: Path, record: Record, log: Path, *, env=None, port=None):
        self.executable, self.record, self.log = executable, record, log
        self.port = port or free_port()
        self.env = test_environment()
        self.env.update(env or {})
        self.process = None
        self.stream = None

    def start(self):
        self.stream = self.log.open("a")
        self.process = subprocess.Popen(
            [str(self.executable), str(self.record.path), str(self.port)],
            cwd=self.record.path, env=self.env, stdout=self.stream,
            stderr=subprocess.STDOUT, start_new_session=True,
        )
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                self.stop()
                raise AssertionError("API exited before listening:\n" + self.log.read_text())
            try:
                request(self.port, "/api/hale/v1/applications")
                return self
            except (OSError, http.client.HTTPException):
                time.sleep(0.05)
        self.stop()
        raise AssertionError("API did not become reachable:\n" + self.log.read_text())

    def stop(self):
        if self.process is not None:
            if self.process.poll() is None:
                os.killpg(self.process.pid, signal.SIGTERM)
                try:
                    self.process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    os.killpg(self.process.pid, signal.SIGKILL)
                    self.process.wait(timeout=3)
            self.process = None
        if self.stream is not None:
            self.stream.close()
            self.stream = None

    def get(self, path, **kwargs):
        return request(self.port, path, **kwargs)


class FakeIssuer:
    """Local authorization-code fixture, matching principal_oidc_test.hl.

    The native principal authenticates ID tokens through the server-to-server
    token exchange. This fixture is not a signature-verification conformance test.
    """

    def __init__(self):
        fixture = self
        self.codes = {}
        self.exchanges = 0

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def answer(self, status, body=None, **headers):
                raw = encoded(body).encode() if body is not None else b""
                self.send_response(status)
                for key, value in headers.items():
                    self.send_header(key.replace("_", "-"), value)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(raw)))
                self.end_headers()
                self.wfile.write(raw)

            def do_GET(self):
                route = urlsplit(self.path)
                query = parse_qs(route.query)
                if route.path == "/.well-known/openid-configuration":
                    self.answer(200, {"issuer": fixture.base,
                        "authorization_endpoint": fixture.base + "/authorize",
                        "token_endpoint": fixture.base + "/token"})
                elif route.path == "/authorize":
                    code = "code-" + str(len(fixture.codes) + fixture.exchanges + 1)
                    fixture.codes[code] = {
                        "nonce": query["nonce"][0],
                        "sub": query.get("login_hint", ["alice-sub"])[0],
                    }
                    self.answer(302, Location=query["redirect_uri"][0] + "?" +
                                urlencode({"code": code, "state": query["state"][0]}))
                else:
                    self.answer(404, {"error": "not_found"})

            def do_POST(self):
                data = self.rfile.read(int(self.headers.get("Content-Length", "0"))).decode()
                form = parse_qs(data)
                if self.path != "/token" or form.get("client_secret") != ["fake-secret"]:
                    self.answer(401, {"error": "invalid_client"})
                    return
                claims = fixture.codes.pop(form.get("code", [""])[0], None)
                if claims is None:
                    self.answer(400, {"error": "invalid_grant"})
                    return
                fixture.exchanges += 1
                claims.update(iss=fixture.base, aud="dna-api-tests", exp=int(time.time()) + 600)
                b64 = lambda value: base64.urlsafe_b64encode(encoded(value).encode()).decode().rstrip("=")
                token = b64({"alg": "RS256"}) + "." + b64(claims) + ".fixture"
                self.answer(200, {"id_token": token, "token_type": "Bearer"})

        self.http = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.http.daemon_threads = True
        self.base = f"http://127.0.0.1:{self.http.server_port}"
        self.thread = threading.Thread(target=self.http.serve_forever, daemon=True)

    def start(self):
        self.thread.start()
        return self

    def stop(self):
        self.http.shutdown()
        self.http.server_close()
        self.thread.join(timeout=3)
