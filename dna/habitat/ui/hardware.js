// The operator face, screen 2: hardware. The hosts the habitat places on,
// what each offers, the host key first seen there and the allocations placed
// on it; add a host, probe a pending one, remove one. A removal is planned
// first (Commands::plan_remove_host): when the plan stops or deletes
// anything on the host, the plan is shown and the host's name is typed back
// to confirm, which Commands::remove_host checks before it executes it.
//
// Every call is the generated client's (head-client.js); nothing else
// speaks HTTP. A refusal is the row's own outcome text, shown as it came.
import {
  commandsHosts,
  commandsAllocations,
  commandsAddHost,
  commandsProbeHost,
  commandsPlanRemoveHost,
  commandsRemoveHost,
} from "./head-client.js";
import { hostLabel } from "./host-label.js";

const endpoint = location.origin + "/api/hale/v1/head/surface";
const $ = (id) => document.getElementById(id);

function parse(list) {
  try { return JSON.parse(list || "[]"); } catch { return []; }
}

function stateOf(host) {
  if (host.state === "ready") return "up";
  if (host.state === "pending-key") return "degraded";
  return "down";
}

// The allocations placed on a host, once per organism: "home: memory,
// nerves, head" rather than each allocation's full key.
function placedLine(placed) {
  const by = new Map();
  for (const x of placed) {
    const [key, role] = x.allocation.split("/");
    const who = x.organism || key;
    by.set(who, [...(by.get(who) || []), x.role || role]);
  }
  return [...by].map(([who, roles]) => who + ": " + roles.join(", ")).join("\n") || "none";
}

function button(label, onclick) {
  const b = document.createElement("button");
  b.type = "button";
  b.textContent = label;
  b.addEventListener("click", onclick);
  return b;
}

function outcome(text) {
  const el = $("host-outcome");
  el.textContent = text;
  el.hidden = !text;
}

// The text of a refused or failed call, as the row or the head said it.
function refusedText(out) {
  if (out.kind === "refusal") return out.refusal.kind + ": " + out.refusal.reason;
  return "";
}

function keyLine(host, line) {
  $("key-host").textContent = host;
  $("key-line").textContent = line;
  $("key-copied").textContent = "";
  $("host-key").hidden = !line;
}

async function act(call, host) {
  const out = await call({ endpoint }, { name: host.name });
  if (out.kind !== "result") { outcome(refusedText(out)); return; }
  const r = out.value;
  outcome(r.ok ? "" : r.code + ": " + r.why);
  if (r.ok && r.authorize) keyLine(host.name, r.authorize);
  await refresh();
}

function itemLine(i) {
  const measure = i.measure && i.measure !== "-" ? i.measure : "";
  const rec = i.recoverable && i.recoverable !== "-" ? "recoverable: " + i.recoverable : "";
  const tail = [measure, rec].filter(Boolean).join("; ");
  return i.what + (tail ? " (" + tail + ")" : "");
}

// The removal's report: each item done, failed or left.
function reportText(r) {
  const report = parse(r.report);
  const lines = report.map((x) => x.state + ": " + x.what + (x.why ? " (" + x.why + ")" : ""));
  return (r.ok ? "" : r.code + ": " + r.why + "\n") + lines.join("\n");
}

async function removeHost(host, confirm, digest) {
  const out = await commandsRemoveHost({ endpoint }, { name: host, confirm, digest });
  if (out.kind !== "result") { outcome(refusedText(out)); return; }
  $("remove-confirm").hidden = true;
  outcome(reportText(out.value));
  await refresh();
}

// A removal planned: with nothing to stop or delete there it goes ahead;
// otherwise the plan is shown and waits for the host's name.
async function planRemove(host) {
  const out = await commandsPlanRemoveHost({ endpoint }, { name: host.name });
  if (out.kind !== "result") { outcome(refusedText(out)); return; }
  const p = out.value;
  if (!p.ok) { outcome(p.code + ": " + p.why); return; }
  if (!p.confirm) { await removeHost(host.name, "", p.digest); return; }
  const list = $("remove-plan");
  list.replaceChildren();
  for (const i of parse(p.plan)) {
    const li = document.createElement("li");
    li.textContent = itemLine(i);
    list.append(li);
  }
  $("remove-host").textContent = host.name;
  $("remove-typed").value = "";
  $("remove-confirm").dataset.host = host.name;
  // the plan shown: the row refuses when what it would do changed since
  $("remove-confirm").dataset.digest = p.digest;
  $("remove-confirm").hidden = false;
  outcome("");
}

export async function refresh() {
  let hosts, allocs;
  try {
    const h = await commandsHosts({ endpoint }, { limit: 200 });
    const a = await commandsAllocations({ endpoint }, { limit: 1000 });
    if (h.kind !== "result" || a.kind !== "result") {
      outcome(refusedText(h.kind !== "result" ? h : a));
      return;
    }
    hosts = parse(h.value.hosts);
    allocs = parse(a.value.allocations);
  } catch (e) {
    outcome("the head did not answer");
    return;
  }
  // one block per host: its name and state, then a line per fact
  const rows = $("host-rows");
  rows.replaceChildren();
  for (const h of hosts) {
    const placed = allocs.filter((x) => x.host === h.name);
    const block = document.createElement("article");
    block.className = "host";
    const head = document.createElement("h3");
    const state = document.createElement("span");
    state.className = "state";
    state.textContent = h.state;
    state.dataset.state = stateOf(h);
    head.append(hostLabel(h.name), " ", state);
    const facts = document.createElement("dl");
    const fact = (k, v, cls) => {
      const dt = document.createElement("dt");
      dt.textContent = k;
      const dd = document.createElement("dd");
      dd.textContent = v;
      if (cls) dd.className = cls;
      facts.append(dt, dd);
    };
    if (h.address || h.user) fact("login", (h.user ? h.user + "@" : "") + (h.address || h.name) + (h.port ? ":" + h.port : ""));
    fact("offers", (h.offers || "nothing yet").split(" ").filter(Boolean).join(", "));
    if (h.os || h.arch) fact("system", [h.os, h.arch].filter(Boolean).join(" "));
    fact("host key", h.fingerprint || "none yet");
    fact("runs", placedLine(placed), "list");
    const actions = document.createElement("p");
    actions.className = "actions";
    if (h.state === "pending-key") {
      actions.append(button("probe", () => act(commandsProbeHost, h)), " ");
    }
    actions.append(button("remove", () => planRemove(h)));
    block.append(head, facts, actions);
    rows.append(block);
  }
  $("hosts-empty").hidden = hosts.length > 0;
}

$("add-form").addEventListener("submit", async (ev) => {
  ev.preventDefault();
  const name = $("add-name").value.trim();
  const out = await commandsAddHost(
    { endpoint },
    { name, user: $("add-user").value.trim(), port: Number($("add-port").value) || 22 },
  );
  if (out.kind !== "result") { outcome(refusedText(out)); return; }
  const r = out.value;
  if (!r.ok) { outcome(r.code + ": " + r.why); return; }
  outcome("");
  // the line that crosses to the target
  keyLine(name, r.authorize);
  $("add-name").value = "";
  await refresh();
});

$("key-copy").addEventListener("click", async () => {
  const line = $("key-line").textContent;
  try {
    await navigator.clipboard.writeText(line);
    $("key-copied").textContent = "copied";
  } catch {
    // no clipboard: select the line so it can be copied by hand
    const range = document.createRange();
    range.selectNodeContents($("key-line"));
    const sel = getSelection();
    sel.removeAllRanges();
    sel.addRange(range);
    $("key-copied").textContent = "selected";
  }
});

$("remove-form").addEventListener("submit", async (ev) => {
  ev.preventDefault();
  // the row checks the name typed back, and refuses a mismatch
  await removeHost(
    $("remove-confirm").dataset.host,
    $("remove-typed").value.trim(),
    $("remove-confirm").dataset.digest,
  );
});

$("remove-cancel").addEventListener("click", () => { $("remove-confirm").hidden = true; });
