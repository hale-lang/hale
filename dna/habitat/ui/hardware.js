// The operator face, screen 2: hardware. The hosts the habitat places on,
// what each offers, the host key first seen there and the allocations placed
// on it; add a host, probe a pending one, remove one.
//
// Every call is the generated client's (head-client.js); nothing else
// speaks HTTP. A refusal is the row's own outcome text, shown as it came.
import {
  commandsHosts,
  commandsAllocations,
  commandsAddHost,
  commandsProbeHost,
  commandsRemoveHost,
} from "./head-client.js";

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

function cell(text, cls) {
  const td = document.createElement("td");
  td.textContent = text;
  if (cls) td.className = cls;
  return td;
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
  const rows = $("host-rows");
  rows.replaceChildren();
  for (const h of hosts) {
    const placed = allocs.filter((x) => x.host === h.name);
    const tr = document.createElement("tr");
    const s = cell(h.state);
    s.dataset.state = stateOf(h);
    const actions = document.createElement("td");
    if (h.state === "pending-key") {
      actions.append(button("probe", () => act(commandsProbeHost, h)), " ");
    }
    actions.append(button("remove", () => act(commandsRemoveHost, h)));
    tr.append(
      cell(h.name),
      s,
      cell(h.offers || "", "wrap"),
      cell(h.fingerprint || "none yet", "wrap"),
      cell(placed.map((x) => x.allocation).join(" ") || "none", "wrap"),
      actions,
    );
    rows.append(tr);
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
