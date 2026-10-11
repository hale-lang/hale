#!/usr/bin/env python3
# A person's agentic session, scripted, over `hale mcp`: the agent a person drives claims the Work
# routed to their position, reads its brief (the hat the head serves, with its hands), works the
# hands in the attempt's own worktree, and hands the change back with `submit --from-worktree`.
#
#   mcp-session.py <project> <position> <needle>
#
# Prints one JSON object: the attempt, each step's state, and the error if a step failed (exit 1).
# The hale binary is $HALE_BIN; the head and the ID token come from HALE_DNA_API / HALE_DNA_ID_TOKEN.
import json, os, subprocess, sys

project, position, needle = sys.argv[1:4]
NOTE = "// The person's note: a change made in a session the person drove.\n"
proc = subprocess.Popen([os.environ.get("HALE_BIN", "hale"), "mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, cwd=project)
ids = iter(range(1, 1000))
report = {"steps": []}


def rpc(method, params):
    i = next(ids)
    proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": i, "method": method, "params": params}) + "\n")
    proc.stdin.flush()
    while True:
        line = proc.stdout.readline()
        if not line:
            raise RuntimeError("hale mcp closed")
        msg = json.loads(line)
        if msg.get("id") == i:
            return msg


def tool(name, args):
    msg = rpc("tools/call", {"name": name, "arguments": args})
    if "error" in msg:
        raise RuntimeError("%s: %s" % (name, msg["error"]))
    text = "".join(c.get("text", "") for c in msg["result"].get("content", []))
    # the result is the verb's transcript: `$ hale dna work …`, its status line, then its JSON object
    out = {"text": text}
    at = text.find("\n{")
    for start in ([0] if text.startswith("{") else []) + ([at + 1] if at >= 0 else []):
        try:
            out = json.loads(text[start:]); break
        except Exception:
            continue
    report["steps"].append({"tool": name, "verb": args.get("verb", args.get("name")), "error": bool(msg["result"].get("isError")), "state": out.get("state", out.get("attempt", {}).get("state", ""))})
    return out, bool(msg["result"].get("isError"))


def fail(why):
    report["error"] = why
    print(json.dumps(report)); proc.kill(); sys.exit(1)


try:
    rpc("initialize", {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "dna-cycle", "version": "1"}})
    proc.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n"); proc.stdin.flush()
    listed = [t["name"] for t in rpc("tools/list", {})["result"]["tools"]]
    if "hale_dna_work" not in listed or "hale_dna_hand" not in listed:
        fail("hale mcp lists no hale_dna_work / hale_dna_hand: %s" % listed)
    claimed, err = tool("hale_dna_work", {"verb": "next", "args": ["--as", position], "project": project})
    attempt = claimed.get("attempt", {})
    if err or attempt.get("state") != "claimed":
        fail("next: %s" % json.dumps(claimed)[:300])
    report["attempt"] = attempt.get("attempt_id")
    brief, err = tool("hale_dna_work", {"verb": "brief", "args": ["--attempt", report["attempt"]], "project": project})
    hat = brief.get("hat", {})
    if err or needle not in json.dumps(hat):
        fail("the brief is not the routed Work's: %s" % json.dumps(brief)[:300])
    report["hands"] = hat.get("tool_grant", "")
    read, err = tool("hale_dna_hand", {"attempt": report["attempt"], "name": "read", "arguments": {"path": "api/todo.hl"}, "project": project})
    if err or read.get("state") != "ok":
        fail("read: %s" % json.dumps(read)[:300])
    source = read.get("output", "")
    text = source if source.startswith(NOTE) else NOTE + source
    wrote, err = tool("hale_dna_hand", {"attempt": report["attempt"], "name": "edit", "arguments": {"path": "api/todo.hl", "text": text}, "project": project})
    if err or wrote.get("state") != "ok":
        fail("edit: %s" % json.dumps(wrote)[:300])
    checked, err = tool("hale_dna_hand", {"attempt": report["attempt"], "name": "check", "arguments": {"path": "api"}, "project": project})
    if err or checked.get("state") != "ok":
        fail("check: %s" % json.dumps(checked)[:300])
    handed, err = tool("hale_dna_work", {"verb": "submit", "args": ["--as", position, "--attempt", report["attempt"], "--token", str(attempt.get("token")), "--from-worktree", "--result", "the person's note, from a session"], "project": project})
    if err or handed.get("attempt", {}).get("state") != "requested":
        fail("submit: %s" % json.dumps(handed)[:300])
    report["submitted"] = True
except Exception as e:
    fail(str(e))
print(json.dumps(report)); proc.stdin.close(); proc.wait(timeout=10)
