#!/usr/bin/env python3
# voice, as a sapling stands in for it: one OpenAI-compatible endpoint whose
# one seat relays to a local model. A request is answered by the seat (the
# model behind it answers whole); asked with `stream: true`, the answer goes
# back as server-sent events, a word a chunk, then `[DONE]`.
#
# It serves the habitat's models (`[dna.provides.models]`): a request carries
# a tenant's key as a bearer token, a member of the `tenant-key` family the
# habitat draws, one per organism, and stages in voice's reader vault
# (HALE_VAULT_DIR); the members are listed by their prefix
# (HALE_SECRET_TENANT_KEY_FAMILY) at each request, so a tenant keyed while
# voice runs is admitted without a restart. `GET /v1/models` lists what it
# serves, each model's `voice.available` and `voice.capabilities.tools`
# said, as voice does (its manifest's `extension = "voice"`).
import json, os, socket, urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(os.environ["VOICE_PORT"])
SEAT = os.environ["VOICE_SEAT"]
VAULT = os.environ.get("HALE_VAULT_DIR", "")
FAMILY = os.environ.get("HALE_SECRET_TENANT_KEY_FAMILY", "")


def tenant_of(headers):
    """The tenant a request's bearer token keys (its member's label), or None.
    With no family handed, every request is admitted (no habitat keys it)."""
    if not FAMILY:
        return ""
    auth = headers.get("Authorization", "")
    if not auth.startswith("Bearer "):
        return None
    token = auth[len("Bearer "):].strip()
    try:
        names = os.listdir(VAULT)
    except OSError:
        return None
    for name in sorted(names):
        if not name.startswith(FAMILY):
            continue
        try:
            with open(os.path.join(VAULT, name)) as f:
                if token and f.read() == token:
                    return name[len(FAMILY):]
        except OSError:
            pass
    return None


def ask_seat(req):
    body = dict(req)
    body["stream"] = False
    urls = [SEAT]
    if "127.0.0.1" in SEAT:
        urls.append(SEAT.replace("127.0.0.1", "host.docker.internal"))
    last_err = None
    for url in urls:
        try:
            r = urllib.request.Request(url, data=json.dumps(body).encode(), headers={"Content-Type": "application/json"})
            with urllib.request.urlopen(r, timeout=30) as resp:
                answer = json.loads(resp.read())
            return answer["choices"][0]["message"].get("content") or ""
        except Exception as e:
            last_err = e
    raise last_err


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def reply(self, status, ctype, data):
        self.send_response(status)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def unauthorized(self):
        return self.reply(401, "application/json", json.dumps({"error": {"message": "a tenant's key, as a bearer token"}}).encode())

    def do_GET(self):
        if self.path == "/healthz":
            return self.reply(200, "text/plain", b"ok")
        if self.path == "/v1/models":
            if tenant_of(self.headers) is None:
                return self.unauthorized()
            return self.reply(200, "application/json", json.dumps({"object": "list", "data": [{"id": "local", "object": "model", "owned_by": "voice", "voice": {"available": True, "capabilities": {"tools": False}}}]}).encode())
        self.reply(404, "text/plain", b"not found")

    def do_POST(self):
        if self.path != "/v1/chat/completions":
            return self.reply(404, "text/plain", b"not found")
        if tenant_of(self.headers) is None:
            return self.unauthorized()
        req = json.loads(self.rfile.read(int(self.headers.get("Content-Length", "0") or 0)) or b"{}")
        try:
            text = ask_seat(req)
        except Exception as e:
            return self.reply(502, "application/json", json.dumps({"error": {"message": "the seat did not answer: %s" % e}}).encode())
        model = req.get("model", "local")
        if not req.get("stream"):
            out = {"id": "voice-1", "object": "chat.completion", "model": model,
                   "choices": [{"index": 0, "message": {"role": "assistant", "content": text}, "finish_reason": "stop"}]}
            return self.reply(200, "application/json", json.dumps(out).encode())
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Connection", "close")
        self.end_headers()
        for word in text.split(" "):
            chunk = {"id": "voice-1", "object": "chat.completion.chunk", "model": model,
                     "choices": [{"index": 0, "delta": {"content": word + " "}, "finish_reason": None}]}
            self.wfile.write(b"data: " + json.dumps(chunk).encode() + b"\n\n")
            self.wfile.flush()
        self.wfile.write(b"data: [DONE]\n\n")
        self.wfile.flush()
        self.close_connection = True


class Server(ThreadingHTTPServer):
    # the host's name resolves to ::1 or 127.0.0.1: one dual-stack socket
    # answers both, as voice answers on the host's interfaces
    address_family = socket.AF_INET6

    def server_bind(self):
        self.socket.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 0)
        super().server_bind()


Server(("::", PORT), Handler).serve_forever()
