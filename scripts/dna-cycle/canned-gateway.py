#!/usr/bin/env python3
# A canned OpenAI-compatible chat endpoint for the cycle's legs: POST /v1/chat/completions answers
# from a script, turn by turn, over real HTTP. No model, no tokens.
#
#   canned-gateway.py <port> <log>
#
# The scenario is chosen by a needle in the request's prompt (its system and user messages); the turn is the
# number of assistant messages already in the request. Each request is logged as one JSON line in
# <log>: the model asked for, the metadata, the Idempotency-Key, the tools offered, the scenario and
# the turn, so the walk can check what crossed the wire.
#
# Scenarios:
#   patch   (the objective names "steward's note"): read api/todo.hl, then edit it to carry the note
#           on its first line, then answer.
#   assess  (an Assessment): answer at once with an assessment, calling no tool.
import json, sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT, LOG = int(sys.argv[1]), sys.argv[2]
NOTE = "// The steward's note: the list's api stays one surface.\n"


def call(cid, name, args):
    return {"id": cid, "type": "function", "function": {"name": name, "arguments": json.dumps(args)}}


def answer(req):
    msgs = req.get("messages") or []
    prompt = "\n".join(m.get("content") or "" for m in msgs if m.get("role") in ("system", "user"))
    turn = sum(1 for m in msgs if m.get("role") == "assistant")
    if "steward's note" in prompt:
        if turn == 0:
            return "patch", turn, {"role": "assistant", "content": None, "tool_calls": [call("c1", "read", {"path": "api/todo.hl"})]}
        if turn == 1:
            source = next((m.get("content", "") for m in reversed(msgs) if m.get("role") == "tool"), "")
            if source.startswith("error:") or not source:
                return "patch", turn, {"role": "assistant", "content": "the file could not be read: " + source[:200]}
            text = source if source.startswith(NOTE) else NOTE + source
            return "patch", turn, {"role": "assistant", "content": None, "tool_calls": [call("c2", "edit", {"path": "api/todo.hl", "text": text})]}
        return "patch", turn, {"role": "assistant", "content": "The note is on the first line of api/todo.hl."}
    return "assess", turn, {"role": "assistant", "content": "assessment: worth doing; the change is one line in the list's own locus"}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_POST(self):
        if not self.path.endswith("/chat/completions"):
            self.send_error(404); return
        body = self.rfile.read(int(self.headers.get("Content-Length", "0") or 0))
        try:
            req = json.loads(body)
        except Exception:
            self.send_error(400); return
        scenario, turn, message = answer(req)
        with open(LOG, "a") as f:
            f.write(json.dumps({
                "model": req.get("model", ""), "metadata": req.get("metadata", {}),
                "idempotency_key": self.headers.get("Idempotency-Key", ""),
                "authorized": self.headers.get("Authorization", "").startswith("Bearer "),
                "tools": [t.get("function", {}).get("name") for t in req.get("tools") or []],
                "scenario": scenario, "turn": turn,
            }) + "\n")
        out = json.dumps({
            "id": "canned-%s-%d" % (scenario, turn), "object": "chat.completion", "model": req.get("model", ""),
            "choices": [{"index": 0, "message": message, "finish_reason": "tool_calls" if message.get("tool_calls") else "stop"}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15},
        }).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(out)))
        self.end_headers()
        self.wfile.write(out)


ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
