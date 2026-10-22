#!/usr/bin/env python3
# voice, as a sapling stands in for it: one OpenAI-compatible endpoint whose
# one seat relays to a local model. A request is answered by the seat (the
# model behind it answers whole); asked with `stream: true`, the answer goes
# back as server-sent events, a word a chunk, then `[DONE]`.
import json, os, socket, urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(os.environ["VOICE_PORT"])
SEAT = os.environ["VOICE_SEAT"]


def ask_seat(req):
    body = dict(req)
    body["stream"] = False
    r = urllib.request.Request(SEAT, data=json.dumps(body).encode(), headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(r, timeout=30) as resp:
        answer = json.loads(resp.read())
    return answer["choices"][0]["message"].get("content") or ""


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

    def do_GET(self):
        if self.path == "/healthz":
            return self.reply(200, "text/plain", b"ok")
        if self.path == "/v1/models":
            return self.reply(200, "application/json", json.dumps({"object": "list", "data": [{"id": "local", "object": "model"}]}).encode())
        self.reply(404, "text/plain", b"not found")

    def do_POST(self):
        if self.path != "/v1/chat/completions":
            return self.reply(404, "text/plain", b"not found")
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
