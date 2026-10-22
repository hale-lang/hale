// The operator face, screen 3: infrastructure. What the habitat has
// allocated (role, instance, provider, host, and the state its provider
// observes), the policy that says which provider a role is allocated by,
// a host picker per allocation (place_allocation) and apply
// (apply_allocations), which answers the pulse: what each provider
// observes after it. Moving a memory that holds a record is refused until
// the person confirms starting it empty (a re-apply with fresh).
import {
  commandsAllocations,
  commandsPolicy,
  commandsHosts,
  commandsPlaceAllocation,
  commandsApplyAllocations,
} from "./head-client.js";

const endpoint = location.origin + "/api/hale/v1/head/surface";
const $ = (id) => document.getElementById(id);

function parse(list) {
  try { return JSON.parse(list || "[]"); } catch { return []; }
}

function cell(text) {
  const td = document.createElement("td");
  td.textContent = text;
  return td;
}

function stateCell(a) {
  const td = cell(a.state || "not observed");
  if (a.detail) td.title = a.detail;
  if (a.state === "up" || a.state === "degraded" || a.state === "down") td.dataset.state = a.state;
  return td;
}

function outcome(text) {
  const el = $("allocation-outcome");
  el.textContent = text;
  el.hidden = !text;
}

function refusedText(out) {
  if (out.kind === "refusal") return out.refusal.kind + ": " + out.refusal.reason;
  return "";
}

// The hosts an allocation can be placed on: this machine and the ready ones.
function placeable(hosts) {
  return ["local", ...hosts.filter((h) => h.state === "ready").map((h) => h.name)];
}

function picker(a, names) {
  const sel = document.createElement("select");
  for (const n of names) {
    const o = document.createElement("option");
    o.value = n;
    o.textContent = n;
    sel.append(o);
  }
  sel.value = names.includes(a.host) ? a.host : (a.host ? "" : "local");
  sel.addEventListener("change", async () => {
    const out = await commandsPlaceAllocation({ endpoint }, { allocation: a.allocation, host: sel.value });
    if (out.kind !== "result") outcome(refusedText(out));
    else outcome(out.value.ok ? "" : out.value.code + ": " + out.value.why);
    await refresh();
  });
  return sel;
}

function showPulse(reply) {
  const rows = $("pulse-rows");
  rows.replaceChildren();
  for (const p of parse(reply.applied)) {
    const tr = document.createElement("tr");
    const s = cell(p.state || "not observed");
    if (p.state === "up" || p.state === "degraded" || p.state === "down") s.dataset.state = p.state;
    tr.append(cell(p.allocation), cell(p.host), s, cell(p.ok ? p.detail || "applied" : p.why));
    rows.append(tr);
  }
}

async function apply(fresh) {
  $("apply-confirm").hidden = true;
  const out = await commandsApplyAllocations({ endpoint }, { host: "", fresh });
  if (out.kind !== "result") { outcome(refusedText(out)); return; }
  const r = out.value;
  showPulse(r);
  const applied = parse(r.applied);
  const needsFresh = !fresh && applied.find((p) => !p.ok && (p.why || "").includes("--fresh"));
  if (needsFresh) {
    // the refusal's text, as the row said it, and the way through it
    $("apply-confirm-why").textContent = needsFresh.allocation + ": " + needsFresh.why;
    $("apply-confirm").hidden = false;
  }
  outcome(r.ok || needsFresh ? "" : r.code + ": " + r.why);
  await refresh();
}

export async function refresh() {
  let allocs, policy, hosts;
  try {
    const a = await commandsAllocations({ endpoint }, { limit: 1000 });
    const p = await commandsPolicy({ endpoint }, { limit: 200 });
    const h = await commandsHosts({ endpoint }, { limit: 200 });
    const bad = [a, p, h].find((o) => o.kind !== "result");
    if (bad) {
      outcome(refusedText(bad) || "the head did not answer");
      return;
    }
    allocs = parse(a.value.allocations);
    policy = parse(p.value.policy);
    hosts = parse(h.value.hosts);
  } catch (e) {
    outcome("the head did not answer");
    return;
  }
  const names = placeable(hosts);
  const rows = $("allocation-rows");
  rows.replaceChildren();
  for (const a of allocs) {
    const tr = document.createElement("tr");
    const where = document.createElement("td");
    where.append(picker(a, names));
    tr.append(cell(a.role), cell(a.instance), cell(a.provider), cell(a.host || "none"), stateCell(a), where);
    rows.append(tr);
  }
  $("allocations-empty").hidden = allocs.length > 0;
  const prows = $("policy-rows");
  prows.replaceChildren();
  for (const r of policy) {
    const tr = document.createElement("tr");
    tr.append(cell(r.role), cell(r.provider), cell(r.profile));
    prows.append(tr);
  }
}

$("apply-all").addEventListener("click", () => apply(false));
$("apply-fresh").addEventListener("click", () => apply(true));
$("apply-cancel").addEventListener("click", () => { $("apply-confirm").hidden = true; });
